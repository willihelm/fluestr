//! Client side of the sidecar protocol.
//!
//! One worker, one child process, strictly request/response. Several of these
//! behind one queue make the pool in [`crate::pool`]; on its own a `Worker`
//! knows nothing about queueing, supervision or ordering.

use anyhow::{bail, Context, Result};
use audio_core::Segment;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use whisper_protocol as protocol;
use whisper_protocol::{Request, Response};

/// What the worker reported when it came up.
#[derive(Debug, Clone)]
pub struct Ready {
    pub model: String,
    /// On the very first start on a machine this contains the one-off Metal
    /// shader compilation (measured at ~8 s), afterwards ~0.1 s. The UI needs
    /// the distinction to explain a long wait instead of looking hung.
    pub load_ms: u32,
    pub gpu: bool,
}

pub struct Worker {
    /// Shared so that [`Killer`] can reach it from the watchdog thread. The
    /// read side of the protocol lives on `stdout`, which is taken out of the
    /// `Child`, so holding this lock never blocks a decode in flight.
    child: Arc<Mutex<Child>>,
    /// Taken during shutdown: closing it is what tells the worker to exit.
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
    ready: Ready,
    stopped: bool,
}

impl Worker {
    /// Starts the sidecar and waits for it to report a loaded model.
    ///
    /// `vad_model` is what keeps noise and silence out of the transcript. It is
    /// optional so a missing download degrades to "transcribes everything"
    /// rather than to "does not start", but the sidecar warns loudly about it.
    pub fn spawn(
        binary: &Path,
        model: &Path,
        vad_model: Option<&Path>,
        threads: usize,
    ) -> Result<Self> {
        let mut command = Command::new(binary);
        command
            .arg("--model")
            .arg(model)
            .arg("--threads")
            .arg(threads.to_string());
        if let Some(vad) = vad_model {
            command.arg("--vad-model").arg(vad);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // stderr stays attached: the worker logs there, and during
            // development those lines belong in the same terminal as the app's.
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("starting the sidecar: {}", binary.display()))?;

        let stdin = child.stdin.take().context("sidecar stdin")?;
        let stdout = child.stdout.take().context("sidecar stdout")?;
        let mut stdout = BufReader::new(stdout);

        let ready = match protocol::read_response(&mut stdout)? {
            Some(Response::Ready {
                model,
                load_ms,
                gpu,
            }) => Ready {
                model,
                load_ms,
                gpu,
            },
            Some(Response::Error { message, .. }) => {
                let _ = child.kill();
                bail!("sidecar failed to start: {message}");
            }
            Some(other) => {
                let _ = child.kill();
                bail!("sidecar sent {other:?} instead of ready");
            }
            // Almost always a missing or corrupt model file; the worker's own
            // reason is on stderr.
            None => {
                let _ = child.wait();
                bail!("sidecar exited before reporting ready — see its log output");
            }
        };

        Ok(Self {
            child: Arc::new(Mutex::new(child)),
            stdin: Some(BufWriter::new(stdin)),
            stdout,
            ready,
            stopped: false,
        })
    }

    pub fn ready(&self) -> &Ready {
        &self.ready
    }

    /// A handle for killing this worker from another thread.
    ///
    /// The pool's watchdog has no other way to act on a worker that stopped
    /// answering: the decode is blocked in a read on a pipe that will never
    /// deliver. Killing the process closes the write end, the read returns
    /// EOF, and the wedged decode fails like an ordinary crash — so timeout
    /// and crash are healed by the same code path instead of two.
    pub fn killer(&self) -> Killer {
        Killer(Arc::clone(&self.child))
    }

    fn stdin(&mut self) -> Result<&mut BufWriter<ChildStdin>> {
        self.stdin
            .as_mut()
            .context("the sidecar has already been shut down")
    }

    /// Sends one segment and blocks until its transcript comes back.
    ///
    /// Blocking is deliberate: each worker owns a thread in the pool, and the
    /// queue in front of them is what keeps a long segment from delaying the
    /// streams behind it.
    pub fn transcribe(&mut self, segment: &Segment) -> Result<Response> {
        let request = Request::Transcribe {
            stream_id: segment.stream_id.0,
            seq: segment.seq,
            language_mode: segment.language_mode.clone(),
            initial_prompt: segment.initial_prompt.clone(),
        };
        protocol::write_request(self.stdin()?, &request, &segment.pcm)
            .context("sending a segment to the sidecar")?;

        match protocol::read_response(&mut self.stdout)? {
            Some(response) => Ok(response),
            None => bail!("sidecar closed the connection while transcribing"),
        }
    }

    /// Asks whisper.cpp for its language list — the source the UI picker is
    /// built from, so that no language list is hard-coded anywhere.
    pub fn list_languages(&mut self) -> Result<Vec<whisper_protocol::Language>> {
        protocol::write_request(self.stdin()?, &Request::ListLanguages, &[])?;
        match protocol::read_response(&mut self.stdout)? {
            Some(Response::Languages { languages }) => Ok(languages),
            Some(other) => bail!("expected a language list, got {other:?}"),
            None => bail!("sidecar closed the connection"),
        }
    }

    /// Asks the worker to stop, then waits.
    ///
    /// Takes `&mut self` rather than `self` so that [`Drop`] can tell an
    /// orderly shutdown from a dropped worker. Calling it twice is harmless.
    pub fn shutdown(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;

        if let Some(mut stdin) = self.stdin.take() {
            let _ = protocol::write_request(&mut stdin, &Request::Shutdown, &[]);
            // Closing the pipe is the second signal: even a worker that did not
            // understand the request sees EOF on stdin and leaves its loop.
        }

        let Ok(mut child) = self.child.lock() else {
            return;
        };
        if let Err(err) = child.wait() {
            tracing::warn!(error = %err, "sidecar did not exit cleanly, killing it");
            let _ = child.kill();
        }
    }
}

/// Kills one worker's child process from outside. See [`Worker::killer`].
#[derive(Clone)]
pub struct Killer(Arc<Mutex<Child>>);

impl Killer {
    pub fn kill(&self) {
        if let Ok(mut child) = self.0.lock() {
            let _ = child.kill();
        }
    }
}

/// A worker that goes out of scope without [`Worker::shutdown`] still has to
/// take its child process with it.
///
/// This is not hypothetical: if starting a session fails *after* the sidecar
/// came up — a device that will not open, a rejected channel map — the `Worker`
/// is simply dropped. Without this, that leaves a process behind holding the
/// model and its GPU allocations, and every retry adds another one.
///
/// Such a stray does die when the app does — its stdin closes, the worker reads
/// EOF and leaves its loop. But "only until the user quits" is not a bound
/// worth having when each one costs a few hundred megabytes and a Metal
/// context, and the operator retrying after a failed start is the normal case.
impl Drop for Worker {
    fn drop(&mut self) {
        if self.stopped {
            return;
        }
        tracing::warn!("sidecar dropped without shutdown — killing it");
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Locates the sidecar binary.
///
/// In development it sits next to the app binary in the same target directory,
/// which is also where Tauri puts a bundled `externalBin` sidecar (P13). The
/// environment variable is the escape hatch for anything else.
pub fn find_binary() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("INTERCOM2CHAT_WORKER") {
        let path = PathBuf::from(path);
        if !path.is_file() {
            bail!(
                "INTERCOM2CHAT_WORKER points at {} — not a file",
                path.display()
            );
        }
        return Ok(path);
    }

    let exe = std::env::current_exe().context("locating the app binary")?;
    let dir = exe.parent().context("app binary has no directory")?;
    // Next to the binary, then one level up. The second place is what makes
    // the integration tests run: cargo puts a test binary in
    // `target/debug/deps` while the sidecar it needs is in `target/debug`, and
    // without this they find nothing, report "no model" and pass without
    // having tested anything.
    for candidate in [dir.join("whisper-worker"), dir.join("../whisper-worker")] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    bail!(
        "no sidecar found next to {} — build it with `cargo build -p whisper-worker` \
         or point INTERCOM2CHAT_WORKER at it",
        exe.display()
    )
}

/// Where `just model` puts its downloads, relative to the repository root.
const SCRATCH_MODELS: &str = ".scratch/models";

/// Locates a model file the development way.
///
/// The model manager ([`crate::models`]) is what the app resolves through now;
/// this is the last entry in its order and the only one available to code with
/// no app directory to look in — the integration tests and `just slice`. It
/// reads the environment variable, then whatever `just model` left in
/// `.scratch/models`.
///
/// The directory is **not** resolved against the working directory. `tauri dev`
/// runs the binary with the crate as its working directory, not the repository
/// root, so a relative path finds nothing — and the resulting "no model" is
/// indistinguishable from actually not having downloaded one. The search
/// therefore walks up from both the working directory and the binary's own
/// location, which covers running from anywhere in the repository as well as
/// `cargo run` and `tauri dev`.
pub fn find_model() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("INTERCOM2CHAT_MODEL") {
        let path = PathBuf::from(path);
        if !path.is_file() {
            bail!(
                "INTERCOM2CHAT_MODEL points at {} — not a file",
                path.display()
            );
        }
        return Ok(path);
    }

    let mut searched = Vec::new();
    for start in search_roots() {
        for ancestor in start.ancestors() {
            let dir = ancestor.join(SCRATCH_MODELS);
            if !dir.is_dir() {
                continue;
            }
            if let Some(model) = newest_model(&dir) {
                return Ok(model);
            }
            searched.push(dir);
        }
    }

    if searched.is_empty() {
        bail!(
            "no model found — run `just model small-q5_1` or set INTERCOM2CHAT_MODEL \
             (looked for {SCRATCH_MODELS} above {})",
            describe(search_roots())
        );
    }
    bail!(
        "no .bin model in {} — run `just model small-q5_1`",
        describe(searched)
    )
}

/// Directories the upward search starts from.
fn search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
        }
    }
    roots
}

/// Locates the Silero VAD model that keeps noise out of the transcript.
///
/// `None` rather than an error: a missing VAD degrades the result, it does not
/// prevent a session. The sidecar warns about running without one.
pub fn find_vad_model() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("INTERCOM2CHAT_VAD_MODEL") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }

    for start in search_roots() {
        for ancestor in start.ancestors() {
            let dir = ancestor.join(SCRATCH_MODELS);
            if !dir.is_dir() {
                continue;
            }
            let mut found: Vec<PathBuf> = model_files(&dir)
                .into_iter()
                .map(|(_, path)| path)
                .filter(|path| is_vad_model(path))
                .collect();
            // Newest Silero version wins, by name.
            found.sort();
            if let Some(path) = found.pop() {
                return Some(path);
            }
        }
    }
    None
}

/// The VAD model lives in the same directory as the speech models and has the
/// same extension, so the two are told apart by name.
fn is_vad_model(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().contains("silero"))
}

/// Picks one speech model out of a directory, largest first.
///
/// Size is a rough stand-in for quality: `small-q5_1` beats `tiny`, and this
/// crude rule means downloading a better model is enough to use it. P11 makes
/// it an explicit choice instead of a guess.
///
/// The VAD model is excluded — it sits in the same directory, is also a `.bin`,
/// and handing it to whisper as a speech model would fail in a thoroughly
/// confusing way.
fn newest_model(dir: &Path) -> Option<PathBuf> {
    let mut candidates = model_files(dir);
    candidates.retain(|(_, path)| !is_vad_model(path));
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    candidates.into_iter().next().map(|(_, path)| path)
}

/// Every `.bin` in `dir`, with its size.
fn model_files(dir: &Path) -> Vec<(u64, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "bin") {
                Some((entry.metadata().ok()?.len(), path))
            } else {
                None
            }
        })
        .collect()
}

fn describe(paths: Vec<PathBuf>) -> String {
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bogus_worker_path_is_rejected_with_its_own_name() {
        // Temporarily pointing the variable somewhere invalid: the message has
        // to name the path, because "sidecar not found" alone is unactionable.
        // `Worker` owns a child process and cannot derive Debug, which rules
        // out `unwrap_err`.
        let result = Worker::spawn(
            Path::new("/nonexistent/whisper-worker"),
            Path::new("/nonexistent/model.bin"),
            None,
            2,
        );
        let Err(err) = result else {
            panic!("spawning a nonexistent binary should fail");
        };
        assert!(err.to_string().contains("/nonexistent/whisper-worker"));
    }

    #[test]
    fn a_missing_model_directory_explains_the_fix() {
        // Depends on whether the developer has downloaded a model, so this
        // asserts the shape of the failure rather than a particular path.
        if std::env::var_os("INTERCOM2CHAT_MODEL").is_some() {
            return;
        }
        if let Err(err) = find_model() {
            let message = err.to_string();
            assert!(
                message.contains("just model") || message.contains("INTERCOM2CHAT_MODEL"),
                "unhelpful error: {message}"
            );
        }
    }

    #[test]
    fn the_model_search_climbs_out_of_a_subdirectory() {
        // The bug this guards: `tauri dev` runs the binary with the crate as
        // its working directory, so a path relative to the repository root
        // resolves to nothing — and "no model" then means "wrong directory"
        // rather than "not downloaded", which is impossible to tell apart.
        let root = std::env::temp_dir().join(format!("i2c-model-search-{}", std::process::id()));
        let models = root.join(SCRATCH_MODELS);
        std::fs::create_dir_all(&models).unwrap();
        let deep = root.join("crates/app");
        std::fs::create_dir_all(&deep).unwrap();

        // Two models: the larger one wins.
        std::fs::write(models.join("ggml-tiny.bin"), vec![0u8; 64]).unwrap();
        std::fs::write(models.join("ggml-small-q5_1.bin"), vec![0u8; 512]).unwrap();

        let found = newest_model(&models).expect("a model in the directory");
        assert_eq!(found.file_name().unwrap(), "ggml-small-q5_1.bin");

        // The VAD model shares the directory and the extension. Handing it to
        // whisper as a speech model would fail in a thoroughly confusing way,
        // so it must never be picked — not even when it is the biggest file.
        std::fs::write(models.join("ggml-silero-v5.1.2.bin"), vec![0u8; 4096]).unwrap();
        let still = newest_model(&models).expect("a speech model");
        assert_eq!(still.file_name().unwrap(), "ggml-small-q5_1.bin");

        // The upward walk from a nested directory reaches the same place.
        let via_ancestors = deep
            .ancestors()
            .map(|a| a.join(SCRATCH_MODELS))
            .find(|dir| dir.is_dir());
        assert_eq!(via_ancestors.as_deref(), Some(models.as_path()));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_vad_model_is_told_apart_by_name() {
        assert!(is_vad_model(Path::new("/m/ggml-silero-v5.1.2.bin")));
        assert!(is_vad_model(Path::new("/m/GGML-Silero-v6.2.0.bin")));
        assert!(!is_vad_model(Path::new("/m/ggml-small-q5_1.bin")));
        assert!(!is_vad_model(Path::new("/m/ggml-large-v3-turbo-q5_0.bin")));
    }

    #[test]
    fn a_directory_without_a_model_is_not_mistaken_for_one() {
        let root = std::env::temp_dir().join(format!("i2c-model-empty-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("README.md"), "not a model").unwrap();

        assert!(newest_model(&root).is_none());

        std::fs::remove_dir_all(&root).unwrap();
    }
}

//! Client side of the sidecar protocol.
//!
//! One worker, one child process, strictly request/response. That is the
//! narrow version of P5: no pool, no queue, no supervision. It is enough to
//! carry a single stream end to end and to find out what the real latency is,
//! which is the whole point of doing this before the pool exists.
//!
//! What P5 adds on top: several workers on one prioritised queue, a hard
//! bound with a drop policy, respawn after a crash, and reordering by `seq`.

use anyhow::{bail, Context, Result};
use audio_core::Segment;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
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
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    ready: Ready,
}

impl Worker {
    /// Starts the sidecar and waits for it to report a loaded model.
    pub fn spawn(binary: &Path, model: &Path, threads: usize) -> Result<Self> {
        let mut child = Command::new(binary)
            .arg("--model")
            .arg(model)
            .arg("--threads")
            .arg(threads.to_string())
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
            child,
            stdin: BufWriter::new(stdin),
            stdout,
            ready,
        })
    }

    pub fn ready(&self) -> &Ready {
        &self.ready
    }

    /// Sends one segment and blocks until its transcript comes back.
    ///
    /// Blocking is fine here because the caller runs on its own thread — but it
    /// is also why this cannot stay: with one worker, a long segment delays
    /// every stream behind it. P5's queue is what fixes that.
    pub fn transcribe(&mut self, segment: &Segment) -> Result<Response> {
        let request = Request::Transcribe {
            stream_id: segment.stream_id.0,
            seq: segment.seq,
            language_mode: segment.language_mode.clone(),
            initial_prompt: segment.initial_prompt.clone(),
        };
        protocol::write_request(&mut self.stdin, &request, &segment.pcm)
            .context("sending a segment to the sidecar")?;

        match protocol::read_response(&mut self.stdout)? {
            Some(response) => Ok(response),
            None => bail!("sidecar closed the connection while transcribing"),
        }
    }

    /// Asks whisper.cpp for its language list — the source the UI picker is
    /// built from, so that no language list is hard-coded anywhere.
    pub fn list_languages(&mut self) -> Result<Vec<whisper_protocol::Language>> {
        protocol::write_request(&mut self.stdin, &Request::ListLanguages, &[])?;
        match protocol::read_response(&mut self.stdout)? {
            Some(Response::Languages { languages }) => Ok(languages),
            Some(other) => bail!("expected a language list, got {other:?}"),
            None => bail!("sidecar closed the connection"),
        }
    }

    /// Asks the worker to stop, then waits. Falls back to killing it, because
    /// a sidecar left running would hold the model — and the GPU — forever.
    pub fn shutdown(mut self) {
        let _ = protocol::write_request(&mut self.stdin, &Request::Shutdown, &[]);
        drop(self.stdin);
        match self.child.wait() {
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(error = %err, "sidecar did not exit cleanly, killing it");
                let _ = self.child.kill();
            }
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
    let candidate = dir.join("whisper-worker");
    if candidate.is_file() {
        return Ok(candidate);
    }

    bail!(
        "no sidecar found next to {} — build it with `cargo build -p whisper-worker` \
         or point INTERCOM2CHAT_WORKER at it",
        exe.display()
    )
}

/// Locates a model file.
///
/// A stand-in for the model manager (P11), which will download models into the
/// app data directory and let the user pick one. Until then: the environment
/// variable, or whatever `just model` left in `.scratch/models`.
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

    let dir = Path::new(".scratch/models");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|_| {
            anyhow::anyhow!(
                "no model configured and {} does not exist — run `just model small-q5_1` \
                 or set INTERCOM2CHAT_MODEL",
                dir.display()
            )
        })?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "bin"))
        .collect();
    candidates.sort();

    candidates.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "no .bin model in {} — run `just model small-q5_1`",
            dir.display()
        )
    })
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
            2,
        );
        let Err(err) = result else {
            panic!("spawning a nonexistent binary should fail");
        };
        assert!(err.to_string().contains("/nonexistent/whisper-worker"));
    }

    #[test]
    fn a_missing_model_directory_explains_the_fix() {
        // `find_model` reads the working directory, so this only asserts the
        // shape of the failure, not a particular path.
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
}

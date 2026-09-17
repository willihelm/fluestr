//! The model manager (P11).
//!
//! Until now a model was whatever `just model` had left in `.scratch/models`,
//! which is fine for the machine this was written on and useless everywhere
//! else: a fresh install finds nothing, says "no model", and the only way out
//! is a terminal and a URL. This module is what replaces that — a curated list
//! of models the app knows how to fetch, a download that survives being
//! interrupted, and a checksum that decides whether what arrived is what was
//! asked for.
//!
//! Three things are worth knowing before reading on.
//!
//! **The checksum is not a formality.** These files are 60–600 MB fetched over
//! a network the app does not control, and a truncated or tampered `.bin` does
//! not announce itself: whisper.cpp either refuses to load it with a message
//! about magic numbers, or — worse — loads it and transcribes noise. Verifying
//! before the file is put in place is what turns that into one clear sentence.
//! The published SHA256 of each file is baked in below; a mismatch deletes the
//! download rather than keeping it.
//!
//! **Downloads resume.** 600 MB on a venue's network is not a reliable
//! proposition, and restarting from zero after each failure is how somebody
//! gives up. The partial file is kept as `<name>.bin.part` and continued with a
//! range request; only a complete, verified file is renamed into place, so the
//! directory never holds a `.bin` that is not a whole model.
//!
//! **Nothing here is required.** The catalogue is a convenience, not a gate: an
//! operator on a machine that will never see the internet can point the app at
//! a `.bin` they carried in on a stick ([`Selection::path`]), and the
//! development search in [`crate::worker`] still works. See [`resolve`] for the
//! order those are tried in.

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// What a model is for.
///
/// The two are not interchangeable and live in the same directory with the
/// same extension, so the distinction is carried explicitly rather than
/// guessed from the file name the way [`crate::worker`] has to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelKind {
    /// Transcribes. Handed to whisper.cpp as `--model`.
    Speech,
    /// Decides whether a stretch of audio is speech at all. Without it whisper
    /// transcribes noise and silence, and invents sentences for both.
    Vad,
}

/// One model the app knows how to fetch.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    /// Stable identifier, also the file name: `ggml-{id}.bin`. This is what a
    /// profile stores, so it must not change once shipped.
    pub id: &'static str,
    pub kind: ModelKind,
    /// Shown in the picker.
    pub label: &'static str,
    /// One line on when to choose this one. The operator picking a model is an
    /// audio engineer, not somebody who knows what a quantised tensor is.
    pub note: &'static str,
    pub url: &'static str,
    /// SHA256 of the finished file, as the host publishes it.
    pub sha256: &'static str,
    /// Expected size. Used for the progress bar before the first byte arrives
    /// and as the cheap check that runs before the expensive one.
    pub bytes: u64,
}

/// The models the app offers to download.
///
/// Deliberately short. A picker with thirty entries is a research project, and
/// the question an engineer actually has is "faster or better?" — so there is
/// one recommendation, one step down and one step up.
///
/// Sizes and checksums are the ones published for these files; see
/// [`Entry::sha256`] for what they are for.
pub const CATALOGUE: &[Entry] = &[
    Entry {
        id: "base-q5_1",
        kind: ModelKind::Speech,
        label: "Base (quantised)",
        note: "Fastest. For weak machines, or when latency matters more than the odd wrong word.",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin",
        sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        bytes: 59_707_625,
    },
    Entry {
        id: "small-q5_1",
        kind: ModelKind::Speech,
        label: "Small (quantised)",
        note: "Recommended. The measured ~330 ms end to end is this model; it handles DE/EN code switching.",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin",
        sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        bytes: 190_085_487,
    },
    Entry {
        id: "large-v3-turbo-q5_0",
        kind: ModelKind::Speech,
        label: "Large v3 turbo (quantised)",
        note: "Most accurate, and slower. Worth measuring against your own audio before a show, not during one.",
        url:
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        bytes: 574_041_195,
    },
    Entry {
        id: "silero-v5.1.2",
        kind: ModelKind::Vad,
        label: "Silero VAD v5.1.2",
        note: "Speech detection. Small, and the app is much worse without it — install it.",
        url: "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin",
        sha256: "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
        bytes: 885_098,
    },
];

/// What a machine with nothing configured should end up with.
///
/// "Latency first" is the project's stated priority, and §8 of measurements.md
/// is this model — recommending anything else would mean every number in that
/// file describes a setup nobody runs.
pub const DEFAULT_SPEECH: &str = "small-q5_1";
pub const DEFAULT_VAD: &str = "silero-v5.1.2";

/// The models directory, inside the app's data directory.
pub const DIR_NAME: &str = "models";

pub fn entry(id: &str) -> Option<&'static Entry> {
    CATALOGUE.iter().find(|e| e.id == id)
}

/// The file a catalogue entry installs as.
///
/// The same `ggml-<id>.bin` convention `just model` uses, so a model downloaded
/// by hand into the app's directory is simply found, and the two ways of
/// getting one do not produce two naming schemes.
pub fn file_name(id: &str) -> String {
    format!("ggml-{id}.bin")
}

pub fn installed_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(file_name(id))
}

/// Where an interrupted download waits to be continued.
fn part_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{}.part", file_name(id)))
}

/// Which models the operator chose. The slice of [`crate::config::Config`]
/// this module cares about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// Catalogue id of the speech model.
    pub speech: Option<String>,
    /// Catalogue id of the VAD model.
    pub vad: Option<String>,
    /// A `.bin` from outside the catalogue, for a machine that cannot reach
    /// the internet at all. Wins over `speech` when set — somebody who typed a
    /// path meant it.
    pub path: Option<String>,
}

/// The models a session will actually run with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub speech: PathBuf,
    /// `None` degrades the result, it does not prevent a session — see
    /// [`crate::worker::Worker::spawn`].
    pub vad: Option<PathBuf>,
    /// Where the speech model came from, for the log and for the UI. Without
    /// it "no model" and "a model you did not mean" look identical.
    pub source: Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// Downloaded by the model manager into the app's own directory.
    Managed,
    /// A path the operator typed. The offline fallback.
    Manual,
    /// `INTERCOM2CHAT_MODEL`. Developer escape hatch, and it outranks
    /// everything so that a test run is never at the mercy of whatever the
    /// developer last clicked in the UI.
    Environment,
    /// Left by `just model` in `.scratch/models`. Development only.
    Development,
}

/// Works out which model files a session should use.
///
/// The order is the point, so it is spelled out rather than left to the reader
/// of the `if` chain:
///
/// 1. `INTERCOM2CHAT_MODEL` — a test that sets it must get it, whatever is
///    configured.
/// 2. The manual path, if the operator set one. Typing a path is an explicit
///    act and outranks a picker they may have left on a default.
/// 3. The selected catalogue model, if it is installed.
/// 4. The default catalogue model, if it happens to be installed — a machine
///    that downloaded one and never opened the picker should still run.
/// 5. Whatever `just model` left in `.scratch/models`, so the development
///    workflow this replaces keeps working.
///
/// Falling off the end is the "no model" state, and it is an error with an
/// actionable sentence rather than a silent malfunction: the previous
/// behaviour, a session that starts and transcribes nothing, is the thing this
/// package exists to remove.
pub fn resolve(dir: &Path, selection: &Selection) -> Result<Resolved> {
    let vad = resolve_vad(dir, selection);

    if let Ok(path) = std::env::var("INTERCOM2CHAT_MODEL") {
        let path = PathBuf::from(path);
        if !path.is_file() {
            bail!(
                "INTERCOM2CHAT_MODEL points at {} — not a file",
                path.display()
            );
        }
        return Ok(Resolved {
            speech: path,
            vad,
            source: Source::Environment,
        });
    }

    if let Some(path) = selection.path.as_deref().filter(|p| !p.trim().is_empty()) {
        let path = PathBuf::from(path);
        if !path.is_file() {
            bail!(
                "the model file set in Settings is not there: {}",
                path.display()
            );
        }
        return Ok(Resolved {
            speech: path,
            vad,
            source: Source::Manual,
        });
    }

    for id in [selection.speech.as_deref(), Some(DEFAULT_SPEECH)]
        .into_iter()
        .flatten()
    {
        let path = installed_path(dir, id);
        if path.is_file() {
            return Ok(Resolved {
                speech: path,
                vad,
                source: Source::Managed,
            });
        }
    }

    // A selected model that is not installed is worth its own sentence: it is
    // the state a half-finished download leaves behind, and "no model at all"
    // would send the operator looking in the wrong place.
    if let Some(id) = selection.speech.as_deref() {
        if entry(id).is_some() {
            bail!("the selected model '{id}' is not downloaded yet — install it in Settings");
        }
    }

    if let Ok(path) = crate::worker::find_model() {
        return Ok(Resolved {
            speech: path,
            vad,
            source: Source::Development,
        });
    }

    bail!("no speech model installed — download one on the Settings page")
}

/// The VAD half, which never fails: a missing one costs quality, not the
/// session, and the sidecar says so loudly enough on its own.
fn resolve_vad(dir: &Path, selection: &Selection) -> Option<PathBuf> {
    if let Ok(path) = std::env::var("INTERCOM2CHAT_VAD_MODEL") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    for id in [selection.vad.as_deref(), Some(DEFAULT_VAD)]
        .into_iter()
        .flatten()
    {
        let path = installed_path(dir, id);
        if path.is_file() {
            return Some(path);
        }
    }

    crate::worker::find_vad_model()
}

/// How far a download has got. One of these goes to the UI per chunk, throttled
/// by the caller.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: String,
    /// Bytes on disk, resumed ones included — so a continued download picks up
    /// the bar where it left it rather than restarting at zero.
    pub received: u64,
    pub total: u64,
    pub stage: Stage,
    /// Set when `stage` is `failed`.
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Downloading,
    /// Hashing 600 MB is not instant, and a progress bar that sits at 100 %
    /// while it happens looks like a hang.
    Verifying,
    Done,
    /// The partial file is kept; pressing download again continues it.
    Cancelled,
    Failed,
}

/// Downloads one catalogue entry into `dir`, resuming if a part file is there.
///
/// `report` is called as bytes arrive and again at each stage change. `cancel`
/// is checked between chunks: cancelling leaves the part file alone, because
/// the whole reason it exists is that this may take several attempts.
pub async fn download(
    entry: &'static Entry,
    dir: &Path,
    cancel: Arc<AtomicBool>,
    report: impl Fn(Progress),
) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating the model directory {}", dir.display()))?;

    let target = installed_path(dir, entry.id);
    let part = part_path(dir, entry.id);

    // Already there and already correct: nothing to do. Checked rather than
    // assumed, because "installed" is what the picker shows and a file that
    // was truncated after the fact should not keep claiming to be a model.
    if target.is_file() && verify(&target, entry.sha256)? {
        report(Progress {
            id: entry.id.into(),
            received: entry.bytes,
            total: entry.bytes,
            stage: Stage::Done,
            message: None,
        });
        return Ok(target);
    }

    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    // A part file larger than the finished model is not a resume point, it is
    // debris from an interrupted write or a changed URL. Starting over is the
    // only thing a range request cannot recover from.
    if have >= entry.bytes {
        let _ = std::fs::remove_file(&part);
        have = 0;
    }

    let client = reqwest::Client::builder()
        .user_agent(concat!("intercom2chat/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("building the HTTP client")?;

    let mut request = client.get(entry.url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }

    let response = request
        .send()
        .await
        .with_context(|| format!("requesting {}", entry.url))?;

    if !response.status().is_success() {
        bail!(
            "the download server answered {} for {}",
            response.status(),
            entry.url
        );
    }

    // Asked to continue and got the whole file instead: the server does not do
    // ranges, so the part file is worthless and the write starts from the
    // front. Appending here would splice two copies together, and the checksum
    // would be the only thing that noticed — after the full download.
    let resumed = have > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if have > 0 && !resumed {
        tracing::info!(
            id = entry.id,
            "the server ignored the range request, starting over"
        );
        have = 0;
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .with_context(|| format!("opening {}", part.display()))?;

    let mut received = have;
    let mut stream = response.bytes_stream();
    // The bar moves in visible steps rather than on every chunk: eight bridge
    // events per second is plenty for a human, and a 600 MB file arrives in
    // tens of thousands of chunks.
    let mut last_reported = received;
    let step = (entry.bytes / 200).max(256 * 1024);

    report(Progress {
        id: entry.id.into(),
        received,
        total: entry.bytes,
        stage: Stage::Downloading,
        message: None,
    });

    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            // Deliberately without removing the part file.
            report(Progress {
                id: entry.id.into(),
                received,
                total: entry.bytes,
                stage: Stage::Cancelled,
                message: None,
            });
            bail!("cancelled");
        }

        let chunk = chunk.context("reading the download")?;
        use std::io::Write;
        file.write_all(&chunk)
            .with_context(|| format!("writing {}", part.display()))?;
        received += chunk.len() as u64;

        if received - last_reported >= step {
            last_reported = received;
            report(Progress {
                id: entry.id.into(),
                received,
                total: entry.bytes,
                stage: Stage::Downloading,
                message: None,
            });
        }
    }
    drop(file);

    report(Progress {
        id: entry.id.into(),
        received,
        total: entry.bytes,
        stage: Stage::Verifying,
        message: None,
    });

    // The cheap check first: a size that does not match cannot have the right
    // hash, and saying "truncated" is more useful than "checksum mismatch".
    let size = std::fs::metadata(&part)?.len();
    if size != entry.bytes {
        std::fs::remove_file(&part).ok();
        bail!(
            "the download is {size} bytes, expected {} — it was cut short and has been discarded",
            entry.bytes
        );
    }

    if !verify(&part, entry.sha256)? {
        // Removed, not kept: a file that fails its checksum must never be one
        // resume away from being installed, and keeping it is how a corrupt
        // model survives every later attempt.
        std::fs::remove_file(&part).ok();
        bail!(
            "the downloaded file does not match its published checksum and has been discarded — \
             it is damaged or was tampered with"
        );
    }

    std::fs::rename(&part, &target).with_context(|| format!("installing {}", target.display()))?;

    report(Progress {
        id: entry.id.into(),
        received,
        total: entry.bytes,
        stage: Stage::Done,
        message: None,
    });

    Ok(target)
}

/// Hashes a file and compares it to a hex digest.
///
/// In one pass at the end rather than incrementally as chunks arrive, because
/// a resumed download would have to hash the part it did not fetch anyway —
/// and reading 600 MB back costs about a second against a download measured in
/// minutes.
pub fn verify(path: &Path, expected: &str) -> Result<bool> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("reading {} to check it", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).context("hashing the model")?;
    Ok(hex::encode(hasher.finalize()).eq_ignore_ascii_case(expected))
}

/// Removes an installed model, and any half-finished download of it.
pub fn remove(dir: &Path, id: &str) -> Result<()> {
    let entry = entry(id).ok_or_else(|| anyhow!("no such model: {id}"))?;
    let target = installed_path(dir, entry.id);
    if target.exists() {
        std::fs::remove_file(&target).with_context(|| format!("removing {}", target.display()))?;
    }
    let part = part_path(dir, entry.id);
    if part.exists() {
        let _ = std::fs::remove_file(&part);
    }
    Ok(())
}

/// One catalogue entry as the Settings page sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub id: String,
    pub kind: ModelKind,
    pub label: String,
    pub note: String,
    pub bytes: u64,
    pub installed: bool,
    /// Bytes of an interrupted download waiting to be continued. Shown so that
    /// "download" can honestly say "resume".
    pub partial: u64,
    pub recommended: bool,
}

/// The whole picker's state in one answer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsView {
    /// Shown on the page: an operator who has to put a file there by hand
    /// cannot be expected to know where macOS hides it.
    pub dir: String,
    pub models: Vec<ModelStatus>,
    /// The speech model a session would start with right now, and where it
    /// comes from. `None` is the "no model" state, with `problem` saying so.
    pub active: Option<String>,
    pub source: Option<Source>,
    pub problem: Option<String>,
    /// Whether speech detection would be running. A session without it works
    /// and transcribes noise, which is worth a warning rather than silence.
    pub vad: bool,
}

pub fn view(dir: &Path, selection: &Selection) -> ModelsView {
    let models = CATALOGUE
        .iter()
        .map(|entry| {
            let installed = installed_path(dir, entry.id).is_file();
            ModelStatus {
                id: entry.id.into(),
                kind: entry.kind,
                label: entry.label.into(),
                note: entry.note.into(),
                bytes: entry.bytes,
                installed,
                partial: if installed {
                    0
                } else {
                    std::fs::metadata(part_path(dir, entry.id))
                        .map(|m| m.len())
                        .unwrap_or(0)
                },
                recommended: entry.id == DEFAULT_SPEECH || entry.id == DEFAULT_VAD,
            }
        })
        .collect();

    match resolve(dir, selection) {
        Ok(resolved) => ModelsView {
            dir: dir.display().to_string(),
            models,
            active: Some(resolved.speech.display().to_string()),
            source: Some(resolved.source),
            problem: None,
            vad: resolved.vad.is_some(),
        },
        Err(err) => ModelsView {
            dir: dir.display().to_string(),
            models,
            active: None,
            source: None,
            problem: Some(err.to_string()),
            vad: resolve_vad(dir, selection).is_some(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    fn temp_dir() -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "intercom2chat-models-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("creating the test directory");
        dir
    }

    /// `INTERCOM2CHAT_MODEL` outranks everything by design, which would make
    /// every resolution test below pass or fail depending on the developer's
    /// shell. The tests that are about the order below it say so by skipping.
    fn environment_is_clean() -> bool {
        std::env::var_os("INTERCOM2CHAT_MODEL").is_none()
    }

    fn install(dir: &Path, id: &str) -> PathBuf {
        let path = installed_path(dir, id);
        std::fs::write(&path, b"not really a model").expect("writing the stand-in");
        path
    }

    #[test]
    fn every_catalogue_entry_has_a_usable_checksum() {
        // A typo here is not caught by anything else until a download fails
        // on a real machine, having spent ten minutes fetching the file.
        for entry in CATALOGUE {
            assert_eq!(
                entry.sha256.len(),
                64,
                "{}: a SHA256 is 64 hex characters",
                entry.id
            );
            assert!(
                entry.sha256.chars().all(|c| c.is_ascii_hexdigit()),
                "{}: not hex",
                entry.id
            );
            assert!(entry.bytes > 0, "{}: no expected size", entry.id);
            assert!(
                entry.url.starts_with("https://"),
                "{}: models are not fetched over plain HTTP",
                entry.id
            );
        }
    }

    #[test]
    fn the_defaults_are_in_the_catalogue() {
        // They are what an unconfigured machine resolves to, so a rename that
        // misses one of them has to fail here rather than at someone's start.
        assert_eq!(
            entry(DEFAULT_SPEECH).map(|e| e.kind),
            Some(ModelKind::Speech)
        );
        assert_eq!(entry(DEFAULT_VAD).map(|e| e.kind), Some(ModelKind::Vad));
    }

    #[test]
    fn a_machine_with_nothing_installed_says_what_to_do() {
        // "Nothing installed" has to mean nothing the *development* search can
        // find either — on the machine this was written on `.scratch/models`
        // is full, and that fallback is deliberately allowed to reach outside
        // the empty directory this test creates.
        if !environment_is_clean() || crate::worker::find_model().is_ok() {
            return;
        }
        let dir = temp_dir();
        let view = view(&dir, &Selection::default());

        assert!(view.active.is_none());
        let problem = view.problem.expect("a machine without a model has one");
        assert!(
            problem.contains("Settings") || problem.contains("download"),
            "unactionable: {problem}"
        );
        // And the catalogue is still offered, because that is the way out.
        assert!(view.models.iter().any(|m| m.id == DEFAULT_SPEECH));
    }

    #[test]
    fn an_installed_default_is_found_without_being_selected() {
        if !environment_is_clean() {
            return;
        }
        let dir = temp_dir();
        let expected = install(&dir, DEFAULT_SPEECH);

        let resolved = resolve(&dir, &Selection::default()).expect("the installed default");
        assert_eq!(resolved.speech, expected);
        assert_eq!(resolved.source, Source::Managed);
    }

    #[test]
    fn a_typed_path_beats_the_picker() {
        if !environment_is_clean() {
            return;
        }
        // The offline fallback: somebody carried a model in on a stick and
        // typed where it is. That is an explicit act, and a picker left on a
        // default must not override it.
        let dir = temp_dir();
        install(&dir, DEFAULT_SPEECH);
        let elsewhere = dir.join("carried-in.bin");
        std::fs::write(&elsewhere, b"not really a model").unwrap();

        let resolved = resolve(
            &dir,
            &Selection {
                speech: Some(DEFAULT_SPEECH.into()),
                vad: None,
                path: Some(elsewhere.display().to_string()),
            },
        )
        .expect("the typed path");

        assert_eq!(resolved.speech, elsewhere);
        assert_eq!(resolved.source, Source::Manual);
    }

    #[test]
    fn a_typed_path_that_is_not_there_is_an_error_not_a_fallback() {
        if !environment_is_clean() {
            return;
        }
        // Quietly falling back to the catalogue would mean an operator who
        // mistyped a path runs a different model than the one they chose and
        // is never told.
        let dir = temp_dir();
        install(&dir, DEFAULT_SPEECH);

        let err = resolve(
            &dir,
            &Selection {
                speech: None,
                vad: None,
                path: Some("/nowhere/model.bin".into()),
            },
        )
        .expect_err("a path that is not there");
        assert!(err.to_string().contains("/nowhere/model.bin"));
    }

    #[test]
    fn a_selected_model_that_is_not_downloaded_says_exactly_that() {
        if !environment_is_clean() {
            return;
        }
        // The state a cancelled download leaves behind. "No model installed"
        // would send the operator looking for a problem they do not have.
        let dir = temp_dir();
        let err = resolve(
            &dir,
            &Selection {
                speech: Some("large-v3-turbo-q5_0".into()),
                vad: None,
                path: None,
            },
        )
        .expect_err("a selected model that is not there");
        assert!(
            err.to_string().contains("large-v3-turbo-q5_0"),
            "{err} does not name the model"
        );
    }

    #[test]
    fn the_vad_is_optional_and_the_speech_model_is_not() {
        if !environment_is_clean() {
            return;
        }
        let dir = temp_dir();
        install(&dir, DEFAULT_SPEECH);

        let resolved = resolve(&dir, &Selection::default()).expect("a speech model");
        // Whether a VAD is found depends on the developer's `.scratch`, so the
        // assertion is that the *speech* side did not depend on it.
        assert!(resolved.speech.is_file());

        install(&dir, DEFAULT_VAD);
        let with_vad = resolve(&dir, &Selection::default()).expect("still resolves");
        assert_eq!(with_vad.vad, Some(installed_path(&dir, DEFAULT_VAD)));
    }

    #[test]
    fn a_file_is_measured_against_its_checksum_not_its_name() {
        let dir = temp_dir();
        let path = dir.join("thing.bin");
        std::fs::write(&path, b"abc").unwrap();

        // SHA256("abc")
        let correct = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify(&path, correct).unwrap());
        assert!(verify(&path, &correct.to_uppercase()).unwrap());
        assert!(!verify(&path, &"0".repeat(64)).unwrap());

        // The case the check exists for: one byte different.
        std::fs::write(&path, b"abd").unwrap();
        assert!(!verify(&path, correct).unwrap());
    }

    #[test]
    fn removing_a_model_takes_its_half_download_with_it() {
        // Otherwise "remove and try again" leaves the failed attempt in place
        // and the retry resumes into exactly the state that was removed.
        let dir = temp_dir();
        install(&dir, DEFAULT_SPEECH);
        let part = part_path(&dir, DEFAULT_SPEECH);
        std::fs::write(&part, b"half").unwrap();

        remove(&dir, DEFAULT_SPEECH).expect("removing");

        assert!(!installed_path(&dir, DEFAULT_SPEECH).exists());
        assert!(!part.exists());
    }

    #[test]
    fn a_half_download_is_reported_so_the_button_can_say_resume() {
        let dir = temp_dir();
        std::fs::write(part_path(&dir, DEFAULT_SPEECH), vec![0u8; 4096]).unwrap();

        let view = view(&dir, &Selection::default());
        let status = view
            .models
            .iter()
            .find(|m| m.id == DEFAULT_SPEECH)
            .expect("the default is in the catalogue");
        assert!(!status.installed);
        assert_eq!(status.partial, 4096);
    }
}

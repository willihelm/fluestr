//! Tauri backend.
//!
//! Holds the capture session and the worker pool, and forwards their output to
//! the UI as events, and owns the configuration file the Settings page is
//! saved to (P12), and the model manager's downloads (P11). The rest of the
//! command surface (`update_stream_config`, group CRUD, `search_session`) is
//! still to come — what is here carries up to eight channels of one
//! multichannel device through the pipeline into the chat.

pub mod config;
pub mod models;
pub mod pool;
pub mod resources;
pub mod session;
pub mod worker;

use config::Config;
use models::{ModelsView, Progress};
use session::{
    DeviceOption, EventSink, LevelsEvent, OverloadEvent, Session, SessionConfig, SessionState,
    TranscriptEvent,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

/// Event names on the bridge. Constants because the UI subscribes by string
/// and a typo there fails silently — nothing arrives, nothing complains.
pub const EVENT_TRANSCRIPT: &str = "transcript";
/// Plural: one event carries every stream's meter, so the cost of the meters
/// does not grow with the channel count.
pub const EVENT_LEVELS: &str = "levels";
pub const EVENT_OVERLOAD: &str = "overload";
pub const EVENT_SESSION_STATE: &str = "session-state";
/// How far a model download has got (P11).
pub const EVENT_MODEL_PROGRESS: &str = "model-progress";
/// What the app and its workers are costing the machine, once a second (P8).
pub const EVENT_RESOURCES: &str = "resources";

/// Sends session output over the Tauri bridge.
struct TauriEvents(AppHandle);

impl EventSink for TauriEvents {
    fn transcript(&self, event: TranscriptEvent) {
        let _ = self.0.emit(EVENT_TRANSCRIPT, event);
    }

    fn levels(&self, event: LevelsEvent) {
        let _ = self.0.emit(EVENT_LEVELS, event);
    }

    fn overload(&self, event: OverloadEvent) {
        let _ = self.0.emit(EVENT_OVERLOAD, event);
    }

    fn state(&self, event: SessionState) {
        let _ = self.0.emit(EVENT_SESSION_STATE, event);
    }
}

#[derive(Default)]
struct AppState {
    session: Mutex<Option<Session>>,
    /// Where the configuration file lives, resolved once at setup.
    ///
    /// Resolved there rather than per command because it depends on the app's
    /// identifier and on the platform, and a command that cannot answer where
    /// its own configuration is has nothing useful to do. `None` means the
    /// resolver refused — see [`run`]: the app still starts, and saving says
    /// why it cannot.
    config_path: Mutex<Option<PathBuf>>,
    /// Where downloaded models go. Resolved once at setup, like the
    /// configuration path and for the same reason. `None` means the platform
    /// would not say where the app may keep data, which makes downloading
    /// impossible and has to be said out loud rather than failing per click.
    models_dir: Mutex<Option<PathBuf>>,
    /// The cancel flag of each download in flight, by model id.
    ///
    /// Keyed by id rather than being a single flag because the speech model
    /// and the VAD are two different downloads an operator may reasonably
    /// start together — and cancelling one of them must not stop the other.
    downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// The sidecar's language list, once it has been asked for.
    ///
    /// Cached because it is a table compiled into whisper.cpp — it cannot
    /// change while the app runs — and because fetching it means spawning a
    /// process, which should not happen every time Settings is opened.
    languages: Mutex<Option<Vec<whisper_protocol::Language>>>,
}

impl AppState {
    /// The configuration path, or the reason there is none.
    fn config_path(&self) -> Result<PathBuf, String> {
        self.config_path
            .lock()
            .map_err(|_| "configuration path poisoned".to_string())?
            .clone()
            .ok_or_else(|| "no configuration directory on this system".to_string())
    }

    fn models_dir(&self) -> Result<PathBuf, String> {
        self.models_dir
            .lock()
            .map_err(|_| "model directory poisoned".to_string())?
            .clone()
            .ok_or_else(|| "no application data directory on this system".to_string())
    }

    /// The stored model choice. A configuration that cannot be read resolves
    /// as "nothing chosen", which lands on the default — the model picker is
    /// not the place to surface a broken configuration file, and `load_config`
    /// already did.
    fn selection(&self) -> models::Selection {
        self.config_path()
            .map(|path| config::load(&path).selection())
            .unwrap_or_default()
    }
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The stored configuration, or the default one.
///
/// Infallible on purpose where it can be: a first start, an unreadable file
/// and a file from a newer version all come back as an empty configuration and
/// a log line rather than as an error the UI would have to render instead of
/// the app. What it cannot paper over is not knowing where to look, which is
/// the one case that also makes saving impossible and therefore has to be
/// visible.
#[tauri::command]
fn load_config(state: tauri::State<'_, AppState>) -> Result<Config, String> {
    Ok(config::load(&state.config_path()?))
}

/// Writes the whole configuration.
///
/// The whole of it, not a patch: it is a few kilobytes that a human edits at
/// human speed, and a partial update would need the two sides to agree on
/// which half is authoritative — which is exactly the drift the single file
/// exists to avoid.
#[tauri::command]
fn save_config(state: tauri::State<'_, AppState>, config: Config) -> Result<(), String> {
    let path = state.config_path()?;
    config::save(&path, &config).map_err(|err| {
        // With the chain of causes, which the UI string flattens away.
        tracing::error!(error = ?err, "saving the configuration failed");
        err.to_string()
    })
}

// --- The model manager (P11) ---------------------------------------------

/// The catalogue, what of it is installed, and which model a session would
/// actually start with.
///
/// One command rather than three, because the three answers are only useful
/// together: "small is installed" means nothing without "and it is the one in
/// use", and the page would otherwise render a picker and its active state
/// from two round trips that can disagree.
/// The choice the page is currently showing, when it differs from the stored
/// one.
///
/// The Settings page saves through a debounce, so the file answers with what
/// was selected a second ago: asked without this, the view would tell an
/// operator who just clicked a radio button that a different model is running.
/// The page knows what it is showing, so it says, and the banner and the radio
/// can never disagree.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectionArg {
    model: Option<String>,
    vad_model: Option<String>,
    model_path: Option<String>,
}

/// Asks for a model file with the system's own file dialog.
///
/// `current` is what the field holds, and the dialog opens on it: a second
/// attempt after picking the wrong `.bin` should not start again at whatever
/// the operating system last remembered, which on a stick full of models is
/// the whole walk back through the directory tree.
///
/// `None` is a cancelled dialog, which is not an error and must not read as
/// one: it is the most likely outcome of opening a picker, and the path in
/// Settings stays exactly as it was.
///
/// `async` on purpose. A synchronous command runs on the main thread, which is
/// the thread the dialog has to be driven from — blocking it there is a hung
/// window rather than a picker.
#[tauri::command]
async fn pick_model_file(app: AppHandle, current: Option<String>) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;

    let mut dialog = app
        .dialog()
        .file()
        // A whisper model is a `.bin`, and a dialog that offers every file on
        // the disk invites the one mistake this field can make. "All files"
        // stays available in the dialog itself for anything unusual.
        .add_filter("Whisper model", &["bin"]);

    // Both halves are checked against the disk before they are used. A typed
    // path that is half-finished or points at an unplugged stick is exactly
    // when somebody reaches for the button, and a dialog asked to open
    // somewhere that is not there opens somewhere unpredictable instead.
    if let Some(current) = current.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        let current = Path::new(current);
        if let Some(dir) = current.parent().filter(|dir| dir.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        if current.is_file() {
            if let Some(name) = current.file_name().and_then(|name| name.to_str()) {
                dialog = dialog.set_file_name(name);
            }
        }
    }

    dialog.blocking_pick_file().map(|file| file.to_string())
}

#[tauri::command]
fn list_models(
    state: tauri::State<'_, AppState>,
    selection: Option<SelectionArg>,
) -> Result<ModelsView, String> {
    let selection = match selection {
        Some(arg) => models::Selection {
            speech: arg.model,
            vad: arg.vad_model,
            path: arg.model_path,
        },
        None => state.selection(),
    };
    Ok(models::view(&state.models_dir()?, &selection))
}

/// Fetches one catalogue model, resuming an interrupted attempt.
///
/// Returns as soon as the download is *running*, not when it finishes: this is
/// minutes of work, and a command that blocked for it would freeze the window
/// and hit Tauri's own timeouts. Progress arrives as `model-progress` events,
/// which is also what a resumed or cancelled download reports through.
#[tauri::command]
async fn download_model(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let dir = state.models_dir()?;
    let entry = models::entry(&id).ok_or_else(|| format!("no such model: {id}"))?;

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut downloads = state
            .downloads
            .lock()
            .map_err(|_| "download state poisoned".to_string())?;
        // Pressing the button twice must not start two writers on one part
        // file. They would interleave, and only the checksum at the end would
        // notice — after both had run to completion.
        if downloads.contains_key(&id) {
            return Err(format!("{id} is already downloading"));
        }
        downloads.insert(id.clone(), Arc::clone(&cancel));
    }

    tauri::async_runtime::spawn(async move {
        let progress_app = app.clone();
        let result = models::download(entry, &dir, cancel, move |progress| {
            let _ = progress_app.emit(EVENT_MODEL_PROGRESS, progress);
        })
        .await;

        // Cleared before the failure is reported, so that the retry the
        // operator is about to press is not refused as "already downloading".
        if let Some(state) = app.try_state::<AppState>() {
            if let Ok(mut downloads) = state.downloads.lock() {
                downloads.remove(&id);
            }
        }

        if let Err(err) = result {
            // Cancelling is not a failure and has already reported itself.
            if err.to_string() == "cancelled" {
                tracing::info!(id, "download cancelled");
                return;
            }
            tracing::error!(id, error = ?err, "downloading a model failed");
            let _ = app.emit(
                EVENT_MODEL_PROGRESS,
                Progress {
                    id,
                    received: 0,
                    total: entry.bytes,
                    stage: models::Stage::Failed,
                    message: Some(err.to_string()),
                },
            );
        }
    });

    Ok(())
}

/// Stops a download in flight. What has arrived is kept, so pressing download
/// again continues rather than starts over.
#[tauri::command]
fn cancel_download(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let downloads = state
        .downloads
        .lock()
        .map_err(|_| "download state poisoned".to_string())?;
    if let Some(cancel) = downloads.get(&id) {
        cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// Deletes an installed model, and any half-finished download of it.
///
/// Worth having rather than leaving to the Finder: these are the largest files
/// the app ever writes, in a directory macOS does its best to hide.
#[tauri::command]
fn remove_model(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    models::remove(&state.models_dir()?, &id).map_err(|err| {
        tracing::error!(error = ?err, "removing a model failed");
        err.to_string()
    })
}

/// Every language the sidecar can be given, for the picker on the channel
/// setup.
///
/// Fetched from whisper.cpp rather than listed here, so that no language list
/// exists anywhere in this repository: adding one is a whisper.cpp version,
/// never a code change of ours.
#[tauri::command]
fn list_languages(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<whisper_protocol::Language>, String> {
    let mut cached = state
        .languages
        .lock()
        .map_err(|_| "language list poisoned")?;
    if let Some(languages) = cached.as_ref() {
        return Ok(languages.clone());
    }

    let languages = worker::list_languages().map_err(|err| {
        tracing::error!(error = ?err, "asking the sidecar for its language list failed");
        err.to_string()
    })?;
    *cached = Some(languages.clone());
    Ok(languages)
}

#[tauri::command]
fn list_devices() -> Result<Vec<DeviceOption>, String> {
    session::list_devices().map_err(|e| e.to_string())
}

/// Starts a session and returns its running state.
///
/// Blocks until the model is loaded, which on a machine's very first start
/// includes the one-off Metal shader compilation (~8 s). Returning the state
/// rather than only emitting it means the button the operator pressed gets its
/// own answer; the events carry what happens after that.
#[tauri::command]
fn start_session(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    config: SessionConfig,
) -> Result<SessionState, String> {
    let mut slot = state.session.lock().map_err(|_| "session state poisoned")?;

    // A session whose pipeline has already ended — device unplugged, sidecar
    // lost — is still sitting in the slot, because the failure is discovered
    // on the processing thread and nothing there can reach this state. Left
    // alone it makes the next Start fail with "a session is already running"
    // while the UI, which saw the failure, offers Start rather than Stop, and
    // nothing but restarting the app gets out of it. Clearing it here is also
    // what joins its threads and releases its sidecar.
    if slot.as_ref().is_some_and(Session::is_finished) {
        tracing::info!("clearing the previous session, whose pipeline had ended");
        if let Some(finished) = slot.take() {
            finished.stop();
        }
    }

    if slot.is_some() {
        return Err("a session is already running".into());
    }

    // Which model, decided before anything is opened. A session that starts,
    // captures audio and only then discovers it has nothing to transcribe with
    // is the silent malfunction P11 exists to remove — and the message has to
    // name the fix, because "no model" is not something an operator can act on.
    let models = match state.models_dir() {
        Ok(dir) => Some(models::resolve(&dir, &state.selection()).map_err(|err| {
            tracing::error!(error = ?err, "no usable model");
            err.to_string()
        })?),
        // No app directory: the model manager cannot work here, but the
        // development search still can, so this is not fatal on its own.
        Err(err) => {
            tracing::warn!(error = %err, "no model directory — falling back to the local search");
            None
        }
    };

    let events: Arc<dyn EventSink> = Arc::new(TauriEvents(app));
    match Session::start(config, events.clone(), models) {
        Ok((session, running)) => {
            *slot = Some(session);
            Ok(running)
        }
        Err(err) => {
            // With the chain of causes, which the UI string flattens away.
            tracing::error!(error = ?err, "starting the session failed");
            let message = err.to_string();
            // Also as an event, so a failure is visible in the same place a
            // successful start is, rather than only in the caller's catch.
            events.state(SessionState::Failed {
                message: message.clone(),
            });
            Err(message)
        }
    }
}

#[tauri::command]
fn stop_session(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let session = state
        .session
        .lock()
        .map_err(|_| "session state poisoned")?
        .take();
    if let Some(session) = session {
        session.stop();
    }
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = AppState::default();
            // A missing configuration directory is not a reason to refuse to
            // start: the app transcribes perfectly well without ever saving a
            // profile, and an operator in front of a show is better served by
            // a running app that cannot remember than by no app at all. It is
            // reported, and every save says so again.
            match app.path().app_config_dir() {
                Ok(dir) => {
                    let path = dir.join(config::FILE_NAME);
                    tracing::info!(path = %path.display(), "configuration file");
                    *state.config_path.lock().expect("fresh state") = Some(path);
                }
                Err(err) => {
                    tracing::error!(error = %err, "no configuration directory — nothing will be saved");
                }
            }
            // Models go under the app's *data* directory, not its
            // configuration one: they are hundreds of megabytes of cache that
            // can always be fetched again, and on macOS that is the difference
            // between something the user's backup carries around forever and
            // something it can skip.
            match app.path().app_data_dir() {
                Ok(dir) => {
                    let dir = dir.join(models::DIR_NAME);
                    tracing::info!(path = %dir.display(), "model directory");
                    *state.models_dir.lock().expect("fresh state") = Some(dir);
                }
                Err(err) => {
                    tracing::error!(error = %err, "no data directory — models cannot be downloaded");
                }
            }
            app.manage(state);

            // The resource indicator runs for as long as the window does, not
            // only while a session does: an operator opening the app on a
            // machine that is already loaded should see that before they press
            // Start, and the figures after Stop are how the memory a session
            // held is seen to come back.
            let handle = app.handle().clone();
            resources::spawn(move |usage| {
                let _ = handle.emit(EVENT_RESOURCES, usage);
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            load_config,
            save_config,
            pick_model_file,
            list_models,
            download_model,
            cancel_download,
            remove_model,
            list_devices,
            list_languages,
            start_session,
            stop_session
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the Tauri application");
}

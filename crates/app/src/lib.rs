//! Tauri backend.
//!
//! Holds the capture session and the worker pool, and forwards their output to
//! the UI as events, and owns the configuration file the Settings page is
//! saved to (P12). The rest of the command surface (`update_stream_config`,
//! group CRUD, `search_session`) and the SQLite persistence of *transcripts*
//! (P10) are still to come — what is here carries up to eight channels of one
//! multichannel device through the pipeline into the chat.

pub mod config;
pub mod pool;
pub mod session;
pub mod worker;

use config::Config;
use session::{
    DeviceOption, EventSink, LevelsEvent, OverloadEvent, Session, SessionConfig, SessionState,
    TranscriptEvent,
};
use std::path::PathBuf;
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

    let events: Arc<dyn EventSink> = Arc::new(TauriEvents(app));
    match Session::start(config, events.clone()) {
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
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            load_config,
            save_config,
            list_devices,
            start_session,
            stop_session
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the Tauri application");
}

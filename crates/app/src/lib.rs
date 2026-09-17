//! Tauri backend.
//!
//! Holds the capture session and the worker pool, and forwards their output to
//! the UI as events. The rest of the command surface (`update_stream_config`,
//! group CRUD, `search_session`) and the SQLite persistence (P10) are still to
//! come — what is here carries up to eight channels of one multichannel device
//! through the pipeline into the chat.

pub mod pool;
pub mod session;
pub mod worker;

use session::{
    DeviceOption, EventSink, LevelsEvent, OverloadEvent, Session, SessionConfig, SessionState,
    TranscriptEvent,
};
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
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
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
            app.manage(AppState::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            list_devices,
            start_session,
            stop_session
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the Tauri application");
}

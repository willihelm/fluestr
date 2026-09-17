//! Tauri backend.
//!
//! Holds the capture session and the sidecar, and forwards their output to the
//! UI as events. The worker pool (P5), the full command surface (P6) and the
//! SQLite persistence (P10) still to come — what is here is the narrow
//! vertical slice: one channel, one worker, one chat list.

pub mod session;
pub mod worker;

use audio_core::{LanguageCode, LanguageMode};
use serde::{Deserialize, Serialize};
use session::{
    DeviceOption, EventSink, LevelEvent, Session, SessionConfig, SessionState, TranscriptEvent,
};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

/// Configuration of a single stream (= one intercom channel).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamConfig {
    /// Channel index on the multichannel interface.
    pub channel: u16,
    /// Free-form label: "Regie", "FOH", "Licht".
    pub label: String,
    pub language_mode: LanguageMode,
    /// Name of the assigned vocabulary list (P15).
    pub vocabulary: Option<String>,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            channel: 0,
            label: String::new(),
            language_mode: LanguageMode::Fixed {
                language: LanguageCode::german(),
            },
            vocabulary: None,
        }
    }
}

/// Event names on the bridge. Constants because the UI subscribes by string
/// and a typo there fails silently — nothing arrives, nothing complains.
pub const EVENT_TRANSCRIPT: &str = "transcript";
pub const EVENT_LEVEL: &str = "level";
pub const EVENT_SESSION_STATE: &str = "session-state";

/// Sends session output over the Tauri bridge.
struct TauriEvents(AppHandle);

impl EventSink for TauriEvents {
    fn transcript(&self, event: TranscriptEvent) {
        let _ = self.0.emit(EVENT_TRANSCRIPT, event);
    }

    fn level(&self, event: LevelEvent) {
        let _ = self.0.emit(EVENT_LEVEL, event);
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

/// Starts a session. Blocks until the model is loaded, which on a machine's
/// very first start includes the one-off Metal shader compilation (~8 s).
#[tauri::command]
fn start_session(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    config: SessionConfig,
) -> Result<(), String> {
    let mut slot = state.session.lock().map_err(|_| "session state poisoned")?;
    if slot.is_some() {
        return Err("a session is already running".into());
    }

    let events: Arc<dyn EventSink> = Arc::new(TauriEvents(app));
    match Session::start(config, events.clone()) {
        Ok(session) => {
            *slot = Some(session);
            Ok(())
        }
        Err(err) => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_config_defaults_to_fixed_german() {
        let cfg = StreamConfig::default();
        assert_eq!(cfg.language_mode.whisper_language(), Some("de"));
        assert!(!cfg.language_mode.is_auto());
    }

    #[test]
    fn stream_config_serialises_for_the_ui() {
        let cfg = StreamConfig {
            channel: 2,
            label: "Regie".into(),
            language_mode: LanguageMode::Auto {
                candidates: vec![LanguageCode::german(), LanguageCode::english()],
            },
            vocabulary: Some("Broadcast".into()),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: StreamConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.label, "Regie");
        assert_eq!(back.channel, 2);
        assert!(back.language_mode.is_auto());
    }
}

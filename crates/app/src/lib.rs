//! Tauri backend.
//!
//! Will later hold the worker pool (P5), the capture session (P1) and the
//! SQLite persistence (P10). For now it is scaffolding plus the commands the
//! UI needs in order to avoid a hard-coded language list.

use audio_core::{LanguageCode, LanguageMode};
use serde::{Deserialize, Serialize};

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

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_version])
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

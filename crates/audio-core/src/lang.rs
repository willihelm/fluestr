//! Language modelling.
//!
//! Deliberately *not* an enum with `De`/`En` variants: the language list comes
//! from whisper.cpp at runtime (`whisper_lang_str`), so adding a language is
//! pure configuration rather than a code change. See the "Language strategy"
//! section of the plan.

use serde::{Deserialize, Serialize};
use std::fmt;

/// An ISO 639-1 code as whisper.cpp expects it ("de", "en", "fr", ...).
///
/// An open newtype rather than an enum on purpose — every language whisper.cpp
/// supports is valid without this code having to know about it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LanguageCode(String);

impl LanguageCode {
    pub fn new(code: impl Into<String>) -> Self {
        Self(code.into().trim().to_ascii_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Convenience constructors for the common case and for tests. These are
    /// defaults, *not* a restriction on what is allowed.
    pub fn german() -> Self {
        Self::new("de")
    }

    pub fn english() -> Self {
        Self::new("en")
    }
}

impl fmt::Display for LanguageCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How the language for a stream is determined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum LanguageMode {
    /// A fixed language — the default. The multilingual model carries
    /// foreign-language technical terms over verbatim; a vocabulary prompt
    /// (P15) reinforces that further.
    Fixed { language: LanguageCode },

    /// Automatic detection, **restricted to these candidates**. Unrestricted
    /// detection across all ~99 whisper languages is far more error-prone.
    Auto { candidates: Vec<LanguageCode> },
}

impl LanguageMode {
    /// The language code the sidecar should set — `None` means "let whisper
    /// detect it".
    pub fn whisper_language(&self) -> Option<&str> {
        match self {
            Self::Fixed { language } => Some(language.as_str()),
            Self::Auto { .. } => None,
        }
    }

    /// The candidates the language stabiliser (P3b) restricts its argmax to.
    pub fn candidates(&self) -> &[LanguageCode] {
        match self {
            Self::Fixed { .. } => &[],
            Self::Auto { candidates } => candidates,
        }
    }

    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto { .. })
    }
}

impl Default for LanguageMode {
    fn default() -> Self {
        Self::Fixed {
            language: LanguageCode::german(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_code_normalises_input() {
        assert_eq!(LanguageCode::new("  DE ").as_str(), "de");
        assert_eq!(LanguageCode::new("EN"), LanguageCode::english());
    }

    #[test]
    fn any_language_can_be_constructed() {
        // The whole point of the newtype: a third language costs no code change.
        let fr = LanguageCode::new("fr");
        let mode = LanguageMode::Fixed {
            language: fr.clone(),
        };
        assert_eq!(mode.whisper_language(), Some("fr"));
        assert_eq!(fr.to_string(), "fr");
    }

    #[test]
    fn fixed_yields_a_language_auto_yields_none() {
        let fixed = LanguageMode::default();
        assert_eq!(fixed.whisper_language(), Some("de"));
        assert!(fixed.candidates().is_empty());
        assert!(!fixed.is_auto());

        let auto = LanguageMode::Auto {
            candidates: vec![LanguageCode::german(), LanguageCode::english()],
        };
        assert_eq!(auto.whisper_language(), None);
        assert_eq!(auto.candidates().len(), 2);
        assert!(auto.is_auto());
    }

    #[test]
    fn serialisation_is_stable() {
        // This format crosses the sidecar boundary (P4) and lands in the
        // database (P10), so it is pinned down here.
        let json = serde_json::to_string(&LanguageMode::default()).unwrap();
        assert_eq!(json, r#"{"mode":"fixed","language":"de"}"#);

        let auto = LanguageMode::Auto {
            candidates: vec![LanguageCode::german(), LanguageCode::english()],
        };
        let json = serde_json::to_string(&auto).unwrap();
        assert_eq!(json, r#"{"mode":"auto","candidates":["de","en"]}"#);
    }
}

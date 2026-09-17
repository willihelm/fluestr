//! The segment — the unit that travels from the segmenter (P3) through the
//! worker pool (P5) to the sidecar (P4).

use crate::lang::LanguageMode;
use serde::{Deserialize, Serialize};

/// Internal processing sample rate. whisper.cpp expects exactly this.
pub const SAMPLE_RATE: u32 = 16_000;

/// Stable identifier of a logical stream (= one intercom channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamId(pub u16);

/// A contiguous stretch of speech from one stream, ready for transcription.
#[derive(Debug, Clone)]
pub struct Segment {
    pub stream_id: StreamId,
    /// Per-stream sequence number. The result aggregator (P5) uses it to
    /// restore ordering, because workers do not necessarily reply in order.
    pub seq: u64,
    /// Offset from the start of the session.
    pub t_start: std::time::Duration,
    pub t_end: std::time::Duration,
    /// 16 kHz mono, normalised to [-1, 1].
    pub pcm: Vec<f32>,
    pub language_mode: LanguageMode,
    /// Vocabulary prompt (P15), already assembled and length-capped.
    pub initial_prompt: Option<String>,
}

impl Segment {
    pub fn duration(&self) -> std::time::Duration {
        self.t_end.saturating_sub(self.t_start)
    }

    /// Duration derived from the actual sample count — independent of the
    /// timestamps and therefore usable as a consistency check.
    pub fn pcm_duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs_f64(self.pcm.len() as f64 / SAMPLE_RATE as f64)
    }
}

/// The result of a transcription as it comes back from the sidecar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub stream_id: StreamId,
    pub seq: u64,
    pub text: String,
    /// The language whisper.cpp detected. Recorded **always**, even for a
    /// fixed language — it costs nothing and makes misdetections analysable
    /// after the fact.
    pub detected_language: Option<String>,
    /// Probability vector across all languages, the basis for the language
    /// stabiliser (P3b). Only populated in auto mode.
    pub language_probabilities: Option<Vec<(String, f32)>>,
    /// whisper.cpp's own estimate that the audio held no speech. See
    /// [`is_only_non_speech`] for the other half of the same problem.
    pub no_speech_probability: f32,
    pub decode_ms: u32,
}

/// True when the text carries no spoken words — only whisper's sound-event
/// annotations, or nothing at all.
///
/// An intercom channel is silent or carries only room noise for long stretches,
/// and whisper captions that as `[BREAK]`, `(music)` or `(Bell)`. Those are not
/// communication and must not reach the chat.
///
/// The decoder is told not to produce them in the first place
/// (`suppress_nst`), so this is the second line rather than the first — it
/// catches what still slips through. Text that merely *contains* an annotation
/// survives: "Standby (fünf)" is somebody talking.
pub fn is_only_non_speech(text: &str) -> bool {
    let mut depth = 0usize;
    let mut fenced = false;
    let mut spoken = false;
    let mut unbalanced = false;

    for ch in text.chars() {
        match ch {
            '[' | '(' | '<' | '{' => depth += 1,
            ']' | ')' | '>' | '}' => {
                if depth == 0 {
                    unbalanced = true;
                } else {
                    depth -= 1;
                }
            }
            // Musical notes and asterisks fence an annotation without nesting,
            // so they toggle rather than count.
            '\u{266a}' | '\u{266b}' | '*' => fenced = !fenced,
            _ if depth == 0 && !fenced && (ch.is_alphanumeric() || ch == '\'') => spoken = true,
            _ => {}
        }
    }

    // A delimiter left open means the text was cut off, or the bracket was
    // never an annotation to begin with. Then keep it: showing a stray
    // "(music" is a far smaller harm than silently dropping a sentence that
    // happened to contain one bracket, and silent loss is the one failure this
    // project does not accept.
    if depth != 0 || fenced || unbalanced {
        return false;
    }

    !spoken
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn segment_with_samples(n: usize) -> Segment {
        Segment {
            stream_id: StreamId(3),
            seq: 42,
            t_start: Duration::from_millis(1_000),
            t_end: Duration::from_millis(2_500),
            pcm: vec![0.0; n],
            language_mode: LanguageMode::default(),
            initial_prompt: None,
        }
    }

    #[test]
    fn duration_from_timestamps() {
        let seg = segment_with_samples(0);
        assert_eq!(seg.duration(), Duration::from_millis(1_500));
    }

    #[test]
    fn duration_from_samples() {
        // 24000 samples at 16 kHz = 1.5 s — matching the timestamps above.
        let seg = segment_with_samples(24_000);
        assert_eq!(seg.pcm_duration(), Duration::from_millis(1_500));
        assert_eq!(seg.pcm_duration(), seg.duration());
    }

    #[test]
    fn duration_does_not_underflow_on_reversed_timestamps() {
        let mut seg = segment_with_samples(0);
        seg.t_end = Duration::ZERO;
        assert_eq!(seg.duration(), Duration::ZERO);
    }

    #[test]
    fn whispers_sound_annotations_are_recognised_as_non_speech() {
        // All of these actually appeared in the chat during the first live run
        // on a quiet channel.
        for annotation in [
            "[BREAK]",
            "(music)",
            "(Bell)",
            "[BLANK_AUDIO]",
            "[Music]",
            "*coughs*",
            "\u{266a}\u{266a}",
            "[ Silence ]",
            "(applause) [music]",
            "",
            "   ",
            "...",
            "- ",
        ] {
            assert!(
                is_only_non_speech(annotation),
                "{annotation:?} should count as non-speech"
            );
        }
    }

    #[test]
    fn actual_speech_survives_the_filter() {
        // German on purpose: this is speech data, not code.
        for spoken in [
            "Standby für Cue fünf",
            "Go.",
            "Nice.",
            // An annotation next to real words is still somebody talking.
            "Standby (fünf)",
            "[BREAK] und jetzt Blackout",
            "5",
        ] {
            assert!(
                !is_only_non_speech(spoken),
                "{spoken:?} should count as speech"
            );
        }
    }

    #[test]
    fn an_unclosed_bracket_does_not_swallow_the_rest() {
        // A truncated annotation must not make the words after it disappear.
        assert!(!is_only_non_speech("(music"));
        assert!(!is_only_non_speech("Blackout)"));
    }
}

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
    pub decode_ms: u32,
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
}

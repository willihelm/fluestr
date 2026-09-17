//! Voice activity detection.
//!
//! The trait is the point of this module. P3 replaces the implementation below
//! with `webrtc-vad` or the Silero model that ships with whisper-rs; the
//! segmenter must not have to change when that happens, so it only ever sees
//! [`Vad`].
//!
//! [`EnergyVad`] is deliberately the crudest thing that works: it is good
//! enough to get speech out of a quiet room and into the pipeline, which is all
//! the first end-to-end slice needs. It is *not* good enough for an intercom
//! feed with constant background noise — that is exactly what P3 is for.

/// Decides frame by frame whether a stretch of audio contains speech.
///
/// Implementations are stateful: a detector that cannot see what came before
/// cannot apply hysteresis, and without hysteresis every pause between two
/// words ends a segment.
pub trait Vad: Send {
    /// Samples per frame at [`SAMPLE_RATE`](crate::SAMPLE_RATE). The segmenter
    /// feeds exactly this many.
    fn frame_samples(&self) -> usize;

    fn is_speech(&mut self, frame: &[f32]) -> bool;
}

/// Frame length in milliseconds. 20 ms is the granularity every VAD worth
/// having works at, and it bounds how precisely a segment boundary can land.
pub const FRAME_MS: usize = 20;

/// Provisional detector: frame energy against an adaptive noise floor.
///
/// The noise floor tracks the quiet passages only, so a room tone that rises
/// during a show does not slowly turn into permanent "speech". What it cannot
/// do is tell speech from any *other* loud sound — a chair scrape opens a
/// segment just as reliably as a word does.
pub struct EnergyVad {
    frame_samples: usize,
    /// Exponential moving average of the frame RMS while not speaking.
    noise_floor: f32,
    speaking: bool,
    /// Speech starts above `noise_floor * onset_ratio` …
    onset_ratio: f32,
    /// … and only ends below `noise_floor * release_ratio`. The gap is the
    /// hysteresis that keeps a segment alive across the pause inside a word.
    release_ratio: f32,
    /// Below this, nothing counts as speech regardless of the noise floor.
    /// Without it a perfectly silent input would drive the floor towards zero
    /// and then detect speech in its own dither.
    absolute_floor: f32,
    /// Adaptation rate of the noise floor. Slow on purpose: fast adaptation
    /// lets a long sentence be absorbed into the floor.
    adaptation: f32,
}

impl EnergyVad {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            frame_samples: sample_rate as usize * FRAME_MS / 1000,
            // Starting guess for a quiet room, corrected within a few frames.
            noise_floor: 0.003,
            speaking: false,
            onset_ratio: 3.0,
            release_ratio: 1.6,
            // ≈ -46 dBFS.
            absolute_floor: 0.005,
            adaptation: 0.05,
        }
    }

    /// Raises the bar for what counts as speech. Exposed for the settings
    /// screen (P12) and for tests.
    pub fn with_thresholds(mut self, onset_ratio: f32, absolute_floor: f32) -> Self {
        self.onset_ratio = onset_ratio.max(1.0);
        self.release_ratio = self.onset_ratio.min(1.0 + (self.onset_ratio - 1.0) * 0.3);
        self.absolute_floor = absolute_floor.max(0.0);
        self
    }

    pub fn noise_floor(&self) -> f32 {
        self.noise_floor
    }
}

impl Vad for EnergyVad {
    fn frame_samples(&self) -> usize {
        self.frame_samples
    }

    fn is_speech(&mut self, frame: &[f32]) -> bool {
        let level = rms(frame);

        let threshold = if self.speaking {
            self.noise_floor * self.release_ratio
        } else {
            self.noise_floor * self.onset_ratio
        };

        self.speaking = level > threshold.max(self.absolute_floor);

        // Adapt only on quiet frames, so speech never raises the floor that is
        // supposed to detect it.
        if !self.speaking {
            self.noise_floor += self.adaptation * (level - self.noise_floor);
        }

        self.speaking
    }
}

/// Root mean square of a frame. Zero for an empty frame.
pub fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f32 = frame.iter().map(|s| s * s).sum();
    (sum / frame.len() as f32).sqrt()
}

/// Largest absolute sample — what a clipping indicator needs (P8).
pub fn peak(frame: &[f32]) -> f32 {
    frame.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SAMPLE_RATE;
    use std::f32::consts::TAU;

    fn frames(vad: &EnergyVad, amplitude: f32, count: usize) -> Vec<Vec<f32>> {
        let n = vad.frame_samples();
        (0..count)
            .map(|f| {
                (0..n)
                    .map(|i| {
                        let t = (f * n + i) as f32 / SAMPLE_RATE as f32;
                        amplitude * (TAU * 300.0 * t).sin()
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_20_ms_frame_at_16_khz_is_320_samples() {
        let vad = EnergyVad::new(SAMPLE_RATE);
        assert_eq!(vad.frame_samples(), 320);
    }

    #[test]
    fn silence_is_not_speech() {
        let mut vad = EnergyVad::new(SAMPLE_RATE);
        let silence = vec![0.0; vad.frame_samples()];
        for _ in 0..50 {
            assert!(!vad.is_speech(&silence));
        }
    }

    #[test]
    fn a_loud_tone_is_speech() {
        let mut vad = EnergyVad::new(SAMPLE_RATE);
        let loud = frames(&vad, 0.3, 5);
        assert!(loud.iter().any(|f| vad.is_speech(f)));
    }

    #[test]
    fn quiet_room_tone_is_absorbed_into_the_noise_floor() {
        // A constant low-level hiss must not read as speech forever — that
        // would make the segmenter emit one endless segment.
        let mut vad = EnergyVad::new(SAMPLE_RATE);
        let hiss = frames(&vad, 0.01, 60);

        let late: Vec<bool> = hiss.iter().map(|f| vad.is_speech(f)).collect();
        assert!(
            !late[40..].iter().any(|&s| s),
            "steady hiss should stop counting as speech"
        );
    }

    #[test]
    fn speech_still_stands_out_above_a_raised_floor() {
        let mut vad = EnergyVad::new(SAMPLE_RATE);
        for frame in frames(&vad, 0.01, 60) {
            vad.is_speech(&frame);
        }
        // Ten times the background is unambiguous.
        let loud = frames(&vad, 0.1, 5);
        assert!(loud.iter().any(|f| vad.is_speech(f)));
    }

    #[test]
    fn hysteresis_holds_through_a_dip() {
        let mut vad = EnergyVad::new(SAMPLE_RATE);
        for frame in frames(&vad, 0.2, 5) {
            vad.is_speech(&frame);
        }
        assert!(vad.speaking);

        // A dip that would be below the onset threshold but above release —
        // the gap inside a word, not the end of one.
        let dip = frames(&vad, 0.02, 1);
        assert!(
            vad.is_speech(&dip[0]),
            "a short dip must not end the segment"
        );
    }

    #[test]
    fn rms_and_peak_are_what_they_say() {
        assert_eq!(rms(&[]), 0.0);
        assert_eq!(peak(&[]), 0.0);
        assert!((rms(&[1.0, -1.0, 1.0, -1.0]) - 1.0).abs() < 1e-6);
        assert_eq!(peak(&[0.2, -0.7, 0.3]), 0.7);
    }
}

//! Input gain per stream.
//!
//! An intercom desk does not hand out eight channels at one level: the stage
//! manager's beltpack is hot, the follow-spot's matrix output is twenty dB
//! below it, and whisper's accuracy falls off on the quiet one long before a
//! human would call it inaudible. The fix at the desk is a trim knob, so the
//! fix here is one too.
//!
//! Applied on the processing thread rather than in the capture callback. The
//! callback de-interleaves and nothing else — it is the one place in the app
//! that must never grow work — and the level meters are computed from the same
//! buffer this scales, which is what makes the knob something an operator can
//! actually set: they turn it and watch the bar it feeds.
//!
//! Whole decibels, not a float factor. It is what a trim knob is marked in, it
//! is what an operator writing their setup down would write down, and it keeps
//! the stored configuration readable and comparable.

use std::sync::atomic::{AtomicI32, Ordering};

/// The quietest the trim goes. Below this a channel is better fixed at the
/// desk than here: twenty dB of digital gain already lifts the noise floor
/// with the speech.
pub const MIN_DB: i32 = -20;

/// The loudest. Ten times the amplitude, which is what a line-level output
/// patched into a microphone input needs, and past which the only thing left
/// to amplify is the interface's own hiss.
pub const MAX_DB: i32 = 20;

/// Brings a setting into range. A file edited by hand, or written by a newer
/// version with a wider range, is clamped rather than refused — a channel with
/// an odd trim still has to start.
pub fn clamp_db(db: i32) -> i32 {
    db.clamp(MIN_DB, MAX_DB)
}

/// The linear amplitude factor of a setting in decibels.
pub fn factor(db: i32) -> f32 {
    if db == 0 {
        // Not just an optimisation: exactly 1.0, so an untouched channel is
        // bit-for-bit the audio the device delivered.
        return 1.0;
    }
    10f32.powf(db as f32 / 20.0)
}

/// One stream's trim, readable and writable from anywhere.
///
/// An atomic rather than a channel of changes, unlike the language: a knob
/// being dragged produces a change per pointer event, and the processing
/// thread only ever needs the value that is current when it next pumps — not
/// the fifty it passed through on the way there.
#[derive(Debug)]
pub struct Gain {
    db: AtomicI32,
}

impl Gain {
    pub fn new(db: i32) -> Self {
        Self {
            db: AtomicI32::new(clamp_db(db)),
        }
    }

    pub fn db(&self) -> i32 {
        self.db.load(Ordering::Relaxed)
    }

    pub fn set_db(&self, db: i32) {
        self.db.store(clamp_db(db), Ordering::Relaxed);
    }

    /// Scales a buffer in place.
    ///
    /// Clamped to the full-scale range. Whisper would take a sample of 4.0
    /// without complaining, but the peak meter beside the knob would then have
    /// no top to read against — and an operator turning a trim up needs to be
    /// told where "too far" is, by the meter, at the moment they pass it.
    pub fn apply(&self, samples: &mut [f32]) {
        let factor = factor(self.db());
        if factor == 1.0 {
            return;
        }
        for sample in samples {
            *sample = (*sample * factor).clamp(-1.0, 1.0);
        }
    }
}

impl Default for Gain {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_leaves_the_samples_alone() {
        let gain = Gain::new(0);
        let mut samples = [0.1, -0.25, 0.5];
        gain.apply(&mut samples);
        assert_eq!(samples, [0.1, -0.25, 0.5]);
    }

    #[test]
    fn six_db_is_about_twice_the_amplitude() {
        assert!((factor(6) - 1.9953).abs() < 0.001);
        assert!((factor(-6) - 0.5012).abs() < 0.001);
        assert!((factor(20) - 10.0).abs() < 0.001);
    }

    #[test]
    fn a_setting_out_of_range_is_clamped_not_refused() {
        assert_eq!(clamp_db(99), MAX_DB);
        assert_eq!(clamp_db(-99), MIN_DB);
        assert_eq!(Gain::new(99).db(), MAX_DB);
        let gain = Gain::new(0);
        gain.set_db(-99);
        assert_eq!(gain.db(), MIN_DB);
    }

    #[test]
    fn a_boost_stops_at_full_scale() {
        // Otherwise the peak meter this feeds has no top to read against.
        let gain = Gain::new(20);
        let mut samples = [0.5, -0.5, 0.01];
        gain.apply(&mut samples);
        assert_eq!(samples[0], 1.0);
        assert_eq!(samples[1], -1.0);
        assert!((samples[2] - 0.1).abs() < 0.001);
    }

    #[test]
    fn a_change_reaches_a_buffer_scaled_afterwards() {
        let gain = Gain::new(0);
        let mut first = [0.5];
        gain.apply(&mut first);
        gain.set_db(-6);
        let mut second = [0.5];
        gain.apply(&mut second);

        assert_eq!(first, [0.5]);
        assert!((second[0] - 0.2506).abs() < 0.001);
    }
}

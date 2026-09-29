//! A shoutbox that follows the room (P26): the signal processing.
//!
//! One channel of intercom is passed through to a talkback speaker, and its
//! level follows an SPL measurement microphone slowly — quiet in the quiet
//! moments of a show, above the room in the loud ones. What installation DSPs
//! call ambient noise compensation.
//!
//! Everything here is pure and testable without a device. The app crate
//! (`crates/app/src/outputs.rs`) opens the streams and runs [`Controller`] on a
//! thread of its own; this module has the parts that decide what is heard:
//!
//! - [`WeightingFilter`] — A-weighting, so the room is measured the way an
//!   SPL meter measures it;
//! - [`Controller`] — the measurement window, the gap gate, the curve and the
//!   slew limits;
//! - [`ShoutboxSource`] — the output callback: program from a ring, the gain
//!   ramped and a limiter last.
//!
//! **The shoutbox hears itself.** The measurement mic is in the same room, so
//! a louder shoutbox reads as a louder room and asks for more gain — a
//! positive loop that ends at the ceiling. The answer installation ANC uses,
//! and the one intercom makes natural, is to measure *in the gaps*: the room is
//! only measured while the shoutbox is silent, and the gain is frozen while it
//! plays. Intercom talk lasts seconds and a room changes over tens of them, so
//! the frozen gain is the right one — and the level never moves in the middle
//! of a sentence, only between them.

use crate::playback::SampleSource;
use crate::ring::RingSource;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Below this the reference reads as a mic that is not there rather than as
/// a quiet room: a real measurement mic and preamp hiss well above it. A
/// pulled cable read as a quiet room would turn the shoutbox down exactly in
/// the finale, so it is refused instead. Weighted dBFS.
pub const REFERENCE_FLOOR_DBFS: f32 = -100.0;

/// How long the reference may deliver nothing at all before that counts as a
/// failure rather than a scheduling hiccup.
const REFERENCE_SILENT_FOR: Duration = Duration::from_millis(500);

/// Time constant of the gain ramp in the output callback. The controller's
/// steps arrive every few tens of milliseconds; smoothed over this they are
/// inaudible rather than a staircase.
const RAMP_SECONDS: f32 = 0.05;

/// How fast the limiter lets go again.
const LIMITER_RELEASE_SECONDS: f32 = 0.2;

/// One second-order section, transposed direct form II.
#[derive(Debug, Clone)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    /// The bilinear transform of `(b0 + b1·s + b2·s²) / (a0 + a1·s + a2·s²)`.
    fn bilinear(analog_b: [f64; 3], analog_a: [f64; 3], rate: f64) -> Self {
        let k = 2.0 * rate;
        let [b0, b1, b2] = analog_b;
        let [a0, a1, a2] = analog_a;
        let d0 = a0 + a1 * k + a2 * k * k;
        Self {
            b0: (b0 + b1 * k + b2 * k * k) / d0,
            b1: (2.0 * b0 - 2.0 * b2 * k * k) / d0,
            b2: (b0 - b1 * k + b2 * k * k) / d0,
            a1: (2.0 * a0 - 2.0 * a2 * k * k) / d0,
            a2: (a0 - a1 * k + a2 * k * k) / d0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// |H| at `freq`, for normalising the cascade.
    fn magnitude(&self, freq: f64, rate: f64) -> f64 {
        let w = std::f64::consts::TAU * freq / rate;
        let (c1, s1) = (w.cos(), -w.sin());
        let (c2, s2) = ((2.0 * w).cos(), -(2.0 * w).sin());
        let num = (
            self.b0 + self.b1 * c1 + self.b2 * c2,
            self.b1 * s1 + self.b2 * s2,
        );
        let den = (
            1.0 + self.a1 * c1 + self.a2 * c2,
            self.a1 * s1 + self.a2 * s2,
        );
        (num.0.hypot(num.1)) / (den.0.hypot(den.1))
    }
}

/// A-weighting as a cascade of biquads, normalised to 0 dB at 1 kHz.
///
/// From the analog definition in IEC 61672: poles at 20.6 Hz (twice), 107.7 Hz,
/// 737.9 Hz and 12.2 kHz (twice), bilinear-transformed with each pole
/// prewarped. Class 1 accuracy is not the point — this steers a gain, it does
/// not certify a venue — but the shape has to be right, or a bass-heavy show
/// reads as far louder in dB(A) than any meter in the room would say.
pub struct WeightingFilter {
    sections: Vec<Biquad>,
    gain: f64,
}

impl WeightingFilter {
    pub fn new(rate: u32) -> Self {
        let rate = rate as f64;
        // Prewarped so each pole sits where the analog one does.
        let pole = |hz: f64| 2.0 * rate * (std::f64::consts::PI * hz / rate).tan();
        let (w1, w2, w3, w4) = (
            pole(20.598_997),
            pole(107.652_65),
            pole(737.862_23),
            pole(12_194.217),
        );

        let high_pass_pair =
            |w: f64| Biquad::bilinear([0.0, 0.0, 1.0], [w * w, 2.0 * w, 1.0], rate);
        let low_pass_pair = |w: f64| Biquad::bilinear([1.0, 0.0, 0.0], [w * w, 2.0 * w, 1.0], rate);

        let sections = vec![
            high_pass_pair(w1),
            Biquad::bilinear([0.0, 0.0, 1.0], [w2 * w3, w2 + w3, 1.0], rate),
            low_pass_pair(w4),
        ];

        let at_1k: f64 = sections
            .iter()
            .map(|s| s.magnitude(1_000.0, rate))
            .product();
        Self {
            sections,
            gain: if at_1k > 0.0 { 1.0 / at_1k } else { 1.0 },
        }
    }

    /// The weighted mean square of `samples`, or `None` for an empty block.
    pub fn mean_square(&mut self, samples: &[f32]) -> Option<f64> {
        if samples.is_empty() {
            return None;
        }
        let mut sum = 0.0f64;
        for &x in samples {
            let mut y = x as f64;
            for section in &mut self.sections {
                y = section.process(y);
            }
            y *= self.gain;
            sum += y * y;
        }
        Some(sum / samples.len() as f64)
    }
}

/// Decibels of a mean square, with silence as `-inf` rather than a panic.
pub fn db_of_mean_square(mean_square: f64) -> f32 {
    (10.0 * mean_square.log10()) as f32
}

/// Linear amplitude of a gain in decibels.
pub fn linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// The gain the room asks for: a line with two corners.
///
/// Below `quiet_db` the shoutbox plays at `base_gain_db`; above it the gain
/// rises by `slope` dB for every dB of room, up to `max_gain_db`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve {
    /// The room level, in the controller's units (calibrated dB, or dBFS
    /// when no calibration is set), below which nothing changes.
    pub quiet_db: f32,
    pub base_gain_db: f32,
    /// dB of gain per dB of room. 0 switches the following off; 1 keeps the
    /// shoutbox a fixed distance above the room.
    pub slope: f32,
    /// The most the curve will ever ask for. The limiter behind it is a
    /// second line, not a replacement.
    pub max_gain_db: f32,
}

impl Curve {
    pub fn target(&self, level_db: f32) -> f32 {
        let above = (level_db - self.quiet_db).max(0.0);
        let max = self.max_gain_db.max(self.base_gain_db);
        (self.base_gain_db + self.slope.clamp(0.0, 2.0) * above).min(max)
    }
}

/// Everything the controller is set by. See `crates/app/src/config.rs` for
/// what an operator sees of each.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlSettings {
    pub curve: Curve,
    /// How fast the gain may rise, in dB per second.
    pub rise_db_per_s: f32,
    /// How fast it may fall, in dB per second. Infinite is no limit: the
    /// gain then follows the averaged room down as fast as it moves.
    pub fall_db_per_s: f32,
    /// The averaging time constant of the room measurement.
    pub window: Duration,
    /// Added to weighted dBFS to read dB SPL: the calibration, or the
    /// estimate the app reads an uncalibrated mic through.
    pub calibration_db: f32,
    /// Above this pre-gain peak, in dBFS, the shoutbox counts as playing.
    pub gate_threshold_dbfs: f32,
    /// How long it keeps counting as playing after the last peak above the
    /// threshold — the tail of a word, and the room's own reverb of it.
    pub gate_hold: Duration,
}

/// What one step of the controller found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// The averaged room level in the controller's units, once anything has
    /// been measured. Measured in the gaps only, so it stands still while the
    /// shoutbox plays — this is what the gain follows.
    pub level_db: Option<f32>,
    /// The room as the mic hears it now, gate or not, averaged the same way.
    /// For the meter: includes the shoutbox itself while it plays, which is
    /// why the gain does not follow it.
    pub live_db: Option<f32>,
    /// What the curve asks for at that level.
    pub target_db: f32,
    /// The gain in force, which follows the target at the slew limits.
    pub gain_db: f32,
    /// Whether the room was measured in this step: false while the shoutbox
    /// plays (the gap gate) or the reference is unusable.
    pub measuring: bool,
    /// False when the reference reads as a missing mic. The gain is then held
    /// rather than followed down.
    pub reference_ok: bool,
}

/// The measurement and the control law, stepped from a thread of its own.
pub struct Controller {
    settings: ControlSettings,
    /// Exponentially averaged weighted mean square of the room, in the gaps.
    mean_square: Option<f64>,
    /// The same, ungated.
    live: Option<f64>,
    /// How much room has been averaged so far, until it reaches the window.
    filled: Duration,
    gain_db: f32,
    /// Whether the gain has been set from a full window yet. Before that it
    /// sits at the base gain; the first full window then sets it outright,
    /// so an app started into a loud room does not spend the next minute
    /// climbing to where it belongs.
    settled: bool,
    /// How long the gate stays shut after the last peak.
    hold_left: Duration,
    /// How long the reference has delivered nothing.
    silent_for: Duration,
}

impl Controller {
    pub fn new(settings: ControlSettings) -> Self {
        Self {
            gain_db: settings.curve.base_gain_db,
            settings,
            mean_square: None,
            live: None,
            filled: Duration::ZERO,
            settled: false,
            hold_left: Duration::ZERO,
            silent_for: Duration::ZERO,
        }
    }

    /// Takes new settings without forgetting the room: a curve moved during
    /// a show applies from the next step, at the slew limits.
    pub fn set_settings(&mut self, settings: ControlSettings) {
        self.settings = settings;
    }

    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    /// One step.
    ///
    /// `reference` is the weighted mean square of what the mic delivered
    /// since the last step, or `None` if it delivered nothing. `program_peak`
    /// is the largest pre-gain sample the shoutbox played in the same time,
    /// linear. `dt` is how long the step covers.
    pub fn step(&mut self, reference: Option<f64>, program_peak: f32, dt: Duration) -> Reading {
        let s = self.settings;

        // The gap gate. Pre-gain, so that "is the shoutbox playing" does not
        // depend on how loud it is currently being played.
        if program_peak > 0.0 && 20.0 * program_peak.log10() > s.gate_threshold_dbfs {
            self.hold_left = s.gate_hold;
        } else {
            self.hold_left = self.hold_left.saturating_sub(dt);
        }
        let gap = self.hold_left.is_zero();

        // Whether the reference is a mic at all.
        let reference_ok = match reference {
            None => {
                self.silent_for += dt;
                self.silent_for < REFERENCE_SILENT_FOR
            }
            Some(ms) => {
                self.silent_for = Duration::ZERO;
                ms > 0.0 && db_of_mean_square(ms) > REFERENCE_FLOOR_DBFS
            }
        };

        let window = s.window.as_secs_f64().max(0.05);
        let alpha = 1.0 - (-dt.as_secs_f64() / window).exp();
        let average = |previous: Option<f64>, ms: f64| {
            Some(match previous {
                None => ms,
                Some(previous) => previous + (ms - previous) * alpha,
            })
        };
        let usable = reference.filter(|_| reference_ok);
        if let Some(ms) = usable {
            self.live = average(self.live, ms);
        }
        let measuring = gap && usable.is_some();
        if let Some(ms) = usable.filter(|_| measuring) {
            self.mean_square = average(self.mean_square, ms);
            self.filled = (self.filled + dt).min(s.window);
        }

        let calibrated = |ms: f64| db_of_mean_square(ms) + s.calibration_db;
        let level_db = self.mean_square.map(calibrated);
        let live_db = self.live.map(calibrated);
        let target_db = match level_db {
            Some(level) => s.curve.target(level),
            None => s.curve.base_gain_db,
        };

        // Followed only while measuring: in a gap, and with a mic that is
        // there. While the shoutbox plays the gain is frozen, and with the
        // mic gone it is held where it was — towards audible, not down.
        if measuring {
            if !self.settled && self.filled >= s.window {
                self.gain_db = target_db;
                self.settled = true;
            } else if self.settled {
                let dt = dt.as_secs_f32();
                let delta = target_db - self.gain_db;
                let step = if delta > 0.0 {
                    delta.min(s.rise_db_per_s.max(0.0) * dt)
                } else {
                    delta.max(-s.fall_db_per_s.max(0.0) * dt)
                };
                self.gain_db += step;
            }
        }

        // A curve moved under the gain still bounds it: a lowered maximum is
        // in force from this step, not once the slew has caught up.
        let max = s.curve.max_gain_db.max(s.curve.base_gain_db);
        self.gain_db = self.gain_db.min(max);

        Reading {
            level_db,
            live_db,
            target_db,
            gain_db: self.gain_db,
            measuring,
            reference_ok,
        }
    }
}

/// What the shoutbox's output callback and its controller share.
///
/// Atomics only: one side is the realtime thread. `f32`s travel as their
/// bits.
#[derive(Debug)]
pub struct Shared {
    /// The linear gain the output ramps towards. 0 is mute.
    gain: AtomicU32,
    /// The largest pre-gain sample since the controller last read it.
    peak: AtomicU32,
    /// The deepest limiter gain since last read, linear.
    limiter: AtomicU32,
    /// Callbacks that found the program ring short.
    underruns: AtomicU64,
    /// Samples thrown away to keep the latency bounded.
    skipped: AtomicU64,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            gain: AtomicU32::new(0f32.to_bits()),
            peak: AtomicU32::new(0f32.to_bits()),
            limiter: AtomicU32::new(1f32.to_bits()),
            underruns: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
        }
    }
}

impl Shared {
    pub fn set_gain(&self, linear: f32) {
        self.gain
            .store(linear.max(0.0).to_bits(), Ordering::Relaxed);
    }

    /// The pre-gain peak since the last call, and resets it.
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0f32.to_bits(), Ordering::Relaxed))
    }

    /// The deepest limiter gain since the last call, in dB (0 = not
    /// limiting), and resets it.
    pub fn take_limiting_db(&self) -> f32 {
        let linear = f32::from_bits(self.limiter.swap(1f32.to_bits(), Ordering::Relaxed));
        20.0 * linear.max(1e-6).log10()
    }

    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    pub fn skipped(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }

    /// `fetch_max` on the bits is a max on the value: for non-negative
    /// floats the IEEE ordering and the integer ordering agree.
    fn note_peak(&self, peak: f32) {
        self.peak
            .fetch_max(peak.max(0.0).to_bits(), Ordering::Relaxed);
    }

    fn note_limiter(&self, gain: f32) {
        self.limiter
            .fetch_min(gain.max(0.0).to_bits(), Ordering::Relaxed);
    }
}

/// The output callback of the shoutbox.
///
/// Program from a ring the input callback fills, the gain ramped towards
/// what the controller set, and a peak limiter last, so that nothing leaves
/// above the ceiling whatever the curve asked for.
pub struct ShoutboxSource {
    program: RingSource,
    shared: Arc<Shared>,
    /// Whether enough program is buffered to play without running dry.
    primed: bool,
    /// Whether anything has been played yet. What piles up before that is
    /// the capture having opened before the output did — not a dropout, and
    /// not counted as one.
    started: bool,
    /// What has to be buffered before playing starts, beside one callback.
    prime: usize,
    /// More than this buffered is latency nobody asked for, and is skipped.
    ceiling_fill: usize,
    gain: f32,
    ramp: f32,
    limiter: f32,
    release: f32,
    ceiling: f32,
}

impl ShoutboxSource {
    /// `ceiling_dbfs` is the most any sample may leave at.
    pub fn new(program: RingSource, shared: Arc<Shared>, rate: u32, ceiling_dbfs: f32) -> Self {
        let rate = rate.max(1) as f32;
        Self {
            program,
            shared,
            primed: false,
            started: false,
            // 10 ms of margin over one callback, and at most 60 ms before
            // the excess is skipped: the pass-through's latency is bounded by
            // construction rather than by the clocks agreeing.
            prime: (rate / 100.0) as usize,
            ceiling_fill: (rate * 0.06) as usize,
            gain: 0.0,
            ramp: 1.0 - (-1.0 / (RAMP_SECONDS * rate)).exp(),
            limiter: 1.0,
            release: 1.0 - (-1.0 / (LIMITER_RELEASE_SECONDS * rate)).exp(),
            ceiling: linear(ceiling_dbfs.min(0.0)),
        }
    }
}

impl SampleSource for ShoutboxSource {
    fn fill(&mut self, out: &mut [f32]) {
        let need = out.len();
        let available = self.program.available();

        if !self.primed && available >= self.prime + need {
            self.primed = true;
        }
        if self.primed {
            let limit = self.ceiling_fill.max(self.prime + 2 * need);
            if available > limit {
                let excess = available - (self.prime + need);
                let skipped = self.program.skip(excess);
                if self.started {
                    self.shared
                        .skipped
                        .fetch_add(skipped as u64, Ordering::Relaxed);
                }
            }
            let got = self.program.read(out);
            self.started = true;
            if got < need {
                // Played what there was and starts priming again: a steady
                // trickle of short callbacks is worse than one gap.
                out[got..].fill(0.0);
                self.primed = false;
                self.shared.underruns.fetch_add(1, Ordering::Relaxed);
            }
        }

        let target = f32::from_bits(self.shared.gain.load(Ordering::Relaxed));
        let mut peak = 0.0f32;
        let mut deepest = 1.0f32;
        for sample in out.iter_mut() {
            peak = peak.max(sample.abs());
            self.gain += (target - self.gain) * self.ramp;
            let y = *sample * self.gain;
            let level = y.abs();
            if level * self.limiter > self.ceiling {
                self.limiter = self.ceiling / level;
            } else {
                self.limiter += (1.0 - self.limiter) * self.release;
            }
            deepest = deepest.min(self.limiter);
            // The limiter is instant, so this clamp only ever catches
            // rounding — and it is what makes the ceiling a promise.
            *sample = (y * self.limiter).clamp(-self.ceiling, self.ceiling);
        }
        self.shared.note_peak(peak);
        self.shared.note_limiter(deepest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::SampleSink;
    use crate::ring;
    use std::f32::consts::TAU;

    fn sine(freq: f32, rate: u32, seconds: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize)
            .map(|n| 0.5 * (TAU * freq * n as f32 / rate as f32).sin())
            .collect()
    }

    /// The weighting of `freq`, in dB, measured on a sine after the filter
    /// has settled.
    fn weighted_db(freq: f32) -> f32 {
        let rate = 48_000;
        let signal = sine(freq, rate, 2.0);
        let mut filter = WeightingFilter::new(rate);
        // Settle, then measure the second half.
        filter.mean_square(&signal[..rate as usize]);
        let weighted = filter.mean_square(&signal[rate as usize..]).unwrap();
        let flat = 0.125f64; // mean square of a 0.5 sine
        db_of_mean_square(weighted / flat)
    }

    #[test]
    fn a_weighting_has_the_shape_of_the_standard() {
        // IEC 61672 table values.
        for (freq, expected, tolerance) in [
            (1_000.0, 0.0, 0.1),
            (100.0, -19.1, 0.5),
            (31.5, -39.4, 1.0),
            (4_000.0, 1.0, 0.5),
            (10_000.0, -2.5, 1.5),
        ] {
            let got = weighted_db(freq);
            assert!(
                (got - expected).abs() <= tolerance,
                "A at {freq} Hz: {got:.2} dB, expected {expected}"
            );
        }
    }

    fn settings() -> ControlSettings {
        ControlSettings {
            curve: Curve {
                quiet_db: -40.0,
                base_gain_db: -12.0,
                slope: 1.0,
                max_gain_db: 0.0,
            },
            rise_db_per_s: 2.0,
            fall_db_per_s: 0.5,
            window: Duration::from_secs(2),
            calibration_db: 0.0,
            gate_threshold_dbfs: -50.0,
            gate_hold: Duration::from_millis(1_500),
        }
    }

    /// The mean square of a room at `dbfs`.
    fn room(dbfs: f32) -> Option<f64> {
        Some(10f64.powf(dbfs as f64 / 10.0))
    }

    const TICK: Duration = Duration::from_millis(20);

    /// Runs `seconds` of steps at one room level and program peak.
    fn run(c: &mut Controller, dbfs: f32, peak: f32, seconds: f32) -> Reading {
        let mut last = None;
        for _ in 0..(seconds / TICK.as_secs_f32()) as usize {
            last = Some(c.step(room(dbfs), peak, TICK));
        }
        last.unwrap()
    }

    #[test]
    fn the_curve_has_two_corners() {
        let curve = settings().curve;
        assert_eq!(curve.target(-60.0), -12.0);
        assert_eq!(curve.target(-40.0), -12.0);
        assert_eq!(curve.target(-30.0), -2.0);
        assert_eq!(curve.target(-10.0), 0.0);
    }

    #[test]
    fn a_quiet_room_keeps_the_base_gain_and_a_loud_one_raises_it() {
        let mut c = Controller::new(settings());
        let quiet = run(&mut c, -55.0, 0.0, 5.0);
        assert!((quiet.gain_db + 12.0).abs() < 0.01);

        let mut c = Controller::new(settings());
        let loud = run(&mut c, -32.0, 0.0, 5.0);
        // The first full window sets it outright.
        assert!((loud.gain_db + 4.0).abs() < 0.2, "{loud:?}");
    }

    #[test]
    fn it_rises_faster_than_it_falls() {
        let mut c = Controller::new(settings());
        run(&mut c, -55.0, 0.0, 3.0);
        // The room gets loud: 2 dB/s up, so a second later it is ~2 dB up —
        // the averaging window slows it further, never speeds it.
        let after = run(&mut c, -30.0, 0.0, 1.0);
        assert!(
            after.gain_db > -12.0 && after.gain_db <= -12.0 + 2.0 + 0.01,
            "{after:?}"
        );

        run(&mut c, -30.0, 0.0, 10.0);
        let top = c.gain_db();
        // And back down at a quarter of that.
        let after = run(&mut c, -55.0, 0.0, 1.0);
        assert!(top - after.gain_db <= 0.5 + 0.01, "{top} -> {after:?}");
    }

    #[test]
    fn an_unlimited_fall_follows_the_averaged_room_down() {
        let mut c = Controller::new(ControlSettings {
            fall_db_per_s: f32::INFINITY,
            ..settings()
        });
        run(&mut c, -30.0, 0.0, 10.0);
        let top = c.gain_db();
        // No slew limit on the way down: only the averaging window, whose
        // two seconds take the room most of the way back in three.
        let after = run(&mut c, -55.0, 0.0, 3.0);
        assert!(top - after.gain_db > 5.0, "{top} -> {after:?}");
        assert!(after.gain_db.is_finite() && after.gain_db >= -12.0);
        let settled = run(&mut c, -55.0, 0.0, 20.0);
        assert!((settled.gain_db + 12.0).abs() < 0.1, "{settled:?}");
    }

    #[test]
    fn the_gain_is_frozen_while_the_shoutbox_plays() {
        let mut c = Controller::new(settings());
        run(&mut c, -55.0, 0.0, 3.0);
        let before = c.gain_db();
        // Somebody talks for a minute, and the mic hears the shoutbox at a
        // level that would ask for the maximum — the loop the gate exists to
        // break.
        let held = c.step(room(-55.0), 0.0, TICK).level_db;
        let during = run(&mut c, -20.0, 0.5, 60.0);
        assert!(!during.measuring);
        assert_eq!(during.gain_db, before);
        // The level the gain follows stands still; the meter does not.
        assert_eq!(during.level_db, held);
        assert!((during.live_db.unwrap() + 20.0).abs() < 0.1, "{during:?}");
    }

    #[test]
    fn the_gate_holds_past_the_last_word() {
        let mut c = Controller::new(settings());
        run(&mut c, -55.0, 0.0, 3.0);
        c.step(room(-55.0), 0.5, TICK);
        let right_after = run(&mut c, -55.0, 0.0, 1.0);
        assert!(!right_after.measuring);
        let later = run(&mut c, -55.0, 0.0, 1.0);
        assert!(later.measuring);
    }

    #[test]
    fn a_pulled_mic_holds_the_gain_instead_of_dropping_it() {
        let mut c = Controller::new(settings());
        run(&mut c, -30.0, 0.0, 5.0);
        let before = c.gain_db();

        // Digital silence, then nothing at all.
        let silent = run(&mut c, f32::NEG_INFINITY, 0.0, 10.0);
        assert!(!silent.reference_ok);
        assert_eq!(silent.gain_db, before);

        let mut last = silent;
        for _ in 0..100 {
            last = c.step(None, 0.0, TICK);
        }
        assert!(!last.reference_ok);
        assert_eq!(last.gain_db, before);
    }

    #[test]
    fn a_lowered_maximum_applies_at_once() {
        let mut c = Controller::new(settings());
        run(&mut c, -20.0, 0.0, 5.0);
        let mut s = settings();
        s.curve.max_gain_db = -6.0;
        c.set_settings(s);
        let r = c.step(room(-20.0), 0.0, TICK);
        assert!(r.gain_db <= -6.0);
    }

    #[test]
    fn the_output_never_leaves_above_the_ceiling() {
        let rate = 48_000;
        let (mut sink, source) = ring::channel(rate, 1.0);
        let shared = Arc::new(Shared::default());
        // A gain far beyond anything sane, on a full-scale program.
        shared.set_gain(linear(24.0));
        let mut out = ShoutboxSource::new(source, shared.clone(), rate, -1.0);
        let ceiling = linear(-1.0);

        let program = sine(440.0, rate, 0.5);
        let mut worst = 0.0f32;
        for chunk in program.chunks(512) {
            sink.push(chunk);
            let mut buffer = vec![0.0; 256];
            out.fill(&mut buffer);
            worst = buffer.iter().fold(worst, |w, s| w.max(s.abs()));
        }
        assert!(worst <= ceiling + 1e-6, "{worst} > {ceiling}");
        assert!(shared.take_limiting_db() < 0.0);
    }

    #[test]
    fn the_pass_through_waits_for_a_margin_and_bounds_its_latency() {
        let rate = 48_000;
        let (mut sink, source) = ring::channel(rate, 1.0);
        let shared = Arc::new(Shared::default());
        shared.set_gain(1.0);
        let mut out = ShoutboxSource::new(source, shared.clone(), rate, 0.0);

        // Not enough yet: silence, and nothing consumed.
        sink.push(&[0.1; 100]);
        let mut buffer = vec![0.0; 256];
        out.fill(&mut buffer);
        assert!(buffer.iter().all(|&s| s == 0.0));

        // What piles up before anything played is the capture having opened
        // first: skipped, and not counted as a dropout.
        sink.push(&vec![0.1; rate as usize / 2]);
        out.fill(&mut buffer);
        assert_eq!(shared.skipped(), 0);
        assert!(out.program.available() < (rate as f32 * 0.06) as usize);

        // Half a second piling up once it plays is latency, and is skipped
        // rather than played half a second late — and counted.
        sink.push(&vec![0.1; rate as usize / 2]);
        out.fill(&mut buffer);
        assert!(shared.skipped() > 0);
        assert!(out.program.available() < (rate as f32 * 0.06) as usize);
    }
}

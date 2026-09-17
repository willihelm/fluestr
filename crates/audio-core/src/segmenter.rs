//! Cutting a continuous stream into segments worth transcribing.
//!
//! Fixed windows are the obvious approach and the wrong one: they cut words in
//! half, and whisper has no way to know that the fragment it got was a
//! fragment. So the cut follows the speech instead — start on onset, end on a
//! pause — with a hard ceiling so that one person talking without a break does
//! not hold the whole pipeline hostage.
//!
//! Two details carry most of the quality:
//!
//! - **Pre-roll.** The detector only knows speech started once it is already
//!   underway, so the segment reaches back into the buffer. Without it every
//!   segment loses its first consonant.
//! - **Cutting at the quietest point.** When the ceiling forces a cut, the
//!   boundary is placed at the lowest-energy frame of the recent past rather
//!   than exactly on the limit — which is far more likely to fall between two
//!   words than inside one.

use crate::lang::LanguageMode;
use crate::segment::{Segment, StreamId, SAMPLE_RATE};
use crate::vad::{rms, Vad};
use std::collections::VecDeque;
use std::time::Duration;

/// Thresholds for one stream. All of them are settings (P12); the defaults are
/// the plan's starting points, not measured optima.
#[derive(Debug, Clone)]
pub struct SegmenterConfig {
    /// Audio kept before the detected onset.
    pub pre_roll: Duration,
    /// Silence that ends a segment.
    pub silence_timeout: Duration,
    /// Hard ceiling. A speaker who never pauses would otherwise push the
    /// latency past anything useful.
    pub max_segment: Duration,
    /// Anything shorter is discarded — clicks, coughs, a knock on the desk.
    pub min_segment: Duration,
    pub language_mode: LanguageMode,
    /// Vocabulary prompt (P15).
    pub initial_prompt: Option<String>,
}

impl Default for SegmenterConfig {
    fn default() -> Self {
        Self {
            pre_roll: Duration::from_millis(300),
            silence_timeout: Duration::from_millis(500),
            max_segment: Duration::from_secs(6),
            min_segment: Duration::from_millis(300),
            language_mode: LanguageMode::default(),
            initial_prompt: None,
        }
    }
}

fn samples(duration: Duration) -> usize {
    (duration.as_secs_f64() * SAMPLE_RATE as f64) as usize
}

/// Turns a 16 kHz mono stream into [`Segment`]s.
///
/// One instance per stream: everything here is per-stream state, and the
/// sequence numbers the result aggregator (P5) relies on are per stream too.
pub struct Segmenter {
    stream_id: StreamId,
    vad: Box<dyn Vad>,
    config: SegmenterConfig,

    frame: usize,
    pre_roll_capacity: usize,
    silence_frames: usize,
    max_samples: usize,
    min_samples: usize,
    /// How far back a forced cut looks for a quiet spot.
    flush_window_frames: usize,

    /// Samples that do not yet complete a frame.
    pending: Vec<f32>,
    scratch: Vec<f32>,
    /// Recent audio, kept for the pre-roll.
    history: VecDeque<f32>,

    speaking: bool,
    /// Audio of the segment being built, pre-roll included.
    current: Vec<f32>,
    /// How much of `current` is pre-roll — needed to map a frame index back to
    /// a sample offset.
    current_pre_roll: usize,
    /// RMS per analysed frame of `current`, for the forced cut.
    frame_levels: Vec<f32>,
    silence_run: usize,

    seq: u64,
    /// Samples seen since the session started; the basis for the timestamps.
    consumed: u64,
    /// Sample index at which `current` begins.
    current_start: u64,
}

impl Segmenter {
    pub fn new(stream_id: StreamId, vad: Box<dyn Vad>, config: SegmenterConfig) -> Self {
        let frame = vad.frame_samples().max(1);
        let silence_frames = (samples(config.silence_timeout) / frame).max(1);
        Self {
            stream_id,
            frame,
            pre_roll_capacity: samples(config.pre_roll),
            silence_frames,
            max_samples: samples(config.max_segment).max(frame),
            min_samples: samples(config.min_segment),
            // The plan's "quietest point in the last 500 ms".
            flush_window_frames: silence_frames.max(1),
            pending: Vec::with_capacity(4 * frame),
            scratch: Vec::with_capacity(frame),
            history: VecDeque::with_capacity(samples(config.pre_roll) + frame),
            speaking: false,
            current: Vec::new(),
            current_pre_roll: 0,
            frame_levels: Vec::new(),
            silence_run: 0,
            seq: 0,
            consumed: 0,
            current_start: 0,
            vad,
            config,
        }
    }

    /// Feeds 16 kHz mono samples and returns whatever segments completed.
    ///
    /// Usually empty — a segment finishes every few seconds at most, so
    /// returning a `Vec` costs an allocation on the rare call rather than on
    /// every one.
    pub fn push(&mut self, input: &[f32]) -> Vec<Segment> {
        let mut finished = Vec::new();
        self.pending.extend_from_slice(input);

        // Taken out and put back so the borrow checker sees the frame buffer
        // and `self` as separate; nothing is allocated in the steady state.
        let mut scratch = std::mem::take(&mut self.scratch);
        while self.pending.len() >= self.frame {
            scratch.clear();
            scratch.extend_from_slice(&self.pending[..self.frame]);
            self.pending.drain(..self.frame);
            self.process_frame(&scratch, &mut finished);
        }
        self.scratch = scratch;

        finished
    }

    /// Closes whatever is open, for the end of a session.
    ///
    /// Applies the minimum length like any other segment: the tail of a
    /// stopped session is not more interesting than a cough was.
    pub fn finish(&mut self) -> Option<Segment> {
        if !self.speaking {
            return None;
        }
        self.speaking = false;
        self.close_current(self.current.len())
    }

    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// Segments emitted so far. Also the next sequence number.
    pub fn emitted(&self) -> u64 {
        self.seq
    }

    fn process_frame(&mut self, frame: &[f32], finished: &mut Vec<Segment>) {
        let level = rms(frame);
        let speech = self.vad.is_speech(frame);
        self.consumed += frame.len() as u64;

        if !self.speaking {
            if speech {
                self.speaking = true;
                self.current.clear();
                self.frame_levels.clear();
                self.silence_run = 0;

                self.current_pre_roll = self.history.len();
                self.current.extend(self.history.iter().copied());
                self.current_start =
                    self.consumed - frame.len() as u64 - self.current_pre_roll as u64;

                self.current.extend_from_slice(frame);
                self.frame_levels.push(level);
            }
        } else {
            self.current.extend_from_slice(frame);
            self.frame_levels.push(level);

            if speech {
                self.silence_run = 0;
            } else {
                self.silence_run += 1;
            }

            if self.silence_run >= self.silence_frames {
                self.speaking = false;
                // Keep a short tail so the last word does not end abruptly,
                // but not the whole pause — that is only decode time.
                let trim = self.silence_run.saturating_sub(2) * self.frame;
                let keep = self.current.len().saturating_sub(trim);
                if let Some(segment) = self.close_current(keep) {
                    finished.push(segment);
                }
            } else if self.current.len() >= self.max_samples {
                if let Some(segment) = self.force_flush() {
                    finished.push(segment);
                }
            }
        }

        self.remember(frame);
    }

    /// Emits `current[..keep]` as a segment and resets the state.
    ///
    /// `None` when the result is below the minimum length. The sequence number
    /// is only spent on a segment that is actually emitted, so a gap in the
    /// sequence always means a lost segment rather than a discarded one.
    fn close_current(&mut self, keep: usize) -> Option<Segment> {
        let keep = keep.min(self.current.len());
        let pcm: Vec<f32> = self.current[..keep].to_vec();
        self.current.clear();
        self.frame_levels.clear();
        self.current_pre_roll = 0;
        self.silence_run = 0;

        if pcm.len() < self.min_samples {
            return None;
        }

        let start = self.current_start;
        let segment = self.build(pcm, start);
        Some(segment)
    }

    /// Cuts at the quietest frame of the recent past when the ceiling is hit.
    ///
    /// Whatever follows the cut stays in the buffer and becomes the start of
    /// the next segment — the speaker has not stopped talking, after all.
    fn force_flush(&mut self) -> Option<Segment> {
        let first = self
            .frame_levels
            .len()
            .saturating_sub(self.flush_window_frames);
        let cut_frame = (first..self.frame_levels.len())
            .min_by(|&a, &b| {
                self.frame_levels[a]
                    .partial_cmp(&self.frame_levels[b])
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(self.frame_levels.len().saturating_sub(1));

        let cut = (self.current_pre_roll + (cut_frame + 1) * self.frame).min(self.current.len());

        let pcm: Vec<f32> = self.current[..cut].to_vec();
        let rest: Vec<f32> = self.current[cut..].to_vec();
        let start = self.current_start;

        self.current = rest;
        self.frame_levels.drain(..=cut_frame);
        self.current_pre_roll = 0;
        self.current_start = start + cut as u64;

        if pcm.len() < self.min_samples {
            return None;
        }
        Some(self.build(pcm, start))
    }

    fn build(&mut self, pcm: Vec<f32>, start_sample: u64) -> Segment {
        let seq = self.seq;
        self.seq += 1;
        let end_sample = start_sample + pcm.len() as u64;
        Segment {
            stream_id: self.stream_id,
            seq,
            t_start: at(start_sample),
            t_end: at(end_sample),
            pcm,
            language_mode: self.config.language_mode.clone(),
            initial_prompt: self.config.initial_prompt.clone(),
        }
    }

    fn remember(&mut self, frame: &[f32]) {
        if self.pre_roll_capacity == 0 {
            return;
        }
        self.history.extend(frame.iter().copied());
        while self.history.len() > self.pre_roll_capacity {
            self.history.pop_front();
        }
    }
}

fn at(sample: u64) -> Duration {
    Duration::from_secs_f64(sample as f64 / SAMPLE_RATE as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vad::EnergyVad;
    use std::f32::consts::TAU;

    fn tone(seconds: f32, amplitude: f32) -> Vec<f32> {
        let n = (SAMPLE_RATE as f32 * seconds) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                amplitude * (TAU * 220.0 * t).sin()
            })
            .collect()
    }

    fn silence(seconds: f32) -> Vec<f32> {
        vec![0.0; (SAMPLE_RATE as f32 * seconds) as usize]
    }

    fn segmenter(config: SegmenterConfig) -> Segmenter {
        Segmenter::new(StreamId(0), Box::new(EnergyVad::new(SAMPLE_RATE)), config)
    }

    /// Feeds a whole signal in realistic block sizes.
    fn run(segmenter: &mut Segmenter, signal: &[f32]) -> Vec<Segment> {
        let mut out = Vec::new();
        for block in signal.chunks(1600) {
            out.extend(segmenter.push(block));
        }
        out
    }

    #[test]
    fn a_pause_separates_two_segments() {
        let mut seg = segmenter(SegmenterConfig::default());
        let mut signal = silence(0.5);
        signal.extend(tone(1.0, 0.3));
        signal.extend(silence(1.0));
        signal.extend(tone(1.0, 0.3));
        signal.extend(silence(1.0));

        let segments = run(&mut seg, &signal);

        assert_eq!(segments.len(), 2, "one segment per burst of speech");
        assert_eq!(segments[0].seq, 0);
        assert_eq!(segments[1].seq, 1);
        // The second burst starts at 2.5 s, the pre-roll reaches 300 ms back.
        let start = segments[1].t_start.as_millis() as i64;
        assert!(
            (2_150..=2_500).contains(&start),
            "second segment starts at {start} ms"
        );
    }

    #[test]
    fn the_pre_roll_reaches_back_before_the_onset() {
        // The detector cannot fire until speech is already audible, so without
        // a pre-roll the first consonant is gone.
        let mut seg = segmenter(SegmenterConfig::default());
        let mut signal = silence(0.5);
        signal.extend(tone(1.0, 0.3));
        signal.extend(silence(1.0));

        let segments = run(&mut seg, &signal);
        assert_eq!(segments.len(), 1);

        // Speech begins at 500 ms; with 300 ms of pre-roll the segment has to
        // start earlier than that.
        let start = segments[0].t_start.as_millis() as i64;
        assert!(start < 500, "segment starts at {start} ms, expected < 500");
        assert!(start >= 150, "pre-roll is bounded, got {start} ms");
    }

    #[test]
    fn a_click_is_discarded() {
        let mut seg = segmenter(SegmenterConfig::default());
        let mut signal = silence(0.5);
        signal.extend(tone(0.05, 0.5)); // 50 ms, below the 300 ms minimum
        signal.extend(silence(1.0));

        let segments = run(&mut seg, &signal);
        // The pre-roll is silence, so even with it the result stays short.
        assert!(
            segments.is_empty() || segments[0].pcm_duration() >= Duration::from_millis(300),
            "a 50 ms click must not become a segment"
        );
    }

    #[test]
    fn a_continuous_talker_is_cut_at_the_ceiling() {
        let config = SegmenterConfig {
            max_segment: Duration::from_secs(2),
            ..Default::default()
        };
        let mut seg = segmenter(config);
        let mut signal = silence(0.3);
        signal.extend(tone(7.0, 0.3));
        signal.extend(silence(1.0));

        let segments = run(&mut seg, &signal);

        assert!(
            segments.len() >= 3,
            "7 s of unbroken speech with a 2 s ceiling should yield at least 3 segments, got {}",
            segments.len()
        );
        for segment in &segments {
            assert!(
                segment.pcm_duration() <= Duration::from_millis(2_100),
                "segment of {:?} exceeds the ceiling",
                segment.pcm_duration()
            );
        }
    }

    #[test]
    fn forced_cuts_leave_no_gap_in_the_audio() {
        // The remainder after a forced cut has to start the next segment,
        // otherwise a word disappears at every ceiling.
        let config = SegmenterConfig {
            max_segment: Duration::from_secs(2),
            ..Default::default()
        };
        let mut seg = segmenter(config);
        let mut signal = silence(0.3);
        signal.extend(tone(5.0, 0.3));
        signal.extend(silence(1.0));

        let segments = run(&mut seg, &signal);
        assert!(segments.len() >= 2);

        for pair in segments.windows(2) {
            assert_eq!(
                pair[0].t_end, pair[1].t_start,
                "a forced cut must be seamless"
            );
        }
    }

    #[test]
    fn silence_alone_produces_nothing() {
        let mut seg = segmenter(SegmenterConfig::default());
        assert!(run(&mut seg, &silence(5.0)).is_empty());
        assert!(seg.finish().is_none());
        assert_eq!(seg.emitted(), 0);
    }

    #[test]
    fn finish_closes_a_segment_that_is_still_open() {
        let mut seg = segmenter(SegmenterConfig::default());
        let mut signal = silence(0.3);
        signal.extend(tone(1.0, 0.3));

        let during = run(&mut seg, &signal);
        assert!(during.is_empty(), "no pause yet, so nothing is closed");

        let tail = seg.finish().expect("the open segment should be emitted");
        assert!(tail.pcm_duration() >= Duration::from_millis(900));
        assert!(seg.finish().is_none(), "nothing left to close");
    }

    #[test]
    fn timestamps_match_the_sample_count() {
        let mut seg = segmenter(SegmenterConfig::default());
        let mut signal = silence(0.5);
        signal.extend(tone(1.0, 0.3));
        signal.extend(silence(1.0));

        let segments = run(&mut seg, &signal);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].duration(), segments[0].pcm_duration());
    }

    #[test]
    fn the_configured_language_and_prompt_travel_with_the_segment() {
        let config = SegmenterConfig {
            language_mode: LanguageMode::default(),
            // German on purpose: this is vocabulary data, not code.
            initial_prompt: Some("Intercom-Kommandos: Standby, Cue, Go.".into()),
            ..Default::default()
        };
        let mut seg = segmenter(config);
        let mut signal = silence(0.3);
        signal.extend(tone(1.0, 0.3));
        signal.extend(silence(1.0));

        let segments = run(&mut seg, &signal);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].language_mode.whisper_language(), Some("de"));
        assert!(segments[0]
            .initial_prompt
            .as_deref()
            .unwrap()
            .contains("Standby"));
    }

    #[test]
    fn block_size_does_not_change_the_segmentation() {
        // The processing thread cannot control how much audio it gets per pump.
        let mut signal = silence(0.5);
        signal.extend(tone(1.0, 0.3));
        signal.extend(silence(1.0));
        signal.extend(tone(0.8, 0.3));
        signal.extend(silence(1.0));

        let mut small = segmenter(SegmenterConfig::default());
        let mut large = segmenter(SegmenterConfig::default());

        let mut a = Vec::new();
        for block in signal.chunks(97) {
            a.extend(small.push(block));
        }
        let b = large.push(&signal);

        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.t_start, y.t_start);
            assert_eq!(x.pcm.len(), y.pcm.len());
        }
    }
}

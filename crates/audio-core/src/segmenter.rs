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
//!   words than inside one. Frames the detector called silence are preferred
//!   over merely quiet ones: a soft vowel has a low level too.
//! - **Overlap across a forced cut.** A cut that lands inside a word leaves
//!   half of it at the end of one segment and half at the start of the next,
//!   and whisper gets neither. So the continuation reaches back across the
//!   cut, the same way the pre-roll reaches back before an onset. The word is
//!   then whole in at least one of the two segments, at the price of the
//!   overlap being transcribed twice. That trade is deliberate: a duplicated
//!   word is readable, a bisected one is not.

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
    /// Audio a forced cut leaves to *both* sides of the boundary.
    ///
    /// Long enough to contain a whole word, because the point is that a word
    /// the cut runs through survives intact on the later side. It comes out of
    /// `max_segment` rather than adding to it — the segment stays within the
    /// ceiling, it just advances less far.
    pub forced_overlap: Duration,
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
            forced_overlap: Duration::from_millis(500),
            min_segment: Duration::from_millis(300),
            language_mode: LanguageMode::default(),
            initial_prompt: None,
        }
    }
}

/// Fade applied to either side of a forced cut.
///
/// Cutting a waveform at an arbitrary sample leaves a step to zero, which is a
/// click — broadband, and right at the edge where whisper is least anchored.
/// Five milliseconds is inaudible as a fade and removes it.
const FADE: Duration = Duration::from_millis(5);

/// What the cut-point search knows about one analysed frame.
#[derive(Debug, Clone, Copy)]
struct FrameStat {
    level: f32,
    speech: bool,
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
    /// Audio carried across a forced cut into the next segment.
    overlap_samples: usize,
    fade_samples: usize,

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
    /// One entry per analysed frame of `current`, for the forced cut.
    frames: Vec<FrameStat>,
    silence_run: usize,
    /// Whether `current` began at a forced cut rather than at an onset. Such a
    /// segment starts mid-waveform and needs the fade.
    continuation: bool,

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
            // Bounded by the ceiling: an overlap as long as the segment would
            // never let the stream advance.
            overlap_samples: samples(config.forced_overlap).min(samples(config.max_segment) / 2),
            fade_samples: samples(FADE),
            pending: Vec::with_capacity(4 * frame),
            scratch: Vec::with_capacity(frame),
            history: VecDeque::with_capacity(samples(config.pre_roll) + frame),
            speaking: false,
            current: Vec::new(),
            current_pre_roll: 0,
            frames: Vec::new(),
            silence_run: 0,
            continuation: false,
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

    /// Puts this stream onto another language while it is running.
    ///
    /// The mode is read when a segment is *closed*, so a change lands on the
    /// next segment to finish — including one that is being spoken right now.
    /// That is the right end of the choice: somebody who switches language
    /// mid-sentence is telling you about the sentence they are in the middle
    /// of, not about the one after it.
    ///
    /// Nothing else about the segmenter changes. Speech detection does not
    /// know what language it is listening to, so the boundaries this finds are
    /// the same either way.
    pub fn set_language_mode(&mut self, mode: LanguageMode) {
        self.config.language_mode = mode;
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
                self.frames.clear();
                self.silence_run = 0;
                self.continuation = false;

                self.current_pre_roll = self.history.len();
                self.current.extend(self.history.iter().copied());
                self.current_start =
                    self.consumed - frame.len() as u64 - self.current_pre_roll as u64;

                self.current.extend_from_slice(frame);
                self.frames.push(FrameStat { level, speech });
            }
        } else {
            self.current.extend_from_slice(frame);
            self.frames.push(FrameStat { level, speech });

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
        let mut pcm: Vec<f32> = self.current[..keep].to_vec();
        // A segment that began at a forced cut starts mid-waveform; one that
        // began at an onset starts in the pre-roll and needs nothing. The end
        // is a real pause either way.
        if self.continuation {
            self.fade_in(&mut pcm);
        }
        self.current.clear();
        self.frames.clear();
        self.current_pre_roll = 0;
        self.silence_run = 0;
        self.continuation = false;

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
    /// the next segment — the speaker has not stopped talking, after all — and
    /// so does `forced_overlap` worth of audio from *before* it, so that a word
    /// the boundary runs through is whole on at least one side.
    fn force_flush(&mut self) -> Option<Segment> {
        let cut_frame = self.cut_frame();
        let cut = (self.current_pre_roll + (cut_frame + 1) * self.frame).min(self.current.len());
        // Reaching into the pre-roll is fine: it is ordinary audio, and the
        // only thing that makes it pre-roll is which segment claimed it first.
        let carry = cut.saturating_sub(self.overlap_samples);

        let mut pcm: Vec<f32> = self.current[..cut].to_vec();
        let rest: Vec<f32> = self.current[carry..].to_vec();
        let start = self.current_start;

        if self.continuation {
            self.fade_in(&mut pcm);
        }
        self.fade_out(&mut pcm);

        self.current = rest;
        self.frames.drain(..=cut_frame);
        // The carried audio is pre-roll for the next segment in exactly the
        // sense the onset path means it: samples that precede the first frame
        // `frames` describes.
        self.current_pre_roll = cut - carry;
        self.current_start = start + carry as u64;
        self.continuation = true;

        if pcm.len() < self.min_samples {
            return None;
        }
        Some(self.build(pcm, start))
    }

    /// The frame of `frames` to cut after.
    ///
    /// Within the search window, a frame the detector called silence beats a
    /// merely quiet one — a soft vowel has a low level too, and cutting inside
    /// it is exactly what this is trying to avoid. Among equals the quietest
    /// wins, and ties go to the earliest, which leaves the boundary as far as
    /// possible from the ceiling.
    fn cut_frame(&self) -> usize {
        let last = self.frames.len().saturating_sub(1);
        let window = self.frames.len().saturating_sub(self.flush_window_frames)..self.frames.len();
        let has_silence = window.clone().any(|i| !self.frames[i].speech);

        window
            .filter(|&i| !has_silence || !self.frames[i].speech)
            .min_by(|&a, &b| {
                self.frames[a]
                    .level
                    .partial_cmp(&self.frames[b].level)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(last)
    }

    /// Ramps the first [`FADE`] of a segment up from zero.
    fn fade_in(&self, pcm: &mut [f32]) {
        let n = self.fade_samples.min(pcm.len());
        for (i, sample) in pcm[..n].iter_mut().enumerate() {
            *sample *= i as f32 / n as f32;
        }
    }

    /// Ramps the last [`FADE`] of a segment down to zero.
    fn fade_out(&self, pcm: &mut [f32]) {
        let n = self.fade_samples.min(pcm.len());
        let start = pcm.len() - n;
        for (i, sample) in pcm[start..].iter_mut().enumerate() {
            *sample *= 1.0 - (i as f32 / n as f32);
        }
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
    use crate::lang::LanguageCode;
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
            assert!(
                pair[1].t_start <= pair[0].t_end,
                "a forced cut must not drop audio: {:?} then {:?}",
                pair[0].t_end,
                pair[1].t_start
            );
        }
    }

    #[test]
    fn a_forced_cut_hands_the_overlap_to_the_next_segment() {
        // The point of the overlap: a word the boundary runs through is whole
        // on the later side, even though it is transcribed twice.
        let config = SegmenterConfig {
            max_segment: Duration::from_secs(2),
            forced_overlap: Duration::from_millis(400),
            ..Default::default()
        };
        let mut seg = segmenter(config);
        let mut signal = silence(0.3);
        signal.extend(tone(5.0, 0.3));
        signal.extend(silence(1.0));

        let segments = run(&mut seg, &signal);
        assert!(segments.len() >= 2);

        for pair in segments.windows(2) {
            let overlap = pair[0].t_end.saturating_sub(pair[1].t_start);
            assert!(
                (Duration::from_millis(340)..=Duration::from_millis(400)).contains(&overlap),
                "expected ~400 ms of overlap, got {overlap:?}"
            );
        }
    }

    #[test]
    fn the_overlap_does_not_push_a_segment_past_the_ceiling() {
        // It comes out of `max_segment`, not on top of it — the ceiling is a
        // latency promise and the overlap must not quietly break it.
        let config = SegmenterConfig {
            max_segment: Duration::from_secs(2),
            forced_overlap: Duration::from_millis(500),
            ..Default::default()
        };
        let mut seg = segmenter(config);
        let mut signal = silence(0.3);
        signal.extend(tone(7.0, 0.3));
        signal.extend(silence(1.0));

        for segment in run(&mut seg, &signal) {
            assert!(
                segment.pcm_duration() <= Duration::from_millis(2_100),
                "segment of {:?} exceeds the ceiling",
                segment.pcm_duration()
            );
        }
    }

    #[test]
    fn a_forced_cut_prefers_a_gap_the_detector_found() {
        // A quiet passage that is still speech is a bad place to cut. When the
        // search window holds a real gap, the boundary belongs there even if
        // some spoken frame happens to be quieter.
        let config = SegmenterConfig {
            max_segment: Duration::from_secs(2),
            // No overlap, so the boundary is readable straight off the
            // timestamps.
            forced_overlap: Duration::ZERO,
            ..Default::default()
        };
        let mut seg = segmenter(config);
        let mut signal = silence(0.3);
        // A gap at 1.7 s, well inside the 500 ms search window before the 2 s
        // ceiling, then loud speech again right up to it.
        signal.extend(tone(1.4, 0.3));
        signal.extend(silence(0.1));
        signal.extend(tone(4.0, 0.3));
        signal.extend(silence(1.0));

        let segments = run(&mut seg, &signal);
        assert!(segments.len() >= 2);

        let boundary = segments[0].t_end.as_millis() as i64;
        assert!(
            (1_650..=1_850).contains(&boundary),
            "the cut should land in the gap around 1.7 s, got {boundary} ms"
        );
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

    /// The language of a running stream can be changed without rebuilding
    /// anything (P6). A guest who speaks English arrives on a channel that is
    /// already open, and tearing down capture to say so is not an option.
    #[test]
    fn the_language_can_be_changed_while_the_stream_runs() {
        let mut seg = segmenter(SegmenterConfig::default());
        let mut signal = silence(0.3);
        signal.extend(tone(1.0, 0.3));
        signal.extend(silence(1.0));

        let before = run(&mut seg, &signal);
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].language_mode.whisper_language(), Some("de"));

        seg.set_language_mode(LanguageMode::Fixed {
            language: LanguageCode::new("en"),
        });

        let after = run(&mut seg, &signal);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].language_mode.whisper_language(), Some("en"));
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

//! Silero speech detection, in front of the segmenter (P3).
//!
//! [`EnergyVad`](crate::EnergyVad) measures loudness, and loudness is all it
//! can measure: a bell, a chair scrape and a spoken word look the same to it.
//! Silero models speech, so it is the only thing in the chain that can answer
//! the question the segmenter actually asks.
//!
//! Running it here rather than only in the sidecar (P4) saves two things: the
//! IPC round trip, and every decode spent on a segment that was never speech —
//! measured at 2 of 11 segments in a live session, each costing a full ~320 ms
//! decode before the text filter threw it away.
//!
//! # Why a rolling context, and not just the frame
//!
//! Silero is an LSTM, and `whisper_vad_detect_speech` clears its hidden state
//! at the start of *every* call. A call with one frame in it therefore judges
//! that frame with no memory of the syllable it belongs to. So each call is
//! handed the preceding [`CONTEXT_WINDOWS`] frames as well and only the last
//! probability — the one belonging to the current frame — is read. The
//! network sees a running context; the segmenter still gets one answer per
//! frame.
//!
//! The extra windows are what this costs: one call evaluates
//! `CONTEXT_WINDOWS + 1` of them instead of one. A window is well under a
//! tenth of a millisecond, which buys the accuracy cheaply — see the
//! `runs_far_faster_than_realtime` test.
//!
//! # Failing open
//!
//! If the model refuses to run, a frame counts as speech. The gate in the
//! sidecar is still behind this one, so the cost of failing open is a wasted
//! decode; the cost of failing closed is an operator who speaks and is never
//! transcribed. Silent loss is the one failure this project does not accept.

use crate::vad::Vad;
use crate::{AudioError, Result};
use std::collections::VecDeque;
use std::path::Path;
use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

/// How many preceding frames are replayed to warm the LSTM before the current
/// one is judged. Four windows are ~128 ms at 16 kHz — about a syllable, which
/// is the context the decision needs.
pub const CONTEXT_WINDOWS: usize = 4;

/// Probability above which a frame starts speech.
pub const DEFAULT_ONSET: f32 = 0.5;

/// Probability below which speech ends. Lower than the onset on purpose: the
/// gap inside a word must not end a segment. The segmenter's `silence_timeout`
/// is the second, coarser half of the same hysteresis.
pub const DEFAULT_RELEASE: f32 = 0.35;

/// Fallback window size if the model cannot be probed for its own. Silero at
/// 16 kHz uses 512 samples (32 ms); the value is read from the model file
/// rather than compiled in, so this is only a last resort.
const FALLBACK_WINDOW: usize = 512;

/// Samples used to probe the model's window size. A large power of two, so the
/// division below is exact for every plausible window.
const PROBE_SAMPLES: usize = 16_384;

/// Speech detection with the Silero model that whisper.cpp ships.
///
/// One instance per stream: the context buffer and the hysteresis are
/// per-stream state, and so is the model context itself.
pub struct SileroVad {
    ctx: WhisperVadContext,
    /// The model's own window size, probed rather than assumed.
    window: usize,
    context_windows: usize,
    /// The frames replayed in front of the current one.
    history: VecDeque<f32>,
    /// Context plus current frame; reused so the steady state allocates
    /// nothing.
    buffer: Vec<f32>,
    speaking: bool,
    onset: f32,
    release: f32,
    last_probability: f32,
    failures: u64,
}

impl SileroVad {
    /// Loads the Silero model. One thread: a call evaluates a handful of tiny
    /// windows, and splitting that across threads costs more in
    /// synchronisation than it saves — with eight streams, each with its own
    /// context, it would also oversubscribe the machine the decoders need.
    pub fn new(model: &Path) -> Result<Self> {
        Self::with_threads(model, 1)
    }

    pub fn with_threads(model: &Path, threads: i32) -> Result<Self> {
        // Otherwise whisper.cpp and ggml write on their own file descriptors.
        // Here that is noise rather than corruption — the app's stdout carries
        // no protocol — but the VAD logs three lines per call at info level,
        // fifty times a second per stream, and that has to be routable.
        whisper_rs::install_logging_hooks();

        let path = model.to_str().ok_or_else(|| {
            AudioError::Vad(format!("VAD model path is not UTF-8: {}", model.display()))
        })?;

        let mut params = WhisperVadContextParams::default();
        params.set_n_threads(threads.max(1));
        // CPU, deliberately. On GPU ggml aborts the process outright: it
        // reports "no GPU found" for the VAD backend and then trips its own
        // assertion. Beyond that a second Metal user would contend with the
        // decoders for the same queue — see measurements.md §8.
        params.set_use_gpu(false);

        let mut ctx = WhisperVadContext::new(path, params)
            .map_err(|e| AudioError::Vad(format!("loading the VAD model {path}: {e:?}")))?;

        let window = probe_window(&mut ctx);

        Ok(Self {
            ctx,
            window,
            context_windows: CONTEXT_WINDOWS,
            history: VecDeque::with_capacity(CONTEXT_WINDOWS * window),
            buffer: Vec::with_capacity((CONTEXT_WINDOWS + 1) * window),
            speaking: false,
            onset: DEFAULT_ONSET,
            release: DEFAULT_RELEASE,
            last_probability: 0.0,
            failures: 0,
        })
    }

    /// Thresholds for the settings screen (P12). The Silero threshold trades
    /// decode time against the risk of losing a quietly spoken command, which
    /// is a judgement about the room, not a constant.
    pub fn with_thresholds(mut self, onset: f32, release: f32) -> Self {
        self.onset = onset.clamp(0.0, 1.0);
        self.release = release.clamp(0.0, self.onset);
        self
    }

    /// How much context each decision is given, in frames. Zero judges every
    /// frame with a cold LSTM — cheaper and measurably worse.
    pub fn with_context_windows(mut self, windows: usize) -> Self {
        self.context_windows = windows;
        self.history.clear();
        self
    }

    /// Speech probability of the frame last judged. For the UI (P8) and for
    /// tests that need to see *how* certain the decision was.
    pub fn probability(&self) -> f32 {
        self.last_probability
    }

    /// Frames the model refused to judge, and which therefore passed as
    /// speech.
    pub fn failures(&self) -> u64 {
        self.failures
    }

    /// The model's window size in samples at 16 kHz.
    pub fn window(&self) -> usize {
        self.window
    }

    /// Probability of the current frame, with the context replayed in front of
    /// it. `None` when the model refused.
    fn score(&mut self, frame: &[f32]) -> Option<f32> {
        self.buffer.clear();
        self.buffer.extend(self.history.iter().copied());
        self.buffer.extend_from_slice(frame);
        // A short frame is zero-padded into a whole window by whisper.cpp, so
        // the last probability still belongs to this frame and to no other.

        match self.ctx.detect_speech(&self.buffer) {
            Ok(()) => self.ctx.probabilities().last().copied(),
            Err(err) => {
                if self.failures == 0 {
                    tracing::warn!(error = ?err, "Silero refused a frame — passing it as speech");
                }
                self.failures += 1;
                None
            }
        }
    }

    fn remember(&mut self, frame: &[f32]) {
        let capacity = self.context_windows * self.window;
        if capacity == 0 {
            return;
        }
        self.history.extend(frame.iter().copied());
        while self.history.len() > capacity {
            self.history.pop_front();
        }
    }
}

impl Vad for SileroVad {
    fn frame_samples(&self) -> usize {
        self.window
    }

    fn is_speech(&mut self, frame: &[f32]) -> bool {
        let speech = match self.score(frame) {
            Some(probability) => {
                self.last_probability = probability;
                let threshold = if self.speaking {
                    self.release
                } else {
                    self.onset
                };
                probability >= threshold
            }
            // Failing open: see the module docs.
            None => true,
        };

        self.speaking = speech;
        self.remember(frame);
        speech
    }
}

/// Asks the model how long its window is, by giving it a known number of
/// samples and counting the probabilities that come back.
///
/// whisper.cpp reads the window size from the model file and exposes it
/// nowhere, and the whole scheme here rests on one probability meaning exactly
/// one frame. Probing costs a millisecond once; hard-coding 512 would be a
/// silent mismatch the day a model with another window appears.
fn probe_window(ctx: &mut WhisperVadContext) -> usize {
    let probe = vec![0.0f32; PROBE_SAMPLES];
    let probs = match ctx.detect_speech(&probe) {
        Ok(()) => ctx.probabilities().len(),
        Err(_) => 0,
    };

    let window = PROBE_SAMPLES.checked_div(probs).unwrap_or(FALLBACK_WINDOW);

    // A window outside this range means the probe was misread rather than that
    // the model is unusual — 4 ms to 256 ms at 16 kHz covers anything Silero
    // has ever shipped.
    if (64..=4096).contains(&window) {
        window
    } else {
        tracing::warn!(window, "implausible VAD window, using the default");
        FALLBACK_WINDOW
    }
}

/// Thresholds for gating a whole segment at once, for the sidecar's backstop.
///
/// Kept here so the two layers that run the same model are configured in the
/// same place rather than drifting apart.
pub fn segment_params() -> WhisperVadParams {
    let mut params = WhisperVadParams::default();
    params.set_threshold(DEFAULT_ONSET);
    // Shorter than the default 250 ms: a single "Go." is a legitimate intercom
    // call and must not be filtered out as too brief.
    params.set_min_speech_duration(150);
    params
}

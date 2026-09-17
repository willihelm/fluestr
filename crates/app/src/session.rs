//! Up to eight channels, end to end: microphone → rings → 16 kHz → segments →
//! pool → chat.
//!
//! The pool behind the segmenters was always indifferent to how many streams
//! fed it; what was single-stream was everything in front of it. Now the
//! session builds one pipeline per configured channel — ring, resampler,
//! speech detector, segmenter — over **one** capture of **one** multichannel
//! device, and they all submit into the same prioritised queue.
//!
//! # Threads
//!
//! - the **capture thread** inside `audio-core`, because `cpal::Stream` is
//!   `!Send` on macOS. One thread for the device, whatever the channel count:
//!   the de-interleaver splits the interleaved buffer into the streams' rings
//!   inside the callback;
//! - **one processing thread for all streams**, which drains each ring in
//!   turn, resamples and segments;
//! - the pool's **decode threads**, one per sidecar, plus its watchdog;
//! - the **aggregator thread**, which puts results back in order per stream
//!   and turns them into events.
//!
//! One processing thread rather than one per stream is a deliberate choice.
//! The work is small and bounded — resampling runs at ~2200× realtime and
//! Silero costs ~1.3 % of a core per stream, so eight streams are ~12 % of one
//! core — and a single thread means the level events for all streams are
//! sampled over the same interval instead of eight slightly different ones.
//! What must *not* share a thread is decoding, and it does not: a decode takes
//! hundreds of milliseconds, and a processing thread waiting for one would let
//! the rings overflow while the app was busy transcribing the audio before it.
//! That is why [`WorkerPool::submit`] never blocks.

use crate::pool::{Completed, Outcome, PoolConfig, PoolEvent, Reorder, WorkerPool};
use anyhow::{bail, Context, Result};
use audio_core::{
    ring, vad, AudioBackend, CaptureConfig, CaptureEvent, CaptureSession, ChannelAssignment,
    ChannelMap, CpalBackend, EnergyVad, LanguageCode, LanguageMode, Segmenter, SegmenterConfig,
    SileroVad, StreamId, StreamProcessor, Vad, SAMPLE_RATE,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long one stream's ring holds audio before the processing thread has to
/// have drained it. Four seconds is the buffer that absorbs a decode that runs
/// long; at 48 kHz it costs ~770 KB per stream, which is not worth economising
/// on.
const RING_SECONDS: f32 = 4.0;

/// How often the processing thread wakes up and pumps every stream. Short
/// enough that no ring fills, long enough not to spin.
const PUMP_INTERVAL: Duration = Duration::from_millis(20);

/// Level events per second — one event carrying **all** streams, not one per
/// stream.
///
/// Anything faster is invisible to the eye, and eight separate events at this
/// rate is 160 crossings of the IPC bridge per second, each one waking the
/// WebView. Bundling them makes the cost independent of the channel count.
const LEVEL_HZ: u32 = 20;

/// The channel count the queue bound, the worker count and the latency budget
/// were reasoned for.
///
/// Not a limit: exceeding it is allowed and only warned about, because the
/// numbers behind it are reasoned defaults rather than measured ones until the
/// P14 reference load test exists. An operator who patches nine channels
/// should find out from the queue-depth display, not from a refusal to start.
const BUDGETED_STREAMS: usize = 8;

/// Above this, whisper's own judgement that the audio held no speech is
/// trusted and the segment is dropped.
///
/// An intercom channel is silent or carries only room noise for long stretches,
/// and an energy VAD cannot tell a bell or a burst of music from a spoken word
/// — whisper can, and this is where that judgement is applied. 0.6 is
/// whisper.cpp's own default for the same threshold.
///
/// Note where the cost falls: the segment is decoded first and discarded
/// afterwards, so this buys correctness, not time. Since P3 that cost is
/// rarely paid — [`speech_detector`] runs Silero in front of the segmenter, so
/// a bell no longer opens a segment in the first place. What remains here is
/// the last of three layers, and it stays because the first one falls back to
/// the energy detector when the VAD model is missing.
const NO_SPEECH_THRESHOLD: f32 = 0.6;

/// One intercom channel, as the UI configures it.
///
/// The stream id is *not* in here: it is the position in
/// [`SessionConfig::streams`], assigned by the backend when the session
/// starts, so the UI cannot hand out two streams with the same id. The channel
/// index is zero-based, like the audio API; the UI converts at its own
/// boundary and nowhere else.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamConfig {
    /// Channel index on the multichannel device.
    pub channel: u16,
    /// Free-form label: "Regie", "FOH", "Licht".
    pub label: String,
    /// Primary language, fixed. Auto mode is P3b.
    pub language: String,
    /// Vocabulary prompt (P15 will manage named lists; here it is a text box).
    pub initial_prompt: Option<String>,
}

/// What the UI sends to start a session.
///
/// One device for all streams, which is the constraint the whole audio layer
/// is built on: one multichannel interface, de-interleaved by the app. Two
/// devices would mean two clocks drifting apart with no common timebase to
/// order the chat by.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    /// Device name, as returned by `list_devices`.
    pub device: String,
    pub streams: Vec<StreamConfig>,
}

/// Identifies one run of the pipeline.
///
/// Sequence numbers are per stream and start again at zero every time a
/// session starts, because each stream gets a fresh segmenter. That makes
/// `(stream, seq)` ambiguous the moment a second session runs — and Start is
/// deliberately non-destructive, so the chat holds lines from several sessions
/// at once. `(session, stream, seq)` is what actually identifies a segment,
/// here and in the P10 schema.
///
/// Counted per process, which is as long as the in-memory chat lives.
/// Persistence will need an id that survives a restart; that belongs with P10,
/// not here.
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// A finished transcript on its way to the chat view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEvent {
    /// See [`NEXT_SESSION_ID`]: `seq` alone does not identify a segment.
    pub session_id: u64,
    pub stream_id: u16,
    pub seq: u64,
    pub label: String,
    pub t_start_ms: u64,
    pub t_end_ms: u64,
    pub text: String,
    pub detected_language: Option<String>,
    /// Time whisper.cpp spent decoding.
    pub decode_ms: u32,
    /// From the end of the spoken segment to the text being ready — the number
    /// the whole "latency first" decision rests on. It includes the silence
    /// the segmenter waits out before it closes a segment, which is why that
    /// timeout is a latency setting and not just a quality one.
    pub latency_ms: u32,
}

/// Level and health of one stream.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamLevel {
    pub stream_id: u16,
    pub rms: f32,
    pub peak: f32,
    /// Cumulative overrun count from that stream's ring (P2). Non-zero means
    /// audio was lost, which the operator has to be able to see.
    pub dropped: u64,
}

/// Every stream's level, in one event at [`LEVEL_HZ`].
///
/// All streams together rather than one event each: the meters are read side
/// by side, so they should also be *sampled* side by side — over the same
/// interval, arriving in the same frame. Eight independent events cost eight
/// bridge crossings to show eight bars that are each a different age.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelsEvent {
    pub streams: Vec<StreamLevel>,
}

/// A segment that was thrown away before it could be decoded.
///
/// The pool drops the stalest segment when its queue is full, and an operator
/// who spoke and saw no text has to be able to tell that from a channel that
/// was not listening. Silent loss is explicitly forbidden, so this event is
/// the other half of that policy.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverloadEvent {
    /// See [`NEXT_SESSION_ID`]: `seq` alone does not identify a segment.
    pub session_id: u64,
    pub stream_id: u16,
    /// Sequence number of the segment that was dropped.
    pub seq: u64,
    /// Cumulative count for this stream, so the UI can show a total rather
    /// than only the latest incident.
    pub dropped: u64,
}

/// One running stream, as the backend actually mapped it.
///
/// Echoed back rather than left to the UI's own copy of the configuration:
/// this is the authoritative answer to "which channel ended up under which
/// label", and it is the only place the stream ids the events carry are
/// defined.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningStream {
    pub stream_id: u16,
    pub channel: u16,
    pub label: String,
}

/// Lifecycle of the session, as the UI needs to render it.
#[derive(Debug, Clone, Serialize)]
// `rename_all` covers the variant names, `rename_all_fields` their contents —
// both are needed for the UI to read `state: "running"` and `sampleRate`.
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionState {
    Running {
        /// What the transcripts arriving from now on are tagged with.
        session_id: u64,
        device: String,
        streams: Vec<RunningStream>,
        sample_rate: u32,
        model: String,
        /// Includes the one-off Metal shader compilation on a machine's first
        /// ever start (~8 s); afterwards ~0.1 s.
        load_ms: u32,
        gpu: bool,
    },
    Stopped,
    Failed {
        message: String,
    },
}

/// Anything the session reports back. A trait rather than `tauri::AppHandle`
/// so the pipeline stays independent of the UI layer.
pub trait EventSink: Send + Sync + 'static {
    fn transcript(&self, event: TranscriptEvent);
    fn levels(&self, event: LevelsEvent);
    fn overload(&self, event: OverloadEvent);
    fn state(&self, event: SessionState);
}

/// Everything one channel needs between the ring and the queue.
///
/// One per configured stream, all owned by the processing thread. The level
/// window lives here too, because it has to be accumulated per stream and
/// emptied when the bundled event goes out.
struct Pipeline {
    processor: StreamProcessor,
    segmenter: Segmenter,
    /// Audio since the last level event.
    window: Vec<f32>,
}

/// A running session. Dropping it stops everything.
pub struct Session {
    stop: Arc<AtomicBool>,
    capture: Option<Box<dyn CaptureSession>>,
    processing: Option<JoinHandle<()>>,
    aggregating: Option<JoinHandle<()>>,
    /// Shared with the processing thread, which submits into it.
    pool: Arc<WorkerPool>,
    pub sample_rate: u32,
}

impl Session {
    /// Returns the running state alongside the session rather than only
    /// emitting it.
    ///
    /// The initial transition is what the operator is waiting on after
    /// pressing Start, so it travels back as the command's own reply. Events
    /// then carry everything that happens *later* — a device unplugged, the
    /// sidecar lost, the session stopped. Making the first state depend on the
    /// event channel too would mean a broken bridge looks like a backend that
    /// hangs, which is exactly the failure this arrangement avoids.
    pub fn start(
        config: SessionConfig,
        events: Arc<dyn EventSink>,
    ) -> Result<(Self, SessionState)> {
        if config.streams.is_empty() {
            bail!("no channels configured");
        }
        if config.streams.len() > BUDGETED_STREAMS {
            tracing::warn!(
                streams = config.streams.len(),
                budgeted = BUDGETED_STREAMS,
                "more streams than the queue bound and worker count were reasoned for"
            );
        }

        let backend = CpalBackend::new();
        let device_id = audio_core::DeviceId(config.device.clone());
        let info = backend.device(&device_id)?;

        for stream in &config.streams {
            if stream.channel >= info.channels {
                // Numbered and labelled the way the UI shows it. The index is
                // zero-based because the audio API is, but an error an
                // operator reads has no business exposing that — and with
                // eight rows on screen it also has to say *which* one is
                // wrong.
                bail!(
                    "'{}' is on channel {}, which does not exist on '{}' — it has {} channels",
                    stream.label,
                    stream.channel as u32 + 1,
                    info.name,
                    info.channels
                );
            }
        }

        // The sidecar comes up first. Loading the model can take seconds, and
        // capturing audio that nothing is ready to transcribe would only fill
        // the rings with material that is stale by the time anyone looks at it.
        let binary = crate::worker::find_binary()?;
        let model = crate::worker::find_model()?;
        let vad_model = crate::worker::find_vad_model();
        if vad_model.is_none() {
            tracing::warn!(
                "no Silero VAD model found — noise and silence will be transcribed; \
                 run `just vad-model`"
            );
        }
        let (pool, results) = WorkerPool::spawn(PoolConfig::new(binary, model, vad_model.clone()))
            .context("starting the sidecar")?;
        let pool = Arc::new(pool);
        let ready = pool.ready().clone();

        let rate = info.default_sample_rate;

        // Built in one pass so the sinks stay index-aligned with the channel
        // map — the de-interleaver pairs them by position, and a mismatch
        // there puts the lighting desk's words under the sound engineer's
        // label without anything crashing.
        let mut assignments = Vec::with_capacity(config.streams.len());
        let mut sinks: Vec<Box<dyn audio_core::SampleSink>> =
            Vec::with_capacity(config.streams.len());
        let mut sources = Vec::with_capacity(config.streams.len());
        let mut running = Vec::with_capacity(config.streams.len());

        for (index, stream) in config.streams.iter().enumerate() {
            let stream_id = StreamId(index as u16);
            let (sink, source) = ring::channel(rate, RING_SECONDS);
            assignments.push(ChannelAssignment {
                stream_id,
                channel: stream.channel,
            });
            sinks.push(Box::new(sink));
            sources.push(source);
            running.push(RunningStream {
                stream_id: stream_id.0,
                channel: stream.channel,
                label: stream.label.clone(),
            });
        }

        let channel_map = ChannelMap::new(assignments, info.channels)?;

        let (capture_events, capture_event_rx) = mpsc::channel::<CaptureEvent>();
        let capture = backend.open(
            CaptureConfig {
                device: device_id,
                channel_map,
                sample_rate: Some(rate),
            },
            sinks,
            capture_events,
        )?;

        let granted = capture.sample_rate();
        if granted != rate {
            bail!("device granted {granted} Hz instead of the requested {rate} Hz");
        }

        // Only once the device has actually opened: a detector per stream
        // means a whisper.cpp VAD context per stream, and there is no point
        // paying for eight of them for a device that was going to refuse.
        let mut pipelines = Vec::with_capacity(config.streams.len());
        for (index, (stream, source)) in config.streams.iter().zip(sources).enumerate() {
            pipelines.push(Pipeline {
                processor: StreamProcessor::new(source, granted)?,
                segmenter: Segmenter::new(
                    StreamId(index as u16),
                    // The same model file serves both layers: the detector in
                    // front of this segmenter and the sidecar's backstop
                    // behind it. Each stream needs its own instance — the
                    // detector carries per-stream state.
                    speech_detector(vad_model.as_deref()),
                    SegmenterConfig {
                        language_mode: LanguageMode::Fixed {
                            language: LanguageCode::new(&stream.language),
                        },
                        initial_prompt: stream.initial_prompt.clone(),
                        ..Default::default()
                    },
                ),
                window: Vec::new(),
            });
        }

        let labels: HashMap<u16, String> = running
            .iter()
            .map(|s| (s.stream_id, s.label.clone()))
            .collect();

        let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let stop = Arc::new(AtomicBool::new(false));
        let aggregating = spawn_aggregator(session_id, results, labels, events.clone());
        let processing = spawn_processor(
            pipelines,
            pool.clone(),
            capture_event_rx,
            stop.clone(),
            events.clone(),
        );

        tracing::info!(
            session_id,
            device = %info.name,
            streams = config.streams.len(),
            sample_rate = granted,
            detector = if vad_model.is_some() { "silero" } else { "energy" },
            workers = pool.workers(),
            "session running"
        );

        let state = SessionState::Running {
            session_id,
            device: info.name,
            streams: running,
            sample_rate: granted,
            model: ready.model,
            load_ms: ready.load_ms,
            gpu: ready.gpu,
        };

        Ok((
            Self {
                stop,
                capture: Some(capture),
                processing: Some(processing),
                aggregating: Some(aggregating),
                pool,
                sample_rate: granted,
            },
            state,
        ))
    }

    /// Stops in the order the data flows, so nothing is left half-processed.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(capture) = self.capture.take() {
            if let Err(err) = capture.stop() {
                tracing::warn!(error = %err, "stopping the capture");
            }
        }
        // The processing thread flushes whatever was still being spoken on
        // every stream into the queue as it leaves, so the pool must outlive
        // it.
        if let Some(handle) = self.processing.take() {
            let _ = handle.join();
        }
        self.pool.shutdown();
        // Only once the pool has closed its results channel does the
        // aggregator release what it was still holding back and report the
        // session as stopped.
        if let Some(handle) = self.aggregating.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The detector the segmenter asks "is this speech".
///
/// Silero when the model is there, energy when it is not. The fallback is
/// deliberate rather than an error: an energy detector opens segments for a
/// bell and a chair scrape, but it does hear speech, and the sidecar's gate
/// still stands behind it. A session that runs badly is recoverable; one that
/// refuses to start because a 0.9 MB file is missing is not.
///
/// Called once per stream — the detector holds per-stream state, so eight
/// channels mean eight instances of it.
pub fn speech_detector(vad_model: Option<&std::path::Path>) -> Box<dyn Vad> {
    let Some(path) = vad_model else {
        return Box::new(EnergyVad::new(SAMPLE_RATE));
    };

    match SileroVad::new(path) {
        Ok(vad) => {
            // Per stream, so at debug: the session logs once which detector it
            // ended up with.
            tracing::debug!(
                model = %path.display(),
                frame_samples = vad.frame_samples(),
                "speech detection in front of the segmenter"
            );
            Box::new(vad)
        }
        Err(err) => {
            // Loud, because the difference is what ends up in the chat: from
            // here on a bell can open a segment.
            tracing::warn!(
                model = %path.display(),
                error = %err,
                "Silero failed to load — falling back to the energy detector"
            );
            Box::new(EnergyVad::new(SAMPLE_RATE))
        }
    }
}

fn spawn_processor(
    mut pipelines: Vec<Pipeline>,
    pool: Arc<WorkerPool>,
    capture_events: mpsc::Receiver<CaptureEvent>,
    stop: Arc<AtomicBool>,
    events: Arc<dyn EventSink>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("processing".into())
        .spawn(move || {
            let mut buffer: Vec<f32> = Vec::with_capacity(SAMPLE_RATE as usize);
            let level_interval = Duration::from_millis(1000 / LEVEL_HZ as u64);
            let mut last_level = Instant::now();

            while !stop.load(Ordering::Relaxed) {
                // A device that disappeared ends the session; reporting it is
                // the difference between "nothing happens" and an explanation.
                // It takes every stream with it — there is one device.
                if let Ok(event) = capture_events.try_recv() {
                    match event {
                        CaptureEvent::Disconnected { device, reason } => {
                            events.state(SessionState::Failed {
                                message: format!("audio device '{device}' disconnected: {reason}"),
                            });
                            break;
                        }
                        CaptureEvent::Error { message } => {
                            tracing::warn!(message, "capture error");
                        }
                    }
                }

                let mut failed = None;
                for pipeline in &mut pipelines {
                    buffer.clear();
                    if let Err(err) = pipeline.processor.pump(&mut buffer) {
                        failed = Some(err.to_string());
                        break;
                    }

                    pipeline.window.extend_from_slice(&buffer);
                    for segment in pipeline.segmenter.push(&buffer) {
                        pool.submit(segment, Instant::now());
                    }
                }
                if let Some(message) = failed {
                    events.state(SessionState::Failed { message });
                    break;
                }

                if last_level.elapsed() >= level_interval {
                    events.levels(LevelsEvent {
                        streams: pipelines
                            .iter_mut()
                            .map(|pipeline| {
                                let level = StreamLevel {
                                    stream_id: pipeline.segmenter.stream_id().0,
                                    rms: vad::rms(&pipeline.window),
                                    peak: vad::peak(&pipeline.window),
                                    dropped: pipeline.processor.source().stats().samples_dropped(),
                                };
                                pipeline.window.clear();
                                level
                            })
                            .collect(),
                    });
                    last_level = Instant::now();
                }

                std::thread::sleep(PUMP_INTERVAL);
            }

            // Whatever was still being spoken on any channel when stop was
            // pressed.
            for pipeline in &mut pipelines {
                if let Some(segment) = pipeline.segmenter.finish() {
                    pool.submit(segment, Instant::now());
                }
            }
        })
        .expect("spawning the processing thread")
}

/// Why a decoded segment does not belong in the chat, or `None` if it does.
///
/// Public so the integration test can assert the whole rule rather than
/// re-implement it.
///
/// Returns the reason rather than a bool so the rejection can be logged.
/// Nothing here may drop anything quietly: an operator who spoke and saw no
/// text needs to be able to find out which of these fired.
pub fn discard_reason(text: &str, no_speech_probability: f32) -> Option<&'static str> {
    if text.is_empty() {
        return Some("empty");
    }
    // whisper's own verdict, and the only one in the chain that actually
    // models the difference between speech and other sound.
    if no_speech_probability > NO_SPEECH_THRESHOLD {
        return Some("no speech");
    }
    // The decoder is told not to caption sound events at all; this catches
    // what still gets through.
    if audio_core::is_only_non_speech(text) {
        return Some("annotation only");
    }
    None
}

/// Turns pool results into UI events.
///
/// Two jobs the single worker did not have. It puts results back in order —
/// several workers decode at once and a short segment overtakes a long one,
/// and a chat log read out of order is actively misleading. Ordering is *per
/// stream*: results from different channels are independent and must never
/// wait for each other, or one busy channel would hold up every quiet one.
/// And it owns the end of the session: the pool closing its results channel is
/// what tells the UI that everything spoken has been accounted for.
fn spawn_aggregator(
    session_id: u64,
    results: Receiver<PoolEvent>,
    labels: HashMap<u16, String>,
    events: Arc<dyn EventSink>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("aggregate".into())
        .spawn(move || {
            let mut reorder = Reorder::new();
            let mut dropped: HashMap<u16, u64> = HashMap::new();
            let mut fatal: Option<String> = None;

            for event in results {
                match event {
                    PoolEvent::Completed(completed) => {
                        for ready in reorder.accept(completed) {
                            emit(session_id, &events, &labels, ready, &mut dropped);
                        }
                    }
                    // Not a session error: the pool healed itself, and the
                    // segment that was in flight has already been retried or
                    // reported. Worth a line because it is the only thing that
                    // explains a gap in an otherwise healthy transcript.
                    PoolEvent::WorkerRestarted { worker, reason } => {
                        tracing::warn!(worker, reason, "a sidecar was replaced");
                    }
                    // Remembered rather than reported straight away: results
                    // from the workers that are still finishing belong in the
                    // chat before the message saying it is over.
                    PoolEvent::Exhausted { message } => fatal = Some(message),
                }
            }

            // The pool is finished, so a segment still missing is never going
            // to arrive. Holding its successors back now would lose the last
            // words spoken rather than the one segment that went astray.
            for ready in reorder.drain() {
                emit(session_id, &events, &labels, ready, &mut dropped);
            }

            match fatal {
                Some(message) => events.state(SessionState::Failed { message }),
                None => events.state(SessionState::Stopped),
            }
        })
        .expect("spawning the aggregator thread")
}

fn emit(
    session_id: u64,
    events: &Arc<dyn EventSink>,
    labels: &HashMap<u16, String>,
    completed: Completed,
    dropped: &mut HashMap<u16, u64>,
) {
    let Completed {
        stream_id,
        seq,
        t_start,
        t_end,
        closed_at,
        outcome,
    } = completed;
    let stream_id = stream_id.0;

    match outcome {
        Outcome::Text {
            text,
            detected_language,
            no_speech_probability,
            decode_ms,
        } => {
            // Three ways a segment turns out not to be speech, all logged
            // rather than dropped quietly: the operator has to be able to find
            // out why nothing appeared.
            if let Some(reason) = discard_reason(&text, no_speech_probability) {
                tracing::info!(
                    stream_id,
                    seq,
                    decode_ms,
                    no_speech_probability,
                    reason,
                    "not speech, dropped"
                );
                return;
            }

            let latency_ms = closed_at.elapsed().as_millis() as u32;
            // Length and timings, not the words: this is an operator's
            // intercom traffic, and a terminal log is not where it belongs by
            // default. The text itself sits one level lower, for when a run
            // has to be debugged deliberately.
            tracing::info!(
                stream_id,
                seq,
                decode_ms,
                latency_ms,
                chars = text.chars().count(),
                "transcript"
            );
            tracing::debug!(stream_id, seq, text = %text, "transcript text");

            events.transcript(TranscriptEvent {
                session_id,
                stream_id,
                // A result for a stream that was never configured cannot
                // happen, and an empty label in the chat would be a puzzle
                // rather than a clue — so it says which id it was.
                label: labels
                    .get(&stream_id)
                    .cloned()
                    .unwrap_or_else(|| format!("stream {stream_id}")),
                seq,
                t_start_ms: t_start.as_millis() as u64,
                t_end_ms: t_end.as_millis() as u64,
                text,
                detected_language,
                decode_ms,
                latency_ms,
            });
        }
        // One segment lost, the session intact — a sidecar that rejected it,
        // or a worker that died with it twice over.
        Outcome::Failed(message) => {
            tracing::warn!(stream_id, seq, message, "a segment was lost");
        }
        Outcome::Overloaded => {
            let total = dropped.entry(stream_id).or_default();
            *total += 1;
            tracing::warn!(stream_id, seq, total, "segment dropped, queue full");
            events.overload(OverloadEvent {
                session_id,
                stream_id,
                seq,
                dropped: *total,
            });
        }
    }
}

/// Everything the device picker needs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceOption {
    pub name: String,
    pub channels: u16,
    pub default_sample_rate: u32,
    pub is_default: bool,
}

pub fn list_devices() -> Result<Vec<DeviceOption>> {
    let backend = CpalBackend::new();
    Ok(backend
        .input_devices()?
        .into_iter()
        .map(|info| DeviceOption {
            name: info.name,
            channels: info.channels,
            default_sample_rate: info.default_sample_rate,
            is_default: info.is_default,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_core::StreamId;
    use std::sync::Mutex;

    /// Collects what the UI would have been shown.
    #[derive(Default)]
    struct Recorder {
        transcripts: Mutex<Vec<TranscriptEvent>>,
        overloads: Mutex<Vec<OverloadEvent>>,
        states: Mutex<Vec<SessionState>>,
    }

    impl EventSink for Recorder {
        fn transcript(&self, event: TranscriptEvent) {
            self.transcripts.lock().unwrap().push(event);
        }
        fn levels(&self, _: LevelsEvent) {}
        fn overload(&self, event: OverloadEvent) {
            self.overloads.lock().unwrap().push(event);
        }
        fn state(&self, event: SessionState) {
            self.states.lock().unwrap().push(event);
        }
    }

    fn completed(stream: u16, seq: u64, outcome: Outcome) -> PoolEvent {
        PoolEvent::Completed(Completed {
            stream_id: StreamId(stream),
            seq,
            t_start: Duration::from_millis(seq * 500),
            t_end: Duration::from_millis(seq * 500 + 500),
            closed_at: Instant::now(),
            outcome,
        })
    }

    fn spoken_on(stream: u16, seq: u64, text: &str) -> PoolEvent {
        completed(
            stream,
            seq,
            Outcome::Text {
                text: text.into(),
                detected_language: Some("de".into()),
                no_speech_probability: 0.01,
                decode_ms: 320,
            },
        )
    }

    fn spoken(seq: u64, text: &str) -> PoolEvent {
        spoken_on(0, seq, text)
    }

    /// Runs the aggregator over a fixed sequence of pool events and returns
    /// everything the UI saw.
    fn aggregate(events: Vec<PoolEvent>) -> Arc<Recorder> {
        aggregate_for([(0, "Regie"), (1, "FOH"), (2, "Licht")], events)
    }

    fn aggregate_for(
        labels: impl IntoIterator<Item = (u16, &'static str)>,
        events: Vec<PoolEvent>,
    ) -> Arc<Recorder> {
        aggregate_as(1, labels, events)
    }

    fn aggregate_as(
        session_id: u64,
        labels: impl IntoIterator<Item = (u16, &'static str)>,
        events: Vec<PoolEvent>,
    ) -> Arc<Recorder> {
        let recorder = Arc::new(Recorder::default());
        let sink: Arc<dyn EventSink> = recorder.clone();
        let (tx, rx) = mpsc::channel();
        let labels = labels
            .into_iter()
            .map(|(id, label)| (id, label.to_string()))
            .collect();
        let handle = spawn_aggregator(session_id, rx, labels, sink);
        for event in events {
            tx.send(event).unwrap();
        }
        drop(tx);
        handle.join().unwrap();
        recorder
    }

    #[test]
    fn transcripts_reach_the_chat_in_order_however_the_workers_finished() {
        // Several workers decode at once and a short segment overtakes a long
        // one. Read out of order a chat log is not merely untidy, it says
        // something different from what was said.
        let recorder = aggregate(vec![
            spoken(2, "drei"),
            spoken(0, "eins"),
            spoken(1, "zwei"),
        ]);

        let seen: Vec<String> = recorder
            .transcripts
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.text.clone())
            .collect();
        assert_eq!(seen, ["eins", "zwei", "drei"]);
    }

    #[test]
    fn one_channel_waiting_for_a_result_does_not_hold_up_another() {
        // The reason ordering is per stream and not global. Channel 0's
        // segment 0 is still decoding; channel 1 has no reason to wait for it,
        // and if it did, one busy channel would stall every quiet one.
        let recorder = aggregate(vec![
            spoken_on(1, 0, "FOH eins"),
            spoken_on(1, 1, "FOH zwei"),
            spoken_on(0, 0, "Regie eins"),
        ]);

        let seen: Vec<(u16, String)> = recorder
            .transcripts
            .lock()
            .unwrap()
            .iter()
            .map(|t| (t.stream_id, t.text.clone()))
            .collect();
        assert_eq!(
            seen,
            [
                (1, "FOH eins".to_string()),
                (1, "FOH zwei".to_string()),
                (0, "Regie eins".to_string()),
            ]
        );
    }

    #[test]
    fn every_transcript_carries_its_own_streams_label() {
        // One label for the whole session was fine with one channel. With
        // eight it is the only thing that says who spoke.
        let recorder = aggregate(vec![spoken_on(0, 0, "eins"), spoken_on(2, 0, "zwei")]);
        let seen: Vec<String> = recorder
            .transcripts
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.label.clone())
            .collect();
        assert_eq!(seen, ["Regie", "Licht"]);
    }

    #[test]
    fn a_dropped_segment_does_not_hold_back_the_words_after_it() {
        // The reason the pool reports segments it gave up on rather than
        // staying quiet about them: the ones behind it would never appear.
        let recorder = aggregate(vec![
            spoken(1, "zwei"),
            completed(0, 0, Outcome::Overloaded),
            spoken(2, "drei"),
        ]);

        let texts: Vec<String> = recorder
            .transcripts
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.text.clone())
            .collect();
        assert_eq!(texts, ["zwei", "drei"]);

        // And the loss itself is visible, not just absent.
        let overloads = recorder.overloads.lock().unwrap();
        assert_eq!(overloads.len(), 1);
        assert_eq!(overloads[0].seq, 0);
        assert_eq!(overloads[0].dropped, 1);
    }

    #[test]
    fn overload_is_counted_per_stream() {
        // A running total across all channels would tell an operator the app
        // is behind but not on which channel they lost words.
        let recorder = aggregate(vec![
            completed(0, 0, Outcome::Overloaded),
            completed(1, 0, Outcome::Overloaded),
            completed(0, 1, Outcome::Overloaded),
        ]);
        let overloads = recorder.overloads.lock().unwrap();
        let counts: Vec<(u16, u64)> = overloads.iter().map(|o| (o.stream_id, o.dropped)).collect();
        assert_eq!(counts, [(0, 1), (1, 1), (0, 2)]);
    }

    #[test]
    fn the_last_words_survive_a_segment_that_never_came_back() {
        // At shutdown the missing predecessor is never going to arrive.
        // Holding its successors back would lose the end of the session.
        let recorder = aggregate(vec![spoken(1, "zwei"), spoken(2, "drei")]);
        assert_eq!(recorder.transcripts.lock().unwrap().len(), 2);
    }

    #[test]
    fn non_speech_is_filtered_on_the_way_to_the_chat() {
        let recorder = aggregate(vec![
            spoken(0, "[BREAK]"),
            spoken(1, "Standby für Cue fünf"),
        ]);
        let texts = recorder.transcripts.lock().unwrap();
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[0].text, "Standby für Cue fünf");
    }

    #[test]
    fn a_clean_run_ends_as_stopped_and_an_exhausted_pool_as_failed() {
        // The UI has to tell "you pressed Stop" from "there is nothing left to
        // transcribe with", and both arrive the same way.
        let clean = aggregate(vec![spoken(0, "eins")]);
        assert!(matches!(
            clean.states.lock().unwrap().as_slice(),
            [SessionState::Stopped]
        ));

        let broken = aggregate(vec![
            spoken(0, "eins"),
            PoolEvent::Exhausted {
                message: "no sidecar could be restarted".into(),
            },
        ]);
        // The transcript still gets through: results already in flight belong
        // in the chat before the message saying it is over.
        assert_eq!(broken.transcripts.lock().unwrap().len(), 1);
        assert!(matches!(
            broken.states.lock().unwrap().as_slice(),
            [SessionState::Failed { .. }]
        ));
    }

    #[test]
    fn a_second_session_does_not_collide_with_the_first() {
        // Start is deliberately non-destructive, so the chat still holds the
        // previous session's lines — and every stream gets a fresh segmenter,
        // so sequence numbers begin again at zero. `(stream, seq)` therefore
        // repeats across sessions, and the chat list keys on it: the second
        // session's first line arrived with the same identity as the first
        // session's and the keyed list threw, which stopped the chat
        // rendering altogether. What identifies a segment is
        // `(session, stream, seq)`.
        let first = aggregate_as(1, [(0, "Regie")], vec![spoken(0, "eins")]);
        let second = aggregate_as(2, [(0, "Regie")], vec![spoken(0, "zwei")]);

        let id = |r: &Arc<Recorder>| {
            let t = r.transcripts.lock().unwrap();
            (t[0].session_id, t[0].stream_id, t[0].seq)
        };
        assert_eq!(id(&first), (1, 0, 0));
        assert_eq!(id(&second), (2, 0, 0));
        assert_ne!(id(&first), id(&second));
    }

    #[test]
    fn every_session_gets_its_own_id() {
        // Two sessions in one process must never be handed the same id, or
        // the collision above comes back with a longer key.
        let first = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let second = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        assert_ne!(first, second);
    }

    #[test]
    fn the_transcript_event_matches_what_the_ui_reads() {
        // The field names cross the IPC bridge, so they are pinned down here
        // rather than discovered by a blank chat view.
        let json = serde_json::to_value(TranscriptEvent {
            session_id: 3,
            stream_id: 0,
            seq: 4,
            label: "Regie".into(),
            t_start_ms: 1_200,
            t_end_ms: 3_400,
            // German on purpose: speech data, not code.
            text: "Standby für Cue fünf".into(),
            detected_language: Some("de".into()),
            decode_ms: 412,
            latency_ms: 980,
        })
        .unwrap();

        assert_eq!(json["sessionId"], 3);
        assert_eq!(json["streamId"], 0);
        assert_eq!(json["tStartMs"], 1_200);
        assert_eq!(json["detectedLanguage"], "de");
        assert_eq!(json["latencyMs"], 980);
    }

    #[test]
    fn levels_cross_the_bridge_as_one_event_for_all_streams() {
        // Eight meters, one event: the shape the UI indexes by `streamId`.
        let json = serde_json::to_value(LevelsEvent {
            streams: vec![
                StreamLevel {
                    stream_id: 0,
                    rms: 0.1,
                    peak: 0.4,
                    dropped: 0,
                },
                StreamLevel {
                    stream_id: 1,
                    rms: 0.0,
                    peak: 0.0,
                    dropped: 128,
                },
            ],
        })
        .unwrap();

        assert_eq!(json["streams"][0]["streamId"], 0);
        assert_eq!(json["streams"][1]["dropped"], 128);
    }

    #[test]
    fn session_state_is_tagged_for_the_ui() {
        let running = serde_json::to_value(SessionState::Running {
            session_id: 7,
            device: "MacBook Pro Microphone".into(),
            streams: vec![RunningStream {
                stream_id: 0,
                channel: 2,
                label: "Regie".into(),
            }],
            sample_rate: 48_000,
            model: "ggml-small-q5_1.bin".into(),
            load_ms: 140,
            gpu: true,
        })
        .unwrap();
        assert_eq!(running["state"], "running");
        assert_eq!(running["sessionId"], 7);
        assert_eq!(running["sampleRate"], 48_000);
        assert_eq!(running["streams"][0]["streamId"], 0);
        assert_eq!(running["streams"][0]["channel"], 2);

        let failed = serde_json::to_value(SessionState::Failed {
            message: "no model".into(),
        })
        .unwrap();
        assert_eq!(failed["state"], "failed");
    }

    #[test]
    fn sound_annotations_never_reach_the_chat() {
        // Exactly what showed up on a quiet channel during the first live run.
        assert_eq!(discard_reason("[BREAK]", 0.1), Some("annotation only"));
        assert_eq!(discard_reason("(music)", 0.1), Some("annotation only"));
        assert_eq!(discard_reason("(Bell)", 0.1), Some("annotation only"));
        assert_eq!(discard_reason("", 0.1), Some("empty"));
    }

    #[test]
    fn whispers_own_no_speech_verdict_is_trusted() {
        // Plausible words over silence are the dangerous case: the text looks
        // fine, so nothing about it gives the operator a reason to doubt it.
        assert_eq!(discard_reason("Thank you.", 0.95), Some("no speech"));
        assert_eq!(discard_reason("Thank you.", 0.1), None);
    }

    #[test]
    fn speech_is_kept_even_next_to_an_annotation() {
        // German on purpose: this is speech data, not code.
        assert_eq!(discard_reason("Standby für Cue fünf", 0.02), None);
        assert_eq!(discard_reason("Go.", 0.2), None);
        assert_eq!(discard_reason("[BREAK] dann Blackout", 0.2), None);
    }

    #[test]
    fn the_threshold_is_not_accidentally_inverted() {
        assert_eq!(discard_reason("Go.", NO_SPEECH_THRESHOLD), None);
        assert_eq!(
            discard_reason("Go.", NO_SPEECH_THRESHOLD + 0.01),
            Some("no speech")
        );
    }

    #[test]
    fn the_session_config_is_read_as_the_ui_writes_it() {
        let config: SessionConfig = serde_json::from_str(
            r#"{"device":"Scarlett 18i20","streams":[
                {"channel":0,"label":"Regie","language":"de",
                 "initialPrompt":"Standby, Cue, Go"},
                {"channel":5,"label":"FOH","language":"en","initialPrompt":null}
            ]}"#,
        )
        .unwrap();
        assert_eq!(config.streams.len(), 2);
        assert_eq!(config.streams[0].channel, 0);
        assert_eq!(
            config.streams[0].initial_prompt.as_deref(),
            Some("Standby, Cue, Go")
        );
        // Not contiguous and not ordered: an engineer patches the channels
        // they have, not the first N.
        assert_eq!(config.streams[1].channel, 5);
        assert_eq!(config.streams[1].language, "en");
        assert_eq!(config.streams[1].initial_prompt, None);
    }
}

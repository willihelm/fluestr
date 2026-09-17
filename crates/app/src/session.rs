//! One channel, end to end: microphone → ring → 16 kHz → segments → pool →
//! chat.
//!
//! Still a **single** stream: the multichannel session is P6/P7. What is no
//! longer provisional is everything behind the segmenter — segments go into
//! the queue in [`crate::pool`], which is already indifferent to how many
//! streams feed it.
//!
//! # Threads
//!
//! - the **capture thread** inside `audio-core`, because `cpal::Stream` is
//!   `!Send` on macOS;
//! - the **processing thread**, which drains the ring, resamples and segments;
//! - the pool's **decode threads**, one per sidecar, plus its watchdog;
//! - the **aggregator thread**, which puts results back in order and turns
//!   them into events.
//!
//! The split between processing and decoding matters: a decode takes hundreds
//! of milliseconds, and if the processing thread waited for one, the ring
//! would overflow and audio would be lost while the app was busy transcribing
//! the audio before it. That is also why [`WorkerPool::submit`] never blocks.

use crate::pool::{Completed, Outcome, PoolConfig, PoolEvent, Reorder, WorkerPool};
use anyhow::{bail, Context, Result};
use audio_core::{
    ring, vad, AudioBackend, CaptureConfig, CaptureEvent, CaptureSession, ChannelAssignment,
    ChannelMap, CpalBackend, EnergyVad, LanguageCode, LanguageMode, Segmenter, SegmenterConfig,
    SileroVad, StreamId, StreamProcessor, Vad, SAMPLE_RATE,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long the ring holds audio before the processing thread has to have
/// drained it. Four seconds is generous for one stream; it is the buffer that
/// absorbs a decode that runs long.
const RING_SECONDS: f32 = 4.0;

/// How often the processing thread wakes up. Short enough that the ring never
/// fills, long enough not to spin.
const PUMP_INTERVAL: Duration = Duration::from_millis(20);

/// Level events per second. Anything faster is invisible to the eye and
/// expensive across the IPC bridge — P6 bundles all eight streams into one
/// event at this rate.
const LEVEL_HZ: u32 = 20;

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

/// What the UI sends to start a session.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    /// Device name, as returned by `list_devices`.
    pub device: String,
    /// Channel index on that device.
    pub channel: u16,
    /// Free-form label: "Regie", "FOH", "Licht".
    pub label: String,
    /// Primary language, fixed. Auto mode is P3b.
    pub language: String,
    /// Vocabulary prompt (P15 will manage named lists; here it is a text box).
    pub initial_prompt: Option<String>,
}

/// A finished transcript on its way to the chat view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEvent {
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

/// Level and health of one stream, emitted at [`LEVEL_HZ`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelEvent {
    pub stream_id: u16,
    pub rms: f32,
    pub peak: f32,
    /// Cumulative overrun count from the ring (P2). Non-zero means audio was
    /// lost, which the operator has to be able to see.
    pub dropped: u64,
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
    pub stream_id: u16,
    /// Sequence number of the segment that was dropped.
    pub seq: u64,
    /// Cumulative count for this stream, so the UI can show a total rather
    /// than only the latest incident.
    pub dropped: u64,
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
        device: String,
        channel: u16,
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
    fn level(&self, event: LevelEvent);
    fn overload(&self, event: OverloadEvent);
    fn state(&self, event: SessionState);
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
        let backend = CpalBackend::new();
        let device_id = audio_core::DeviceId(config.device.clone());
        let info = backend.device(&device_id)?;

        if config.channel >= info.channels {
            // Numbered the way the UI shows it. The index is zero-based
            // because the audio API is, but an error an operator reads has no
            // business exposing that.
            bail!(
                "channel {} does not exist on '{}' — it has {} channels",
                config.channel as u32 + 1,
                info.name,
                info.channels
            );
        }

        // The sidecar comes up first. Loading the model can take seconds, and
        // capturing audio that nothing is ready to transcribe would only fill
        // the ring with material that is stale by the time anyone looks at it.
        let binary = crate::worker::find_binary()?;
        let model = crate::worker::find_model()?;
        let vad_model = crate::worker::find_vad_model();
        if vad_model.is_none() {
            tracing::warn!(
                "no Silero VAD model found — noise and silence will be transcribed; \
                 run `just vad-model`"
            );
        }
        // The same model file serves both layers: the detector in front of the
        // segmenter here, and the sidecar's backstop behind it.
        let detector = speech_detector(vad_model.as_deref());
        let (pool, results) = WorkerPool::spawn(PoolConfig::new(binary, model, vad_model))
            .context("starting the sidecar")?;
        let pool = Arc::new(pool);
        let ready = pool.ready().clone();

        let rate = info.default_sample_rate;
        let (sink, source) = ring::channel(rate, RING_SECONDS);

        let stream_id = StreamId(0);
        let channel_map = ChannelMap::new(
            vec![ChannelAssignment {
                stream_id,
                channel: config.channel,
            }],
            info.channels,
        )?;

        let (capture_events, capture_event_rx) = mpsc::channel::<CaptureEvent>();
        let capture = backend.open(
            CaptureConfig {
                device: device_id,
                channel_map,
                sample_rate: Some(rate),
            },
            vec![Box::new(sink)],
            capture_events,
        )?;

        let granted = capture.sample_rate();
        if granted != rate {
            bail!("device granted {granted} Hz instead of the requested {rate} Hz");
        }

        let processor = StreamProcessor::new(source, granted)?;
        let segmenter = Segmenter::new(
            stream_id,
            detector,
            SegmenterConfig {
                language_mode: LanguageMode::Fixed {
                    language: LanguageCode::new(&config.language),
                },
                initial_prompt: config.initial_prompt.clone(),
                ..Default::default()
            },
        );

        let stop = Arc::new(AtomicBool::new(false));
        let aggregating = spawn_aggregator(results, config.label.clone(), events.clone());
        let processing = spawn_processor(
            processor,
            segmenter,
            pool.clone(),
            capture_event_rx,
            stop.clone(),
            events.clone(),
        );

        tracing::info!(
            device = %info.name,
            channel = config.channel,
            sample_rate = granted,
            language = %config.language,
            prompt = config.initial_prompt.is_some(),
            workers = pool.workers(),
            "session running"
        );

        let state = SessionState::Running {
            device: info.name,
            channel: config.channel,
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
        // The processing thread flushes whatever was still being spoken into
        // the queue as it leaves, so the pool must outlive it.
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
pub fn speech_detector(vad_model: Option<&std::path::Path>) -> Box<dyn Vad> {
    let Some(path) = vad_model else {
        return Box::new(EnergyVad::new(SAMPLE_RATE));
    };

    match SileroVad::new(path) {
        Ok(vad) => {
            tracing::info!(
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
    mut processor: StreamProcessor,
    mut segmenter: Segmenter,
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
            let mut window: Vec<f32> = Vec::new();

            while !stop.load(Ordering::Relaxed) {
                // A device that disappeared ends the session; reporting it is
                // the difference between "nothing happens" and an explanation.
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

                buffer.clear();
                if let Err(err) = processor.pump(&mut buffer) {
                    events.state(SessionState::Failed {
                        message: err.to_string(),
                    });
                    break;
                }

                window.extend_from_slice(&buffer);
                for segment in segmenter.push(&buffer) {
                    pool.submit(segment, Instant::now());
                }

                if last_level.elapsed() >= level_interval {
                    events.level(LevelEvent {
                        stream_id: segmenter.stream_id().0,
                        rms: vad::rms(&window),
                        peak: vad::peak(&window),
                        dropped: processor.source().stats().samples_dropped(),
                    });
                    window.clear();
                    last_level = Instant::now();
                }

                std::thread::sleep(PUMP_INTERVAL);
            }

            // Whatever was still being spoken when stop was pressed.
            if let Some(segment) = segmenter.finish() {
                pool.submit(segment, Instant::now());
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
/// and a chat log read out of order is actively misleading. And it owns the
/// end of the session: the pool closing its results channel is what tells the
/// UI that everything spoken has been accounted for.
fn spawn_aggregator(
    results: Receiver<PoolEvent>,
    label: String,
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
                            emit(&events, &label, ready, &mut dropped);
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
                emit(&events, &label, ready, &mut dropped);
            }

            match fatal {
                Some(message) => events.state(SessionState::Failed { message }),
                None => events.state(SessionState::Stopped),
            }
        })
        .expect("spawning the aggregator thread")
}

fn emit(
    events: &Arc<dyn EventSink>,
    label: &str,
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
                stream_id,
                seq,
                label: label.to_string(),
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
        fn level(&self, _: LevelEvent) {}
        fn overload(&self, event: OverloadEvent) {
            self.overloads.lock().unwrap().push(event);
        }
        fn state(&self, event: SessionState) {
            self.states.lock().unwrap().push(event);
        }
    }

    fn completed(seq: u64, outcome: Outcome) -> PoolEvent {
        PoolEvent::Completed(Completed {
            stream_id: StreamId(0),
            seq,
            t_start: Duration::from_millis(seq * 500),
            t_end: Duration::from_millis(seq * 500 + 500),
            closed_at: Instant::now(),
            outcome,
        })
    }

    fn spoken(seq: u64, text: &str) -> PoolEvent {
        completed(
            seq,
            Outcome::Text {
                text: text.into(),
                detected_language: Some("de".into()),
                no_speech_probability: 0.01,
                decode_ms: 320,
            },
        )
    }

    /// Runs the aggregator over a fixed sequence of pool events and returns
    /// everything the UI saw.
    fn aggregate(events: Vec<PoolEvent>) -> Arc<Recorder> {
        let recorder = Arc::new(Recorder::default());
        let sink: Arc<dyn EventSink> = recorder.clone();
        let (tx, rx) = mpsc::channel();
        let handle = spawn_aggregator(rx, "Regie".into(), sink);
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
    fn a_dropped_segment_does_not_hold_back_the_words_after_it() {
        // The reason the pool reports segments it gave up on rather than
        // staying quiet about them: the ones behind it would never appear.
        let recorder = aggregate(vec![
            spoken(1, "zwei"),
            completed(0, Outcome::Overloaded),
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
    fn the_transcript_event_matches_what_the_ui_reads() {
        // The field names cross the IPC bridge, so they are pinned down here
        // rather than discovered by a blank chat view.
        let json = serde_json::to_value(TranscriptEvent {
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

        assert_eq!(json["streamId"], 0);
        assert_eq!(json["tStartMs"], 1_200);
        assert_eq!(json["detectedLanguage"], "de");
        assert_eq!(json["latencyMs"], 980);
    }

    #[test]
    fn session_state_is_tagged_for_the_ui() {
        let running = serde_json::to_value(SessionState::Running {
            device: "MacBook Pro Microphone".into(),
            channel: 0,
            sample_rate: 48_000,
            model: "ggml-small-q5_1.bin".into(),
            load_ms: 140,
            gpu: true,
        })
        .unwrap();
        assert_eq!(running["state"], "running");
        assert_eq!(running["sampleRate"], 48_000);

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
            r#"{"device":"MacBook Pro Microphone","channel":0,"label":"Regie",
                "language":"de","initialPrompt":"Standby, Cue, Go"}"#,
        )
        .unwrap();
        assert_eq!(config.channel, 0);
        assert_eq!(config.language, "de");
        assert_eq!(config.initial_prompt.as_deref(), Some("Standby, Cue, Go"));
    }
}

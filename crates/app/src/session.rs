//! One channel, end to end: microphone → ring → 16 kHz → segments → sidecar →
//! chat.
//!
//! This is the narrow vertical slice, not the MVP. It runs a **single** stream
//! through a **single** worker so that the real end-to-end latency becomes
//! measurable before P3, P5 and P6 are built out separately. Everything it
//! does provisionally is marked as such.
//!
//! # Threads
//!
//! Three, each with a reason:
//!
//! - the **capture thread** inside `audio-core`, because `cpal::Stream` is
//!   `!Send` on macOS;
//! - the **processing thread**, which drains the ring, resamples and segments;
//! - the **decode thread**, which talks to the sidecar.
//!
//! The last split matters: a decode takes hundreds of milliseconds, and if the
//! processing thread waited for it, the ring would overflow and audio would be
//! lost while the app was busy transcribing the audio before it.

use crate::worker::Worker;
use anyhow::{bail, Context, Result};
use audio_core::{
    ring, vad, AudioBackend, CaptureConfig, CaptureEvent, CaptureSession, ChannelAssignment,
    ChannelMap, CpalBackend, EnergyVad, LanguageCode, LanguageMode, Segment, Segmenter,
    SegmenterConfig, StreamId, StreamProcessor, SAMPLE_RATE,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use whisper_protocol::Response;

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
    fn state(&self, event: SessionState);
}

/// A running session. Dropping it stops everything.
pub struct Session {
    stop: Arc<AtomicBool>,
    capture: Option<Box<dyn CaptureSession>>,
    processing: Option<JoinHandle<()>>,
    decoding: Option<JoinHandle<()>>,
    pub sample_rate: u32,
}

impl Session {
    pub fn start(config: SessionConfig, events: Arc<dyn EventSink>) -> Result<Self> {
        let backend = CpalBackend::new();
        let device_id = audio_core::DeviceId(config.device.clone());
        let info = backend.device(&device_id)?;

        if config.channel >= info.channels {
            bail!(
                "channel {} does not exist on '{}' ({} channels)",
                config.channel,
                info.name,
                info.channels
            );
        }

        // The sidecar comes up first. Loading the model can take seconds, and
        // capturing audio that nothing is ready to transcribe would only fill
        // the ring with material that is stale by the time anyone looks at it.
        let binary = crate::worker::find_binary()?;
        let model = crate::worker::find_model()?;
        let worker = Worker::spawn(&binary, &model, 4).context("starting the sidecar")?;
        let ready = worker.ready().clone();

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
            Box::new(EnergyVad::new(SAMPLE_RATE)),
            SegmenterConfig {
                language_mode: LanguageMode::Fixed {
                    language: LanguageCode::new(&config.language),
                },
                initial_prompt: config.initial_prompt.clone(),
                ..Default::default()
            },
        );

        let stop = Arc::new(AtomicBool::new(false));
        let (segment_tx, segment_rx) = mpsc::channel::<(Segment, Instant)>();

        let decoding = spawn_decoder(worker, segment_rx, config.label.clone(), events.clone());
        let processing = spawn_processor(
            processor,
            segmenter,
            segment_tx,
            capture_event_rx,
            stop.clone(),
            events.clone(),
        );

        events.state(SessionState::Running {
            device: info.name,
            channel: config.channel,
            sample_rate: granted,
            model: ready.model,
            load_ms: ready.load_ms,
            gpu: ready.gpu,
        });

        Ok(Self {
            stop,
            capture: Some(capture),
            processing: Some(processing),
            decoding: Some(decoding),
            sample_rate: granted,
        })
    }

    /// Stops in the order the data flows, so nothing is left half-processed:
    /// capture first, then the processing thread flushes its last segment, and
    /// only once the channel is closed does the decoder shut the sidecar down.
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
        if let Some(handle) = self.processing.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.decoding.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn spawn_processor(
    mut processor: StreamProcessor,
    mut segmenter: Segmenter,
    segments: mpsc::Sender<(Segment, Instant)>,
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
                    if segments.send((segment, Instant::now())).is_err() {
                        return;
                    }
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
                let _ = segments.send((segment, Instant::now()));
            }
        })
        .expect("spawning the processing thread")
}

fn spawn_decoder(
    mut worker: Worker,
    segments: mpsc::Receiver<(Segment, Instant)>,
    label: String,
    events: Arc<dyn EventSink>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("decode".into())
        .spawn(move || {
            for (segment, closed_at) in segments {
                match worker.transcribe(&segment) {
                    Ok(Response::Transcript {
                        stream_id,
                        seq,
                        text,
                        detected_language,
                        decode_ms,
                        ..
                    }) => {
                        // Whisper returns an empty string for a segment that
                        // turned out to be noise. Showing it as an empty bubble
                        // would be worse than showing nothing.
                        if text.is_empty() {
                            tracing::debug!(stream_id, seq, "empty transcript, dropped");
                            continue;
                        }
                        events.transcript(TranscriptEvent {
                            stream_id,
                            seq,
                            label: label.clone(),
                            t_start_ms: segment.t_start.as_millis() as u64,
                            t_end_ms: segment.t_end.as_millis() as u64,
                            text,
                            detected_language,
                            decode_ms,
                            latency_ms: closed_at.elapsed().as_millis() as u32,
                        });
                    }
                    Ok(Response::Error { message, .. }) => {
                        tracing::warn!(seq = segment.seq, message, "sidecar rejected a segment");
                    }
                    Ok(other) => {
                        tracing::warn!("unexpected response: {other:?}");
                    }
                    // Without a pool there is nothing to respawn into, so the
                    // session ends. P5 turns this into a restart.
                    Err(err) => {
                        events.state(SessionState::Failed {
                            message: format!("sidecar lost: {err}"),
                        });
                        return;
                    }
                }
            }

            worker.shutdown();
            events.state(SessionState::Stopped);
        })
        .expect("spawning the decode thread")
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

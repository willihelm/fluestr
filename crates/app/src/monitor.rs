//! Levels before the session (P8).
//!
//! A trim is set by ear against a meter, and until now there was no meter
//! until there was a session — which means the setup an operator builds before
//! a show is built blind, and the first thing they learn about a mis-patched
//! or too-quiet channel is that no text appears on it during the show.
//!
//! So: the device is opened, de-interleaved and measured, and nothing else. No
//! model, no sidecar, no segmenter, no transcripts. That is the whole
//! difference from [`crate::session`], and it is what makes this cheap enough
//! to run while somebody is simply looking at the Settings page — it is a few
//! ring buffers and one thread that adds up squares.
//!
//! It emits the same [`LevelsEvent`] a session does, on the same event name.
//! The meters do not care where a reading came from, and a second kind of
//! level event would mean every view learning to read both.
//!
//! One device is opened by one thing at a time. The session takes precedence:
//! [`crate::start_session`] stops the monitor before opening its own capture,
//! and the UI starts it again when the session ends.

use anyhow::{bail, Context, Result};
use audio_core::{
    gain::Gain, ring, vad, AudioBackend, CaptureConfig, CaptureEvent, CaptureSession,
    ChannelAssignment, ChannelMap, CpalBackend, RingSource, StreamId,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::session::{EventSink, LevelsEvent, SessionConfig, StreamLevel};

/// How much audio the rings hold. Far less than a session's, because nothing
/// downstream can fall behind: the reader does not resample, decode or wait on
/// a queue. A second is already generous for a thread that wakes every 10 ms.
const RING_SECONDS: f32 = 1.0;

/// How often the reader empties the rings. The same cadence the session pumps
/// at — a meter that lags the voice it is showing is a meter nobody trusts.
const PUMP_INTERVAL: Duration = Duration::from_millis(10);

/// Meter events per second. The session's rate, so the bars move the same way
/// whether or not a session is running.
const LEVEL_HZ: u32 = 20;

/// An open device being measured.
///
/// Dropping it stops everything, like a session.
pub struct Monitor {
    /// What is being measured, so a request for the same thing can be answered
    /// by leaving it alone. See [`Monitor::covers`].
    device_id: String,
    channels: Vec<u16>,
    stop: Arc<AtomicBool>,
    capture: Option<Box<dyn CaptureSession>>,
    reading: Option<JoinHandle<()>>,
    /// The trim of each live stream, shared with the reader — the same
    /// arrangement [`crate::session::Session`] uses, and for the same reason:
    /// the knob that sets it is dragged.
    gains: HashMap<u16, Arc<Gain>>,
}

impl Monitor {
    /// Opens the device and starts measuring.
    ///
    /// Takes a [`SessionConfig`] rather than a shape of its own: the caller
    /// has one — it is the same channel list it would press Start with — and a
    /// second type would be the same list with the fields this does not use
    /// left out, which is a type that can disagree with the one that matters.
    /// The vocabulary and the languages in it are simply not read.
    ///
    /// A channel the device does not have is skipped and keeps its stream id,
    /// exactly as a session mutes it: the ids are positions in the channel
    /// list, and the meters are keyed on them.
    ///
    /// The soloed channels are heard here as in a session — before a show is
    /// exactly when a channel is checked by ear.
    pub fn start(
        config: &SessionConfig,
        events: Arc<dyn EventSink>,
        solo: Arc<crate::solo::Bus>,
    ) -> Result<Self> {
        if config.streams.is_empty() {
            bail!("no channels configured");
        }

        let backend = CpalBackend::new();
        let info = backend.device(&audio_core::DeviceId(config.device_id.clone()))?;

        let live: Vec<(u16, &crate::session::StreamConfig)> = config
            .streams
            .iter()
            .enumerate()
            .map(|(index, stream)| (index as u16, stream))
            .filter(|(_, stream)| stream.channel < info.channels)
            .collect();

        if live.is_empty() {
            bail!(
                "none of the {} configured channels exists on '{}' — it has {}",
                config.streams.len(),
                info.name,
                info.channels
            );
        }

        let rate = info.default_sample_rate;
        let mut assignments = Vec::with_capacity(live.len());
        let mut sinks: Vec<Box<dyn audio_core::SampleSink>> = Vec::with_capacity(live.len());
        let mut readers = Vec::with_capacity(live.len());
        let mut gains = HashMap::with_capacity(live.len());
        let mut taps = Vec::with_capacity(live.len());

        for &(stream_id, stream) in &live {
            let (sink, source) = ring::channel(rate, RING_SECONDS);
            assignments.push(ChannelAssignment {
                stream_id: StreamId(stream_id),
                channel: stream.channel,
            });
            let gain = Arc::new(Gain::new(stream.gain_db));
            gains.insert(stream_id, gain.clone());
            let (sink, tap) = solo.tap(stream_id, sink, rate, gain.clone());
            sinks.push(sink);
            taps.push(tap);
            readers.push(Reader {
                stream_id,
                source,
                gain,
                window: Vec::new(),
            });
        }

        let (capture_events, capture_event_rx) = mpsc::channel::<CaptureEvent>();
        let capture = backend
            .open(
                CaptureConfig {
                    device: info.id.clone(),
                    channel_map: ChannelMap::new(assignments, info.channels)?,
                    sample_rate: Some(rate),
                },
                sinks,
                capture_events,
            )
            .context("opening the device for metering")?;

        let stop = Arc::new(AtomicBool::new(false));
        let mixer = crate::solo::Mixer::new(solo, rate, taps);
        let reading = spawn_reader(readers, mixer, capture_event_rx, stop.clone(), events);

        tracing::info!(
            device = %info.name,
            streams = live.len(),
            sample_rate = rate,
            "metering the input"
        );

        Ok(Self {
            device_id: config.device_id.clone(),
            channels: config.streams.iter().map(|s| s.channel).collect(),
            stop,
            capture: Some(capture),
            reading: Some(reading),
            gains,
        })
    }

    /// Whether this monitor is already measuring exactly what `config` asks
    /// for.
    ///
    /// The Settings page asks on every edit — a label typed, a colour picked —
    /// and reopening the device for each of those would make the meters
    /// stutter through the one activity they exist to support. Only the device
    /// and the channel map matter here: the trim is changed through
    /// [`Monitor::set_gain`] without reopening anything, and nothing else in
    /// the configuration reaches this at all.
    pub fn covers(&self, config: &SessionConfig) -> bool {
        self.device_id == config.device_id
            && self.channels.len() == config.streams.len()
            && self
                .channels
                .iter()
                .zip(&config.streams)
                .all(|(&channel, stream)| channel == stream.channel)
    }

    /// Trims one stream, as the knob is turned.
    ///
    /// A stream the device has no channel for has no reader and is silently
    /// left alone — the same thing that happens to a muted stream of a running
    /// session, and for the same reason: it keeps its id and its setting, and
    /// there is simply no audio for either to reach.
    pub fn set_gain(&self, stream_id: u16, gain_db: i32) {
        if let Some(gain) = self.gains.get(&stream_id) {
            gain.set_db(gain_db);
        }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The capture first, so nothing is still being written into rings
        // whose reader is on its way out.
        drop(self.capture.take());
        if let Some(handle) = self.reading.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// One stream's end of the metering: its ring, its trim, and the audio since
/// the last event.
struct Reader {
    stream_id: u16,
    source: RingSource,
    gain: Arc<Gain>,
    window: Vec<f32>,
}

fn spawn_reader(
    mut readers: Vec<Reader>,
    mut solo: crate::solo::Mixer,
    capture_events: mpsc::Receiver<CaptureEvent>,
    stop: Arc<AtomicBool>,
    events: Arc<dyn EventSink>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("metering".into())
        .spawn(move || {
            // Sized for one pump at the highest rate a device is likely to
            // run at, so the read below is one call rather than a loop.
            let mut buffer = vec![0.0f32; 4096];
            let level_interval = Duration::from_millis(1000 / LEVEL_HZ as u64);
            let mut last_level = Instant::now();

            while !stop.load(Ordering::Relaxed) {
                // A device unplugged while the Settings page is open ends the
                // metering rather than reporting a failed *session*: there is
                // no session, and telling the chat that one just failed would
                // be a lie about something nobody started. The device list is
                // what says the interface is gone, and it is on the same page.
                if let Ok(event) = capture_events.try_recv() {
                    match event {
                        CaptureEvent::Disconnected { device, reason } => {
                            tracing::warn!(id = %device, reason, "the metered device disconnected");
                            break;
                        }
                        CaptureEvent::Error { message } => {
                            tracing::warn!(message, "metering error");
                        }
                    }
                }

                for reader in &mut readers {
                    loop {
                        let read = reader.source.read(&mut buffer);
                        if read == 0 {
                            break;
                        }
                        // Trimmed before it is measured, so the bar shows what
                        // the knob did — which is the only reason this exists.
                        reader.gain.apply(&mut buffer[..read]);
                        reader.window.extend_from_slice(&buffer[..read]);
                        if read < buffer.len() {
                            break;
                        }
                    }
                }

                solo.pump();

                if last_level.elapsed() >= level_interval {
                    events.levels(LevelsEvent {
                        streams: readers
                            .iter_mut()
                            .map(|reader| {
                                let level = StreamLevel {
                                    stream_id: reader.stream_id,
                                    rms: vad::rms(&reader.window),
                                    peak: vad::peak(&reader.window),
                                    // Counted like a session's, though it
                                    // should never move here: nothing
                                    // downstream can stall, so a non-zero
                                    // value means this thread itself did not
                                    // get scheduled.
                                    dropped: reader.source.stats().samples_dropped(),
                                };
                                reader.window.clear();
                                level
                            })
                            .collect(),
                    });
                    last_level = Instant::now();
                }

                std::thread::sleep(PUMP_INTERVAL);
            }
        })
        .expect("spawning the metering thread")
}

//! [`AudioBackend`] on top of cpal.
//!
//! cpal covers CoreAudio today and WASAPI later, so one implementation serves
//! both targets. The trait stays in place regardless: it is what keeps the
//! rest of the crate free of cpal types, and it is the seam a
//! platform-specific implementation would slot into if cpal ever proves too
//! coarse (a CoreAudio UID for stable device identity being the likely reason,
//! see [`DeviceId`]).
//!
//! # Why the stream gets its own thread
//!
//! `cpal::Stream` is `!Send` on macOS: CoreAudio's property listener holds a
//! plain `FnMut` callback, which makes the whole stream non-transferable. The
//! app needs a session handle it can keep in shared state across threads, so
//! the stream is confined to a thread that owns it from construction to drop,
//! and the handle only talks to that thread through channels. Asserting `Send`
//! on the stream instead would be a lie about what CoreAudio permits.

use crate::capture::{CaptureStats, Demuxer, SampleSink};
use crate::device::{
    AudioBackend, CaptureConfig, CaptureEvent, CaptureSession, DeviceId, DeviceInfo,
};
use crate::{AudioError, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;

pub struct CpalBackend;

impl CpalBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CpalBackend {
    fn default() -> Self {
        Self::new()
    }
}

fn find_device(host: &cpal::Host, id: &DeviceId) -> Result<cpal::Device> {
    let devices = host
        .input_devices()
        .map_err(|e| AudioError::Backend(e.to_string()))?;

    for device in devices {
        if device.name().ok().as_deref() == Some(id.0.as_str()) {
            return Ok(device);
        }
    }
    Err(AudioError::DeviceNotFound(id.0.clone()))
}

/// Builds the `DeviceInfo` for one cpal device.
///
/// Returns `None` for devices that advertise no usable input configuration —
/// those exist and must not appear in the picker.
fn describe(device: &cpal::Device, is_default: bool) -> Option<DeviceInfo> {
    let name = device.name().ok()?;
    let default_config = device.default_input_config().ok()?;

    let mut sample_rates = Vec::new();
    if let Ok(configs) = device.supported_input_configs() {
        for config in configs {
            // A range is advertised as min..max; the endpoints are what a user
            // would actually pick, and enumerating everything in between is
            // both impossible and pointless.
            sample_rates.push(config.min_sample_rate().0);
            sample_rates.push(config.max_sample_rate().0);
        }
    }
    sample_rates.push(default_config.sample_rate().0);
    sample_rates.sort_unstable();
    sample_rates.dedup();

    Some(DeviceInfo {
        id: DeviceId(name.clone()),
        name,
        channels: default_config.channels(),
        default_sample_rate: default_config.sample_rate().0,
        sample_rates,
        is_default,
    })
}

impl AudioBackend for CpalBackend {
    fn input_devices(&self) -> Result<Vec<DeviceInfo>> {
        let host = cpal::default_host();
        let default_name = host.default_input_device().and_then(|d| d.name().ok());

        let devices = host
            .input_devices()
            .map_err(|e| AudioError::Backend(e.to_string()))?;

        Ok(devices
            .filter_map(|device| {
                let is_default = device.name().ok() == default_name;
                describe(&device, is_default)
            })
            .collect())
    }

    fn device(&self, id: &DeviceId) -> Result<DeviceInfo> {
        let host = cpal::default_host();
        let device = find_device(&host, id)?;
        describe(&device, false).ok_or_else(|| {
            AudioError::Backend(format!("device '{id}' advertises no usable input config"))
        })
    }

    fn open(
        &self,
        config: CaptureConfig,
        sinks: Vec<Box<dyn SampleSink>>,
        events: Sender<CaptureEvent>,
    ) -> Result<Box<dyn CaptureSession>> {
        let stats = Arc::new(CaptureStats::default());

        // The owner thread reports back once the stream is up (or failed), so
        // `open` still returns a synchronous Result the caller can act on.
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(u32, u16)>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let thread_stats = stats.clone();
        let handle = std::thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || {
                let stream = match build_stream(config, sinks, events, thread_stats) {
                    Ok((stream, sample_rate, channels)) => {
                        let _ = ready_tx.send(Ok((sample_rate, channels)));
                        stream
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                        return;
                    }
                };

                // Park until stop is requested or the handle is dropped. The
                // stream is created, held and destroyed entirely on this
                // thread, which is what CoreAudio expects.
                let _ = stop_rx.recv();
                drop(stream);
            })
            .map_err(|e| AudioError::Backend(format!("spawning capture thread: {e}")))?;

        let (sample_rate, channels) = ready_rx
            .recv()
            .map_err(|_| AudioError::Backend("capture thread died during startup".into()))??;

        Ok(Box::new(CpalSession {
            stop: Some(stop_tx),
            thread: Some(handle),
            sample_rate,
            channels,
            stats,
        }))
    }
}

/// Runs entirely on the capture thread.
fn build_stream(
    config: CaptureConfig,
    sinks: Vec<Box<dyn SampleSink>>,
    events: Sender<CaptureEvent>,
    stats: Arc<CaptureStats>,
) -> Result<(cpal::Stream, u32, u16)> {
    let host = cpal::default_host();
    let device = find_device(&host, &config.device)?;
    let supported = device
        .default_input_config()
        .map_err(|e| AudioError::Backend(e.to_string()))?;

    let channels = supported.channels();
    let sample_rate = config.sample_rate.unwrap_or(supported.sample_rate().0);
    let stream_config = cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(sample_rate),
        // Let the device choose. A fixed small buffer would lower latency but
        // raises the risk of dropouts across eight streams; P14 is where that
        // trade-off gets measured rather than guessed.
        buffer_size: cpal::BufferSize::Default,
    };

    // Validates the sink count and the channel range before any stream exists,
    // so a bad mapping fails here rather than inside the callback.
    let mut demuxer = Demuxer::new(&config.channel_map, channels, sinks, stats)?;

    let error_device = config.device.clone();

    let stream = device
        .build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Realtime context: no allocation, no lock, no logging. The
                // demuxer owns its sinks and its scratch buffer, so this call
                // touches nothing but preallocated memory.
                demuxer.demux(data);
            },
            move |err| {
                // Not the realtime thread — reporting here is safe.
                let event = match err {
                    cpal::StreamError::DeviceNotAvailable => CaptureEvent::Disconnected {
                        device: error_device.clone(),
                        reason: "device no longer available".into(),
                    },
                    other => CaptureEvent::Error {
                        message: other.to_string(),
                    },
                };
                let _ = events.send(event);
            },
            None,
        )
        .map_err(|e| AudioError::Backend(e.to_string()))?;

    stream
        .play()
        .map_err(|e| AudioError::Backend(e.to_string()))?;

    Ok((stream, sample_rate, channels))
}

/// Handle to a capture running on its own thread. `Send` because it holds only
/// channels and counters — never the stream itself.
struct CpalSession {
    stop: Option<Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    sample_rate: u32,
    channels: u16,
    stats: Arc<CaptureStats>,
}

impl CpalSession {
    fn shutdown(&mut self) -> Result<()> {
        // Dropping the sender alone would already wake the thread; sending is
        // the explicit path and keeps the intent readable.
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| AudioError::Backend("capture thread panicked".into()))?;
        }
        Ok(())
    }
}

impl Drop for CpalSession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl CaptureSession for CpalSession {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn stats(&self) -> &Arc<CaptureStats> {
        &self.stats
    }

    fn stop(mut self: Box<Self>) -> Result<()> {
        self.shutdown()
    }
}

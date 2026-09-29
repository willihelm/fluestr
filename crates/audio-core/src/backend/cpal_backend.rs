//! [`AudioBackend`] on top of cpal.
//!
//! cpal covers CoreAudio today and WASAPI later, so one implementation serves
//! both targets. The trait stays in place regardless: it is what keeps the
//! rest of the crate free of cpal types.
//!
//! Where cpal did prove too coarse is device *identity*: it knows a device by
//! its name and nothing else. That gap is filled beside it rather than by
//! replacing it — [`stable_id`] reads the platform's own id and this module
//! attaches it, so cpal keeps doing the part it is good at. See [`DeviceId`].
//!
//! # Why the stream gets its own thread
//!
//! `cpal::Stream` is `!Send` on macOS: CoreAudio's property listener holds a
//! plain `FnMut` callback, which makes the whole stream non-transferable. The
//! app needs a session handle it can keep in shared state across threads, so
//! the stream is confined to a thread that owns it from construction to drop,
//! and the handle only talks to that thread through channels. Asserting `Send`
//! on the stream instead would be a lie about what CoreAudio permits.

use super::stable_id::{self, Direction};
use crate::capture::{CaptureStats, Demuxer, SampleSink};
use crate::device::{
    AudioBackend, CaptureConfig, CaptureEvent, CaptureSession, DeviceId, DeviceInfo, OutputConfig,
    PlaybackSession,
};
use crate::playback::{SampleSource, Spreader};
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

/// Every usable device in one direction, each with the id the app will store.
///
/// One enumeration for both halves: the ids are assigned from the device
/// *list*, because telling two identically named interfaces apart needs to see
/// them together. Asking per device could not do it.
fn enumerate(host: &cpal::Host, direction: Direction) -> Result<Vec<(cpal::Device, DeviceInfo)>> {
    let (default_name, devices) = match direction {
        Direction::Input => (
            host.default_input_device().and_then(|d| d.name().ok()),
            host.input_devices()
                .map_err(|e| AudioError::Backend(e.to_string()))?
                .collect::<Vec<cpal::Device>>(),
        ),
        Direction::Output => (
            host.default_output_device().and_then(|d| d.name().ok()),
            host.output_devices()
                .map_err(|e| AudioError::Backend(e.to_string()))?
                .collect::<Vec<cpal::Device>>(),
        ),
    };

    let names: Vec<String> = devices
        .iter()
        .map(|d| d.name().unwrap_or_default())
        .collect();
    let ids = stable_id::assign(&names, direction);

    Ok(devices
        .into_iter()
        .zip(ids)
        .filter_map(|(device, id)| {
            let is_default = device.name().ok() == default_name;
            let info = describe(&device, id, is_default, direction)?;
            Some((device, info))
        })
        .collect())
}

/// Looks one device up, by stable id first and by name second.
///
/// The name is the fallback and not the rule: it is what a configuration
/// written before stable ids existed holds, and what a device the platform
/// would not identify still has. Trying the id first means a renamed device
/// is found, and two identical ones are told apart, as soon as the id is
/// stored — while neither case gets worse in the meantime.
fn find_device(
    host: &cpal::Host,
    id: &DeviceId,
    direction: Direction,
) -> Result<(cpal::Device, DeviceInfo)> {
    let mut devices = enumerate(host, direction)?;

    if let Some(index) = devices.iter().position(|(_, info)| &info.id == id) {
        return Ok(devices.swap_remove(index));
    }
    if let Some(index) = devices.iter().position(|(_, info)| info.name == id.0) {
        return Ok(devices.swap_remove(index));
    }
    Err(AudioError::DeviceNotFound(id.0.clone()))
}

/// Builds the `DeviceInfo` for one cpal device.
///
/// Returns `None` for devices that advertise no usable configuration in
/// `direction` — those exist and must not appear in the picker.
fn describe(
    device: &cpal::Device,
    id: Option<String>,
    is_default: bool,
    direction: Direction,
) -> Option<DeviceInfo> {
    let name = device.name().ok()?;
    let default_config = match direction {
        Direction::Input => device.default_input_config().ok()?,
        Direction::Output => device.default_output_config().ok()?,
    };

    let mut sample_rates = Vec::new();
    let configs: Vec<cpal::SupportedStreamConfigRange> = match direction {
        Direction::Input => device
            .supported_input_configs()
            .map(|c| c.collect())
            .unwrap_or_default(),
        Direction::Output => device
            .supported_output_configs()
            .map(|c| c.collect())
            .unwrap_or_default(),
    };
    for config in configs {
        // A range is advertised as min..max; the endpoints are what a user
        // would actually pick, and enumerating everything in between is both
        // impossible and pointless.
        sample_rates.push(config.min_sample_rate().0);
        sample_rates.push(config.max_sample_rate().0);
    }
    sample_rates.push(default_config.sample_rate().0);
    sample_rates.sort_unstable();
    sample_rates.dedup();

    Some(DeviceInfo {
        // The name as a last resort, so every device has *an* id and the rest
        // of the app never has to handle "device without identity".
        id: DeviceId(id.unwrap_or_else(|| name.clone())),
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
        Ok(enumerate(&host, Direction::Input)?
            .into_iter()
            .map(|(_, info)| info)
            .collect())
    }

    fn device(&self, id: &DeviceId) -> Result<DeviceInfo> {
        let host = cpal::default_host();
        find_device(&host, id, Direction::Input).map(|(_, info)| info)
    }

    fn output_devices(&self) -> Result<Vec<DeviceInfo>> {
        let host = cpal::default_host();
        Ok(enumerate(&host, Direction::Output)?
            .into_iter()
            .map(|(_, info)| info)
            .collect())
    }

    fn output_device(&self, id: Option<&DeviceId>) -> Result<DeviceInfo> {
        let host = cpal::default_host();
        find_output(&host, id).map(|(_, info)| info)
    }

    fn open_output(
        &self,
        config: OutputConfig,
        source: Box<dyn SampleSource>,
        events: Sender<CaptureEvent>,
    ) -> Result<Box<dyn PlaybackSession>> {
        // The same arrangement as a capture, for the same reason: the stream
        // is `!Send` on macOS and lives on a thread that owns it.
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(u32, u16)>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let handle = std::thread::Builder::new()
            .name("audio-playback".into())
            .spawn(move || {
                let stream = match build_output_stream(config, source, events) {
                    Ok((stream, sample_rate, channels)) => {
                        let _ = ready_tx.send(Ok((sample_rate, channels)));
                        stream
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                        return;
                    }
                };
                let _ = stop_rx.recv();
                drop(stream);
            })
            .map_err(|e| AudioError::Backend(format!("spawning playback thread: {e}")))?;

        let (sample_rate, channels) = ready_rx
            .recv()
            .map_err(|_| AudioError::Backend("playback thread died during startup".into()))??;

        Ok(Box::new(CpalPlayback {
            stop: Some(stop_tx),
            thread: Some(handle),
            sample_rate,
            channels,
        }))
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
    let (device, info) = find_device(&host, &config.device, Direction::Input)?;
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

    // The device's own id, not the one that was asked for: a configuration
    // still holding a name resolved to *this* device, and an event naming
    // something the app no longer recognises helps nobody.
    let error_device = info.id.clone();

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

/// An output device by id, or the system default with `None`.
fn find_output(host: &cpal::Host, id: Option<&DeviceId>) -> Result<(cpal::Device, DeviceInfo)> {
    match id {
        Some(id) => find_device(host, id, Direction::Output),
        None => {
            let mut devices = enumerate(host, Direction::Output)?;
            let index = devices
                .iter()
                .position(|(_, info)| info.is_default)
                .ok_or_else(|| AudioError::DeviceNotFound("the default output".into()))?;
            Ok(devices.swap_remove(index))
        }
    }
}

/// Runs entirely on the playback thread.
fn build_output_stream(
    config: OutputConfig,
    source: Box<dyn SampleSource>,
    events: Sender<CaptureEvent>,
) -> Result<(cpal::Stream, u32, u16)> {
    let host = cpal::default_host();
    let (device, info) = find_output(&host, Some(&config.device))
        // An empty id is how the system default is asked for.
        .or_else(|err| {
            if config.device.0.is_empty() {
                find_output(&host, None)
            } else {
                Err(err)
            }
        })?;
    let supported = device
        .default_output_config()
        .map_err(|e| AudioError::Backend(e.to_string()))?;

    let channels = supported.channels();
    let sample_rate = config.sample_rate.unwrap_or(supported.sample_rate().0);
    let stream_config = cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let mut spreader = Spreader::new(config.channels, channels, source)?;
    let error_device = info.id.clone();

    let stream = device
        .build_output_stream(
            &stream_config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Realtime context, like the capture callback: the spreader
                // and its source touch nothing but preallocated memory.
                spreader.spread(data);
            },
            move |err| {
                let event = match err {
                    cpal::StreamError::DeviceNotAvailable => CaptureEvent::Disconnected {
                        device: error_device.clone(),
                        reason: "output device no longer available".into(),
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

/// Handle to a playback stream running on its own thread.
struct CpalPlayback {
    stop: Option<Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    sample_rate: u32,
    channels: u16,
}

impl CpalPlayback {
    fn shutdown(&mut self) -> Result<()> {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| AudioError::Backend("playback thread panicked".into()))?;
        }
        Ok(())
    }
}

impl Drop for CpalPlayback {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl PlaybackSession for CpalPlayback {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn stop(mut self: Box<Self>) -> Result<()> {
        self.shutdown()
    }
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

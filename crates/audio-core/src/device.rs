//! Device discovery and the platform abstraction.
//!
//! Everything platform-specific sits behind [`AudioBackend`]. The Windows port
//! (P1, later) implements this trait and nothing above it changes.

use crate::capture::{CaptureStats, ChannelMap, SampleSink};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Identifies an audio device across restarts.
///
/// On macOS this is the CoreAudio device UID — a string the driver keeps
/// across reboots, renames and re-plugs, and which differs between two units
/// of the same model. cpal does not expose it, so it is read beside cpal; see
/// `backend::stable_id`.
///
/// It falls back to the device **name** where the platform offers nothing
/// better: on Windows until the WASAPI port brings its endpoint id, and for
/// the occasional device CoreAudio will not identify. A name is a weak
/// identity — two identical interfaces share one, and a rename breaks it —
/// which is why a lookup may still fail. It is never resolved to "whatever is
/// plugged in": a stored device that is not there stays stored and is marked
/// as absent, and capturing the wrong hardware is the one outcome worth
/// refusing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub String);

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the UI needs in order to offer a device and its channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Stable across restarts; what a configuration stores.
    pub id: DeviceId,
    /// What a human reads. Shown everywhere, stored beside the id so an
    /// absent device can still be named, and never used to look one up while
    /// an id is available.
    pub name: String,
    /// Channel count of the configuration we would actually open — this is the
    /// number the channel picker must be built from.
    pub channels: u16,
    pub default_sample_rate: u32,
    /// Distinct sample rates the device advertises, ascending.
    pub sample_rates: Vec<u32>,
    pub is_default: bool,
}

/// How a capture session should be opened.
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    pub device: DeviceId,
    pub channel_map: ChannelMap,
    /// Desired hardware sample rate. `None` uses the device default.
    /// Conversion to the internal 16 kHz happens downstream (P2).
    pub sample_rate: Option<u32>,
}

/// Something that happened to a running capture, delivered outside the
/// realtime callback.
#[derive(Debug, Clone)]
pub enum CaptureEvent {
    /// The device vanished — unplugged interface, switched-off converter.
    /// The session is over; the app surfaces this and offers to reconnect.
    Disconnected { device: DeviceId, reason: String },
    /// A backend error that did not end the stream.
    Error { message: String },
}

/// A running capture. Dropping it stops the stream.
pub trait CaptureSession: Send {
    /// The hardware sample rate actually granted, which may differ from the
    /// requested one.
    fn sample_rate(&self) -> u32;
    /// Channel count of the opened configuration.
    fn channels(&self) -> u16;
    fn stats(&self) -> &Arc<CaptureStats>;
    /// Stops the stream. Dropping does the same; this exists to make shutdown
    /// explicit and to surface errors.
    fn stop(self: Box<Self>) -> Result<()>;
}

/// Platform abstraction for audio input.
pub trait AudioBackend: Send + Sync {
    /// All usable input devices.
    fn input_devices(&self) -> Result<Vec<DeviceInfo>>;

    /// Looks up one device by id.
    fn device(&self, id: &DeviceId) -> Result<DeviceInfo>;

    /// Opens a capture session.
    ///
    /// `sinks` must be ordered exactly like `config.channel_map`; the caller
    /// owns that correspondence. Events go to `events`, which is read outside
    /// the realtime path.
    fn open(
        &self,
        config: CaptureConfig,
        sinks: Vec<Box<dyn SampleSink>>,
        events: std::sync::mpsc::Sender<CaptureEvent>,
    ) -> Result<Box<dyn CaptureSession>>;
}

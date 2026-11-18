//! Platform backends implementing [`crate::device::AudioBackend`].

mod cpal_backend;
pub mod file_backend;
mod stable_id;

pub use cpal_backend::CpalBackend;

use crate::capture::SampleSink;
use crate::device::{
    AudioBackend, CaptureConfig, CaptureEvent, CaptureSession, DeviceId, DeviceInfo, OutputConfig,
    PlaybackSession,
};
use crate::playback::SampleSource;
use crate::Result;
use std::sync::mpsc::Sender;

/// The interfaces the platform has, plus a WAV file named by a `file:` id
/// ([`file_backend`]).
///
/// What a session and the meters open their input through, so that a file
/// chosen as the device runs the same pipeline a desk would. Listing the
/// devices still lists only the interfaces: a file is not plugged in, it is
/// chosen, and the app remembers which ones were.
#[derive(Default)]
pub struct InputBackend {
    cpal: CpalBackend,
}

impl InputBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl AudioBackend for InputBackend {
    fn input_devices(&self) -> Result<Vec<DeviceInfo>> {
        self.cpal.input_devices()
    }

    fn device(&self, id: &DeviceId) -> Result<DeviceInfo> {
        if file_backend::is_file(id) {
            file_backend::device(id)
        } else {
            self.cpal.device(id)
        }
    }

    fn output_devices(&self) -> Result<Vec<DeviceInfo>> {
        self.cpal.output_devices()
    }

    fn output_device(&self, id: Option<&DeviceId>) -> Result<DeviceInfo> {
        self.cpal.output_device(id)
    }

    fn open_output(
        &self,
        config: OutputConfig,
        source: Box<dyn SampleSource>,
        events: Sender<CaptureEvent>,
    ) -> Result<Box<dyn PlaybackSession>> {
        self.cpal.open_output(config, source, events)
    }

    fn open(
        &self,
        config: CaptureConfig,
        sinks: Vec<Box<dyn SampleSink>>,
        events: Sender<CaptureEvent>,
    ) -> Result<Box<dyn CaptureSession>> {
        if file_backend::is_file(&config.device) {
            file_backend::open(config, sinks, events)
        } else {
            self.cpal.open(config, sinks, events)
        }
    }
}

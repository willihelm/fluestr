//! Audio core of the intercom transcription: capture, ring buffers,
//! resampling and segmentation.
//!
//! This crate knows nothing about Tauri or the UI. That keeps the
//! realtime-critical paths unit-testable and confines the later Windows port
//! (P1) to a single trait, [`device::AudioBackend`].

pub mod backend;
pub mod capture;
pub mod device;
pub mod lang;
pub mod resample;
pub mod ring;
pub mod segment;

pub use backend::CpalBackend;
pub use capture::{CaptureStats, ChannelAssignment, ChannelMap, Demuxer, SampleSink};
pub use device::{AudioBackend, CaptureConfig, CaptureEvent, CaptureSession, DeviceId, DeviceInfo};
pub use lang::{LanguageCode, LanguageMode};
pub use resample::{Resampler, StreamProcessor};
pub use ring::{RingSink, RingSource, RingStats};
pub use segment::{Segment, StreamId, Transcript, SAMPLE_RATE};

/// Errors raised by the audio core.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("audio device not found: {0}")]
    DeviceNotFound(String),

    #[error("channel {channel} does not exist on a device with {available} channels")]
    ChannelOutOfRange { channel: u16, available: u16 },

    #[error("device disconnected during capture: {0}")]
    DeviceDisconnected(String),

    #[error("resampling: {0}")]
    Resample(String),

    #[error("audio backend: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, AudioError>;

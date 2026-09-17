//! Platform backends implementing [`crate::device::AudioBackend`].

mod cpal_backend;

pub use cpal_backend::CpalBackend;

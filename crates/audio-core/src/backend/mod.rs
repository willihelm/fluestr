//! Platform backends implementing [`crate::device::AudioBackend`].

mod cpal_backend;
mod stable_id;

pub use cpal_backend::CpalBackend;

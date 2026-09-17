//! The lock-free hand-off from the audio callback to the processing thread.
//!
//! This is the seam between P1 and P2: capture writes here without ever
//! blocking, and the processing side (resampling, VAD) reads at its own pace.
//! Resampling itself belongs to P2 and deliberately does not happen in the
//! callback.

use crate::capture::SampleSink;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

/// Producer end of a per-stream ring buffer, usable as a [`SampleSink`].
pub struct RingSink {
    producer: HeapProd<f32>,
}

impl SampleSink for RingSink {
    fn push(&mut self, samples: &[f32]) -> usize {
        // `push_slice` takes what fits and reports the rest — exactly the
        // "never block, count the loss" contract the callback needs.
        self.producer.push_slice(samples)
    }
}

/// Consumer end, read from the processing thread.
pub struct RingSource {
    consumer: HeapCons<f32>,
}

impl RingSource {
    /// Copies up to `out.len()` samples out and returns how many were read.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        self.consumer.pop_slice(out)
    }

    /// Samples currently waiting. A steadily growing value means the consumer
    /// is falling behind — surfaced by P8.
    pub fn available(&self) -> usize {
        self.consumer.occupied_len()
    }

    pub fn is_empty(&self) -> bool {
        self.consumer.is_empty()
    }
}

/// Creates a ring buffer sized in seconds of audio at `sample_rate`.
///
/// Sizing is a latency-versus-resilience trade-off: too small and a scheduling
/// hiccup on the processing thread costs samples, too large and stale audio
/// piles up that nobody wants transcribed any more. Two seconds is the
/// starting point; P14 is where it gets tuned against measurements.
pub fn channel(sample_rate: u32, seconds: f32) -> (RingSink, RingSource) {
    let capacity = ((sample_rate as f32 * seconds) as usize).max(1024);
    let (producer, consumer) = HeapRb::<f32>::new(capacity).split();
    (RingSink { producer }, RingSource { consumer })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_survive_the_hand_off_in_order() {
        let (mut sink, mut source) = channel(16_000, 1.0);
        let written: Vec<f32> = (0..100).map(|n| n as f32).collect();

        assert_eq!(sink.push(&written), 100);
        assert_eq!(source.available(), 100);

        let mut out = vec![0.0; 100];
        assert_eq!(source.read(&mut out), 100);
        assert_eq!(out, written);
        assert!(source.is_empty());
    }

    #[test]
    fn a_full_ring_reports_a_short_count_instead_of_blocking() {
        // 1024 is the floor the constructor enforces.
        let (mut sink, _source) = channel(16_000, 0.0);
        let too_much = vec![1.0f32; 5000];

        let accepted = sink.push(&too_much);

        assert!(accepted > 0, "should accept what fits");
        assert!(accepted < too_much.len(), "should not accept everything");
    }

    #[test]
    fn reading_more_than_available_returns_what_there_is() {
        let (mut sink, mut source) = channel(16_000, 1.0);
        sink.push(&[1.0, 2.0, 3.0]);

        let mut out = vec![0.0; 10];
        assert_eq!(source.read(&mut out), 3);
        assert_eq!(&out[..3], &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn capacity_follows_the_sample_rate() {
        let (mut sink, source) = channel(48_000, 2.0);
        // 48 kHz x 2 s = 96000 samples; push more than a second to confirm the
        // ring is genuinely that large rather than clamped to the floor.
        let accepted = sink.push(&vec![0.5f32; 90_000]);
        assert_eq!(accepted, 90_000);
        assert_eq!(source.available(), 90_000);
    }
}

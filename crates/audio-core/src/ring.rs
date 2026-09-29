//! The lock-free hand-off from the audio callback to the processing thread.
//!
//! This is the seam between P1 and P2: capture writes here without ever
//! blocking, and the processing side (resampling, VAD) reads at its own pace.
//! Resampling itself belongs to P2 and deliberately does not happen in the
//! callback.

use crate::capture::SampleSink;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Per-stream counters for one ring buffer.
///
/// [`CaptureStats`](crate::CaptureStats) aggregates the same loss across the
/// whole device; these counters attribute it to a single stream, which is what
/// the health display (P8) has to show — "channel 3 is losing audio" is
/// actionable, "the session is losing audio" is not.
///
/// Written from the audio callback, so atomics rather than a mutex. `Relaxed`
/// is sufficient: these are diagnostics, not synchronisation.
#[derive(Debug, Default)]
pub struct RingStats {
    samples_written: AtomicU64,
    samples_dropped: AtomicU64,
}

impl RingStats {
    /// Samples that made it into the ring.
    pub fn samples_written(&self) -> u64 {
        self.samples_written.load(Ordering::Relaxed)
    }

    /// Samples lost because the ring was full — the overrun count P2 owes P8.
    /// Any non-zero value means the processing thread is not keeping up.
    pub fn samples_dropped(&self) -> u64 {
        self.samples_dropped.load(Ordering::Relaxed)
    }
}

/// Producer end of a per-stream ring buffer, usable as a [`SampleSink`].
pub struct RingSink {
    producer: HeapProd<f32>,
    stats: Arc<RingStats>,
}

impl SampleSink for RingSink {
    fn push(&mut self, samples: &[f32]) -> usize {
        // `push_slice` takes what fits and reports the rest — exactly the
        // "never block, count the loss" contract the callback needs.
        let accepted = self.producer.push_slice(samples);
        self.stats
            .samples_written
            .fetch_add(accepted as u64, Ordering::Relaxed);
        let dropped = samples.len() - accepted;
        if dropped > 0 {
            self.stats
                .samples_dropped
                .fetch_add(dropped as u64, Ordering::Relaxed);
        }
        accepted
    }
}

/// Consumer end, read from the processing thread.
pub struct RingSource {
    consumer: HeapCons<f32>,
    stats: Arc<RingStats>,
}

impl RingSource {
    /// Copies up to `out.len()` samples out and returns how many were read.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        self.consumer.pop_slice(out)
    }

    /// Throws away up to `count` of the oldest samples and returns how many
    /// went. For a reader that plays what it reads (P26) and would rather
    /// lose a few milliseconds than let its latency grow.
    pub fn skip(&mut self, count: usize) -> usize {
        self.consumer.skip(count)
    }

    /// Samples currently waiting. A steadily growing value means the consumer
    /// is falling behind — surfaced by P8.
    pub fn available(&self) -> usize {
        self.consumer.occupied_len()
    }

    pub fn is_empty(&self) -> bool {
        self.consumer.is_empty()
    }

    /// Counters shared with the producer end.
    pub fn stats(&self) -> &Arc<RingStats> {
        &self.stats
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
    let stats = Arc::new(RingStats::default());
    (
        RingSink {
            producer,
            stats: stats.clone(),
        },
        RingSource { consumer, stats },
    )
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

    #[test]
    fn a_stalled_consumer_raises_the_overrun_count_rather_than_panicking() {
        // The P2 acceptance case: the processing thread stops reading and the
        // callback keeps writing. Nothing may panic or block; the loss has to
        // become a number instead.
        let (mut sink, source) = channel(16_000, 0.0);
        let block = vec![0.25f32; 700];

        for _ in 0..3 {
            sink.push(&block);
        }

        let stats = source.stats();
        assert_eq!(stats.samples_written(), 1024, "only the capacity fits");
        assert_eq!(stats.samples_dropped(), 2100 - 1024);
        assert_eq!(source.available(), 1024);
    }

    #[test]
    fn draining_the_ring_makes_room_again_without_resetting_the_count() {
        let (mut sink, mut source) = channel(16_000, 0.0);
        sink.push(&vec![0.5f32; 2000]);
        assert_eq!(source.stats().samples_dropped(), 976);

        let mut out = vec![0.0; 1024];
        assert_eq!(source.read(&mut out), 1024);
        assert_eq!(sink.push(&[1.0, 2.0]), 2);

        // The counter is cumulative over the session — P8 shows total loss,
        // not "loss since someone last looked".
        assert_eq!(source.stats().samples_dropped(), 976);
        assert_eq!(source.stats().samples_written(), 1026);
    }

    #[test]
    fn counters_start_at_zero() {
        let (_sink, source) = channel(16_000, 1.0);
        assert_eq!(source.stats().samples_written(), 0);
        assert_eq!(source.stats().samples_dropped(), 0);
    }
}

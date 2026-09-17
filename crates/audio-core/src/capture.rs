//! Channel mapping and de-interleaving.
//!
//! This is the heart of P1 and the part where a bug is both easy to introduce
//! and hard to notice: a swapped channel puts the lighting desk's words under
//! the sound engineer's label and nothing crashes. It therefore lives here as
//! pure, allocation-free logic with direct tests, separate from any device or
//! backend concern.

use crate::{AudioError, Result, StreamId};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Samples the de-interleaver gathers before handing them to a sink.
///
/// Lives on the stack inside the audio callback, so it must never grow to a
/// size that risks the callback's stack. 1024 f32 = 4 KiB, which comfortably
/// covers a typical 512-frame device buffer in one pass.
const SCRATCH_LEN: usize = 1024;

/// Which hardware channel feeds which logical stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelAssignment {
    pub stream_id: StreamId,
    /// Zero-based channel index on the device.
    pub channel: u16,
}

/// The complete mapping for one capture session.
///
/// Not every hardware channel has to be used, and the mapping does not have to
/// be contiguous or ordered — an engineer may well put channels 2, 5 and 6 on
/// three streams and leave the rest alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMap {
    assignments: Vec<ChannelAssignment>,
}

impl ChannelMap {
    /// Builds a mapping and validates it against the device's channel count.
    ///
    /// Rejects duplicate stream ids and out-of-range channels. Two streams
    /// reading the *same* channel is allowed — mirroring one channel into two
    /// views is unusual but not wrong.
    pub fn new(assignments: Vec<ChannelAssignment>, device_channels: u16) -> Result<Self> {
        if assignments.is_empty() {
            return Err(AudioError::Backend("channel map is empty".into()));
        }

        for assignment in &assignments {
            if assignment.channel >= device_channels {
                return Err(AudioError::ChannelOutOfRange {
                    channel: assignment.channel,
                    available: device_channels,
                });
            }
        }

        let mut seen: Vec<StreamId> = assignments.iter().map(|a| a.stream_id).collect();
        seen.sort_unstable();
        let unique = seen.len();
        seen.dedup();
        if seen.len() != unique {
            return Err(AudioError::Backend(
                "the same stream id is mapped more than once".into(),
            ));
        }

        Ok(Self { assignments })
    }

    pub fn assignments(&self) -> &[ChannelAssignment] {
        &self.assignments
    }

    pub fn len(&self) -> usize {
        self.assignments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
    }

    /// The highest channel index used. Useful for sanity checks against a
    /// device that reports fewer channels than the saved preset expects (P12).
    pub fn highest_channel(&self) -> u16 {
        self.assignments
            .iter()
            .map(|a| a.channel)
            .max()
            .unwrap_or(0)
    }
}

/// Destination for de-interleaved samples of a single stream.
///
/// Implementations are called from the realtime audio callback and must
/// therefore **not allocate, lock, or log**. Returning a short count is the
/// defined way to signal "full"; the caller turns that into an overrun count
/// rather than blocking.
pub trait SampleSink: Send {
    /// Accepts as many samples as fit and returns how many were taken.
    fn push(&mut self, samples: &[f32]) -> usize;
}

/// Counters written from the audio callback and read from anywhere.
///
/// Atomics rather than a mutex: the callback must never block. `Relaxed` is
/// sufficient — these are diagnostics, not synchronisation.
#[derive(Debug, Default)]
pub struct CaptureStats {
    frames_captured: AtomicU64,
    samples_dropped: AtomicU64,
    callbacks: AtomicU64,
}

impl CaptureStats {
    pub fn frames_captured(&self) -> u64 {
        self.frames_captured.load(Ordering::Relaxed)
    }

    /// Samples lost because a sink was full. Any non-zero value means the
    /// consumer is not keeping up — surfaced in the UI by P8.
    pub fn samples_dropped(&self) -> u64 {
        self.samples_dropped.load(Ordering::Relaxed)
    }

    pub fn callbacks(&self) -> u64 {
        self.callbacks.load(Ordering::Relaxed)
    }

    fn add_frames(&self, n: u64) {
        self.frames_captured.fetch_add(n, Ordering::Relaxed);
        self.callbacks.fetch_add(1, Ordering::Relaxed);
    }

    fn add_dropped(&self, n: u64) {
        if n > 0 {
            self.samples_dropped.fetch_add(n, Ordering::Relaxed);
        }
    }
}

/// De-interleaves an interleaved device buffer into per-stream sinks.
///
/// Owns its sinks on purpose. The alternative — handing them in per call —
/// forces the caller to build a slice of trait-object references inside the
/// audio callback, which is exactly the allocation the realtime path must not
/// do. Constructed once outside the callback; [`Demuxer::demux`] allocates
/// nothing.
pub struct Demuxer {
    device_channels: u16,
    /// Channel index per sink, index-aligned with `sinks`.
    channels: Vec<u16>,
    sinks: Vec<Box<dyn SampleSink>>,
    scratch: [f32; SCRATCH_LEN],
    stats: Arc<CaptureStats>,
}

impl Demuxer {
    /// `sinks` must be ordered exactly like `map.assignments()`.
    pub fn new(
        map: &ChannelMap,
        device_channels: u16,
        sinks: Vec<Box<dyn SampleSink>>,
        stats: Arc<CaptureStats>,
    ) -> Result<Self> {
        if sinks.len() != map.len() {
            return Err(AudioError::Backend(format!(
                "{} sinks for {} mapped streams",
                sinks.len(),
                map.len()
            )));
        }
        if map.highest_channel() >= device_channels {
            return Err(AudioError::ChannelOutOfRange {
                channel: map.highest_channel(),
                available: device_channels,
            });
        }

        Ok(Self {
            device_channels,
            channels: map.assignments().iter().map(|a| a.channel).collect(),
            sinks,
            scratch: [0.0; SCRATCH_LEN],
            stats,
        })
    }

    /// Splits `interleaved` and pushes each stream's samples into its sink.
    ///
    /// A trailing partial frame is ignored rather than consumed: consuming it
    /// would shift every later sample by one channel, and a silent permanent
    /// channel swap is far worse than one dropped frame.
    pub fn demux(&mut self, interleaved: &[f32]) {
        let channels = self.device_channels as usize;
        if channels == 0 {
            return;
        }
        let frames = interleaved.len() / channels;
        if frames == 0 {
            return;
        }
        self.stats.add_frames(frames as u64);

        for (sink, &channel) in self.sinks.iter_mut().zip(self.channels.iter()) {
            let mut dropped = 0u64;
            let mut frame = 0usize;

            // Gather in scratch-sized chunks so nothing allocates.
            while frame < frames {
                let take = SCRATCH_LEN.min(frames - frame);
                for (i, slot) in self.scratch[..take].iter_mut().enumerate() {
                    *slot = interleaved[(frame + i) * channels + channel as usize];
                }
                let accepted = sink.push(&self.scratch[..take]);
                dropped += (take - accepted) as u64;
                frame += take;
            }

            self.stats.add_dropped(dropped);
        }
    }

    pub fn stats(&self) -> &Arc<CaptureStats> {
        &self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    /// Handle the test keeps while the demuxer owns the sink itself.
    #[derive(Clone)]
    struct Recorded(Arc<Mutex<Vec<f32>>>);

    impl Recorded {
        fn samples(&self) -> Vec<f32> {
            self.0.lock().unwrap().clone()
        }

        fn len(&self) -> usize {
            self.0.lock().unwrap().len()
        }
    }

    /// Test sink with a fixed capacity, so "full" behaviour is testable.
    struct VecSink {
        taken: Arc<Mutex<Vec<f32>>>,
        capacity: usize,
    }

    impl VecSink {
        /// Not called `new`: it hands back a sink/handle pair rather than Self,
        /// because the demuxer takes ownership of the sink.
        fn bounded(capacity: usize) -> (Box<dyn SampleSink>, Recorded) {
            let taken = Arc::new(Mutex::new(Vec::new()));
            let handle = Recorded(taken.clone());
            (Box::new(Self { taken, capacity }), handle)
        }

        fn unlimited() -> (Box<dyn SampleSink>, Recorded) {
            Self::bounded(usize::MAX)
        }
    }

    impl SampleSink for VecSink {
        fn push(&mut self, samples: &[f32]) -> usize {
            let mut taken = self.taken.lock().unwrap();
            let room = self.capacity.saturating_sub(taken.len());
            let n = room.min(samples.len());
            taken.extend_from_slice(&samples[..n]);
            n
        }
    }

    fn map_of(pairs: &[(u16, u16)], device_channels: u16) -> ChannelMap {
        ChannelMap::new(
            pairs
                .iter()
                .map(|&(stream, channel)| ChannelAssignment {
                    stream_id: StreamId(stream),
                    channel,
                })
                .collect(),
            device_channels,
        )
        .unwrap()
    }

    #[test]
    fn rejects_a_channel_beyond_the_device() {
        let err = ChannelMap::new(
            vec![ChannelAssignment {
                stream_id: StreamId(0),
                channel: 8,
            }],
            8,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            AudioError::ChannelOutOfRange {
                channel: 8,
                available: 8
            }
        ));
    }

    #[test]
    fn rejects_a_duplicated_stream_id() {
        let err = ChannelMap::new(
            vec![
                ChannelAssignment {
                    stream_id: StreamId(1),
                    channel: 0,
                },
                ChannelAssignment {
                    stream_id: StreamId(1),
                    channel: 3,
                },
            ],
            8,
        )
        .unwrap_err();
        assert!(matches!(err, AudioError::Backend(_)));
    }

    #[test]
    fn rejects_an_empty_map() {
        assert!(ChannelMap::new(vec![], 8).is_err());
    }

    #[test]
    fn accepts_a_sparse_unordered_mapping() {
        // Channels need be neither contiguous nor ascending.
        let map = map_of(&[(0, 5), (1, 2), (2, 6)], 8);
        assert_eq!(map.len(), 3);
        assert_eq!(map.highest_channel(), 6);
    }

    #[test]
    fn two_streams_may_share_one_channel() {
        let map = map_of(&[(0, 1), (1, 1)], 4);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn demux_routes_each_channel_to_the_right_sink() {
        // Four channels, frame N carries [N.0, N.1, N.2, N.3] so a swap is
        // immediately visible in the values.
        let interleaved: Vec<f32> = (0..6)
            .flat_map(|frame| (0..4).map(move |ch| frame as f32 + ch as f32 / 10.0))
            .collect();

        // Deliberately reversed and sparse: stream 0 -> channel 3, stream 1 -> channel 0.
        let map = map_of(&[(0, 3), (1, 0)], 4);
        let stats = Arc::new(CaptureStats::default());
        let (sink_a, a) = VecSink::unlimited();
        let (sink_b, b) = VecSink::unlimited();
        let mut demuxer = Demuxer::new(&map, 4, vec![sink_a, sink_b], stats.clone()).unwrap();

        demuxer.demux(&interleaved);

        assert_eq!(a.samples(), vec![0.3, 1.3, 2.3, 3.3, 4.3, 5.3]);
        assert_eq!(b.samples(), vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(stats.frames_captured(), 6);
        assert_eq!(stats.samples_dropped(), 0);
    }

    #[test]
    fn demux_handles_buffers_larger_than_the_scratch_space() {
        // Forces several gather passes per channel — the loop boundary is
        // exactly where an off-by-one would hide.
        let frames = SCRATCH_LEN * 2 + 7;
        let interleaved: Vec<f32> = (0..frames)
            .flat_map(|frame| [frame as f32, -(frame as f32)])
            .collect();

        let map = map_of(&[(0, 0), (1, 1)], 2);
        let stats = Arc::new(CaptureStats::default());
        let (sink_a, a) = VecSink::unlimited();
        let (sink_b, b) = VecSink::unlimited();
        let mut demuxer = Demuxer::new(&map, 2, vec![sink_a, sink_b], stats.clone()).unwrap();

        demuxer.demux(&interleaved);

        assert_eq!(a.len(), frames);
        assert_eq!(b.len(), frames);
        assert_eq!(a.samples()[frames - 1], (frames - 1) as f32);
        assert_eq!(b.samples()[frames - 1], -((frames - 1) as f32));
        assert_eq!(stats.frames_captured(), frames as u64);
    }

    #[test]
    fn a_trailing_partial_frame_is_ignored() {
        // Five samples across four channels: one complete frame plus a stub.
        // Consuming the stub would shift every subsequent sample by one
        // channel — a silent, permanent channel swap.
        let interleaved = vec![0.0, 0.1, 0.2, 0.3, 9.9];
        let map = map_of(&[(0, 0)], 4);
        let stats = Arc::new(CaptureStats::default());
        let (sink, recorded) = VecSink::unlimited();
        let mut demuxer = Demuxer::new(&map, 4, vec![sink], stats.clone()).unwrap();

        demuxer.demux(&interleaved);

        assert_eq!(recorded.samples(), vec![0.0]);
        assert_eq!(stats.frames_captured(), 1);
    }

    #[test]
    fn a_full_sink_is_counted_as_dropped_not_blocked() {
        let interleaved: Vec<f32> = (0..10).map(|n| n as f32).collect();
        let map = map_of(&[(0, 0)], 1);
        let stats = Arc::new(CaptureStats::default());
        let (sink, recorded) = VecSink::bounded(4);
        let mut demuxer = Demuxer::new(&map, 1, vec![sink], stats.clone()).unwrap();

        demuxer.demux(&interleaved);

        assert_eq!(recorded.len(), 4);
        assert_eq!(stats.samples_dropped(), 6);
        // Frames were still observed — capture did not stall.
        assert_eq!(stats.frames_captured(), 10);
    }

    #[test]
    fn an_empty_buffer_is_harmless() {
        let map = map_of(&[(0, 0)], 2);
        let stats = Arc::new(CaptureStats::default());
        let (sink, recorded) = VecSink::unlimited();
        let mut demuxer = Demuxer::new(&map, 2, vec![sink], stats.clone()).unwrap();

        demuxer.demux(&[]);

        assert_eq!(recorded.len(), 0);
        assert_eq!(stats.frames_captured(), 0);
        assert_eq!(stats.callbacks(), 0);
    }

    /// `Demuxer` owns trait objects and so cannot derive `Debug`, which rules
    /// out `unwrap_err`.
    fn expect_err(result: Result<Demuxer>) -> AudioError {
        match result {
            Err(err) => err,
            Ok(_) => panic!("expected an error"),
        }
    }

    #[test]
    fn demuxer_rejects_a_sink_count_mismatch() {
        let map = map_of(&[(0, 0), (1, 1)], 2);
        let (sink, _) = VecSink::unlimited();
        let err = expect_err(Demuxer::new(
            &map,
            2,
            vec![sink],
            Arc::new(CaptureStats::default()),
        ));
        assert!(matches!(err, AudioError::Backend(_)));
    }

    #[test]
    fn demuxer_rejects_a_map_wider_than_the_device() {
        // Guards the preset case (P12): an interface that comes back with
        // fewer channels than the saved mapping expects must fail loudly
        // rather than read past the buffer.
        let map = map_of(&[(0, 6)], 8);
        let err = expect_err(Demuxer::new(
            &map,
            4,
            vec![VecSink::unlimited().0],
            Arc::default(),
        ));
        assert!(matches!(
            err,
            AudioError::ChannelOutOfRange {
                channel: 6,
                available: 4
            }
        ));
    }
}

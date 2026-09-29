//! The way out: mono audio into an output device (P22, P26).
//!
//! Until these two packages the app only ever listened. Everything here runs
//! in, or talks to, an output callback, which is a realtime context exactly
//! like the capture callback in [`crate::capture`]: **no allocation, no lock,
//! no logging**. What cannot be done that way — resampling a replay, dropping
//! the buffer it came in — is done on the other side of a lock-free ring.

use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Samples the spreader gathers per pass, for [`crate::capture`]'s reason:
/// it lives inside the callback, so it is fixed and small.
const SCRATCH_LEN: usize = 1024;

/// Where a playback stream's audio comes from.
///
/// Called from the realtime output callback, so implementations must **not
/// allocate, lock or log**. `out` is mono and arrives zeroed; a source that
/// has nothing to play leaves it that way.
pub trait SampleSource: Send {
    fn fill(&mut self, out: &mut [f32]);
}

/// Writes a mono source into chosen channels of an interleaved device buffer.
///
/// The counterpart of [`crate::Demuxer`], and for the same reason a type of
/// its own with direct tests: a channel written to the wrong output is talkback
/// on the wrong speaker, and nothing crashes.
pub struct Spreader {
    device_channels: u16,
    channels: Vec<u16>,
    source: Box<dyn SampleSource>,
    scratch: [f32; SCRATCH_LEN],
}

impl Spreader {
    /// `channels` are validated here, outside the callback, so that a bad
    /// route fails when it is opened rather than writing past a frame.
    pub fn new(
        channels: Vec<u16>,
        device_channels: u16,
        source: Box<dyn SampleSource>,
    ) -> crate::Result<Self> {
        if channels.is_empty() {
            return Err(crate::AudioError::Backend(
                "an output needs at least one channel".into(),
            ));
        }
        if let Some(&channel) = channels.iter().find(|&&c| c >= device_channels) {
            return Err(crate::AudioError::ChannelOutOfRange {
                channel,
                available: device_channels,
            });
        }
        Ok(Self {
            device_channels,
            channels,
            source,
            scratch: [0.0; SCRATCH_LEN],
        })
    }

    /// Fills one interleaved device buffer. Every channel not routed is
    /// written silent, and a trailing partial frame is silenced too rather
    /// than half-written.
    pub fn spread(&mut self, interleaved: &mut [f32]) {
        interleaved.fill(0.0);
        let channels = self.device_channels as usize;
        if channels == 0 {
            return;
        }
        let frames = interleaved.len() / channels;
        let mut frame = 0usize;
        while frame < frames {
            let take = SCRATCH_LEN.min(frames - frame);
            let scratch = &mut self.scratch[..take];
            scratch.fill(0.0);
            self.source.fill(scratch);
            for (i, &sample) in scratch.iter().enumerate() {
                let base = (frame + i) * channels;
                for &channel in &self.channels {
                    interleaved[base + channel as usize] = sample;
                }
            }
            frame += take;
        }
    }
}

/// What the player is told from outside the callback.
enum Cue {
    Play { id: u64, samples: Vec<f32> },
    Stop,
}

/// How many cues may wait for the callback. A replay button pressed a few
/// times between two callbacks is the most this ever holds.
const CUES: usize = 8;

/// How many finished buffers may wait to be dropped. Freed on the control
/// side the next time anything is played, so this only has to cover the
/// cues in flight.
const RETURNS: usize = 16;

/// What is playing, readable from anywhere.
#[derive(Debug, Default)]
pub struct PlayerState {
    /// The id of the cue that is playing, or 0 for nothing.
    playing: AtomicU64,
}

impl PlayerState {
    /// The id of what is playing, if anything.
    pub fn playing(&self) -> Option<u64> {
        match self.playing.load(Ordering::Relaxed) {
            0 => None,
            id => Some(id),
        }
    }
}

/// The realtime half of a replay player: plays one buffer at a time,
/// mixed onto whatever the output already holds.
///
/// The buffer is *moved* in through a ring and moved back out through a
/// second one when it is done, so the callback neither allocates it nor
/// frees it. Freeing is the half that is easy to forget: dropping a `Vec` in
/// the callback is a call into the allocator on the realtime thread.
pub struct Player {
    cues: HeapCons<Cue>,
    returns: HeapProd<Vec<f32>>,
    current: Option<(u64, Vec<f32>, usize)>,
    state: Arc<PlayerState>,
}

/// The control half: where a replay is started and stopped from.
pub struct PlayerHandle {
    cues: HeapProd<Cue>,
    returns: HeapCons<Vec<f32>>,
    state: Arc<PlayerState>,
}

/// One player and the handle that drives it.
pub fn player() -> (Player, PlayerHandle) {
    let (cue_tx, cue_rx) = HeapRb::<Cue>::new(CUES).split();
    let (ret_tx, ret_rx) = HeapRb::<Vec<f32>>::new(RETURNS).split();
    let state = Arc::new(PlayerState::default());
    (
        Player {
            cues: cue_rx,
            returns: ret_tx,
            current: None,
            state: state.clone(),
        },
        PlayerHandle {
            cues: cue_tx,
            returns: ret_rx,
            state,
        },
    )
}

impl Player {
    /// Adds what is playing onto `out`, sample by sample.
    ///
    /// Adds rather than writes, so the shoutbox can mix a replay onto the
    /// program it is passing through (P26). A source of its own for the
    /// headphones is the same call onto a zeroed buffer.
    pub fn mix_into(&mut self, out: &mut [f32]) {
        while let Some(cue) = self.cues.try_pop() {
            // Whatever was playing is finished either way: a new replay
            // replaces it, and Stop ends it.
            self.finish();
            if let Cue::Play { id, samples } = cue {
                self.state.playing.store(id, Ordering::Relaxed);
                self.current = Some((id, samples, 0));
            }
        }

        let Some((_, samples, position)) = self.current.as_mut() else {
            return;
        };
        let take = (samples.len() - *position).min(out.len());
        for (slot, sample) in out.iter_mut().zip(&samples[*position..*position + take]) {
            *slot += sample;
        }
        *position += take;
        if *position >= samples.len() {
            self.finish();
        }
    }

    fn finish(&mut self) {
        if let Some((_, samples, _)) = self.current.take() {
            self.state.playing.store(0, Ordering::Relaxed);
            // Full means the control side has not collected for a while. The
            // buffer is then freed here after all, which is a rare allocator
            // call rather than a leak.
            let _ = self.returns.try_push(samples);
        }
    }
}

impl SampleSource for Player {
    fn fill(&mut self, out: &mut [f32]) {
        self.mix_into(out);
    }
}

impl PlayerHandle {
    /// Plays `samples`, which must already be at the output's rate, in place
    /// of whatever is playing. `id` is what [`PlayerState::playing`] reports
    /// while it does; 0 is reserved for "nothing".
    pub fn play(&mut self, id: u64, samples: Vec<f32>) -> crate::Result<()> {
        self.collect();
        self.cues
            .try_push(Cue::Play {
                id: id.max(1),
                samples,
            })
            .map_err(|_| crate::AudioError::Backend("the player is not taking cues".into()))
    }

    pub fn stop(&mut self) {
        self.collect();
        let _ = self.cues.try_push(Cue::Stop);
    }

    pub fn state(&self) -> &Arc<PlayerState> {
        &self.state
    }

    /// Frees the buffers the callback has finished with.
    fn collect(&mut self) {
        while self.returns.try_pop().is_some() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source counting up from 1, so a sample landing on the wrong channel
    /// or the wrong frame is visible in its value.
    struct Ramp(f32);

    impl SampleSource for Ramp {
        fn fill(&mut self, out: &mut [f32]) {
            for slot in out {
                self.0 += 1.0;
                *slot = self.0;
            }
        }
    }

    #[test]
    fn the_mono_signal_lands_on_the_chosen_channels_only() {
        let mut spreader = Spreader::new(vec![2, 3], 4, Box::new(Ramp(0.0))).unwrap();
        let mut buffer = vec![9.0; 12];
        spreader.spread(&mut buffer);
        assert_eq!(
            buffer,
            vec![0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 2.0, 2.0, 0.0, 0.0, 3.0, 3.0]
        );
    }

    #[test]
    fn a_buffer_longer_than_the_scratch_space_stays_in_order() {
        let frames = SCRATCH_LEN * 2 + 5;
        let mut spreader = Spreader::new(vec![0], 1, Box::new(Ramp(0.0))).unwrap();
        let mut buffer = vec![0.0; frames];
        spreader.spread(&mut buffer);
        assert!(buffer.iter().enumerate().all(|(i, &s)| s == (i + 1) as f32));
    }

    #[test]
    fn a_route_past_the_device_is_refused_before_anything_plays() {
        assert!(Spreader::new(vec![4], 4, Box::new(Ramp(0.0))).is_err());
        assert!(Spreader::new(vec![], 4, Box::new(Ramp(0.0))).is_err());
    }

    #[test]
    fn a_replay_plays_once_and_reports_itself_while_it_does() {
        let (mut player, mut handle) = player();
        handle.play(7, vec![0.5; 5]).unwrap();

        let mut out = vec![0.0; 3];
        player.fill(&mut out);
        assert_eq!(out, vec![0.5; 3]);
        assert_eq!(handle.state().playing(), Some(7));

        let mut out = vec![0.0; 4];
        player.fill(&mut out);
        assert_eq!(out, vec![0.5, 0.5, 0.0, 0.0]);
        assert_eq!(handle.state().playing(), None);
    }

    #[test]
    fn a_second_replay_replaces_the_first_and_stop_ends_it() {
        let (mut player, mut handle) = player();
        handle.play(1, vec![1.0; 100]).unwrap();
        let mut out = vec![0.0; 10];
        player.fill(&mut out);

        handle.play(2, vec![2.0; 100]).unwrap();
        let mut out = vec![0.0; 2];
        player.fill(&mut out);
        assert_eq!(out, vec![2.0, 2.0]);
        assert_eq!(handle.state().playing(), Some(2));

        handle.stop();
        let mut out = vec![0.0; 2];
        player.fill(&mut out);
        assert_eq!(out, vec![0.0, 0.0]);
        assert_eq!(handle.state().playing(), None);
    }

    #[test]
    fn a_replay_is_mixed_onto_what_is_already_there() {
        let (mut player, mut handle) = player();
        handle.play(1, vec![0.25; 2]).unwrap();
        let mut out = vec![0.5; 2];
        player.mix_into(&mut out);
        assert_eq!(out, vec![0.75; 2]);
    }
}

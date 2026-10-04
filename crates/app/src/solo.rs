//! Hearing an input: the S beside the M.
//!
//! A soloed channel is played on the listen output — the headphones replays
//! go to (P22) — so the operator can hear what is arriving on it. Solo
//! *latches*, the way it does on an intercom panel rather than on a desk's
//! exclusive solo: pressing a second channel adds it, and each one stays in
//! until it is pressed again.
//!
//! Like mute, solo is about the operator and not about the transcription: it
//! changes nothing that is decoded, stored or shown, and it is not saved. It
//! belongs to the app rather than to a session, because it is pressed before
//! a show as much as during one — the meters before Start come off the
//! monitor (P8), and a channel is checked by ear there first. Both the
//! session and the monitor tap their inputs through here, and a solo pressed
//! during the sound check is still in when the session takes the device over.
//!
//! The audio is tapped in the capture callback, before anything else touches
//! it, at the interface's own rate: what the operator hears is the line, not
//! the 16 kHz whisper gets. The channel's trim is applied, so the level in
//! the headphones moves with the knob beside the meter.

use audio_core::gain::Gain;
use audio_core::{ring, Resampler, RingSink, RingSource, SampleSink, Tap};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How much of one channel the tap holds. The mixer empties it every pump;
/// the rest is headroom for a thread that was not scheduled.
const TAP_SECONDS: f32 = 0.5;

/// How far one soloed channel may run ahead of the others before the excess
/// is dropped. They are written in the same callback, so they only differ by
/// what arrived between two reads — and by the moment one was pressed.
const ALIGN_SECONDS: f32 = 0.05;

/// Which channels are soloed, as every view shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "solo.ts")]
pub struct SoloState {
    /// Stream ids, in order.
    pub streams: Vec<u16>,
}

/// Where the mix goes: the listen output's ring, at its rate.
struct Feed {
    sink: RingSink,
    rate: u32,
    /// From the inputs' rate to `rate`, made at the first push.
    resampler: Option<Resampler>,
    out: Vec<f32>,
}

/// The solo flags and the way to the headphones. One per app.
#[derive(Default)]
pub struct Bus {
    /// One flag per stream id that has ever been asked for, shared with the
    /// taps in the capture callback.
    flags: Mutex<BTreeMap<u16, Arc<AtomicBool>>>,
    feed: Mutex<Option<Feed>>,
}

impl Bus {
    /// The flag of one stream, made on first use.
    fn flag(&self, stream_id: u16) -> Arc<AtomicBool> {
        self.flags
            .lock()
            .map(|mut flags| flags.entry(stream_id).or_default().clone())
            .unwrap_or_default()
    }

    pub fn set(&self, stream_id: u16, soloed: bool) {
        self.flag(stream_id).store(soloed, Ordering::Relaxed);
    }

    pub fn state(&self) -> SoloState {
        SoloState {
            streams: self
                .flags
                .lock()
                .map(|flags| {
                    flags
                        .iter()
                        .filter(|(_, on)| on.load(Ordering::Relaxed))
                        .map(|(&id, _)| id)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    pub fn any(&self) -> bool {
        !self.state().streams.is_empty()
    }

    /// Where the mix goes from now on: the listen output, as it was (re)opened.
    pub(crate) fn connect(&self, sink: RingSink, rate: u32) {
        if let Ok(mut feed) = self.feed.lock() {
            *feed = Some(Feed {
                sink,
                rate,
                resampler: None,
                out: Vec::new(),
            });
        }
    }

    /// The listen output went away.
    pub(crate) fn disconnect(&self) {
        if let Ok(mut feed) = self.feed.lock() {
            *feed = None;
        }
    }

    /// Hands one stretch of the mix to the listen output.
    fn push(&self, samples: &[f32], rate: u32) {
        let Ok(mut feed) = self.feed.lock() else {
            return;
        };
        let Some(feed) = feed.as_mut() else {
            return;
        };
        if feed.resampler.as_ref().map(Resampler::input_rate) != Some(rate) {
            feed.resampler = Resampler::between(rate, feed.rate).ok();
        }
        let Some(resampler) = feed.resampler.as_mut() else {
            return;
        };
        feed.out.clear();
        if resampler.process(samples, &mut feed.out).is_ok() {
            let _ = feed.sink.push(&feed.out);
        }
    }

    /// Wraps one stream's capture sink so that it is also heard while the
    /// stream is soloed. The sink goes to the capture; the tap to a [`Mixer`].
    pub fn tap(
        &self,
        stream_id: u16,
        sink: RingSink,
        rate: u32,
        gain: Arc<Gain>,
    ) -> (Box<dyn SampleSink>, TapReader) {
        let on = self.flag(stream_id);
        let (copy, source) = ring::channel(rate, TAP_SECONDS);
        (
            Box::new(Tap::new(sink, copy, on.clone())),
            TapReader { source, on, gain },
        )
    }
}

/// The reading end of one stream's tap.
pub struct TapReader {
    source: RingSource,
    on: Arc<AtomicBool>,
    gain: Arc<Gain>,
}

/// Mixes the soloed streams of one capture and sends them on. Lives on the
/// thread that reads the capture, and is pumped with it.
pub struct Mixer {
    bus: Arc<Bus>,
    rate: u32,
    taps: Vec<TapReader>,
    scratch: Vec<f32>,
    mix: Vec<f32>,
}

impl Mixer {
    pub fn new(bus: Arc<Bus>, rate: u32, taps: Vec<TapReader>) -> Self {
        Self {
            bus,
            rate,
            taps,
            scratch: Vec::new(),
            mix: Vec::new(),
        }
    }

    pub fn pump(&mut self) {
        let mut ready = usize::MAX;
        let mut any = false;
        for tap in &mut self.taps {
            if tap.on.load(Ordering::Relaxed) {
                any = true;
                ready = ready.min(tap.source.available());
            } else if !tap.source.is_empty() {
                // Released since the last pump: what it left must not be
                // played the next time it is pressed.
                tap.source.skip(tap.source.available());
            }
        }
        if !any || ready == 0 {
            return;
        }

        let align = (self.rate as f32 * ALIGN_SECONDS) as usize;
        self.mix.clear();
        self.mix.resize(ready, 0.0);
        self.scratch.resize(ready, 0.0);
        for tap in self
            .taps
            .iter_mut()
            .filter(|t| t.on.load(Ordering::Relaxed))
        {
            let ahead = tap.source.available().saturating_sub(ready);
            if ahead > align {
                tap.source.skip(ahead);
            }
            let read = tap.source.read(&mut self.scratch[..ready]);
            tap.gain.apply(&mut self.scratch[..read]);
            for (m, s) in self.mix.iter_mut().zip(&self.scratch[..read]) {
                *m += s;
            }
        }
        self.bus.push(&self.mix, self.rate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(bus: &Bus, rate: u32) -> RingSource {
        let (sink, source) = ring::channel(rate, 1.0);
        bus.connect(sink, rate);
        source
    }

    fn drain(source: &mut RingSource) -> Vec<f32> {
        let mut out = vec![0.0; source.available()];
        source.read(&mut out);
        out
    }

    #[test]
    fn only_soloed_streams_are_heard_and_they_are_mixed() {
        let bus = Arc::new(Bus::default());
        let mut headphones = heard(&bus, 1000);
        let mut sinks = Vec::new();
        let mut taps = Vec::new();
        for id in 0..3 {
            let (sink, _) = ring::channel(1000, 1.0);
            let (sink, tap) = bus.tap(id, sink, 1000, Arc::new(Gain::new(0)));
            sinks.push(sink);
            taps.push(tap);
        }
        let mut mixer = Mixer::new(bus.clone(), 1000, taps);

        bus.set(0, true);
        bus.set(2, true);
        assert_eq!(bus.state().streams, vec![0, 2]);
        for (id, sink) in sinks.iter_mut().enumerate() {
            sink.push(&[0.1 * (id + 1) as f32; 4]);
        }
        mixer.pump();
        let mix = drain(&mut headphones);
        assert_eq!(mix.len(), 4);
        assert!(mix.iter().all(|s| (s - 0.4).abs() < 1e-6), "{mix:?}");
    }

    #[test]
    fn a_released_solo_leaves_nothing_behind_for_the_next_press() {
        let bus = Arc::new(Bus::default());
        let mut headphones = heard(&bus, 1000);
        let (sink, _) = ring::channel(1000, 1.0);
        let (mut sink, tap) = bus.tap(5, sink, 1000, Arc::new(Gain::new(0)));
        let mut mixer = Mixer::new(bus.clone(), 1000, vec![tap]);

        bus.set(5, true);
        sink.push(&[0.5; 8]);
        bus.set(5, false);
        mixer.pump();
        assert!(drain(&mut headphones).is_empty());

        bus.set(5, true);
        sink.push(&[0.25; 2]);
        mixer.pump();
        assert_eq!(drain(&mut headphones), vec![0.25; 2]);
    }

    #[test]
    fn the_trim_reaches_the_headphones() {
        let bus = Arc::new(Bus::default());
        let mut headphones = heard(&bus, 1000);
        let (sink, _) = ring::channel(1000, 1.0);
        let gain = Arc::new(Gain::new(0));
        let (mut sink, tap) = bus.tap(0, sink, 1000, gain.clone());
        let mut mixer = Mixer::new(bus.clone(), 1000, vec![tap]);
        bus.set(0, true);
        gain.set_db(-6);
        sink.push(&[0.5; 2]);
        mixer.pump();
        let mix = drain(&mut headphones);
        assert!(mix.iter().all(|s| (s - 0.25).abs() < 0.01), "{mix:?}");
    }

    #[test]
    fn the_capture_keeps_every_sample_whether_or_not_it_is_heard() {
        let bus = Bus::default();
        let (sink, mut transcribed) = ring::channel(1000, 1.0);
        let (mut sink, _tap) = bus.tap(0, sink, 1000, Arc::new(Gain::new(0)));
        sink.push(&[0.1; 3]);
        bus.set(0, true);
        sink.push(&[0.2; 3]);
        assert_eq!(drain(&mut transcribed).len(), 6);
    }
}

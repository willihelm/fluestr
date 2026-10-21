//! Who is talking right now, before there is any text to show for it (P23).
//!
//! The chat is silent for the whole of the wait between somebody opening their
//! mouth and the text arriving — the pause the segmenter waits out, then the
//! decode. Nothing here makes that shorter. What it does is let the operator
//! see *who* it is at once: the moment the speech gate opens on a channel, the
//! views put a placeholder where that channel's bubble will land.
//!
//! The hard part is not the start but the end. A placeholder that stays is a
//! lie on the operator's screen, and a gate that opened does not always lead
//! to a line in the chat: a cough is shorter than the minimum segment, whisper
//! returns nothing or only an annotation the chat filters out, a segment is
//! shed under overload (P5) or lost with a sidecar. So a channel is not active
//! merely while its gate is open, but until every segment that gate produced
//! has been *accounted for* — and the pool answers every segment exactly once,
//! whatever became of it, which is what makes the count below come back to
//! zero. The views keep a timeout behind this as a safety net; it is not the
//! mechanism.
//!
//! Edge-triggered: one event when a channel's state changes, never per frame.
//! The levels event already covers the per-frame case.
//!
//! The live preview (P28) rides on the same event: what has been heard of the
//! words on their way so far, decoded while they are still being spoken. It
//! is part of the channel's state like the gate is, so every view that shows
//! the placeholder — a group window, the ticker, the remote, a window opened
//! mid-sentence — gets it the same way and loses it the same way.

use crate::session::EventSink;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A channel started or stopped having something on its way to the chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct SpeechActivity {
    /// See [`crate::session::TranscriptEvent::session_id`].
    pub session_id: u64,
    pub stream_id: u16,
    /// The channel's label as it stands, so a placeholder for a channel that
    /// has not spoken yet this session can say whose it is.
    pub label: String,
    /// The speech gate is open: somebody is talking.
    pub speaking: bool,
    /// Segments of this channel have been closed and have not come back yet:
    /// what was said is being transcribed. Separate from `speaking` because
    /// the two overlap — a long sentence is cut while it is still being
    /// spoken — and a view may want to say which it is.
    pub decoding: bool,
    /// When the current stretch of activity began, in milliseconds on the
    /// session clock — the one `t_start_ms` and `t_end_ms` are on. What a view
    /// compares with the last bubble's end to decide whether the words on
    /// their way will join it or start a new one.
    pub since_ms: u64,
    /// What has been heard so far of the words on their way (P28): the last
    /// preview of every segment still being decoded, then the one still being
    /// spoken. Empty when there is none — the live preview is off, the first
    /// one has not come back yet, or it was not speech.
    ///
    /// Provisional by nature. Each preview replaces the one before it, and
    /// the transcript replaces all of them; nothing here is ever a line of the
    /// chat, and nothing here is seen by the rules.
    pub preview: String,
}

impl SpeechActivity {
    /// Whether there is anything to show for this channel.
    pub fn active(&self) -> bool {
        self.speaking || self.decoding
    }
}

#[derive(Debug, Clone, Default)]
struct Stream {
    gate: bool,
    /// Segments submitted and not yet settled.
    outstanding: u32,
    since_ms: u64,
    /// Counts the segments this channel has opened, so that a preview can be
    /// matched to the one it was taken from (P28). Moves whenever the open
    /// segment stops being open — closed and submitted, or abandoned with the
    /// gate — so a preview that comes back after that is recognisably stale.
    generation: u64,
    /// The latest preview of the segment still being spoken.
    open: String,
    /// The latest preview of each segment that has closed and not come back,
    /// by sequence number — what stays on screen until its transcript does.
    closing: BTreeMap<u64, String>,
}

impl Stream {
    fn visible(&self) -> (bool, bool, String) {
        let preview = self
            .closing
            .values()
            .chain(std::iter::once(&self.open))
            .map(|text| text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        (self.gate, self.outstanding > 0, preview)
    }
}

/// What the processing thread and the aggregator tell each other about each
/// channel, turned into [`SpeechActivity`] events.
///
/// Written from both threads — the processing thread opens and closes the gate
/// and submits segments, the aggregator settles them — so the state is behind
/// one lock, and the event is sent while it is held. Two threads each
/// emitting outside it could deliver "ended" before the "started" it ends.
pub struct Activity {
    session_id: u64,
    started: Instant,
    labels: Arc<Mutex<HashMap<u16, String>>>,
    events: Arc<dyn EventSink>,
    streams: Mutex<HashMap<u16, Stream>>,
}

impl Activity {
    pub fn new(
        session_id: u64,
        started: Instant,
        labels: Arc<Mutex<HashMap<u16, String>>>,
        events: Arc<dyn EventSink>,
    ) -> Self {
        Self {
            session_id,
            started,
            labels,
            events,
            streams: Mutex::new(HashMap::new()),
        }
    }

    /// The speech gate of `stream` opened or closed.
    ///
    /// Closing it abandons whatever preview the open segment had: a segment
    /// that closed properly was submitted first and took its preview with it,
    /// so what is left here belongs to one too short to be decoded at all.
    pub fn gate(&self, stream: u16, open: bool) {
        self.change(stream, |s| {
            s.gate = open;
            if !open {
                s.open.clear();
                s.generation += 1;
            }
        });
    }

    /// A segment of `stream` is on its way to the pool. Called *before* it is
    /// submitted, so the aggregator cannot settle it first.
    ///
    /// Its preview moves with it, to stay on screen until its text arrives;
    /// whatever is spoken next is a new segment with previews of its own.
    pub fn submitted(&self, stream: u16, seq: u64) {
        self.change(stream, |s| {
            s.outstanding += 1;
            let open = std::mem::take(&mut s.open);
            if !open.is_empty() {
                s.closing.insert(seq, open);
            }
            s.generation += 1;
        });
    }

    /// A segment of `stream` came back from the pool — as text, as nothing, or
    /// as a loss. Called after its transcript went out, if it had one, so a
    /// view sees the text land before the placeholder goes.
    pub fn settled(&self, stream: u16, seq: u64) {
        self.change(stream, |s| {
            s.outstanding = s.outstanding.saturating_sub(1);
            s.closing.remove(&seq);
        });
    }

    /// What a preview of `stream`'s open segment is tagged with when it is
    /// asked for (P28), so that [`Activity::preview`] can tell whether that
    /// segment is still the one being spoken when the text comes back.
    pub fn generation(&self, stream: u16) -> u64 {
        self.streams
            .lock()
            .ok()
            .and_then(|streams| streams.get(&stream).map(|s| s.generation))
            .unwrap_or(0)
    }

    /// A preview of `stream` came back (P28). Shown if the segment it was
    /// taken from is still open, dropped if it has closed since — the
    /// segment's own text is on its way by then, and a late preview would
    /// put older words over newer ones.
    pub fn preview(&self, stream: u16, generation: u64, text: &str) {
        self.change(stream, |s| {
            if s.gate && s.generation == generation {
                s.open = text.trim().to_string();
            }
        });
    }

    /// The session is over: whatever is still showing ends now. Nothing more
    /// is coming for any channel, whether or not the count agreed.
    pub fn end(&self) {
        let ids: Vec<u16> = match self.streams.lock() {
            Ok(streams) => streams.keys().copied().collect(),
            Err(_) => return,
        };
        for stream in ids {
            self.change(stream, |s| *s = Stream::default());
        }
    }

    fn change(&self, stream: u16, f: impl FnOnce(&mut Stream)) {
        let Ok(mut streams) = self.streams.lock() else {
            return;
        };
        let state = streams.entry(stream).or_default();
        let before = state.visible();
        let was_active = before.0 || before.1;
        f(state);
        let after = state.visible();
        if after == before {
            return;
        }
        let (speaking, decoding, preview) = after;
        if !was_active {
            state.since_ms = self.started.elapsed().as_millis() as u64;
        }
        let label = self
            .labels
            .lock()
            .ok()
            .and_then(|labels| labels.get(&stream).cloned())
            .unwrap_or_else(|| format!("stream {stream}"));
        self.events.activity(SpeechActivity {
            session_id: self.session_id,
            stream_id: stream,
            label,
            speaking,
            decoding,
            since_ms: state.since_ms,
            preview,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{LevelsEvent, OverloadEvent, SessionState, TranscriptEvent};

    #[derive(Default)]
    struct Recorder(Mutex<Vec<SpeechActivity>>);

    impl EventSink for Recorder {
        fn transcript(&self, _: TranscriptEvent) {}
        fn levels(&self, _: LevelsEvent) {}
        fn overload(&self, _: OverloadEvent) {}
        fn state(&self, _: SessionState) {}
        fn activity(&self, event: SpeechActivity) {
            self.0.lock().unwrap().push(event);
        }
    }

    fn activity() -> (Activity, Arc<Recorder>) {
        let recorder = Arc::new(Recorder::default());
        let labels = Arc::new(Mutex::new(HashMap::from([(0, "Regie".to_string())])));
        let activity = Activity::new(1, Instant::now(), labels, recorder.clone());
        (activity, recorder)
    }

    fn states(recorder: &Recorder) -> Vec<(bool, bool)> {
        recorder
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|e| (e.speaking, e.decoding))
            .collect()
    }

    #[test]
    fn a_sentence_is_active_from_the_gate_until_its_text_is_back() {
        let (activity, recorder) = activity();
        activity.gate(0, true);
        activity.submitted(0, 0);
        activity.gate(0, false);
        activity.settled(0, 0);

        assert_eq!(
            states(&recorder),
            [(true, false), (true, true), (false, true), (false, false)]
        );
        assert_eq!(recorder.0.lock().unwrap()[0].label, "Regie");
    }

    #[test]
    fn a_cough_ends_the_moment_the_gate_closes() {
        // Shorter than the minimum segment: nothing is submitted, so nothing
        // is waited for.
        let (activity, recorder) = activity();
        activity.gate(0, true);
        activity.gate(0, false);
        assert_eq!(states(&recorder), [(true, false), (false, false)]);
    }

    #[test]
    fn nothing_is_sent_when_nothing_changed() {
        // The processing thread reports the gate on every pump it moves; a
        // second segment in flight is not a new state either.
        let (activity, recorder) = activity();
        activity.gate(0, true);
        activity.gate(0, true);
        activity.submitted(0, 0);
        activity.submitted(0, 1);
        activity.settled(0, 0);
        assert_eq!(states(&recorder), [(true, false), (true, true)]);
    }

    #[test]
    fn every_segment_has_to_come_back_before_it_ends() {
        let (activity, recorder) = activity();
        activity.gate(0, true);
        activity.submitted(0, 0);
        activity.submitted(0, 1);
        activity.gate(0, false);
        activity.settled(0, 0);
        assert_eq!(states(&recorder).last(), Some(&(false, true)));
        activity.settled(0, 1);
        assert_eq!(states(&recorder).last(), Some(&(false, false)));
    }

    #[test]
    fn a_sentence_carried_on_keeps_the_moment_it_began() {
        let (activity, recorder) = activity();
        activity.gate(0, true);
        activity.submitted(0, 0);
        activity.gate(0, false);
        std::thread::sleep(std::time::Duration::from_millis(5));
        activity.gate(0, true);
        let events = recorder.0.lock().unwrap();
        assert!(events.iter().all(|e| e.since_ms == events[0].since_ms));
    }

    #[test]
    fn the_end_of_the_session_ends_everything() {
        let (activity, recorder) = activity();
        activity.gate(0, true);
        activity.submitted(0, 0);
        activity.gate(3, true);
        activity.end();
        let events = recorder.0.lock().unwrap();
        for stream in [0, 3] {
            let last = events.iter().rev().find(|e| e.stream_id == stream).unwrap();
            assert!(!last.active(), "stream {stream} still active");
        }
        assert_eq!(
            events.iter().find(|e| e.stream_id == 3).unwrap().label,
            "stream 3"
        );
    }

    fn previews(recorder: &Recorder) -> Vec<String> {
        recorder
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.preview.clone())
            .collect()
    }

    #[test]
    fn a_preview_fills_the_placeholder_until_the_text_is_back() {
        let (activity, recorder) = activity();
        activity.gate(0, true);
        let open = activity.generation(0);
        activity.preview(0, open, "Achtung");
        activity.preview(0, open, "Achtung, Licht");
        activity.submitted(0, 0);
        activity.gate(0, false);
        activity.settled(0, 0);
        assert_eq!(
            previews(&recorder),
            [
                "",
                "Achtung",
                "Achtung, Licht",
                "Achtung, Licht",
                "Achtung, Licht",
                ""
            ]
        );
    }

    #[test]
    fn a_preview_of_a_segment_that_has_closed_is_dropped() {
        // The real decode is on its way; the late preview would put older
        // words over the newer ones of the segment after it.
        let (activity, recorder) = activity();
        activity.gate(0, true);
        let first = activity.generation(0);
        activity.submitted(0, 0);
        activity.preview(0, first, "stale");
        assert!(previews(&recorder).iter().all(|p| p.is_empty()));
    }

    #[test]
    fn a_long_sentence_shows_the_closed_part_and_the_open_part() {
        // A forced cut: the first part is decoding, the second still spoken.
        let (activity, recorder) = activity();
        activity.gate(0, true);
        activity.preview(0, activity.generation(0), "erster Teil");
        activity.submitted(0, 0);
        activity.preview(0, activity.generation(0), "zweiter");
        assert_eq!(previews(&recorder).last().unwrap(), "erster Teil zweiter");
        activity.settled(0, 0);
        assert_eq!(previews(&recorder).last().unwrap(), "zweiter");
    }

    #[test]
    fn a_cough_takes_its_preview_with_it() {
        let (activity, recorder) = activity();
        activity.gate(0, true);
        let open = activity.generation(0);
        activity.preview(0, open, "hm");
        activity.gate(0, false);
        activity.preview(0, open, "hm");
        let events = recorder.0.lock().unwrap();
        let last = events.last().unwrap();
        assert!(!last.active());
        assert!(last.preview.is_empty());
    }
}

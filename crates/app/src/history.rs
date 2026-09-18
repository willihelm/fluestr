//! What a window opened later has to be caught up on.
//!
//! The chat lives in the window (P10 was set aside), and until now that meant
//! it lived in *one* window: whatever had been transcribed was in the main
//! view's own array and nowhere else. A detached group window (P7) breaks that
//! assumption — it is opened in the middle of a show and has to show what was
//! already said, not start blank at the moment it appeared.
//!
//! So the bridge keeps what it has emitted. Not as persistence — nothing here
//! reaches the disk, and closing the app still ends the chat — but as the one
//! copy every view is caught up from, which is also what makes two windows
//! agree about what "Clear all communication" cleared.

use std::collections::{BTreeMap, VecDeque};

use serde::Serialize;

use crate::session::{SessionState, TranscriptEvent};

/// How many transcripts a window opened later can be caught up on.
///
/// A bound rather than the whole session, because this grows for as long as
/// the app is open and nothing ever asks for the beginning of it: the chat is
/// read at its end, and reaching further back is a page at a time. Ten
/// thousand segments is several hours of eight busy channels, and at a few
/// hundred bytes each it is a couple of megabytes — small next to a loaded
/// model, and large enough that no show reaches it.
///
/// The window that was already open keeps its own list and is unaffected; what
/// this decides is how much history a *newly* opened one is given.
pub const CAPACITY: usize = 10_000;

/// When one session's clock was zero, in milliseconds since the Unix epoch.
///
/// Kept per session rather than as one value: Start is non-destructive, so the
/// chat holds lines from every session that has run since it was last cleared,
/// and each of them needs its own zero point to be shown as a time of day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct SessionStart {
    pub session_id: u64,
    pub started_at_ms: u64,
}

/// Everything a window needs to render the chat as it stands.
///
/// One command rather than three, for the same reason `list_models` is one:
/// the three answers are only useful together. Transcripts without their
/// sessions' zero points cannot be timed, and without the session state the
/// window cannot say whether what it is showing is still running.
#[derive(Debug, Clone, Default, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct Snapshot {
    pub transcripts: Vec<TranscriptEvent>,
    pub sessions: Vec<SessionStart>,
    /// The last session state that was reported, or `None` when nothing has
    /// run yet in this process.
    pub state: Option<SessionState>,
}

/// The transcripts the bridge has emitted, and the session state around them.
#[derive(Debug, Default)]
pub struct History {
    transcripts: VecDeque<TranscriptEvent>,
    /// Session id to zero point. A map rather than a list so a session state
    /// that arrives twice — the answer to `start_session` and the event —
    /// records one entry.
    sessions: BTreeMap<u64, u64>,
    state: Option<SessionState>,
    /// The session currently running, if any. What [`History::clear`] has to
    /// keep: the lines still arriving from it need their zero point.
    running: Option<u64>,
}

impl History {
    /// Records a transcript on its way to the views, alert ranges and all.
    ///
    /// After the matching, not before: a window caught up from here has to
    /// highlight the same characters as the one that was already open, and
    /// running the matcher again over old text would mark the terms of today
    /// on a sentence that was spoken under yesterday's list.
    pub fn push(&mut self, event: TranscriptEvent) {
        if self.transcripts.len() >= CAPACITY {
            self.transcripts.pop_front();
        }
        self.transcripts.push_back(event);
    }

    /// Records a session state, and the zero point a running one carries.
    pub fn note_state(&mut self, state: &SessionState) {
        match state {
            SessionState::Running {
                session_id,
                started_at_ms,
                ..
            } => {
                self.sessions.insert(*session_id, *started_at_ms);
                self.running = Some(*session_id);
            }
            SessionState::Stopped | SessionState::Failed { .. } => self.running = None,
        }
        self.state = Some(state.clone());
    }

    /// Empties the chat: every transcript, from every session and every group.
    ///
    /// The running session's zero point survives, because the lines arriving
    /// from now on are still on its clock. Everything else goes — the zero
    /// points that are left belong to lines that no longer exist.
    pub fn clear(&mut self) {
        self.transcripts.clear();
        let running = self.running;
        self.sessions.retain(|id, _| Some(*id) == running);
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            transcripts: self.transcripts.iter().cloned().collect(),
            sessions: self
                .sessions
                .iter()
                .map(|(&session_id, &started_at_ms)| SessionStart {
                    session_id,
                    started_at_ms,
                })
                .collect(),
            state: self.state.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcript(session_id: u64, seq: u64) -> TranscriptEvent {
        TranscriptEvent {
            session_id,
            stream_id: 0,
            seq,
            label: "Regie".into(),
            t_start_ms: seq * 1000,
            t_end_ms: seq * 1000 + 800,
            text: format!("segment {seq}"),
            detected_language: None,
            decode_ms: 10,
            latency_ms: 20,
            alerts: Vec::new(),
        }
    }

    fn running(session_id: u64, started_at_ms: u64) -> SessionState {
        SessionState::Running {
            session_id,
            started_at_ms,
            device: "Scarlett".into(),
            streams: Vec::new(),
            sample_rate: 48_000,
            model: "small".into(),
            load_ms: 100,
            gpu: true,
        }
    }

    #[test]
    fn a_snapshot_is_what_was_emitted_in_order() {
        let mut history = History::default();
        history.note_state(&running(1, 1_700_000_000_000));
        history.push(transcript(1, 0));
        history.push(transcript(1, 1));

        let snapshot = history.snapshot();
        assert_eq!(
            snapshot
                .transcripts
                .iter()
                .map(|t| t.seq)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(
            snapshot.sessions,
            [SessionStart {
                session_id: 1,
                started_at_ms: 1_700_000_000_000
            }]
        );
        assert!(matches!(snapshot.state, Some(SessionState::Running { .. })));
    }

    #[test]
    fn the_oldest_transcripts_fall_off_the_front() {
        let mut history = History::default();
        for seq in 0..(CAPACITY as u64 + 5) {
            history.push(transcript(1, seq));
        }

        let snapshot = history.snapshot();
        assert_eq!(snapshot.transcripts.len(), CAPACITY);
        assert_eq!(snapshot.transcripts[0].seq, 5);
    }

    /// Start is non-destructive, so a second session appends to the first
    /// one's lines — and both zero points have to survive, or half the chat
    /// loses its times of day.
    #[test]
    fn every_session_that_ran_keeps_its_zero_point() {
        let mut history = History::default();
        history.note_state(&running(1, 1_000));
        history.push(transcript(1, 0));
        history.note_state(&SessionState::Stopped);
        history.note_state(&running(2, 9_000));
        history.push(transcript(2, 0));

        let snapshot = history.snapshot();
        assert_eq!(
            snapshot.sessions,
            [
                SessionStart {
                    session_id: 1,
                    started_at_ms: 1_000
                },
                SessionStart {
                    session_id: 2,
                    started_at_ms: 9_000
                }
            ]
        );
    }

    #[test]
    fn clearing_keeps_only_the_running_session_s_zero_point() {
        let mut history = History::default();
        history.note_state(&running(1, 1_000));
        history.push(transcript(1, 0));
        history.note_state(&SessionState::Stopped);
        history.note_state(&running(2, 9_000));
        history.push(transcript(2, 0));

        history.clear();

        let snapshot = history.snapshot();
        assert!(snapshot.transcripts.is_empty());
        assert_eq!(
            snapshot.sessions,
            [SessionStart {
                session_id: 2,
                started_at_ms: 9_000
            }]
        );
    }

    /// Clearing while nothing runs leaves nothing behind — there are no lines
    /// left that any of those clocks could time.
    #[test]
    fn clearing_a_stopped_session_keeps_nothing() {
        let mut history = History::default();
        history.note_state(&running(1, 1_000));
        history.push(transcript(1, 0));
        history.note_state(&SessionState::Stopped);

        history.clear();

        assert!(history.snapshot().sessions.is_empty());
    }
}

//! What one instance offers another that reads along (P29).
//!
//! The LAN remote's socket hands a browser the whole operational view and
//! takes every command the app has. That is right for an operator driving the
//! session from a phone, and wrong for another instance that only wants to
//! *read* what this one transcribes: it would be able to stop the show, and it
//! would be sent this machine's meters, settings and model downloads, none of
//! which it has any use for.
//!
//! So the feed is a second socket on the same listener, behind the same key,
//! with a much smaller vocabulary:
//!
//! - **out:** a greeting, the channels on offer, session starts, transcripts
//!   and speech activity — named by channel *id*, never by stream position,
//!   because a position means nothing outside the profile it is a position in;
//! - **in:** two questions — "what has been said so far" and "let me hear this
//!   line" — and nothing that changes anything here.
//!
//! **Only what this machine transcribes is offered.** A channel that is itself
//! read from another instance is never passed on. That one rule is what lets
//! two instances read each other without a line going round in circles.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::activity::SpeechActivity;
use crate::config::Config;
use crate::history::{SessionStart, Snapshot};
use crate::replay::ClipKey;
use crate::session::{StreamConfig, TranscriptEvent};

/// One channel on offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "sources.ts")]
pub struct FeedChannel {
    pub id: String,
    pub label: String,
    pub color: String,
}

/// A transcript, as the feed carries it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedTranscript {
    pub session_id: u64,
    pub channel: String,
    pub seq: u64,
    pub t_start_ms: u64,
    pub t_end_ms: u64,
    pub text: String,
    pub detected_language: Option<String>,
    pub decode_ms: u32,
    pub latency_ms: u32,
    /// Whether the audio is held and may be asked for.
    pub audio: bool,
}

/// Speech activity, as the feed carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedActivity {
    pub session_id: u64,
    pub channel: String,
    pub speaking: bool,
    pub decoding: bool,
    pub since_ms: u64,
    pub preview: String,
}

/// One segment's audio, asked for by channel id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedClip {
    pub session_id: u64,
    pub channel: String,
    pub seq: u64,
}

/// What has been said so far, for a subscriber that has just connected.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedSnapshot {
    pub sessions: Vec<SessionStart>,
    pub transcripts: Vec<FeedTranscript>,
    pub activity: Vec<FeedActivity>,
}

/// Everything the server sends.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Frame {
    /// The first frame on every connection.
    Hello {
        instance: String,
        name: String,
        version: String,
    },
    /// The channels on offer, sent after the greeting and again whenever the
    /// configuration changes.
    Channels {
        channels: Vec<FeedChannel>,
    },
    Session(SessionStart),
    Transcript(FeedTranscript),
    Activity(FeedActivity),
    Result {
        id: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// Everything a subscriber may ask.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "call",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Call {
    Snapshot,
    Replay { segments: Vec<FeedClip> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub call: Call,
}

/// How many sessions the stream-to-channel table remembers. The chat a
/// subscriber is caught up from spans sessions, so the table has to as well;
/// it does not have to span a whole tour.
const SESSIONS_KEPT: usize = 64;

/// Which channel each stream of each session was — the translation from what
/// the pipeline says to what the feed says.
///
/// Written when a session starts, from the configuration it was started with.
/// Not read off the configuration file when a line goes out: the file may
/// have been edited since, and a line belongs to the channel it was spoken on,
/// not to whatever is at that position now.
#[derive(Default)]
pub struct Streams {
    sessions: Mutex<BTreeMap<u64, Vec<Option<String>>>>,
}

impl Streams {
    /// Notes the channels of a session that has just started.
    pub fn started(&self, session_id: u64, streams: &[StreamConfig]) {
        let ids = streams
            .iter()
            .map(|s| {
                if s.remote.unwrap_or(false) {
                    None
                } else {
                    s.id.clone().filter(|id| !id.is_empty())
                }
            })
            .collect();
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(session_id, ids);
            while sessions.len() > SESSIONS_KEPT {
                let oldest = *sessions.keys().next().expect("not empty");
                sessions.remove(&oldest);
            }
        }
    }

    /// The channel id of a stream, if it is offered.
    pub fn channel(&self, session_id: u64, stream_id: u16) -> Option<String> {
        self.sessions
            .lock()
            .ok()?
            .get(&session_id)?
            .get(stream_id as usize)?
            .clone()
    }

    /// The stream of a channel, the other way round — for a clip asked for.
    pub fn stream(&self, session_id: u64, channel: &str) -> Option<u16> {
        let sessions = self.sessions.lock().ok()?;
        let position = sessions
            .get(&session_id)?
            .iter()
            .position(|id| id.as_deref() == Some(channel))?;
        u16::try_from(position).ok()
    }

    pub fn offered(&self, session_id: u64) -> bool {
        self.sessions
            .lock()
            .map(|s| s.contains_key(&session_id))
            .unwrap_or(false)
    }

    pub fn transcript(&self, event: &TranscriptEvent) -> Option<FeedTranscript> {
        let channel = self.channel(event.session_id, event.stream_id)?;
        Some(FeedTranscript {
            session_id: event.session_id,
            channel,
            seq: event.seq,
            t_start_ms: event.t_start_ms,
            t_end_ms: event.t_end_ms,
            text: event.text.clone(),
            detected_language: event.detected_language.clone(),
            decode_ms: event.decode_ms,
            latency_ms: event.latency_ms,
            audio: event.audio,
        })
    }

    pub fn activity(&self, event: &SpeechActivity) -> Option<FeedActivity> {
        let channel = self.channel(event.session_id, event.stream_id)?;
        Some(FeedActivity {
            session_id: event.session_id,
            channel,
            speaking: event.speaking,
            decoding: event.decoding,
            since_ms: event.since_ms,
            preview: event.preview.clone(),
        })
    }

    /// The chat as it stands, in the feed's terms: only what was transcribed
    /// here, and only the sessions that any of it belongs to.
    pub fn snapshot(&self, snapshot: &Snapshot) -> FeedSnapshot {
        let transcripts: Vec<FeedTranscript> = snapshot
            .transcripts
            .iter()
            .filter_map(|t| self.transcript(t))
            .collect();
        FeedSnapshot {
            sessions: snapshot
                .sessions
                .iter()
                .filter(|s| self.offered(s.session_id))
                .copied()
                .collect(),
            transcripts,
            activity: snapshot
                .activity
                .iter()
                .filter_map(|a| self.activity(a))
                .collect(),
        }
    }

    /// The clips asked for, as this machine's replay store knows them.
    pub fn clips(&self, segments: &[FeedClip]) -> Option<Vec<ClipKey>> {
        segments
            .iter()
            .map(|clip| {
                Some(ClipKey {
                    session_id: clip.session_id,
                    stream_id: self.stream(clip.session_id, &clip.channel)?,
                    seq: clip.seq,
                })
            })
            .collect()
    }
}

/// The channels this configuration offers: the active profile's inputs, and
/// none of the channels it reads from somewhere else.
pub fn channels(config: &Config) -> Vec<FeedChannel> {
    config
        .profiles
        .get(config.active)
        .map(|profile| {
            profile
                .channels
                .iter()
                .filter(|c| c.source.is_none() && !c.id.is_empty())
                .map(|c| FeedChannel {
                    id: c.id.clone(),
                    label: c.label.clone(),
                    color: c.color.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Serialises a frame, logging rather than failing: a frame that cannot be
/// written is a bug in a struct here, and the connection is worth more than
/// the one frame.
pub fn text(frame: &Frame) -> Option<String> {
    serde_json::to_string(frame)
        .map_err(|err| tracing::error!(error = %err, "a feed frame could not be serialised"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(id: &str, remote: bool) -> StreamConfig {
        serde_json::from_value(serde_json::json!({
            "channel": 0,
            "label": "x",
            "language": "de",
            "id": id,
            "remote": remote,
        }))
        .expect("a stream")
    }

    #[test]
    fn a_channel_read_from_elsewhere_is_never_passed_on() {
        let streams = Streams::default();
        streams.started(
            7,
            &[stream("a", false), stream("b", true), stream("c", false)],
        );
        assert_eq!(streams.channel(7, 0).as_deref(), Some("a"));
        assert_eq!(streams.channel(7, 1), None);
        assert_eq!(streams.channel(7, 2).as_deref(), Some("c"));
        assert_eq!(streams.stream(7, "c"), Some(2));
        assert_eq!(streams.stream(7, "b"), None);
    }

    #[test]
    fn frames_read_as_they_are_written() {
        let frame = Frame::Transcript(FeedTranscript {
            session_id: 3,
            channel: "abc".into(),
            seq: 4,
            t_start_ms: 10,
            t_end_ms: 20,
            text: "Achtung".into(),
            detected_language: None,
            decode_ms: 1,
            latency_ms: 2,
            audio: true,
        });
        let text = text(&frame).expect("serialised");
        assert!(text.starts_with(r#"{"type":"transcript","#), "{text}");
        let back: Frame = serde_json::from_str(&text).expect("parsed");
        assert!(matches!(back, Frame::Transcript(t) if t.seq == 4));

        let call: Request = serde_json::from_str(
            r#"{"id":1,"call":"replay","segments":[{"sessionId":3,"channel":"abc","seq":4}]}"#,
        )
        .expect("a call");
        assert!(matches!(call.call, Call::Replay { ref segments } if segments.len() == 1));
    }
}

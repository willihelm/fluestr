//! Every channel of one device, end to end: microphone → rings → 16 kHz →
//! segments → pool → chat.
//!
//! The pool behind the segmenters was always indifferent to how many streams
//! fed it; what was single-stream was everything in front of it. Now the
//! session builds one pipeline per configured channel — ring, resampler,
//! speech detector, segmenter — over **one** capture of **one** multichannel
//! device, and they all submit into the same prioritised queue.
//!
//! How many channels that may be is the device's question, not this module's:
//! nothing here caps the count, and a channel the interface does not have is
//! carried along silently rather than refused. What decides how many of them keep up is how
//! many decodes run at once, which is [`crate::pool::PoolConfig::workers`] and
//! an operator's to set.
//!
//! # Threads
//!
//! - the **capture thread** inside `audio-core`, because `cpal::Stream` is
//!   `!Send` on macOS. One thread for the device, whatever the channel count:
//!   the de-interleaver splits the interleaved buffer into the streams' rings
//!   inside the callback;
//! - **one processing thread for all streams**, which drains each ring in
//!   turn, resamples and segments;
//! - the pool's **decode threads**, one per sidecar, plus its watchdog;
//! - the **aggregator thread**, which puts results back in order per stream
//!   and turns them into events.
//!
//! One processing thread rather than one per stream is a deliberate choice.
//! The work is small and bounded — resampling runs at ~2200× realtime and
//! Silero costs ~1.3 % of a core per stream, so eight streams are ~12 % of one
//! core — and a single thread means the level events for all streams are
//! sampled over the same interval instead of eight slightly different ones.
//! What must *not* share a thread is decoding, and it does not: a decode takes
//! hundreds of milliseconds, and a processing thread waiting for one would let
//! the rings overflow while the app was busy transcribing the audio before it.
//! That is why [`WorkerPool::submit`] never blocks.

use crate::pool::{Completed, Outcome, PoolConfig, PoolEvent, Reorder, WorkerPool};
use anyhow::{bail, Context, Result};
use audio_core::{
    gain::Gain, ring, vad, AudioBackend, CaptureConfig, CaptureEvent, CaptureSession,
    ChannelAssignment, ChannelMap, CpalBackend, EnergyVad, LanguageCode, LanguageMode, Segmenter,
    SegmenterConfig, SileroVad, StreamId, StreamProcessor, Vad, SAMPLE_RATE,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long one stream's ring holds audio before the processing thread has to
/// have drained it. Four seconds is the buffer that absorbs a decode that runs
/// long; at 48 kHz it costs ~770 KB per stream, which is not worth economising
/// on.
const RING_SECONDS: f32 = 4.0;

/// How often the processing thread wakes up and pumps every stream. Short
/// enough that no ring fills, long enough not to spin.
const PUMP_INTERVAL: Duration = Duration::from_millis(20);

/// Level events per second — one event carrying **all** streams, not one per
/// stream.
///
/// Anything faster is invisible to the eye, and one event per stream at this
/// rate is 20 crossings of the IPC bridge per second per channel, each one
/// waking the WebView — on a patch of any size, the cost the channel count
/// would drive. Bundling them makes it independent of the channel count
/// instead, which is what lets the count go as high as the device does.
const LEVEL_HZ: u32 = 20;

/// Above this, whisper's own judgement that the audio held no speech is
/// trusted and the segment is dropped.
///
/// An intercom channel is silent or carries only room noise for long stretches,
/// and an energy VAD cannot tell a bell or a burst of music from a spoken word
/// — whisper can, and this is where that judgement is applied. 0.6 is
/// whisper.cpp's own default for the same threshold.
///
/// Note where the cost falls: the segment is decoded first and discarded
/// afterwards, so this buys correctness, not time. Since P3 that cost is
/// rarely paid — [`speech_detector`] runs Silero in front of the segmenter, so
/// a bell no longer opens a segment in the first place. What remains here is
/// the last of three layers, and it stays because the first one falls back to
/// the energy detector when the VAD model is missing.
const NO_SPEECH_THRESHOLD: f32 = 0.6;

/// One intercom channel, as the UI configures it.
///
/// The stream id is *not* in here: it is the position in
/// [`SessionConfig::streams`], assigned by the backend when the session
/// starts, so the UI cannot hand out two streams with the same id. The channel
/// index is zero-based, like the audio API; the UI converts at its own
/// boundary and nowhere else.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct StreamConfig {
    /// Channel index on the multichannel device.
    pub channel: u16,
    /// Free-form label: "Regie", "FOH", "Licht".
    pub label: String,
    /// Primary language, fixed. Auto mode is P3b.
    pub language: String,
    /// Input trim in whole decibels, applied before anything reads the audio
    /// — the speech detector, the segmenter and the meters all see the
    /// scaled signal. See [`audio_core::gain`] for the range and why it is
    /// decibels.
    ///
    /// `default` — zero, unity — so a configuration written before the trim
    /// existed starts at the level the device delivers, which is exactly what
    /// it did before.
    #[serde(default)]
    pub gain_db: i32,
    /// Whether the operator has muted this channel.
    ///
    /// Mute means *not transcribed*, not *not heard*: the channel keeps its
    /// ring, its trim and its meter, and only the segmenter stops being fed.
    /// That is the thing an operator presses M for — a beltpack left open in a
    /// noisy dressing room filling the chat with nothing — and it is also what
    /// makes the button honest afterwards: the bar beside it still moves, so
    /// the channel is visibly alive and visibly not arriving.
    ///
    /// `default` — unmuted — for a configuration written before mute existed,
    /// which is exactly what every channel in it did.
    #[serde(default)]
    pub muted: bool,
}

/// A change to one channel of a session that is already running (P6).
///
/// Four of the five fields of a [`StreamConfig`] can change without the
/// capture being touched, and they are the four that are wrong during a show:
/// the label — somebody else took the position over — the language — the guest
/// director speaks English — the trim, which is wrong the first time anybody
/// speaks on a channel nobody spoke on during the sound check — and the mute,
/// which is wrong the moment a beltpack is left open in a noisy room. All four
/// were a Stop and a Start until this existed, and Stop is the one thing an
/// operator will not do while the room is talking.
///
/// What is *not* here is the device channel. The channel map is fixed when the
/// device opens: it pairs a ring with a channel inside the capture callback,
/// and re-patching it live would mean tearing down the capture — which is Stop
/// and Start with extra steps. A repatch stays a restart, honestly.
///
/// Every field is optional, so a caller can change one without having to
/// restate the others. Nothing given is not an error, it is a change of
/// nothing.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct StreamUpdate {
    /// Which stream — the id the backend assigned at start and every event
    /// carries, not the position in whatever list the caller is looking at.
    pub stream_id: u16,
    // `optional = nullable`: leaving the field out and sending `null` both
    // mean "do not change this one", and the bindings should say so rather
    // than making every caller spell out the half it is not touching.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub label: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub language: Option<String>,
    /// Input trim in whole decibels. Clamped rather than rejected — see
    /// [`audio_core::gain::clamp_db`].
    #[serde(default)]
    #[ts(optional = nullable)]
    pub gain_db: Option<i32>,
    /// Whether the channel is muted — the M beside the meter, pressed during
    /// a show, which is the only time it is ever pressed.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub muted: Option<bool>,
}

/// What the UI sends to start a session.
///
/// One device for all streams, which is the constraint the whole audio layer
/// is built on: one multichannel interface, de-interleaved by the app. Two
/// devices would mean two clocks drifting apart with no common timebase to
/// order the chat by.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct SessionConfig {
    /// The stable id of the device, as `list_devices` returns it — not its
    /// name. The name is what a human reads and what a driver update may
    /// change; it never reaches the capture layer from here.
    ///
    /// A name still resolves, because a configuration written before stable
    /// ids existed holds one and the backend falls back to it (see
    /// `audio_core::DeviceId`). Sending one deliberately is not a supported
    /// path — it cannot tell two identical interfaces apart.
    pub device_id: String,
    pub streams: Vec<StreamConfig>,
    /// The active vocabulary, as whisper's `initial_prompt`, for **every**
    /// stream.
    ///
    /// Session-level rather than per stream on purpose. The terms of a
    /// production — its cue names, its department names, its house jargon —
    /// are the same on every channel, and a per-channel prompt meant
    /// maintaining eight copies of one list that then drifted apart. It is
    /// also the strongest single lever on code-switching quality
    /// (measurements.md §4), so the setup where it is half-applied is the
    /// setup where half the channels quietly transcribe worse.
    ///
    /// `default` so a config written before this field existed still parses
    /// as "no vocabulary" rather than failing to start a session.
    #[serde(default)]
    pub vocabulary: Option<String>,
}

/// Identifies one run of the pipeline.
///
/// Sequence numbers are per stream and start again at zero every time a
/// session starts, because each stream gets a fresh segmenter. That makes
/// `(stream, seq)` ambiguous the moment a second session runs — and Start is
/// deliberately non-destructive, so the chat holds lines from several sessions
/// at once. `(session, stream, seq)` is what actually identifies a segment,
/// here and in the P10 schema.
///
/// Counted per process, which is as long as the in-memory chat lives.
/// Persistence will need an id that survives a restart; that belongs with P10,
/// not here.
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// A finished transcript on its way to the chat view.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct TranscriptEvent {
    /// See [`NEXT_SESSION_ID`]: `seq` alone does not identify a segment.
    pub session_id: u64,
    pub stream_id: u16,
    pub seq: u64,
    pub label: String,
    pub t_start_ms: u64,
    pub t_end_ms: u64,
    pub text: String,
    pub detected_language: Option<String>,
    /// Time whisper.cpp spent decoding.
    pub decode_ms: u32,
    /// From the end of the spoken segment to the text being ready — the number
    /// the whole "latency first" decision rests on. It includes the silence
    /// the segmenter waits out before it closes a segment, which is why that
    /// timeout is a latency setting and not just a quality one.
    pub latency_ms: u32,
    /// Where to mark the text, in UTF-16 code units (P17, P19).
    ///
    /// Left empty here: the pipeline does not know what the operator's rules
    /// say, and they can change between two segments of one sentence. It is
    /// filled in where the event crosses the bridge — once, for every view at
    /// the same time — by whichever rules asked to highlight. See
    /// [`crate::rules`].
    #[serde(default)]
    pub alerts: Vec<crate::alerts::AlertMatch>,
    /// What the rules that matched this segment ask for (P19).
    ///
    /// Filled in beside `alerts` and for the same reason: the question is
    /// answered once rather than by each window. What each window then does
    /// with it is its own — a group window does not flood for a channel it is
    /// not showing, nor for a flash addressed to another view — and *when* it
    /// does it is counted per bubble there, where the bubbles are. See
    /// `ui/src/lib/attention.ts`.
    #[serde(default)]
    pub actions: Vec<crate::rules::Action>,
    /// Which windows the flash is for (P19): the positions of the groups the
    /// firing rules named, or `None` for every window showing the channel.
    ///
    /// Beside `actions` rather than inside it because it qualifies exactly
    /// one of them. A channel says *where it was said* and is already
    /// answered by `stream_id`; a group says *which view it is for*, and only
    /// the view knows which one it is. Read by `ui/src/lib/attention.ts`, and
    /// only when `actions` holds `flash`.
    #[serde(default)]
    pub flash_views: Option<Vec<u16>>,
}

/// Level and health of one stream.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct StreamLevel {
    pub stream_id: u16,
    pub rms: f32,
    pub peak: f32,
    /// Cumulative overrun count from that stream's ring (P2). Non-zero means
    /// audio was lost, which the operator has to be able to see.
    pub dropped: u64,
}

/// Every stream's level, in one event at [`LEVEL_HZ`].
///
/// All streams together rather than one event each: the meters are read side
/// by side, so they should also be *sampled* side by side — over the same
/// interval, arriving in the same frame. Eight independent events cost eight
/// bridge crossings to show eight bars that are each a different age.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct LevelsEvent {
    pub streams: Vec<StreamLevel>,
}

/// A segment that was thrown away before it could be decoded.
///
/// The pool drops the stalest segment when its queue is full, and an operator
/// who spoke and saw no text has to be able to tell that from a channel that
/// was not listening. Silent loss is explicitly forbidden, so this event is
/// the other half of that policy.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct OverloadEvent {
    /// See [`NEXT_SESSION_ID`]: `seq` alone does not identify a segment.
    pub session_id: u64,
    pub stream_id: u16,
    /// Sequence number of the segment that was dropped.
    pub seq: u64,
    /// Cumulative count for this stream, so the UI can show a total rather
    /// than only the latest incident.
    pub dropped: u64,
}

/// One running stream, as the backend actually mapped it.
///
/// Echoed back rather than left to the UI's own copy of the configuration:
/// this is the authoritative answer to "which channel ended up under which
/// label", and it is the only place the stream ids the events carry are
/// defined.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct RunningStream {
    pub stream_id: u16,
    pub channel: u16,
    pub label: String,
    /// The device does not have this channel, so the stream is carried along
    /// silently: it keeps its id, its colour and its place in the groups, and
    /// receives no audio. See [`Session::start`] for why that beats refusing
    /// to start.
    ///
    /// Not called "muted", though it sounds like it: mute is something an
    /// operator does to a channel that is working, and this is a channel that
    /// is not there. The two need separate words because an operator reading
    /// a silent channel has to be able to tell "I pressed M" from "this
    /// interface has four inputs".
    pub absent: bool,
    /// Muted by the operator: the audio arrives and is metered, and nothing
    /// is transcribed from it. See [`StreamConfig::muted`].
    pub muted: bool,
    /// The trim actually in force, which is the configured one clamped into
    /// range. Echoed back for the same reason the channel is: this is the
    /// authority on what the session is doing, and a knob that shows a value
    /// the pipeline is not using is a knob that lies.
    pub gain_db: i32,
}

/// Wall clock now, in milliseconds since the Unix epoch.
///
/// A clock set before 1970 is the only way this can fail, and the answer there
/// is a session that starts anyway with its bubbles timed from the epoch —
/// wrong times read as wrong, a session that refuses to start does not.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Lifecycle of the session, as the UI needs to render it.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
// `rename_all` covers the variant names, `rename_all_fields` their contents —
// both are needed for the UI to read `state: "running"` and `sampleRate`.
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "session.ts")]
pub enum SessionState {
    Running {
        /// What the transcripts arriving from now on are tagged with.
        session_id: u64,
        /// Wall clock at which this session's own clock is zero, in
        /// milliseconds since the Unix epoch.
        ///
        /// A segment carries `t_start_ms` on that session clock, and a time of
        /// day is what an operator reads a transcript against — a log, a cue
        /// sheet, another recording. The conversion needs this one number, and
        /// it belongs here rather than being guessed in the UI from when the
        /// first event happened to arrive.
        started_at_ms: u64,
        device: String,
        streams: Vec<RunningStream>,
        sample_rate: u32,
        model: String,
        /// Includes the one-off Metal shader compilation on a machine's first
        /// ever start (~8 s); afterwards ~0.1 s.
        load_ms: u32,
        gpu: bool,
    },
    Stopped,
    Failed {
        message: String,
    },
}

/// Anything the session reports back. A trait rather than `tauri::AppHandle`
/// so the pipeline stays independent of the UI layer.
pub trait EventSink: Send + Sync + 'static {
    fn transcript(&self, event: TranscriptEvent);
    fn levels(&self, event: LevelsEvent);
    fn overload(&self, event: OverloadEvent);
    fn state(&self, event: SessionState);
}

/// Everything one channel needs between the ring and the queue.
///
/// One per configured stream, all owned by the processing thread. The level
/// window lives here too, because it has to be accumulated per stream and
/// emptied when the bundled event goes out.
struct Pipeline {
    processor: StreamProcessor,
    segmenter: Segmenter,
    /// Shared with [`Session::gains`], so the knob reaches the audio without
    /// this thread having to be told. Read once per pump.
    gain: Arc<Gain>,
    /// Shared with [`Session::mutes`], the same way and for the same reason.
    mute: Arc<AtomicBool>,
    /// What `mute` said at the last pump, so the moment it changes can be
    /// acted on. A mute is not only a thing this thread stops doing: the
    /// segment that was open when it arrived has to be closed and decoded,
    /// or half a sentence sits in the segmenter until the channel is unmuted
    /// and then arrives in the chat minutes late.
    was_muted: bool,
    /// Audio since the last level event, *after* the trim — the meter beside
    /// the knob has to show what turning it did.
    window: Vec<f32>,
}

/// A running session. Dropping it stops everything.
pub struct Session {
    stop: Arc<AtomicBool>,
    capture: Option<Box<dyn CaptureSession>>,
    processing: Option<JoinHandle<()>>,
    aggregating: Option<JoinHandle<()>>,
    /// Shared with the processing thread, which submits into it.
    pool: Arc<WorkerPool>,
    pub sample_rate: u32,
    /// The label each stream's transcripts go out under.
    ///
    /// Shared with the aggregator rather than handed to it, because a rename
    /// has to reach the thread that is already running (see
    /// [`Session::update_stream`]). Read once per transcript, which is a lock
    /// next to a decode that took three hundred milliseconds.
    labels: Labels,
    /// Language changes on their way to the processing thread, which owns the
    /// segmenters.
    ///
    /// A channel rather than shared state: the segmenter is not `Sync` and has
    /// no business being locked on the pump path, and a change is a handful of
    /// bytes a few times an evening.
    languages: mpsc::Sender<(u16, String)>,
    /// The trim of every live stream, shared with the pipelines that apply it.
    ///
    /// Shared state rather than a channel, unlike the language: a knob being
    /// dragged sends a change per pointer event, and the processing thread
    /// only needs the value that is current when it next pumps. A stream the
    /// device has no channel for is not in here — it has no pipeline for a
    /// trim to reach.
    gains: HashMap<u16, Arc<Gain>>,
    /// Whether each live stream is muted, shared with the pipeline that reads
    /// it once per pump.
    ///
    /// Shared state rather than a channel for the trim's reason, plus one of
    /// its own: the pipeline has to notice the *transition*, so that the
    /// sentence somebody was in the middle of when M was pressed is still
    /// decoded rather than left in the segmenter until the channel comes
    /// back. That is a comparison the reader makes, not a message it is sent.
    mutes: HashMap<u16, Arc<AtomicBool>>,
    /// The running state exactly as it was announced.
    ///
    /// Kept so an update can re-announce it — same session id, same zero point,
    /// one label different. Rebuilding it would mean inventing a new
    /// `started_at_ms`, which every bubble's time of day is measured against.
    running: std::sync::Mutex<SessionState>,
    /// Where that re-announcement goes. The same sink the threads report
    /// through, so an update reaches every view the way a start does.
    events: Arc<dyn EventSink>,
}

/// The label each stream's transcripts are emitted under.
///
/// A map rather than a vector because it is keyed by stream id, and the ids of
/// a session with absent channels are not a contiguous range of the
/// pipelines.
type Labels = Arc<std::sync::Mutex<HashMap<u16, String>>>;

impl Session {
    /// Returns the running state alongside the session rather than only
    /// emitting it.
    ///
    /// The initial transition is what the operator is waiting on after
    /// pressing Start, so it travels back as the command's own reply. Events
    /// then carry everything that happens *later* — a device unplugged, the
    /// sidecar lost, the session stopped. Making the first state depend on the
    /// event channel too would mean a broken bridge looks like a backend that
    /// hangs, which is exactly the failure this arrangement avoids.
    /// `models` is what the model manager (P11) resolved. `None` falls back to
    /// the development search in [`crate::worker`] — the tests and `just
    /// slice` have no app directory and no configuration file to resolve
    /// against, and giving them a stub configuration would only mean the path
    /// they exercise is not the one the app runs.
    ///
    /// `workers` is how many sidecars decode in parallel, as the Settings page
    /// set it. `None` is [`crate::pool::DEFAULT_WORKERS`] — the same fallback
    /// and for the same reason as `models`.
    pub fn start(
        config: SessionConfig,
        events: Arc<dyn EventSink>,
        models: Option<crate::models::Resolved>,
        workers: Option<usize>,
    ) -> Result<(Self, SessionState)> {
        if config.streams.is_empty() {
            bail!("no channels configured");
        }
        let backend = CpalBackend::new();
        let requested = audio_core::DeviceId(config.device_id.clone());
        let info = backend.device(&requested)?;
        // What the device answers to from here on. It differs from
        // `requested` exactly when an old configuration was resolved by name,
        // and using the resolved one means the rest of the session — the
        // capture handle, the disconnect event — names the device the app
        // actually opened.
        let device_id = info.id.clone();

        // A configured channel the device does not have is carried along
        // silently, not a refusal to start. A channel map is made for the desk and then
        // carried to a laptop with a two-input interface, and the operator
        // wants the channels that *are* there rather than having to delete the
        // rest and rebuild the setup afterwards.
        //
        // Absent streams keep their stream id. The id is the position in
        // `config.streams`, and the chat colours, the group membership and the
        // meters are all built on it, so dropping a row would move every later
        // channel onto the wrong label — the exact failure P1 is careful about
        // everywhere else.
        let live: Vec<(u16, &StreamConfig)> = config
            .streams
            .iter()
            .enumerate()
            .map(|(index, stream)| (index as u16, stream))
            .filter(|(_, stream)| stream.channel < info.channels)
            .collect();

        if live.is_empty() {
            // Nothing left to listen to: starting would open a device and
            // transcribe silence, which looks like the app being broken.
            bail!(
                "none of the {} configured channels exists on '{}' — it has {} channels",
                config.streams.len(),
                info.name,
                info.channels
            );
        }

        if live.len() < config.streams.len() {
            // Numbered the way the UI shows it — the index is zero-based
            // because the audio API is, but nothing an operator reads has any
            // business exposing that.
            let absent: Vec<String> = config
                .streams
                .iter()
                .filter(|stream| stream.channel >= info.channels)
                .map(|stream| format!("'{}' (ch {})", stream.label, stream.channel as u32 + 1))
                .collect();
            tracing::warn!(
                device = %info.name,
                channels = info.channels,
                absent = %absent.join(", "),
                "carried along silently: the device does not have these channels"
            );
        }

        // The sidecar comes up first. Loading the model can take seconds, and
        // capturing audio that nothing is ready to transcribe would only fill
        // the rings with material that is stale by the time anyone looks at it.
        let binary = crate::worker::find_binary()?;
        let (model, vad_model) = match models {
            Some(resolved) => {
                tracing::info!(
                    model = %resolved.speech.display(),
                    source = ?resolved.source,
                    "model"
                );
                (resolved.speech, resolved.vad)
            }
            // No model manager in the picture: the development search, which
            // is what the integration tests and `just slice` run on.
            None => (
                crate::worker::find_model()?,
                crate::worker::find_vad_model(),
            ),
        };
        if vad_model.is_none() {
            tracing::warn!(
                "no Silero VAD model found — noise and silence will be transcribed; \
                 run `just vad-model`"
            );
        }
        let mut pool_config = PoolConfig::new(binary, model, vad_model.clone());
        if let Some(asked) = workers {
            // Clamped rather than refused: this number reaches here from a
            // file, and a session that will not start is a worse answer to a
            // mistyped one than a session that starts with a count this
            // version will serve.
            pool_config.workers = crate::pool::clamp_workers(asked);
            if pool_config.workers != asked {
                tracing::warn!(
                    asked,
                    using = pool_config.workers,
                    max = crate::pool::MAX_WORKERS,
                    "the configured worker count is out of range"
                );
            }
        }
        let (pool, results) = WorkerPool::spawn(pool_config).context("starting the sidecar")?;
        let pool = Arc::new(pool);
        let ready = pool.ready().clone();

        let rate = info.default_sample_rate;

        // Built in one pass so the sinks stay index-aligned with the channel
        // map — the de-interleaver pairs them by position, and a mismatch
        // there puts the lighting desk's words under the sound engineer's
        // label without anything crashing.
        let mut assignments = Vec::with_capacity(live.len());
        let mut sinks: Vec<Box<dyn audio_core::SampleSink>> = Vec::with_capacity(live.len());
        let mut sources = Vec::with_capacity(live.len());

        for &(stream_id, stream) in &live {
            let (sink, source) = ring::channel(rate, RING_SECONDS);
            assignments.push(ChannelAssignment {
                stream_id: StreamId(stream_id),
                channel: stream.channel,
            });
            sinks.push(Box::new(sink));
            sources.push(source);
        }

        // Every configured stream, the absent ones included: this is the UI's
        // authoritative answer to which channel ended up where, and a channel
        // missing from it would read as one that simply vanished.
        let running: Vec<RunningStream> = config
            .streams
            .iter()
            .enumerate()
            .map(|(index, stream)| RunningStream {
                stream_id: index as u16,
                channel: stream.channel,
                label: stream.label.clone(),
                absent: stream.channel >= info.channels,
                muted: stream.muted,
                gain_db: audio_core::gain::clamp_db(stream.gain_db),
            })
            .collect();

        let channel_map = ChannelMap::new(assignments, info.channels)?;

        let (capture_events, capture_event_rx) = mpsc::channel::<CaptureEvent>();
        let capture = backend.open(
            CaptureConfig {
                device: device_id,
                channel_map,
                sample_rate: Some(rate),
            },
            sinks,
            capture_events,
        )?;

        let granted = capture.sample_rate();
        if granted != rate {
            bail!("device granted {granted} Hz instead of the requested {rate} Hz");
        }

        // Only once the device has actually opened: a detector per stream
        // means a whisper.cpp VAD context per stream, and there is no point
        // paying for eight of them for a device that was going to refuse.
        let mut pipelines = Vec::with_capacity(live.len());
        let mut gains: HashMap<u16, Arc<Gain>> = HashMap::with_capacity(live.len());
        let mut mutes: HashMap<u16, Arc<AtomicBool>> = HashMap::with_capacity(live.len());
        for (&(stream_id, stream), source) in live.iter().zip(sources) {
            // One per live stream, held on both sides: the pipeline scales
            // with it, `update_stream` writes to it.
            let gain = Arc::new(Gain::new(stream.gain_db));
            gains.insert(stream_id, gain.clone());
            // Held on both sides like the trim: the pipeline reads it, the M
            // beside the meter writes it through `update_stream`.
            let muted = stream.muted;
            let mute = Arc::new(AtomicBool::new(muted));
            mutes.insert(stream_id, mute.clone());
            pipelines.push(Pipeline {
                processor: StreamProcessor::new(source, granted)?,
                segmenter: Segmenter::new(
                    StreamId(stream_id),
                    // The same model file serves both layers: the detector in
                    // front of this segmenter and the sidecar's backstop
                    // behind it. Each stream needs its own instance — the
                    // detector carries per-stream state.
                    speech_detector(vad_model.as_deref()),
                    SegmenterConfig {
                        language_mode: LanguageMode::Fixed {
                            language: LanguageCode::new(&stream.language),
                        },
                        // The same vocabulary on every stream — see
                        // [`SessionConfig::vocabulary`].
                        initial_prompt: config.vocabulary.clone(),
                        ..Default::default()
                    },
                ),
                gain,
                mute,
                was_muted: muted,
                window: Vec::new(),
            });
        }

        let labels: Labels = Arc::new(std::sync::Mutex::new(
            running
                .iter()
                .map(|s| (s.stream_id, s.label.clone()))
                .collect(),
        ));

        let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let stop = Arc::new(AtomicBool::new(false));
        let (languages, language_rx) = mpsc::channel();
        let aggregating = spawn_aggregator(session_id, results, labels.clone(), events.clone());
        let processing = spawn_processor(
            pipelines,
            pool.clone(),
            capture_event_rx,
            language_rx,
            stop.clone(),
            events.clone(),
            info.name.clone(),
        );

        tracing::info!(
            session_id,
            device = %info.name,
            streams = live.len(),
            absent = config.streams.len() - live.len(),
            sample_rate = granted,
            detector = if vad_model.is_some() { "silero" } else { "energy" },
            workers = pool.workers(),
            "session running"
        );

        let state = SessionState::Running {
            session_id,
            started_at_ms: now_ms(),
            device: info.name,
            streams: running,
            sample_rate: granted,
            model: ready.model,
            load_ms: ready.load_ms,
            gpu: ready.gpu,
        };

        Ok((
            Self {
                stop,
                capture: Some(capture),
                processing: Some(processing),
                aggregating: Some(aggregating),
                pool,
                sample_rate: granted,
                labels,
                languages,
                gains,
                mutes,
                running: std::sync::Mutex::new(state.clone()),
                events,
            },
            state,
        ))
    }

    /// Renames a running stream, puts it onto another language, or trims its
    /// input (P6).
    ///
    /// The label and the language take effect on the next segment: the label
    /// where the transcript is built, the language where the segmenter hands
    /// one to the queue. Neither rewrites what is already in the chat — a line
    /// was said under the label it was said under, and a chat that renamed its
    /// own history would be lying about a show that has already happened.
    ///
    /// The trim takes effect on the next pump, which is the next few
    /// milliseconds of audio: it is set while listening to the meter it
    /// changes, so anything slower would be a knob that cannot be found by
    /// ear.
    ///
    /// Answers with the running state as it now stands, and announces it, so
    /// every view — the Settings page that asked, a detached group window, a
    /// browser on the LAN — learns the new mapping from the one authority on
    /// it rather than from its own copy of the setup.
    ///
    /// A stream the device has no channel for can be updated like any other.
    /// It has no pipeline, so neither the language nor the trim nor the mute
    /// reaches anything; all three are still recorded in the state it answers
    /// with, because such a channel is one cable away from being live and its
    /// setup belongs to it either way.
    ///
    /// Changes no configuration file. The session is what is being edited
    /// here; storing the change is the Settings page's business, on its own
    /// debounce, and a second writer of that file is exactly what P12 avoids.
    pub fn update_stream(&self, update: StreamUpdate) -> Result<SessionState> {
        let mut running = self
            .running
            .lock()
            .map_err(|_| anyhow::anyhow!("the session state is poisoned"))?;

        let SessionState::Running { streams, .. } = &mut *running else {
            // Only reachable if a caller kept a handle to a session that has
            // already reported itself finished.
            bail!("the session is not running");
        };

        let stream = streams
            .iter_mut()
            .find(|stream| stream.stream_id == update.stream_id)
            .with_context(|| format!("no stream {} in this session", update.stream_id))?;

        if let Some(label) = update.label {
            stream.label.clone_from(&label);
            self.labels
                .lock()
                .map_err(|_| anyhow::anyhow!("the stream labels are poisoned"))?
                .insert(update.stream_id, label);
        }

        if let Some(gain_db) = update.gain_db {
            let gain_db = audio_core::gain::clamp_db(gain_db);
            // The state carries the clamped value, so a caller that asked for
            // more than the range allows is answered with what it actually
            // got rather than with its own request handed back.
            stream.gain_db = gain_db;
            if let Some(gain) = self.gains.get(&update.stream_id) {
                gain.set_db(gain_db);
            }
        }

        if let Some(muted) = update.muted {
            stream.muted = muted;
            if let Some(mute) = self.mutes.get(&update.stream_id) {
                mute.store(muted, Ordering::Relaxed);
            }
            tracing::info!(stream_id = update.stream_id, muted, "stream mute changed");
        }

        if let Some(language) = update.language {
            // A closed channel means the processing thread has already left —
            // the device was unplugged, or Stop is in flight. Not worth an
            // error: the session is ending, and there is no segmenter left for
            // the change to reach.
            let _ = self.languages.send((update.stream_id, language));
        }

        let state = running.clone();
        drop(running);

        self.events.state(state.clone());
        Ok(state)
    }

    /// Stops in the order the data flows, so nothing is left half-processed.
    pub fn stop(mut self) {
        self.shutdown();
    }

    /// Whether the pipeline has left its loop — because the device was
    /// unplugged, the sidecar was lost, or Stop was pressed.
    ///
    /// The failure is discovered on the processing thread, which has no way
    /// back to the `Session` the Tauri state holds: it reports
    /// [`SessionState::Failed`] and ends. Without a way to ask, the dead
    /// session stays in that slot and the next Start is refused with "a
    /// session is already running" — while the UI, which saw the failure,
    /// offers Start rather than Stop. So the question has to be askable.
    pub fn is_finished(&self) -> bool {
        self.processing
            .as_ref()
            .is_some_and(|handle| handle.is_finished())
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(capture) = self.capture.take() {
            if let Err(err) = capture.stop() {
                tracing::warn!(error = %err, "stopping the capture");
            }
        }
        // The processing thread flushes whatever was still being spoken on
        // every stream into the queue as it leaves, so the pool must outlive
        // it.
        if let Some(handle) = self.processing.take() {
            let _ = handle.join();
        }
        self.pool.shutdown();
        // Only once the pool has closed its results channel does the
        // aggregator release what it was still holding back and report the
        // session as stopped.
        if let Some(handle) = self.aggregating.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The detector the segmenter asks "is this speech".
///
/// Silero when the model is there, energy when it is not. The fallback is
/// deliberate rather than an error: an energy detector opens segments for a
/// bell and a chair scrape, but it does hear speech, and the sidecar's gate
/// still stands behind it. A session that runs badly is recoverable; one that
/// refuses to start because a 0.9 MB file is missing is not.
///
/// Called once per stream — the detector holds per-stream state, so eight
/// channels mean eight instances of it.
pub fn speech_detector(vad_model: Option<&std::path::Path>) -> Box<dyn Vad> {
    let Some(path) = vad_model else {
        return Box::new(EnergyVad::new(SAMPLE_RATE));
    };

    match SileroVad::new(path) {
        Ok(vad) => {
            // Per stream, so at debug: the session logs once which detector it
            // ended up with.
            tracing::debug!(
                model = %path.display(),
                frame_samples = vad.frame_samples(),
                "speech detection in front of the segmenter"
            );
            Box::new(vad)
        }
        Err(err) => {
            // Loud, because the difference is what ends up in the chat: from
            // here on a bell can open a segment.
            tracing::warn!(
                model = %path.display(),
                error = %err,
                "Silero failed to load — falling back to the energy detector"
            );
            Box::new(EnergyVad::new(SAMPLE_RATE))
        }
    }
}

fn spawn_processor(
    mut pipelines: Vec<Pipeline>,
    pool: Arc<WorkerPool>,
    capture_events: mpsc::Receiver<CaptureEvent>,
    // Language changes from `Session::update_stream`. Applied here because
    // this thread owns the segmenters.
    languages: mpsc::Receiver<(u16, String)>,
    stop: Arc<AtomicBool>,
    events: Arc<dyn EventSink>,
    // For the disconnect message, and only for that. The event itself carries
    // the device's id, which is a CoreAudio UID and means nothing to the
    // operator reading why their session stopped.
    device_name: String,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("processing".into())
        .spawn(move || {
            let mut buffer: Vec<f32> = Vec::with_capacity(SAMPLE_RATE as usize);
            let level_interval = Duration::from_millis(1000 / LEVEL_HZ as u64);
            let mut last_level = Instant::now();

            while !stop.load(Ordering::Relaxed) {
                // A device that disappeared ends the session; reporting it is
                // the difference between "nothing happens" and an explanation.
                // It takes every stream with it — there is one device.
                if let Ok(event) = capture_events.try_recv() {
                    match event {
                        CaptureEvent::Disconnected { device, reason } => {
                            tracing::warn!(id = %device, "capture device disconnected");
                            events.state(SessionState::Failed {
                                message: format!(
                                    "audio device '{device_name}' disconnected: {reason}"
                                ),
                            });
                            break;
                        }
                        CaptureEvent::Error { message } => {
                            tracing::warn!(message, "capture error");
                        }
                    }
                }

                // Before the pump, so a change made between two wake-ups is
                // in force for the audio this round is about to segment. A
                // stream that is not here is one the device has no channel
                // for: it has no pipeline, and its language reaches nothing.
                for (stream_id, language) in languages.try_iter() {
                    let Some(pipeline) = pipelines
                        .iter_mut()
                        .find(|p| p.segmenter.stream_id().0 == stream_id)
                    else {
                        continue;
                    };
                    tracing::info!(stream_id, language, "stream language changed");
                    pipeline.segmenter.set_language_mode(LanguageMode::Fixed {
                        language: LanguageCode::new(&language),
                    });
                }

                let mut failed = None;
                for pipeline in &mut pipelines {
                    buffer.clear();
                    if let Err(err) = pipeline.processor.pump(&mut buffer) {
                        failed = Some(err.to_string());
                        break;
                    }

                    // Before both of the things that read this buffer: the
                    // speech detector decides on the trimmed signal — which is
                    // the point of trimming a channel nobody was being heard
                    // on — and the meter shows it, which is how the knob is
                    // set at all.
                    pipeline.gain.apply(&mut buffer);

                    // The meter is fed whether or not the channel is muted:
                    // mute stops the transcription, not the listening, and a
                    // bar that still moves beside a lit M is how an operator
                    // sees that the channel is alive and deliberately not
                    // arriving. A dead bar would read as a dead channel.
                    pipeline.window.extend_from_slice(&buffer);

                    let muted = pipeline.mute.load(Ordering::Relaxed);
                    if muted != pipeline.was_muted {
                        pipeline.was_muted = muted;
                        // Muted mid-sentence: close what is open and decode
                        // it. Those words were said before the M was pressed
                        // and belong in the chat; left in the segmenter they
                        // would arrive whenever the channel comes back, under
                        // a timestamp from before the interval.
                        if muted {
                            if let Some(segment) = pipeline.segmenter.finish() {
                                pool.submit(segment, Instant::now());
                            }
                        }
                    }

                    if !muted {
                        for segment in pipeline.segmenter.push(&buffer) {
                            pool.submit(segment, Instant::now());
                        }
                    }
                }
                if let Some(message) = failed {
                    events.state(SessionState::Failed { message });
                    break;
                }

                if last_level.elapsed() >= level_interval {
                    events.levels(LevelsEvent {
                        streams: pipelines
                            .iter_mut()
                            .map(|pipeline| {
                                let level = StreamLevel {
                                    stream_id: pipeline.segmenter.stream_id().0,
                                    rms: vad::rms(&pipeline.window),
                                    peak: vad::peak(&pipeline.window),
                                    dropped: pipeline.processor.source().stats().samples_dropped(),
                                };
                                pipeline.window.clear();
                                level
                            })
                            .collect(),
                    });
                    last_level = Instant::now();
                }

                std::thread::sleep(PUMP_INTERVAL);
            }

            // Whatever was still being spoken on any channel when stop was
            // pressed.
            for pipeline in &mut pipelines {
                if let Some(segment) = pipeline.segmenter.finish() {
                    pool.submit(segment, Instant::now());
                }
            }
        })
        .expect("spawning the processing thread")
}

/// Why a decoded segment does not belong in the chat, or `None` if it does.
///
/// Public so the integration test can assert the whole rule rather than
/// re-implement it.
///
/// Returns the reason rather than a bool so the rejection can be logged.
/// Nothing here may drop anything quietly: an operator who spoke and saw no
/// text needs to be able to find out which of these fired.
pub fn discard_reason(text: &str, no_speech_probability: f32) -> Option<&'static str> {
    if text.is_empty() {
        return Some("empty");
    }
    // whisper's own verdict, and the only one in the chain that actually
    // models the difference between speech and other sound.
    if no_speech_probability > NO_SPEECH_THRESHOLD {
        return Some("no speech");
    }
    // The decoder is told not to caption sound events at all; this catches
    // what still gets through.
    if audio_core::is_only_non_speech(text) {
        return Some("annotation only");
    }
    None
}

/// Turns pool results into UI events.
///
/// Two jobs the single worker did not have. It puts results back in order —
/// several workers decode at once and a short segment overtakes a long one,
/// and a chat log read out of order is actively misleading. Ordering is *per
/// stream*: results from different channels are independent and must never
/// wait for each other, or one busy channel would hold up every quiet one.
/// And it owns the end of the session: the pool closing its results channel is
/// what tells the UI that everything spoken has been accounted for.
fn spawn_aggregator(
    session_id: u64,
    results: Receiver<PoolEvent>,
    labels: Labels,
    events: Arc<dyn EventSink>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("aggregate".into())
        .spawn(move || {
            let mut reorder = Reorder::new();
            let mut dropped: HashMap<u16, u64> = HashMap::new();
            let mut fatal: Option<String> = None;

            for event in results {
                match event {
                    PoolEvent::Completed(completed) => {
                        for ready in reorder.accept(completed) {
                            emit(session_id, &events, &labels, ready, &mut dropped);
                        }
                    }
                    // Not a session error: the pool healed itself, and the
                    // segment that was in flight has already been retried or
                    // reported. Worth a line because it is the only thing that
                    // explains a gap in an otherwise healthy transcript.
                    PoolEvent::WorkerRestarted { worker, reason } => {
                        tracing::warn!(worker, reason, "a sidecar was replaced");
                    }
                    // Remembered rather than reported straight away: results
                    // from the workers that are still finishing belong in the
                    // chat before the message saying it is over.
                    PoolEvent::Exhausted { message } => fatal = Some(message),
                }
            }

            // The pool is finished, so a segment still missing is never going
            // to arrive. Holding its successors back now would lose the last
            // words spoken rather than the one segment that went astray.
            for ready in reorder.drain() {
                emit(session_id, &events, &labels, ready, &mut dropped);
            }

            match fatal {
                Some(message) => events.state(SessionState::Failed { message }),
                None => events.state(SessionState::Stopped),
            }
        })
        .expect("spawning the aggregator thread")
}

fn emit(
    session_id: u64,
    events: &Arc<dyn EventSink>,
    labels: &Labels,
    completed: Completed,
    dropped: &mut HashMap<u16, u64>,
) {
    let Completed {
        stream_id,
        seq,
        t_start,
        t_end,
        closed_at,
        outcome,
    } = completed;
    let stream_id = stream_id.0;

    match outcome {
        Outcome::Text {
            text,
            detected_language,
            no_speech_probability,
            decode_ms,
        } => {
            // Before anything judges the text: cut a decoder repetition loop
            // out of it. A loop is a defect of the decode, not of the speech,
            // so it is repaired rather than dropped — the words before it are
            // usually the real thing somebody said.
            let text = match audio_core::collapse_repetitions(&text) {
                Cow::Borrowed(_) => text,
                Cow::Owned(collapsed) => {
                    tracing::warn!(
                        stream_id,
                        seq,
                        decode_ms,
                        chars_before = text.chars().count(),
                        chars_after = collapsed.chars().count(),
                        "repetition loop collapsed"
                    );
                    collapsed
                }
            };

            // Three ways a segment turns out not to be speech, all logged
            // rather than dropped quietly: the operator has to be able to find
            // out why nothing appeared.
            if let Some(reason) = discard_reason(&text, no_speech_probability) {
                tracing::info!(
                    stream_id,
                    seq,
                    decode_ms,
                    no_speech_probability,
                    reason,
                    "not speech, dropped"
                );
                return;
            }

            let latency_ms = closed_at.elapsed().as_millis() as u32;
            // Length and timings, not the words: this is an operator's
            // intercom traffic, and a terminal log is not where it belongs by
            // default. The text itself sits one level lower, for when a run
            // has to be debugged deliberately.
            tracing::info!(
                stream_id,
                seq,
                decode_ms,
                latency_ms,
                chars = text.chars().count(),
                "transcript"
            );
            tracing::debug!(stream_id, seq, text = %text, "transcript text");

            events.transcript(TranscriptEvent {
                session_id,
                stream_id,
                // Read per transcript rather than captured once, so a rename
                // made mid-show (see [`Session::update_stream`]) is on the
                // next line rather than on the next session.
                //
                // A result for a stream that was never configured cannot
                // happen, and an empty label in the chat would be a puzzle
                // rather than a clue — so it says which id it was. An
                // unreadable map is the same situation from the other side.
                label: labels
                    .lock()
                    .ok()
                    .and_then(|labels| labels.get(&stream_id).cloned())
                    .unwrap_or_else(|| format!("stream {stream_id}")),
                seq,
                t_start_ms: t_start.as_millis() as u64,
                t_end_ms: t_end.as_millis() as u64,
                text,
                detected_language,
                decode_ms,
                latency_ms,
                alerts: Vec::new(),
                actions: Vec::new(),
                flash_views: None,
            });
        }
        // One segment lost, the session intact — a sidecar that rejected it,
        // or a worker that died with it twice over.
        Outcome::Failed(message) => {
            tracing::warn!(stream_id, seq, message, "a segment was lost");
        }
        Outcome::Overloaded => {
            let total = dropped.entry(stream_id).or_default();
            *total += 1;
            tracing::warn!(stream_id, seq, total, "segment dropped, queue full");
            events.overload(OverloadEvent {
                session_id,
                stream_id,
                seq,
                dropped: *total,
            });
        }
    }
}

/// Everything the device picker needs.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "session.ts")]
pub struct DeviceOption {
    /// Stable across restarts, and what a profile stores. The UI shows
    /// `name` and sends this back.
    pub id: String,
    pub name: String,
    pub channels: u16,
    pub default_sample_rate: u32,
    pub is_default: bool,
}

pub fn list_devices() -> Result<Vec<DeviceOption>> {
    let backend = CpalBackend::new();
    Ok(backend
        .input_devices()?
        .into_iter()
        .map(|info| DeviceOption {
            id: info.id.0,
            name: info.name,
            channels: info.channels,
            default_sample_rate: info.default_sample_rate,
            is_default: info.is_default,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_core::StreamId;
    use std::sync::Mutex;

    /// Collects what the UI would have been shown.
    #[derive(Default)]
    struct Recorder {
        transcripts: Mutex<Vec<TranscriptEvent>>,
        overloads: Mutex<Vec<OverloadEvent>>,
        states: Mutex<Vec<SessionState>>,
    }

    impl EventSink for Recorder {
        fn transcript(&self, event: TranscriptEvent) {
            self.transcripts.lock().unwrap().push(event);
        }
        fn levels(&self, _: LevelsEvent) {}
        fn overload(&self, event: OverloadEvent) {
            self.overloads.lock().unwrap().push(event);
        }
        fn state(&self, event: SessionState) {
            self.states.lock().unwrap().push(event);
        }
    }

    fn completed(stream: u16, seq: u64, outcome: Outcome) -> PoolEvent {
        PoolEvent::Completed(Completed {
            stream_id: StreamId(stream),
            seq,
            t_start: Duration::from_millis(seq * 500),
            t_end: Duration::from_millis(seq * 500 + 500),
            closed_at: Instant::now(),
            outcome,
        })
    }

    fn spoken_on(stream: u16, seq: u64, text: &str) -> PoolEvent {
        completed(
            stream,
            seq,
            Outcome::Text {
                text: text.into(),
                detected_language: Some("de".into()),
                no_speech_probability: 0.01,
                decode_ms: 320,
            },
        )
    }

    fn spoken(seq: u64, text: &str) -> PoolEvent {
        spoken_on(0, seq, text)
    }

    /// Runs the aggregator over a fixed sequence of pool events and returns
    /// everything the UI saw.
    fn aggregate(events: Vec<PoolEvent>) -> Arc<Recorder> {
        aggregate_for([(0, "Regie"), (1, "FOH"), (2, "Licht")], events)
    }

    fn aggregate_for(
        labels: impl IntoIterator<Item = (u16, &'static str)>,
        events: Vec<PoolEvent>,
    ) -> Arc<Recorder> {
        aggregate_as(1, labels, events)
    }

    fn aggregate_as(
        session_id: u64,
        labels: impl IntoIterator<Item = (u16, &'static str)>,
        events: Vec<PoolEvent>,
    ) -> Arc<Recorder> {
        let recorder = Arc::new(Recorder::default());
        let sink: Arc<dyn EventSink> = recorder.clone();
        let (tx, rx) = mpsc::channel();
        let labels: Labels = Arc::new(std::sync::Mutex::new(
            labels
                .into_iter()
                .map(|(id, label)| (id, label.to_string()))
                .collect(),
        ));
        let handle = spawn_aggregator(session_id, rx, labels, sink);
        for event in events {
            tx.send(event).unwrap();
        }
        drop(tx);
        handle.join().unwrap();
        recorder
    }

    /// A rename lands on the next line, not on the next session (P6).
    ///
    /// The aggregator holds the labels for the whole session, so the risk is
    /// that it captures them once at start and a rename reaches nothing until
    /// capture is stopped — which is the restart `update_stream_config` exists
    /// to avoid.
    #[test]
    fn a_renamed_stream_is_renamed_from_the_next_transcript_on() {
        let recorder = Arc::new(Recorder::default());
        let sink: Arc<dyn EventSink> = recorder.clone();
        let (tx, rx) = mpsc::channel();
        let labels: Labels = Arc::new(std::sync::Mutex::new(HashMap::from([(
            0,
            "Regie".to_string(),
        )])));
        let handle = spawn_aggregator(1, rx, labels.clone(), sink);

        tx.send(spoken(0, "vor der Umbenennung")).unwrap();
        // Waited for, so the rename cannot race the line before it: the point
        // of the test is which side of the change each line falls on.
        while recorder.transcripts.lock().unwrap().is_empty() {
            std::thread::yield_now();
        }
        labels.lock().unwrap().insert(0, "Inspizienz".to_string());
        tx.send(spoken(0, "nach der Umbenennung")).unwrap();
        drop(tx);
        handle.join().unwrap();

        let seen: Vec<(String, String)> = recorder
            .transcripts
            .lock()
            .unwrap()
            .iter()
            .map(|t| (t.label.clone(), t.text.clone()))
            .collect();
        assert_eq!(
            seen,
            [
                ("Regie".to_string(), "vor der Umbenennung".to_string()),
                ("Inspizienz".to_string(), "nach der Umbenennung".to_string()),
            ],
            "the line spoken before the rename keeps the label it was spoken under"
        );
    }

    #[test]
    fn transcripts_reach_the_chat_in_order_however_the_workers_finished() {
        // Several workers decode at once and a short segment overtakes a long
        // one. Read out of order a chat log is not merely untidy, it says
        // something different from what was said.
        let recorder = aggregate(vec![
            spoken(2, "drei"),
            spoken(0, "eins"),
            spoken(1, "zwei"),
        ]);

        let seen: Vec<String> = recorder
            .transcripts
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.text.clone())
            .collect();
        assert_eq!(seen, ["eins", "zwei", "drei"]);
    }

    #[test]
    fn one_channel_waiting_for_a_result_does_not_hold_up_another() {
        // The reason ordering is per stream and not global. Channel 0's
        // segment 0 is still decoding; channel 1 has no reason to wait for it,
        // and if it did, one busy channel would stall every quiet one.
        let recorder = aggregate(vec![
            spoken_on(1, 0, "FOH eins"),
            spoken_on(1, 1, "FOH zwei"),
            spoken_on(0, 0, "Regie eins"),
        ]);

        let seen: Vec<(u16, String)> = recorder
            .transcripts
            .lock()
            .unwrap()
            .iter()
            .map(|t| (t.stream_id, t.text.clone()))
            .collect();
        assert_eq!(
            seen,
            [
                (1, "FOH eins".to_string()),
                (1, "FOH zwei".to_string()),
                (0, "Regie eins".to_string()),
            ]
        );
    }

    #[test]
    fn every_transcript_carries_its_own_streams_label() {
        // One label for the whole session was fine with one channel. With
        // eight it is the only thing that says who spoke.
        let recorder = aggregate(vec![spoken_on(0, 0, "eins"), spoken_on(2, 0, "zwei")]);
        let seen: Vec<String> = recorder
            .transcripts
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.label.clone())
            .collect();
        assert_eq!(seen, ["Regie", "Licht"]);
    }

    #[test]
    fn a_dropped_segment_does_not_hold_back_the_words_after_it() {
        // The reason the pool reports segments it gave up on rather than
        // staying quiet about them: the ones behind it would never appear.
        let recorder = aggregate(vec![
            spoken(1, "zwei"),
            completed(0, 0, Outcome::Overloaded),
            spoken(2, "drei"),
        ]);

        let texts: Vec<String> = recorder
            .transcripts
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.text.clone())
            .collect();
        assert_eq!(texts, ["zwei", "drei"]);

        // And the loss itself is visible, not just absent.
        let overloads = recorder.overloads.lock().unwrap();
        assert_eq!(overloads.len(), 1);
        assert_eq!(overloads[0].seq, 0);
        assert_eq!(overloads[0].dropped, 1);
    }

    #[test]
    fn overload_is_counted_per_stream() {
        // A running total across all channels would tell an operator the app
        // is behind but not on which channel they lost words.
        let recorder = aggregate(vec![
            completed(0, 0, Outcome::Overloaded),
            completed(1, 0, Outcome::Overloaded),
            completed(0, 1, Outcome::Overloaded),
        ]);
        let overloads = recorder.overloads.lock().unwrap();
        let counts: Vec<(u16, u64)> = overloads.iter().map(|o| (o.stream_id, o.dropped)).collect();
        assert_eq!(counts, [(0, 1), (1, 1), (0, 2)]);
    }

    #[test]
    fn the_last_words_survive_a_segment_that_never_came_back() {
        // At shutdown the missing predecessor is never going to arrive.
        // Holding its successors back would lose the end of the session.
        let recorder = aggregate(vec![spoken(1, "zwei"), spoken(2, "drei")]);
        assert_eq!(recorder.transcripts.lock().unwrap().len(), 2);
    }

    #[test]
    fn non_speech_is_filtered_on_the_way_to_the_chat() {
        let recorder = aggregate(vec![
            spoken(0, "[BREAK]"),
            spoken(1, "Standby für Cue fünf"),
        ]);
        let texts = recorder.transcripts.lock().unwrap();
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[0].text, "Standby für Cue fünf");
    }

    #[test]
    fn a_repetition_loop_is_collapsed_on_the_way_to_the_chat() {
        // The decoder cycled; the words it cycled on were real, so the line
        // belongs in the chat — once.
        let recorder = aggregate(vec![spoken(
            0,
            "Das ist schon jetzt, das ist schon jetzt, das ist schon jetzt, \
             das ist schon jetzt.",
        )]);
        let texts = recorder.transcripts.lock().unwrap();
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[0].text, "Das ist schon jetzt.");
    }

    #[test]
    fn a_clean_run_ends_as_stopped_and_an_exhausted_pool_as_failed() {
        // The UI has to tell "you pressed Stop" from "there is nothing left to
        // transcribe with", and both arrive the same way.
        let clean = aggregate(vec![spoken(0, "eins")]);
        assert!(matches!(
            clean.states.lock().unwrap().as_slice(),
            [SessionState::Stopped]
        ));

        let broken = aggregate(vec![
            spoken(0, "eins"),
            PoolEvent::Exhausted {
                message: "no sidecar could be restarted".into(),
            },
        ]);
        // The transcript still gets through: results already in flight belong
        // in the chat before the message saying it is over.
        assert_eq!(broken.transcripts.lock().unwrap().len(), 1);
        assert!(matches!(
            broken.states.lock().unwrap().as_slice(),
            [SessionState::Failed { .. }]
        ));
    }

    #[test]
    fn a_second_session_does_not_collide_with_the_first() {
        // Start is deliberately non-destructive, so the chat still holds the
        // previous session's lines — and every stream gets a fresh segmenter,
        // so sequence numbers begin again at zero. `(stream, seq)` therefore
        // repeats across sessions, and the chat list keys on it: the second
        // session's first line arrived with the same identity as the first
        // session's and the keyed list threw, which stopped the chat
        // rendering altogether. What identifies a segment is
        // `(session, stream, seq)`.
        let first = aggregate_as(1, [(0, "Regie")], vec![spoken(0, "eins")]);
        let second = aggregate_as(2, [(0, "Regie")], vec![spoken(0, "zwei")]);

        let id = |r: &Arc<Recorder>| {
            let t = r.transcripts.lock().unwrap();
            (t[0].session_id, t[0].stream_id, t[0].seq)
        };
        assert_eq!(id(&first), (1, 0, 0));
        assert_eq!(id(&second), (2, 0, 0));
        assert_ne!(id(&first), id(&second));
    }

    #[test]
    fn every_session_gets_its_own_id() {
        // Two sessions in one process must never be handed the same id, or
        // the collision above comes back with a longer key.
        let first = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        let second = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
        assert_ne!(first, second);
    }

    #[test]
    fn the_transcript_event_matches_what_the_ui_reads() {
        // The field names cross the IPC bridge, so they are pinned down here
        // rather than discovered by a blank chat view.
        let json = serde_json::to_value(TranscriptEvent {
            session_id: 3,
            stream_id: 0,
            seq: 4,
            label: "Regie".into(),
            t_start_ms: 1_200,
            t_end_ms: 3_400,
            // German on purpose: speech data, not code.
            text: "Standby für Cue fünf".into(),
            detected_language: Some("de".into()),
            decode_ms: 412,
            latency_ms: 980,
            alerts: vec![crate::alerts::AlertMatch { start: 8, end: 16 }],
            actions: vec![crate::rules::Action::Highlight],
            flash_views: Some(vec![1]),
        })
        .unwrap();

        assert_eq!(json["sessionId"], 3);
        assert_eq!(json["streamId"], 0);
        assert_eq!(json["tStartMs"], 1_200);
        assert_eq!(json["detectedLanguage"], "de");
        assert_eq!(json["latencyMs"], 980);
        assert_eq!(json["alerts"][0]["start"], 8);
        assert_eq!(json["alerts"][0]["end"], 16);
        assert_eq!(json["actions"][0], "highlight");
        assert_eq!(json["flashViews"][0], 1);
    }

    #[test]
    fn levels_cross_the_bridge_as_one_event_for_all_streams() {
        // Eight meters, one event: the shape the UI indexes by `streamId`.
        let json = serde_json::to_value(LevelsEvent {
            streams: vec![
                StreamLevel {
                    stream_id: 0,
                    rms: 0.1,
                    peak: 0.4,
                    dropped: 0,
                },
                StreamLevel {
                    stream_id: 1,
                    rms: 0.0,
                    peak: 0.0,
                    dropped: 128,
                },
            ],
        })
        .unwrap();

        assert_eq!(json["streams"][0]["streamId"], 0);
        assert_eq!(json["streams"][1]["dropped"], 128);
    }

    #[test]
    fn session_state_is_tagged_for_the_ui() {
        let running = serde_json::to_value(SessionState::Running {
            session_id: 7,
            device: "MacBook Pro Microphone".into(),
            streams: vec![
                RunningStream {
                    stream_id: 0,
                    channel: 2,
                    label: "Regie".into(),
                    absent: false,
                    muted: true,
                    gain_db: 6,
                },
                RunningStream {
                    stream_id: 1,
                    channel: 9,
                    label: "Licht".into(),
                    absent: true,
                    muted: false,
                    gain_db: 0,
                },
            ],
            started_at_ms: 1_758_100_000_000,
            sample_rate: 48_000,
            model: "ggml-small-q5_1.bin".into(),
            load_ms: 140,
            gpu: true,
        })
        .unwrap();
        assert_eq!(running["state"], "running");
        assert_eq!(running["sessionId"], 7);
        assert_eq!(running["startedAtMs"], 1_758_100_000_000u64);
        assert_eq!(running["sampleRate"], 48_000);
        assert_eq!(running["streams"][0]["streamId"], 0);
        assert_eq!(running["streams"][0]["channel"], 2);
        // A channel the device does not have travels back as absent rather
        // than as a failure to start, and the UI renders it from this flag.
        assert_eq!(running["streams"][0]["absent"], false);
        assert_eq!(running["streams"][1]["absent"], true);
        // And mute is the other flag entirely: the operator pressed M on a
        // channel the device *does* have. Two words because they are two
        // things — see [`RunningStream::absent`].
        assert_eq!(running["streams"][0]["muted"], true);
        assert_eq!(running["streams"][1]["muted"], false);
        // The trim in force, so a knob rendered from this state shows what the
        // pipeline is doing rather than what the caller last asked for.
        assert_eq!(running["streams"][0]["gainDb"], 6);

        let failed = serde_json::to_value(SessionState::Failed {
            message: "no model".into(),
        })
        .unwrap();
        assert_eq!(failed["state"], "failed");
    }

    #[test]
    fn sound_annotations_never_reach_the_chat() {
        // Exactly what showed up on a quiet channel during the first live run.
        assert_eq!(discard_reason("[BREAK]", 0.1), Some("annotation only"));
        assert_eq!(discard_reason("(music)", 0.1), Some("annotation only"));
        assert_eq!(discard_reason("(Bell)", 0.1), Some("annotation only"));
        assert_eq!(discard_reason("", 0.1), Some("empty"));
    }

    #[test]
    fn whispers_own_no_speech_verdict_is_trusted() {
        // Plausible words over silence are the dangerous case: the text looks
        // fine, so nothing about it gives the operator a reason to doubt it.
        assert_eq!(discard_reason("Thank you.", 0.95), Some("no speech"));
        assert_eq!(discard_reason("Thank you.", 0.1), None);
    }

    #[test]
    fn speech_is_kept_even_next_to_an_annotation() {
        // German on purpose: this is speech data, not code.
        assert_eq!(discard_reason("Standby für Cue fünf", 0.02), None);
        assert_eq!(discard_reason("Go.", 0.2), None);
        assert_eq!(discard_reason("[BREAK] dann Blackout", 0.2), None);
    }

    #[test]
    fn the_threshold_is_not_accidentally_inverted() {
        assert_eq!(discard_reason("Go.", NO_SPEECH_THRESHOLD), None);
        assert_eq!(
            discard_reason("Go.", NO_SPEECH_THRESHOLD + 0.01),
            Some("no speech")
        );
    }

    #[test]
    fn the_session_config_is_read_as_the_ui_writes_it() {
        let config: SessionConfig = serde_json::from_str(
            r#"{"deviceId":"AppleUSBAudioEngine:Focusrite:1",
                "vocabulary":"Standby, Cue, Go","streams":[
                {"channel":0,"label":"Regie","language":"de"},
                {"channel":5,"label":"FOH","language":"en"}
            ]}"#,
        )
        .unwrap();
        // The id, not the name: the UI picks a device from `list_devices` and
        // sends back what identifies it.
        assert_eq!(config.device_id, "AppleUSBAudioEngine:Focusrite:1");
        assert_eq!(config.streams.len(), 2);
        assert_eq!(config.streams[0].channel, 0);
        // One vocabulary for the whole session, not one per stream.
        assert_eq!(config.vocabulary.as_deref(), Some("Standby, Cue, Go"));
        // Not contiguous and not ordered: an engineer patches the channels
        // they have, not the first N.
        assert_eq!(config.streams[1].channel, 5);
        assert_eq!(config.streams[1].language, "en");
    }

    #[test]
    fn a_session_without_a_vocabulary_still_starts() {
        // The field is optional in both senses: absent parses, and it parses
        // as "no prompt" rather than as a failure to start.
        let config: SessionConfig = serde_json::from_str(
            r#"{"deviceId":"AppleUSBAudioEngine:Focusrite:1",
                "streams":[{"channel":0,"label":"Regie","language":"de"}]}"#,
        )
        .unwrap();
        assert_eq!(config.vocabulary, None);
    }
}

//! Reading along with other instances (P29).
//!
//! One machine transcribes the intercom; another, somewhere else in the
//! building, wants to read what was said — and runs its own app all the same,
//! with its own shoutbox, its own headphones and possibly its own session on
//! its own interface. So this is not a remote control. It is a subscription:
//! the other instance's feed (see [`crate::feed`]) is read, and its lines are
//! fed into this machine's chat as if they had been transcribed here.
//!
//! **Into the chat as rows.** A channel read from another instance is a row of
//! the active profile like any other — with a [`ChannelSource`] instead of an
//! input — so it has a position, a label and a colour, it can be in groups and
//! named by rules, and its lines carry that position as their stream id. The
//! chat, the views, the rules and the history all work by stream id, and
//! nothing in them has to learn about a second kind of channel. A remote
//! channel that has no row is simply not shown: subscribing to an instance is
//! not the same as wanting every one of its channels on this screen.
//!
//! **Sessions are renamed, not shared.** Two instances count their sessions
//! from one, so the other instance's session 3 would collide with this one's.
//! Each remote session is given an id of this machine's, above
//! [`REMOTE_SESSION_BASE`], and carries its own zero point — the other
//! machine's clock, which on one house network is as good as this one's.
//!
//! **What happens to a line here is this machine's business.** The rules run
//! on it, so an alert term set up here fires on what the other position said.
//! Webhooks do not: the instance that transcribed it has its own rules, and a
//! cue fired by both would be a cue fired twice — see `TauriEvents` in the
//! crate root.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::{mpsc, oneshot, watch, Notify};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use crate::activity::SpeechActivity;
use crate::config::{Config, StoredSource};
use crate::discovery::FoundServer;
use crate::feed::{self, FeedActivity, FeedChannel, FeedClip, FeedSnapshot, FeedTranscript, Frame};
use crate::history::SessionStart;
use crate::outputs::ReplayWav;
use crate::replay::ClipKey;
use crate::session::TranscriptEvent;

/// Where the session ids of other instances begin, as this machine calls them.
///
/// Far above anything one process counts to, and far below what a JavaScript
/// number holds exactly — the frontend compares these.
pub const REMOTE_SESSION_BASE: u64 = 1 << 40;

pub fn is_remote_session(id: u64) -> bool {
    id >= REMOTE_SESSION_BASE
}

/// The subscriptions as they stand, sent on every change.
pub const EVENT_SOURCES: &str = "sources-status";

/// A session of another instance became known: its id here and its zero
/// point. Its own event rather than `session-state`, which is this machine's
/// session starting — something a session elsewhere must never look like.
pub const EVENT_SESSION_ADDED: &str = "session-added";

/// How long a connection or a request may take before it is given up on.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a question to another instance may wait for its answer.
const CALL_TIMEOUT: Duration = Duration::from_secs(15);
/// How often an idle link is pinged, and how long silence may last before it
/// counts as gone. A laptop that was closed sends no close frame, and a link
/// that only finds out at the next line finds out too late.
const PING_EVERY: Duration = Duration::from_secs(10);
const SILENCE: Duration = Duration::from_secs(25);
const RECONNECT_MIN: Duration = Duration::from_millis(500);
const RECONNECT_MAX: Duration = Duration::from_secs(5);
/// How many segments a link remembers having passed on, so that a snapshot
/// after a reconnect does not repeat them. More than the history holds.
const SEEN_KEPT: usize = 20_000;

/// What a subscription is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "sources.ts")]
pub enum SourceState {
    /// Switched off in Settings.
    Off,
    Connecting,
    Connected,
    /// It asks for a key, and the one stored is missing or wrong. Not retried
    /// until a key is entered: guessing is rate-limited, and a wrong key
    /// retried every second would lock this machine out.
    NeedsKey,
    /// It was reached before, or never, and is not now. Retried.
    Lost,
}

/// One subscription, for the Settings page and the notices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "sources.ts")]
pub struct SourceStatus {
    pub id: String,
    pub name: String,
    /// Where it is being reached — the stored address, or where discovery
    /// last saw it.
    pub address: String,
    pub port: u16,
    pub state: SourceState,
    pub error: Option<String>,
    pub version: Option<String>,
    /// The channels it offers, as of the last connection.
    pub channels: Vec<FeedChannel>,
    /// Whether it has been reached since this app started. What tells
    /// "still waiting for it to come up" — both machines were switched on a
    /// minute ago — from "it went away", which is worth an operator's
    /// attention.
    pub connected_once: bool,
}

/// Where a remote channel's lines go: a row of the active profile.
#[derive(Debug, Clone)]
struct Route {
    stream_id: u16,
    label: String,
    muted: bool,
}

#[derive(Default)]
struct Sessions {
    next: u64,
    forward: HashMap<(String, u64), u64>,
    back: HashMap<u64, (String, u64)>,
}

impl Sessions {
    /// This machine's id for a session of another instance, and whether it is
    /// new.
    fn local(&mut self, server: &str, remote: u64) -> (u64, bool) {
        if let Some(&id) = self.forward.get(&(server.to_string(), remote)) {
            return (id, false);
        }
        let id = REMOTE_SESSION_BASE + self.next;
        self.next += 1;
        self.forward.insert((server.to_string(), remote), id);
        self.back.insert(id, (server.to_string(), remote));
        (id, true)
    }
}

type Answer = oneshot::Sender<Result<Value, String>>;

struct Link {
    config: StoredSource,
    /// Dropped to end the link's task.
    _stop: watch::Sender<()>,
    calls: mpsc::UnboundedSender<(feed::Call, Answer)>,
    /// Pressed when discovery has news, so a link waiting out its backoff
    /// tries the new address at once.
    wake: Arc<Notify>,
}

/// Every subscription, and what is needed to put their lines into the chat.
#[derive(Default)]
pub struct Sources {
    links: Mutex<HashMap<String, Link>>,
    status: Mutex<BTreeMap<String, SourceStatus>>,
    /// By instance and remote channel id.
    routes: Mutex<HashMap<(String, String), Route>>,
    sessions: Mutex<Sessions>,
}

fn sources_of<R: Runtime>(app: &AppHandle<R>) -> Option<Arc<Sources>> {
    app.try_state::<crate::AppState>()
        .map(|state| Arc::clone(&state.sources))
}

impl Sources {
    pub fn statuses(&self) -> Vec<SourceStatus> {
        self.status
            .lock()
            .map(|s| s.values().cloned().collect())
            .unwrap_or_default()
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut SourceStatus)) -> bool {
        let Ok(mut status) = self.status.lock() else {
            return false;
        };
        let Some(entry) = status.get_mut(id) else {
            return false;
        };
        let before = entry.clone();
        change(entry);
        *entry != before
    }

    fn route(&self, server: &str, channel: &str) -> Option<Route> {
        self.routes
            .lock()
            .ok()?
            .get(&(server.to_string(), channel.to_string()))
            .cloned()
    }

    /// The remote channel a row of the active profile shows.
    fn channel_of(&self, server: &str, stream_id: u16) -> Option<String> {
        self.routes
            .lock()
            .ok()?
            .iter()
            .find(|((s, _), route)| s == server && route.stream_id == stream_id)
            .map(|((_, channel), _)| channel.clone())
    }
}

/// Brings the subscriptions in line with the configuration — at startup and
/// on every save, like the LAN remote and the outputs.
pub fn apply<R: Runtime>(app: &AppHandle<R>, config: &Config) {
    let Some(sources) = sources_of(app) else {
        return;
    };

    // Where each remote channel goes, from the active profile.
    let mut routes = HashMap::new();
    if let Some(profile) = config.profiles.get(config.active) {
        for (position, channel) in profile.channels.iter().enumerate() {
            let (Some(source), Ok(stream_id)) = (&channel.source, u16::try_from(position)) else {
                continue;
            };
            routes.insert(
                (source.server.clone(), source.channel.clone()),
                Route {
                    stream_id,
                    label: channel.label.clone(),
                    muted: channel.muted,
                },
            );
        }
    }
    if let Ok(mut slot) = sources.routes.lock() {
        *slot = routes;
    }

    let wanted: HashMap<String, StoredSource> = config
        .sources
        .iter()
        .map(|s| (s.id.clone(), s.clone()))
        .collect();

    let mut changed = false;
    if let (Ok(mut links), Ok(mut status)) = (sources.links.lock(), sources.status.lock()) {
        // Gone, switched off or changed: the old link goes.
        links.retain(|id, link| {
            let keep = wanted
                .get(id)
                .is_some_and(|source| source.enabled && *source == link.config);
            changed |= !keep;
            keep
        });
        status.retain(|id, _| wanted.contains_key(id));

        for source in wanted.values() {
            let entry = status.entry(source.id.clone()).or_insert_with(|| {
                changed = true;
                SourceStatus {
                    id: source.id.clone(),
                    name: source.name.clone(),
                    address: source.address.clone(),
                    port: source.port,
                    state: SourceState::Off,
                    error: None,
                    version: None,
                    channels: Vec::new(),
                    connected_once: false,
                }
            });
            if !source.enabled {
                if entry.state != SourceState::Off {
                    entry.state = SourceState::Off;
                    entry.error = None;
                    changed = true;
                }
                continue;
            }
            if links.contains_key(&source.id) {
                continue;
            }
            entry.name = source.name.clone();
            entry.state = SourceState::Connecting;
            entry.error = None;
            let (stop, stopped) = watch::channel(());
            let (calls, incoming) = mpsc::unbounded_channel();
            let wake = Arc::new(Notify::new());
            tauri::async_runtime::spawn(run(
                app.clone(),
                source.clone(),
                stopped,
                incoming,
                Arc::clone(&wake),
            ));
            links.insert(
                source.id.clone(),
                Link {
                    config: source.clone(),
                    _stop: stop,
                    calls,
                    wake,
                },
            );
            changed = true;
        }
    }

    if config.sources.iter().any(|s| s.enabled) {
        if let Some(state) = app.try_state::<crate::AppState>() {
            state.discovery.browse(app);
        }
    }
    if changed {
        announce(app, &sources);
    }
}

/// Discovery saw something move: links that are waiting try again now.
pub fn moved<R: Runtime>(app: &AppHandle<R>) {
    let Some(sources) = sources_of(app) else {
        return;
    };
    let wakes: Vec<Arc<Notify>> = match sources.links.lock() {
        Ok(links) => links.values().map(|link| Arc::clone(&link.wake)).collect(),
        Err(_) => return,
    };
    for wake in wakes {
        wake.notify_one();
    }
}

fn announce<R: Runtime>(app: &AppHandle<R>, sources: &Sources) {
    crate::broadcast(app, EVENT_SOURCES, sources.statuses());
}

fn set_state<R: Runtime>(app: &AppHandle<R>, id: &str, state: SourceState, error: Option<String>) {
    let Some(sources) = sources_of(app) else {
        return;
    };
    if sources.update(id, |s| {
        s.state = state;
        s.error = error;
    }) {
        announce(app, &sources);
    }
}

/// Who answers at an address — for an instance added by hand, which discovery
/// cannot see across a VLAN boundary.
pub async fn probe(address: &str, port: u16) -> Result<FoundServer, String> {
    let address = address.trim();
    let ip: std::net::IpAddr = address
        .parse()
        .map_err(|_| format!("{address} is not an IP address"))?;
    let http = http()?;
    let text = http
        .get(format!("http://{}/info", host(ip, port)))
        .send()
        .await
        .map_err(|_| format!("Nothing answers at {address}:{port}."))?
        .error_for_status()
        .map_err(|_| format!("What answers at {address}:{port} is not Fluestr."))?
        .text()
        .await
        .map_err(|err| err.to_string())?;
    let info: Value = serde_json::from_str(&text)
        .map_err(|_| format!("What answers at {address}:{port} is not Fluestr."))?;
    let id = info["instance"].as_str().unwrap_or_default().to_string();
    if id.is_empty() {
        return Err(format!("What answers at {address}:{port} is not Fluestr."));
    }
    Ok(FoundServer {
        id,
        name: info["name"].as_str().unwrap_or(address).to_string(),
        address: ip.to_string(),
        port,
        version: info["version"].as_str().map(str::to_string),
    })
}

fn host(ip: std::net::IpAddr, port: u16) -> String {
    match ip {
        std::net::IpAddr::V4(ip) => format!("{ip}:{port}"),
        std::net::IpAddr::V6(ip) => format!("[{ip}]:{port}"),
    }
}

/// A client for the house network: no proxy — a proxy cannot reach a laptop
/// on the same switch — and short timeouts, because an instance that does not
/// answer within seconds on a LAN is not going to.
fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|err| err.to_string())
}

/// A bubble's audio from the instance that transcribed it.
pub async fn replay_wav<R: Runtime>(
    app: &AppHandle<R>,
    segments: &[ClipKey],
) -> Result<ReplayWav, String> {
    let sources = sources_of(app).ok_or("the application state is not ready")?;
    let gone = || "This audio is no longer held.".to_string();

    let mut server: Option<String> = None;
    let mut clips = Vec::with_capacity(segments.len());
    for key in segments {
        let (from, session) = sources
            .sessions
            .lock()
            .map_err(|_| "the sessions are poisoned")?
            .back
            .get(&key.session_id)
            .cloned()
            .ok_or_else(gone)?;
        if server.as_ref().is_some_and(|s| *s != from) {
            return Err("a bubble cannot span two instances".into());
        }
        let channel = sources.channel_of(&from, key.stream_id).ok_or_else(gone)?;
        clips.push(FeedClip {
            session_id: session,
            channel,
            seq: key.seq,
        });
        server = Some(from);
    }
    let server = server.ok_or_else(gone)?;

    let (answer, answered) = oneshot::channel();
    {
        let links = sources.links.lock().map_err(|_| "the links are poisoned")?;
        let link = links
            .get(&server)
            .ok_or("That instance is not being read any more.")?;
        link.calls
            .send((feed::Call::Replay { segments: clips }, answer))
            .map_err(|_| "That instance is not being read any more.".to_string())?;
    }
    let value = tokio::time::timeout(CALL_TIMEOUT, answered)
        .await
        .map_err(|_| "The other instance did not answer.".to_string())?
        .map_err(|_| "The connection to the other instance was lost.".to_string())??;
    serde_json::from_value(value).map_err(|err| err.to_string())
}

/// The samples of a bubble from another instance, for the listen output.
pub async fn replay_samples<R: Runtime>(
    app: &AppHandle<R>,
    segments: &[ClipKey],
) -> Result<Vec<f32>, String> {
    let wav = replay_wav(app, segments).await?;
    crate::replay::unbase64(&wav.wav)
        .and_then(|bytes| crate::replay::samples_of_wav(&bytes))
        .ok_or_else(|| "The other instance sent audio this one cannot read.".to_string())
}

// --- One link ---------------------------------------------------------------

enum Outcome {
    /// The link was stopped from here.
    Stopped,
    /// It wants a key this machine does not have.
    NeedsKey(String),
    /// It went, or never came; `connected` says whether it had been up.
    Lost { error: String, connected: bool },
}

/// What one link remembers across its reconnections.
#[derive(Default)]
struct Memory {
    seen: HashSet<(u64, String, u64)>,
    order: VecDeque<(u64, String, u64)>,
    /// Streams with something on screen, to be cleared when the link goes.
    active: HashMap<u16, SpeechActivity>,
}

impl Memory {
    /// Whether this segment is new, remembering it if so.
    fn first(&mut self, t: &FeedTranscript) -> bool {
        let key = (t.session_id, t.channel.clone(), t.seq);
        if !self.seen.insert(key.clone()) {
            return false;
        }
        self.order.push_back(key);
        while self.order.len() > SEEN_KEPT {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

async fn run<R: Runtime>(
    app: AppHandle<R>,
    source: StoredSource,
    mut stop: watch::Receiver<()>,
    mut calls: mpsc::UnboundedReceiver<(feed::Call, Answer)>,
    wake: Arc<Notify>,
) {
    let mut memory = Memory::default();
    let mut backoff = RECONNECT_MIN;
    loop {
        // Where discovery last saw it, if it has; otherwise where it was
        // stored. An instance that was given a new lease is found by its id.
        let (address, port) = app
            .try_state::<crate::AppState>()
            .and_then(|state| state.discovery.find(&source.id))
            .map(|found| (found.address, found.port))
            .unwrap_or_else(|| (source.address.clone(), source.port));
        if let Some(sources) = sources_of(&app) {
            if sources.update(&source.id, |s| {
                s.address = address.clone();
                s.port = port;
                if s.state != SourceState::Lost {
                    s.state = SourceState::Connecting;
                }
            }) {
                announce(&app, &sources);
            }
        }

        // A second receiver, so that the door and the handshake — which do
        // not look at `stop` themselves — are cut short as well.
        let mut stopping = stop.clone();
        let outcome = tokio::select! {
            _ = stopping.changed() => Outcome::Stopped,
            outcome = connect(&app, &source, &address, port, &mut calls, &mut stop, &mut memory) => outcome,
        };
        clear_activity(&app, &mut memory);

        match outcome {
            Outcome::Stopped => return,
            Outcome::NeedsKey(message) => {
                set_state(&app, &source.id, SourceState::NeedsKey, Some(message));
                // Until the configuration changes, which restarts the link.
                loop {
                    tokio::select! {
                        _ = stop.changed() => return,
                        call = calls.recv() => match call {
                            Some((_, answer)) => {
                                let _ = answer.send(Err("That instance asks for a key.".into()));
                            }
                            None => return,
                        },
                    }
                }
            }
            Outcome::Lost { error, connected } => {
                tracing::info!(source = %source.name, error, "an instance read along with is not reachable");
                set_state(&app, &source.id, SourceState::Lost, Some(error));
                if connected {
                    backoff = RECONNECT_MIN;
                }
            }
        }

        let until = tokio::time::sleep(backoff);
        tokio::pin!(until);
        loop {
            tokio::select! {
                _ = stop.changed() => return,
                _ = &mut until => break,
                _ = wake.notified() => break,
                call = calls.recv() => match call {
                    Some((_, answer)) => {
                        let _ = answer.send(Err("That instance is not reachable right now.".into()));
                    }
                    None => return,
                },
            }
        }
        backoff = (backoff * 2).min(RECONNECT_MAX);
    }
}

/// Asks the door for a cookie, if it wants one.
async fn admission(base: &str, key: &str) -> Result<Option<String>, Outcome> {
    let lost = |error: String| Outcome::Lost {
        error,
        connected: false,
    };
    let http = http().map_err(lost)?;
    let text = http
        .get(format!("{base}/auth"))
        .send()
        .await
        .map_err(|_| lost("It does not answer.".into()))?
        .text()
        .await
        .map_err(|err| lost(err.to_string()))?;
    let lock: Value = serde_json::from_str(&text)
        .map_err(|_| lost("What answers there is not Fluestr.".into()))?;
    if !lock["required"].as_bool().unwrap_or(false) {
        return Ok(None);
    }
    if key.trim().is_empty() {
        return Err(Outcome::NeedsKey("It asks for a key.".into()));
    }
    let answer = http
        .post(format!("{base}/auth"))
        .body(key.trim().to_string())
        .send()
        .await
        .map_err(|_| lost("It does not answer.".into()))?;
    match answer.status().as_u16() {
        429 => Err(lost(
            "Too many wrong keys from this machine — it is retried in a minute.".into(),
        )),
        403 => Err(Outcome::NeedsKey("That is not its key.".into())),
        status if (200..300).contains(&status) => Ok(answer
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(|v| v.split(';').next())
            .map(|v| v.trim().to_string())
            .next()),
        status => Err(lost(format!("It answered {status}."))),
    }
}

/// One connection, from the door to its end.
async fn connect<R: Runtime>(
    app: &AppHandle<R>,
    source: &StoredSource,
    address: &str,
    port: u16,
    calls: &mut mpsc::UnboundedReceiver<(feed::Call, Answer)>,
    stop: &mut watch::Receiver<()>,
    memory: &mut Memory,
) -> Outcome {
    let lost = |error: String, connected: bool| Outcome::Lost { error, connected };
    let Ok(ip) = address.parse::<std::net::IpAddr>() else {
        return lost(format!("{address} is not an IP address."), false);
    };
    let authority = host(ip, port);
    let cookie = match admission(&format!("http://{authority}"), &source.key).await {
        Ok(cookie) => cookie,
        Err(outcome) => return outcome,
    };

    let mut request = match format!("ws://{authority}/feed").into_client_request() {
        Ok(request) => request,
        Err(err) => return lost(err.to_string(), false),
    };
    if let Some(cookie) = cookie.and_then(|c| c.parse().ok()) {
        request
            .headers_mut()
            .insert(reqwest::header::COOKIE, cookie);
    }
    let mut socket = match tokio::time::timeout(
        CONNECT_TIMEOUT,
        tokio_tungstenite::connect_async(request),
    )
    .await
    {
        Ok(Ok((socket, _))) => socket,
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response)))
            if response.status().as_u16() == 401 =>
        {
            return Outcome::NeedsKey("That is not its key.".into());
        }
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response)))
            if response.status().as_u16() == 404 =>
        {
            return lost(
                "It runs a version of Fluestr that cannot be read along with.".into(),
                false,
            );
        }
        Ok(Err(err)) => return lost(err.to_string(), false),
        Err(_) => return lost("It does not answer.".into(), false),
    };

    // The greeting says who this is. An address handed out again to another
    // machine is not the instance that was subscribed to.
    let hello = tokio::time::timeout(CONNECT_TIMEOUT, socket.next()).await;
    let Ok(Some(Ok(Message::Text(text)))) = hello else {
        return lost("It did not say who it is.".into(), false);
    };
    match serde_json::from_str::<Frame>(&text) {
        Ok(Frame::Hello {
            instance,
            name,
            version,
        }) => {
            if instance != source.id {
                return lost(
                    format!("{address} is another instance now ({name})."),
                    false,
                );
            }
            if let Some(sources) = sources_of(app) {
                if sources.update(&source.id, |s| {
                    s.name = name;
                    s.version = Some(version);
                    s.state = SourceState::Connected;
                    s.error = None;
                    s.connected_once = true;
                }) {
                    announce(app, &sources);
                }
            }
        }
        _ => return lost("It did not say who it is.".into(), false),
    }
    tracing::info!(source = %source.name, address, "reading along");

    let mut next_id: u64 = 1;
    let mut waiting: HashMap<u64, Answer> = HashMap::new();
    // Live lines are held back until the snapshot is in, so what was said
    // before arrives before what is said now.
    let snapshot_id = 0;
    let mut held: Option<Vec<FeedTranscript>> = Some(Vec::new());
    if let Some(text) = request_text(snapshot_id, feed::Call::Snapshot) {
        if socket.send(Message::Text(text.into())).await.is_err() {
            return lost("The connection was lost.".into(), true);
        }
    }

    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut heard = Instant::now();

    let outcome = loop {
        tokio::select! {
            _ = stop.changed() => {
                let _ = socket.close(None).await;
                break Outcome::Stopped;
            }
            _ = ping.tick() => {
                if heard.elapsed() > SILENCE {
                    break lost("It stopped answering.".into(), true);
                }
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break lost("The connection was lost.".into(), true);
                }
            }
            call = calls.recv() => {
                let Some((call, answer)) = call else { break Outcome::Stopped };
                let id = next_id;
                next_id += 1;
                let Some(text) = request_text(id, call) else {
                    let _ = answer.send(Err("the question could not be written".into()));
                    continue;
                };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    let _ = answer.send(Err("The connection to the other instance was lost.".into()));
                    break lost("The connection was lost.".into(), true);
                }
                waiting.insert(id, answer);
            }
            incoming = socket.next() => {
                heard = Instant::now();
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let frame = match serde_json::from_str::<Frame>(&text) {
                            Ok(frame) => frame,
                            Err(err) => {
                                tracing::warn!(error = %err, "another instance sent something that is not a frame");
                                continue;
                            }
                        };
                        match frame {
                            Frame::Result { id, ok, value, error } if id == snapshot_id => {
                                if ok {
                                    if let Some(snapshot) = value.and_then(|v| serde_json::from_value::<FeedSnapshot>(v).ok()) {
                                        take_snapshot(app, &source.id, snapshot, memory);
                                    }
                                } else {
                                    tracing::warn!(error = ?error, "another instance would not say what was said");
                                }
                                for line in held.take().unwrap_or_default() {
                                    take_transcript(app, &source.id, line, memory);
                                }
                            }
                            Frame::Result { id, ok, value, error } => {
                                if let Some(answer) = waiting.remove(&id) {
                                    let _ = answer.send(if ok {
                                        Ok(value.unwrap_or(Value::Null))
                                    } else {
                                        Err(error.unwrap_or_else(|| "the other instance refused".into()))
                                    });
                                }
                            }
                            Frame::Transcript(line) => match held.as_mut() {
                                Some(held) => held.push(line),
                                None => take_transcript(app, &source.id, line, memory),
                            },
                            Frame::Activity(activity) => take_activity(app, &source.id, activity, memory),
                            Frame::Session(start) => take_session(app, &source.id, start),
                            Frame::Channels { channels } => {
                                if let Some(sources) = sources_of(app) {
                                    if sources.update(&source.id, |s| s.channels = channels) {
                                        announce(app, &sources);
                                    }
                                }
                            }
                            Frame::Hello { .. } => {}
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break lost("It closed the connection.".into(), true),
                    Some(Ok(_)) => {}
                    Some(Err(err)) => break lost(err.to_string(), true),
                }
            }
        }
    };

    for (_, answer) in waiting {
        let _ = answer.send(Err("The connection to the other instance was lost.".into()));
    }
    outcome
}

fn request_text(id: u64, call: feed::Call) -> Option<String> {
    serde_json::to_string(&feed::Request { id, call })
        .map_err(|err| tracing::error!(error = %err, "a feed question could not be written"))
        .ok()
}

// --- Into the chat ------------------------------------------------------------

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// This machine's id for a remote session, made known to the views the first
/// time — with its zero point, or a guess at one from the line in hand.
fn local_session<R: Runtime>(
    app: &AppHandle<R>,
    server: &str,
    remote: u64,
    started_at_ms: Option<u64>,
) -> Option<u64> {
    let sources = sources_of(app)?;
    let (id, new) = sources.sessions.lock().ok()?.local(server, remote);
    if new || started_at_ms.is_some() {
        let start = SessionStart {
            session_id: id,
            started_at_ms: started_at_ms.unwrap_or_else(now_ms),
        };
        if let Some(state) = app.try_state::<crate::AppState>() {
            if let Ok(mut history) = state.history.lock() {
                history.note_session(start);
            }
        }
        crate::broadcast(app, EVENT_SESSION_ADDED, start);
    }
    Some(id)
}

fn take_session<R: Runtime>(app: &AppHandle<R>, server: &str, start: SessionStart) {
    local_session(app, server, start.session_id, Some(start.started_at_ms));
}

fn take_snapshot<R: Runtime>(
    app: &AppHandle<R>,
    server: &str,
    snapshot: FeedSnapshot,
    memory: &mut Memory,
) {
    for start in snapshot.sessions {
        take_session(app, server, start);
    }
    for line in snapshot.transcripts {
        take_transcript(app, server, line, memory);
    }
    for activity in snapshot.activity {
        take_activity(app, server, activity, memory);
    }
}

fn take_transcript<R: Runtime>(
    app: &AppHandle<R>,
    server: &str,
    line: FeedTranscript,
    memory: &mut Memory,
) {
    if !memory.first(&line) {
        return;
    }
    let Some(route) = sources_of(app).and_then(|s| s.route(server, &line.channel)) else {
        // Not shown here: subscribed to, but not one of the rows.
        return;
    };
    // Muted here means not wanted here, as it does for an input.
    if route.muted {
        return;
    }
    let started = now_ms().saturating_sub(line.t_end_ms);
    let known = sources_of(app)
        .and_then(|s| {
            s.sessions.lock().ok().map(|sessions| {
                sessions
                    .forward
                    .contains_key(&(server.to_string(), line.session_id))
            })
        })
        .unwrap_or(false);
    let Some(session_id) = local_session(app, server, line.session_id, (!known).then_some(started))
    else {
        return;
    };
    let event = TranscriptEvent {
        session_id,
        stream_id: route.stream_id,
        seq: line.seq,
        label: route.label,
        t_start_ms: line.t_start_ms,
        t_end_ms: line.t_end_ms,
        text: line.text,
        detected_language: line.detected_language,
        decode_ms: line.decode_ms,
        latency_ms: line.latency_ms,
        alerts: Vec::new(),
        actions: Vec::new(),
        flash_views: None,
        audio: line.audio,
    };
    match crate::events(app) {
        Ok(sink) => sink.transcript(event),
        Err(err) => tracing::error!(error = %err, "a line read along with could not be shown"),
    }
}

fn take_activity<R: Runtime>(
    app: &AppHandle<R>,
    server: &str,
    activity: FeedActivity,
    memory: &mut Memory,
) {
    let Some(route) = sources_of(app).and_then(|s| s.route(server, &activity.channel)) else {
        return;
    };
    if route.muted {
        return;
    }
    let Some(session_id) = local_session(app, server, activity.session_id, None) else {
        return;
    };
    let event = SpeechActivity {
        session_id,
        stream_id: route.stream_id,
        label: route.label,
        speaking: activity.speaking,
        decoding: activity.decoding,
        since_ms: activity.since_ms,
        preview: activity.preview,
    };
    if event.active() {
        memory.active.insert(event.stream_id, event.clone());
    } else {
        memory.active.remove(&event.stream_id);
    }
    if let Ok(sink) = crate::events(app) {
        sink.activity(event);
    }
}

/// Takes down whatever a link that went had on screen: a placeholder for
/// somebody talking on a machine that is no longer heard would stay forever.
fn clear_activity<R: Runtime>(app: &AppHandle<R>, memory: &mut Memory) {
    let Ok(sink) = crate::events(app) else {
        memory.active.clear();
        return;
    };
    for (_, mut event) in memory.active.drain() {
        event.speaking = false;
        event.decoding = false;
        event.preview.clear();
        sink.activity(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_session_is_renamed_once_and_stays_renamed() {
        let mut sessions = Sessions::default();
        let (a, new_a) = sessions.local("x", 3);
        let (b, new_b) = sessions.local("y", 3);
        let (again, new_again) = sessions.local("x", 3);
        assert!(new_a && new_b && !new_again);
        assert_eq!(a, again);
        assert_ne!(a, b);
        assert!(is_remote_session(a) && is_remote_session(b));
        assert!(!is_remote_session(3));
        assert_eq!(sessions.back.get(&b), Some(&("y".to_string(), 3)));
    }

    #[test]
    fn a_segment_is_passed_on_once() {
        let mut memory = Memory::default();
        let line = FeedTranscript {
            session_id: 1,
            channel: "c".into(),
            seq: 2,
            t_start_ms: 0,
            t_end_ms: 1,
            text: "x".into(),
            detected_language: None,
            decode_ms: 0,
            latency_ms: 0,
            audio: false,
        };
        assert!(memory.first(&line));
        assert!(!memory.first(&line));
    }

    // --- Two instances in one process -------------------------------------

    fn mock() -> tauri::App<tauri::test::MockRuntime> {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("a mock app");
        app.manage(crate::AppState::default());
        app
    }

    fn line(session_id: u64, seq: u64, text: &str) -> TranscriptEvent {
        TranscriptEvent {
            session_id,
            stream_id: 0,
            seq,
            label: "Regie".into(),
            t_start_ms: seq * 1000,
            t_end_ms: seq * 1000 + 500,
            text: text.into(),
            detected_language: None,
            decode_ms: 1,
            latency_ms: 1,
            alerts: Vec::new(),
            actions: Vec::new(),
            flash_views: None,
            audio: false,
        }
    }

    /// A subscriber's configuration: one input of its own, then the server's
    /// channel `ch-a` as the second row.
    fn subscriber_config(server: &str, port: u16, key: &str) -> Config {
        serde_json::from_value(serde_json::json!({
            "version": crate::config::VERSION,
            "active": 0,
            "sources": [{
                "id": server, "name": "Server", "address": "127.0.0.1",
                "port": port, "key": key, "enabled": true,
            }],
            "profiles": [{
                "name": "Here",
                "device": "",
                "channels": [
                    { "id": "mine", "channel": 0, "label": "Mine", "language": "de", "color": "#000" },
                    { "id": "theirs", "channel": 0, "label": "Their Regie", "language": "de", "color": "#111",
                      "source": { "server": server, "channel": "ch-a" } },
                ],
                "groups": [],
            }],
        }))
        .expect("a configuration")
    }

    async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn history_of(app: &AppHandle<tauri::test::MockRuntime>) -> crate::history::Snapshot {
        app.state::<crate::AppState>()
            .history
            .lock()
            .expect("history")
            .snapshot()
    }

    #[tokio::test]
    async fn another_instance_s_lines_arrive_in_this_chat_as_its_row() {
        const PORT: u16 = 18_861;
        let server = mock();
        let server = server.handle().clone();
        let status = crate::remote::apply(
            &server,
            &crate::remote::RemoteConfig {
                enabled: true,
                address: "127.0.0.1".into(),
                port: PORT,
                key: "Sesam".into(),
            },
        )
        .await;
        assert!(status.running, "not serving: {:?}", status.error);

        // A session on the server, with one line already said.
        let stream: crate::session::StreamConfig = serde_json::from_value(serde_json::json!({
            "channel": 0, "label": "Regie", "language": "de", "id": "ch-a",
        }))
        .expect("a stream");
        server
            .state::<crate::AppState>()
            .remote
            .streams
            .started(5, &[stream]);
        let sink = crate::events(&server).expect("a sink");
        sink.state(crate::session::SessionState::Running {
            session_id: 5,
            started_at_ms: 1_700_000_000_000,
            device: "Desk".into(),
            streams: Vec::new(),
            sample_rate: 48_000,
            model: "small".into(),
            load_ms: 1,
            gpu: false,
        });
        sink.transcript(line(5, 1, "Achtung Bühne"));

        let id = server.state::<crate::AppState>().remote.instance_id();
        let subscriber = mock();
        let subscriber = subscriber.handle().clone();
        apply(&subscriber, &subscriber_config(&id, PORT, "sesam"));

        // What was said before the subscription, from the snapshot.
        wait_for("the snapshot", || {
            history_of(&subscriber).transcripts.len() == 1
        })
        .await;
        let first = history_of(&subscriber).transcripts[0].clone();
        assert_eq!(first.stream_id, 1, "the row the channel is shown as");
        assert_eq!(first.label, "Their Regie");
        assert_eq!(first.text, "Achtung Bühne");
        assert!(is_remote_session(first.session_id));
        assert!(
            history_of(&subscriber)
                .sessions
                .iter()
                .any(|s| s.session_id == first.session_id && s.started_at_ms == 1_700_000_000_000),
            "the remote session's zero point is known"
        );

        // And what is said now, live.
        sink.transcript(line(5, 2, "Licht steht"));
        wait_for("the live line", || {
            history_of(&subscriber).transcripts.len() == 2
        })
        .await;
        assert_eq!(history_of(&subscriber).transcripts[1].text, "Licht steht");

        // The subscriber does not pass the line on: it is not its own.
        let streams = &subscriber.state::<crate::AppState>().remote.streams;
        assert!(streams
            .transcript(&history_of(&subscriber).transcripts[1])
            .is_none());

        crate::remote::apply(&server, &crate::remote::RemoteConfig::default()).await;
    }

    #[tokio::test]
    async fn a_wrong_key_waits_for_the_right_one() {
        const PORT: u16 = 18_862;
        let server = mock();
        let server = server.handle().clone();
        let status = crate::remote::apply(
            &server,
            &crate::remote::RemoteConfig {
                enabled: true,
                address: "127.0.0.1".into(),
                port: PORT,
                key: "Sesam".into(),
            },
        )
        .await;
        assert!(status.running, "not serving: {:?}", status.error);
        let id = server.state::<crate::AppState>().remote.instance_id();

        let subscriber = mock();
        let subscriber = subscriber.handle().clone();
        apply(&subscriber, &subscriber_config(&id, PORT, "falsch"));
        let sources = Arc::clone(&subscriber.state::<crate::AppState>().sources);
        wait_for("the key prompt", || {
            sources
                .statuses()
                .first()
                .is_some_and(|s| s.state == SourceState::NeedsKey)
        })
        .await;

        // Entering the right one is a configuration change, which restarts
        // the link.
        apply(&subscriber, &subscriber_config(&id, PORT, "Sesam"));
        wait_for("the connection", || {
            sources
                .statuses()
                .first()
                .is_some_and(|s| s.state == SourceState::Connected)
        })
        .await;

        crate::remote::apply(&server, &crate::remote::RemoteConfig::default()).await;
    }

    /// Both machines are switched on at once and come up in whichever order
    /// they do: the one reading along simply waits for the other.
    #[tokio::test]
    async fn a_subscriber_that_starts_first_connects_when_the_instance_comes_up() {
        const PORT: u16 = 18_863;
        let server = mock();
        let server = server.handle().clone();
        let id = server.state::<crate::AppState>().remote.instance_id();

        let subscriber = mock();
        let subscriber = subscriber.handle().clone();
        apply(&subscriber, &subscriber_config(&id, PORT, ""));
        let sources = Arc::clone(&subscriber.state::<crate::AppState>().sources);
        wait_for("the first attempt to fail", || {
            sources
                .statuses()
                .first()
                .is_some_and(|s| s.state == SourceState::Lost && !s.connected_once)
        })
        .await;

        let status = crate::remote::apply(
            &server,
            &crate::remote::RemoteConfig {
                enabled: true,
                address: "127.0.0.1".into(),
                port: PORT,
                key: String::new(),
            },
        )
        .await;
        assert!(status.running, "not serving: {:?}", status.error);
        wait_for("the connection", || {
            sources
                .statuses()
                .first()
                .is_some_and(|s| s.state == SourceState::Connected && s.connected_once)
        })
        .await;

        crate::remote::apply(&server, &crate::remote::RemoteConfig::default()).await;
    }

    /// A machine that starts at login often has no address yet when the app
    /// comes up. The remote does not give up on it.
    #[tokio::test]
    async fn a_listener_that_could_not_come_up_is_tried_again() {
        const PORT: u16 = 18_864;
        let blocker = std::net::TcpListener::bind(("127.0.0.1", PORT)).expect("the port");
        let server = mock();
        let server = server.handle().clone();
        let wanted = crate::remote::RemoteConfig {
            enabled: true,
            address: "127.0.0.1".into(),
            port: PORT,
            key: String::new(),
        };
        let status = crate::remote::apply(&server, &wanted).await;
        assert!(!status.running, "the port was taken");
        assert!(status.error.is_some());

        drop(blocker);
        let hub = Arc::clone(&server.state::<crate::AppState>().remote);
        wait_for("the listener to come up by itself", || hub.status().running).await;
        assert!(hub.status().error.is_none());

        crate::remote::apply(&server, &crate::remote::RemoteConfig::default()).await;
    }
}

//! The LAN remote client (P16).
//!
//! The session runs where the audio interface is. The operator is regularly
//! somewhere else — a control room, a rehearsal stage, the other end of a
//! house — and this is what lets a second machine on the same network be that
//! place. The app serves the *same* browser build it renders in its own
//! WebView on one chosen interface, and drives it over one WebSocket.
//!
//! Three things decide the shape of everything here.
//!
//! **The browser is a peer, not an observer.** It receives the whole
//! operational view and it can do what the local app can — start and stop,
//! clear the chat, edit channels, groups, vocabulary, alert terms, profiles.
//! That is only safe from drift because every one of those goes through
//! [`crate::commands::dispatch`], which the Tauri bridge dispatches into as
//! well: there is one implementation of each mutation and the socket is a
//! second caller of it, not a second copy.
//!
//! **One optional key, and it travels in the link.** A remote that has to be
//! logged into is a remote nobody uses, so the whole of the ceremony is
//! scanning the QR code the Settings page already draws: the key rides in the
//! URL's *fragment*, the page trades it for a cookie at [`unlock`] and then
//! wipes it out of the address bar. The cookie lasts 400 days, so a phone is
//! asked once per address and never again. Leave [`RemoteConfig::key`] empty
//! and the listener is open, which is what it has always been.
//!
//! Everything that matters goes through the socket, so [`socket`] is the one
//! place that has to be gated — the bundle itself is served to anyone, because
//! it has to load before it can offer anything. Separately, and regardless of
//! the key, a page from somewhere else must not drive the session out of the
//! operator's own browser: see [`from_this_app`].
//!
//! **This is a lock, not a safe.** The transport is plain HTTP, so the key
//! crosses the network in the clear the first time it is offered. It keeps
//! out somebody on the venue's guest wifi who types the address; it does not
//! keep out somebody reading the wire. TLS would, but only behind a
//! self-signed certificate, and a browser warning that has to be clicked
//! through every time is how an operator learns to click through warnings.
//!
//! **A slow link must fall behind on meters and never on transcripts.** The
//! level meters are 20 Hz of numbers nobody reads a second later; the
//! transcripts are the product. So the per-client sender thins levels to
//! [`LEVELS_INTERVAL`] and lets everything else through untouched.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpListener as StdTcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::{broadcast, watch};

/// The port the picker starts on.
///
/// Above 1024 so it needs no privileges, and outside the range a development
/// machine is likely to have something on — 8080 and 3000 are where everything
/// else on an engineer's laptop already is.
pub const DEFAULT_PORT: u16 = 8787;

/// How many events a client may fall behind before it is cut loose.
///
/// Generous on purpose: a browser tab that has been backgrounded by a screen
/// lock catches up in one burst, and dropping it would be a reconnect the
/// operator sees as a blank chat. Levels are thinned before they reach this,
/// so what fills it is real traffic.
const BACKLOG: usize = 4096;

/// The shortest gap between two level events sent to a remote client.
///
/// The meters arrive at 20 Hz and are read as movement rather than as numbers;
/// five frames a second is movement. What this buys is the headroom the
/// transcripts need on a link that cannot carry both.
const LEVELS_INTERVAL: Duration = Duration::from_millis(200);

/// Kept in step with `EVENT_LEVELS` on the bridge. Named here because this is
/// the one event the sender treats differently from the rest.
const EVENT_LEVELS: &str = crate::EVENT_LEVELS;

/// How every frame [`Hub::publish`] writes begins. The sender looks at the
/// string rather than at a typed value, so the shape it looks for has to be
/// stated once next to the type that produces it.
const FRAME_PREFIX: &str = r#"{"type":"event","event":""#;

/// The cookie a browser that has offered the right key carries afterwards.
const COOKIE: &str = "i2c_remote";

/// How long that cookie lives, in seconds: 400 days.
///
/// The longest anything honours — Chrome silently truncates a longer one — and
/// therefore as close to "never expires" as a cookie gets. It is deliberately
/// not shorter: an operator who is asked for the key again on the morning of a
/// show is an operator who turns the key off. What ends a session in practice
/// is not time anyway. It is the machine's address changing, which makes it a
/// different origin, or the key being replaced.
const COOKIE_MAX_AGE: u64 = 400 * 24 * 60 * 60;

/// Mixed into the cookie's value, so that what sits in the browser's jar is
/// derived from the key rather than being the key.
///
/// No HMAC: the message is this constant and nothing else, so there is no
/// message for an attacker to vary, and anybody who can produce the digest
/// already knows the key. The version in it is so that a later change to how
/// the value is formed invalidates what is stored rather than accepting it.
const COOKIE_DOMAIN: &str = "fluestr-remote-v1:";

/// How many wrong keys one address may offer before it is made to wait.
///
/// The key is eight characters, which is only enough because guessing it is
/// rate-limited. Ten is generous for somebody typing and hopeless for
/// somebody enumerating.
const MAX_FAILURES: u32 = 10;

/// How long an address that ran out of attempts waits before it may try again.
const LOCKOUT: Duration = Duration::from_secs(60);

/// How long a quiet address is remembered for, so the table cannot grow
/// without bound on a network that keeps handing out new leases.
const FORGET: Duration = Duration::from_secs(60 * 60);

/// Whether the listener runs, and where.
///
/// Part of the configuration file (see [`crate::config::Config::remote`]) and
/// therefore saved, loaded and broadcast like everything else on the Settings
/// page. There is deliberately no separate "enable the remote" command: the
/// configuration has one writer, and a second mutation path is how the file
/// and the listener come to disagree about whether the app is serving.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "config.ts", rename = "StoredRemote")]
pub struct RemoteConfig {
    /// Off by default, and the operator turns it on for the evening. Even
    /// with a key in front of it, "not listening" is the safe state and has to
    /// be the one a fresh install is in: the key is a lock on a door, and the
    /// door is still better shut when nobody is expected through it.
    #[serde(default)]
    pub enabled: bool,
    /// The address of the interface to bind, as [`interfaces`] reports it.
    ///
    /// One address, never `0.0.0.0`: an app that serves every interface at
    /// once is an app that is on the venue's guest wifi without anyone having
    /// said so. Empty is "nothing picked yet", which cannot be enabled.
    #[serde(default)]
    pub address: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// What a browser has to offer before the socket is opened for it, or
    /// empty for a listener anyone on the chosen address may drive.
    ///
    /// Stored as it is typed rather than hashed, and that is not an oversight:
    /// it is a shared key an operator has to be able to read off this screen,
    /// put in a QR code and say out loud across a room. There is no account
    /// here to protect — the thing being protected is on the other side of it.
    ///
    /// `#[serde(default)]` — empty, so open — because a file written before
    /// this existed describes a listener that had no key, and reading its
    /// absence as "ask for one" would lock an operator out of their own setup
    /// at an update.
    #[serde(default)]
    pub key: String,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            address: String::new(),
            port: DEFAULT_PORT,
            key: String::new(),
        }
    }
}

/// What the Settings page shows about the listener.
///
/// Reported rather than inferred from the configuration, because the two can
/// legitimately disagree: a port that is already taken, or an address that
/// belongs to an interface which has since gone away, leaves the setting on
/// and the listener down. That is exactly the state that has to be visible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "remote.ts", rename = "RemoteStatus")]
pub struct Status {
    pub running: bool,
    /// What to type into the other machine's browser. `None` while nothing is
    /// being served.
    pub url: Option<String>,
    /// How many browsers are connected. The one number that answers "did it
    /// work" from the machine that cannot see the other screen.
    pub clients: usize,
    /// Why it is not running although it was asked to. `None` when there is
    /// nothing to say.
    pub error: Option<String>,
}

/// One address this machine can be reached on, for the picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "remote.ts")]
pub struct Interface {
    /// The operating system's name for it — `en0`, `en7`. Shown because a
    /// machine on two networks has two plausible addresses and this is what
    /// tells them apart.
    pub name: String,
    pub address: String,
    /// `127.0.0.1`. Offered rather than hidden: it is what an operator tries
    /// first to see whether the thing serves at all, and it reaches no other
    /// machine, which makes it the one address that is safe everywhere.
    pub loopback: bool,
}

/// Every address this machine can be reached on, loopback last.
///
/// IPv4 only. A link-local IPv6 address needs a scope suffix that no browser
/// address bar accepts, and offering an address that cannot be typed in is
/// worse than offering none.
pub fn interfaces() -> Vec<Interface> {
    let mut found: Vec<Interface> = if_addrs::get_if_addrs()
        .map_err(|err| {
            tracing::error!(error = %err, "the network interfaces could not be listed");
            err
        })
        .unwrap_or_default()
        .into_iter()
        .filter_map(|iface| match iface.addr.ip() {
            IpAddr::V4(ip) => Some(Interface {
                name: iface.name,
                address: ip.to_string(),
                loopback: ip.is_loopback(),
            }),
            IpAddr::V6(_) => None,
        })
        .collect();

    // Routable first: the address that reaches the other machine is the answer
    // to the question the picker is asking, and loopback is the footnote.
    found.sort_by(|a, b| a.loopback.cmp(&b.loopback).then(a.name.cmp(&b.name)));
    found
}

/// One frame on its way to every connected browser.
///
/// Serialised once, here, rather than per client: eight streams of levels at
/// 20 Hz is the one thing on this path that happens often enough for the
/// difference to matter.
#[derive(Debug, Serialize)]
struct EventFrame<'a, T> {
    #[serde(rename = "type")]
    kind: &'static str,
    event: &'a str,
    payload: T,
}

/// A running listener. Dropping it stops serving and closes what it served.
struct Listener {
    address: String,
    port: u16,
    /// The key the running server checks against.
    ///
    /// Changed *in place* rather than by rebinding. Rebinding the same address
    /// and port cannot work: dropping a listener only starts its shutdown, and
    /// the task still owns the socket while it winds down — so the new bind
    /// lands on a port that is still held and the operator is told their own
    /// port is already in use. Published here instead, and read per request.
    key: watch::Sender<Arc<str>>,
    /// Bumped to drop every connected browser without stopping the listener.
    ///
    /// Which is the other half of replacing a key: a check that only runs at
    /// the upgrade would leave whoever was already connected connected, and
    /// *New key* has to be the button that puts a phone left in a green room
    /// out. Separate from [`Listener::_stop`] because that one also ends the
    /// accept loop, and this must not.
    kick: watch::Sender<u64>,
    /// Resolves when the serving task has actually returned — and with it let
    /// go of the socket.
    ///
    /// The one thing that makes "down before up" true rather than intended.
    /// See [`released`].
    finished: tokio::sync::oneshot::Receiver<()>,
    /// Dropped with the listener. Every socket task and the accept loop watch
    /// it, and a closed channel is the stop signal — which is what makes
    /// disabling the remote drop the connections immediately rather than
    /// leaving them open until the far end notices.
    _stop: watch::Sender<()>,
}

/// The remote's half of the application state: what is broadcast, to whom, and
/// whether anything is listening at all.
///
/// Always present, even when the listener is off. An app that has to check
/// whether the remote exists before it can emit an event would have that check
/// at every emit site, and one of them would be forgotten.
pub struct Hub {
    events: broadcast::Sender<Arc<str>>,
    clients: Arc<AtomicUsize>,
    listener: Mutex<Option<Listener>>,
    /// Who has been offering wrong keys, and since when.
    ///
    /// On the hub rather than on the listener on purpose: switching the remote
    /// off and on again is one click, and a lockout that a restart cleared
    /// would be a lockout in name only.
    gate: Gate,
    /// Why the last attempt to start failed, kept so the Settings page can be
    /// told about it after the fact — the save that asked is long gone.
    error: Mutex<Option<String>>,
}

impl Default for Hub {
    fn default() -> Self {
        let (events, _) = broadcast::channel(BACKLOG);
        Self {
            events,
            clients: Arc::new(AtomicUsize::new(0)),
            listener: Mutex::new(None),
            gate: Gate::default(),
            error: Mutex::new(None),
        }
    }
}

impl Hub {
    /// Sends one event to every connected browser.
    ///
    /// Silent when nothing is connected, which is the normal case: a broadcast
    /// channel with no receivers drops the value, so the cost of the remote on
    /// a machine that never enables it is one `Arc` allocation per event.
    pub fn publish<T: Serialize>(&self, event: &str, payload: &T) {
        if self.events.receiver_count() == 0 {
            return;
        }

        let frame = EventFrame {
            kind: "event",
            event,
            payload,
        };
        match serde_json::to_string(&frame) {
            // `Arc<str>` because every client gets the same bytes and none of
            // them owns them.
            Ok(text) => {
                let _ = self.events.send(text.into());
            }
            Err(err) => tracing::error!(event, error = %err, "an event could not be serialised"),
        }
    }

    pub fn status(&self) -> Status {
        let listener = self.listener.lock().ok();
        let running = listener.as_ref().and_then(|l| l.as_ref());

        Status {
            running: running.is_some(),
            url: running.map(|l| url_for(&l.address, l.port, &l.key.borrow())),
            clients: self.clients.load(Ordering::Relaxed),
            error: self.error.lock().ok().and_then(|e| e.clone()),
        }
    }

    fn fail(&self, message: String) {
        if let Ok(mut error) = self.error.lock() {
            *error = Some(message);
        }
    }

    fn clear_error(&self) {
        if let Ok(mut error) = self.error.lock() {
            *error = None;
        }
    }
}

/// The URL to open on the other machine — with the key in it, when there is
/// one.
///
/// In the *fragment* rather than the query, and that is the whole reason this
/// is usable: a fragment is never sent to the server, so the key is in no
/// access log and no `Referer`, and the page can take it out of the address
/// bar the moment it has traded it for a cookie. A query string would stay in
/// the browser's history and in every screenshot of it afterwards.
fn url_for(address: &str, port: u16, key: &str) -> String {
    let key = key.trim();
    if key.is_empty() {
        format!("http://{address}:{port}/")
    } else {
        format!("http://{address}:{port}/#k={}", escaped(key))
    }
}

/// A key as it can appear in a fragment.
///
/// A generated key never needs this — it is eight characters of an alphabet
/// chosen to be safe everywhere. A key an operator typed can be anything at
/// all, and a `#` or a space in one would otherwise produce a URL that means
/// something else.
fn escaped(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for byte in key.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Brings the listener in line with the configuration, and says what happened.
///
/// Called on every `save_config` and once at startup, so the listener is a
/// consequence of the configuration rather than of a button — the same
/// arrangement as everything else the Settings page changes. A configuration
/// that asks for what is already running is a no-op, which matters because the
/// page saves through a debounce and typing a port would otherwise rebind
/// once per keystroke.
pub async fn apply<R: Runtime>(app: &AppHandle<R>, config: &RemoteConfig) -> Status {
    let Some(state) = app.try_state::<crate::AppState>() else {
        return Status {
            running: false,
            url: None,
            clients: 0,
            error: Some("the application state is not ready".into()),
        };
    };
    let hub = Arc::clone(&state.remote);
    let wanted = config.enabled && !config.address.trim().is_empty();

    // The listener that has to be waited for before anything is bound, taken
    // out under the lock and dealt with after it. After it on both counts: a
    // `std::sync::Mutex` guard is not `Send`, so holding one across the wait
    // would not compile — and it would be wrong even if it did, because what
    // is being waited for is a task, and waiting for it behind the lock the
    // next save needs is how a save comes to depend on a browser somewhere
    // finishing its close.
    let mut stopping: Option<Listener> = None;
    let mut rebind = false;

    // The lock is taken and released inside this block, and nothing inside it
    // asks the hub for its status: that would take the same lock again, on a
    // mutex that is not reentrant.
    let changed = {
        let mut slot = match hub.listener.lock() {
            Ok(slot) => slot,
            Err(err) => {
                tracing::error!(error = %err, "the listener state is unreadable");
                return hub.status();
            }
        };

        // Where the listener already stands is where it is wanted. Only the
        // key can still differ, and the key never rebinds.
        let in_place = matches!(
            slot.as_ref(),
            Some(running)
                if wanted && running.address == config.address && running.port == config.port
        );

        if in_place {
            let running = slot.as_ref().expect("in place means there is one");
            let next: Arc<str> = config.key.trim().into();
            if *running.key.borrow() == next {
                // The page saves through a debounce, so typing a port arrives
                // as four configurations. Announcing a status that has not
                // moved would be an event per keystroke to every view.
                false
            } else {
                // The running server reads the key per request, so this is the
                // whole of applying a new one — and the browsers that were let
                // in with the old one are put out, because their cookie is
                // derived from it and nothing else would re-check them.
                let _ = running.key.send(next);
                running.kick.send_modify(|epoch| *epoch += 1);
                tracing::info!(
                    "the remote's key was replaced and the browsers it was serving were dropped"
                );
                // Announced, because the status carries the URL and the URL
                // carries the key: a Settings page still showing the old one
                // is a QR code that no longer works.
                true
            }
        } else if slot.is_none() && !wanted {
            // Off and already off, which is every save on every machine that
            // never turns this on.
            false
        } else {
            stopping = slot.take();
            rebind = true;
            true
        }
    };

    if rebind {
        // Down before up, and *actually* down. Dropping a listener only asks
        // its serving task to stop; until that task has returned it is still
        // holding the socket, and binding in that window fails with "the port
        // is already in use" — this app complaining about itself.
        if let Some(old) = stopping {
            released(old).await;
            tracing::info!("the LAN remote client stopped serving");
        }
        bring_up(app, &hub, config, wanted);
    }

    let status = hub.status();
    // Only when something actually happened: the status carries the client
    // count, and announcing it on every keystroke of the Settings page would
    // be an event per keystroke to every window.
    if changed {
        announce(app, &status);
    }
    status
}

/// Binds the listener the configuration asks for, the socket being known to be
/// free by now. Does nothing but clear the last error when none is wanted.
fn bring_up<R: Runtime>(app: &AppHandle<R>, hub: &Arc<Hub>, config: &RemoteConfig, wanted: bool) {
    let mut slot = match hub.listener.lock() {
        Ok(slot) => slot,
        Err(err) => {
            tracing::error!(error = %err, "the listener state is unreadable");
            return;
        }
    };

    if !wanted {
        hub.clear_error();
        return;
    }

    match bind(app, hub, config) {
        Ok(listener) => {
            hub.clear_error();
            *slot = Some(listener);
        }
        Err(err) => {
            tracing::error!(
                address = %config.address,
                port = config.port,
                error = %err,
                "the LAN remote client could not be served"
            );
            hub.fail(err);
        }
    }
}

/// Tells every view what the listener is doing.
///
/// Its own event rather than part of the configuration change that caused it:
/// whether the listener actually came up is not something the configuration
/// knows, and the client count moves on its own afterwards.
pub fn announce<R: Runtime>(app: &AppHandle<R>, status: &Status) {
    crate::broadcast(app, crate::EVENT_REMOTE_STATUS, status.clone());
}

/// How long a listener that has been asked to stop is waited for.
///
/// It normally returns in well under a millisecond: the accept loop is
/// watching the channel that was just closed, and every client task sends a
/// close frame and breaks. The wait is for the case where one of them does
/// not — a browser on a link that has gone away — and a bounded wait that
/// occasionally gives up too early is much better than a save that hangs.
const SHUTDOWN_WAIT: Duration = Duration::from_millis(500);

/// Waits for a stopped listener to let go of its socket.
///
/// `async`, and that is the whole point of it. Blocking the thread here does
/// not work: the task being waited for runs on the same runtime, so a blocking
/// wait stops the very thing it is waiting for and then times out with the
/// port still bound. That was the bug — changing a setting reported "port is
/// already in use" about this app's own port.
async fn released(listener: Listener) {
    // `_stop` is named and dropped rather than left to `..`, and that is the
    // whole of it working. Partially moving out of a value leaves the rest to
    // be dropped when the *binding* goes out of scope — at the end of this
    // function — so with `..` alone the signal the task is waiting for would
    // be sent only after the wait for it had already timed out.
    let Listener {
        finished, _stop, ..
    } = listener;
    drop(_stop);

    // The sender lives in the serving task and goes with it, so what this
    // waits for is the channel closing rather than a value arriving.
    if tokio::time::timeout(SHUTDOWN_WAIT, finished).await.is_err() {
        // Said out loud, because what follows is a bind that may now fail on a
        // port this app is still holding — and that error on its own reads as
        // somebody else's program having taken the port.
        tracing::warn!(
            "the old listener did not let go of its socket within {}ms",
            SHUTDOWN_WAIT.as_millis()
        );
    }
}

/// Opens the socket and starts serving on it.
///
/// Binding happens here, synchronously, so that "the port is already in use"
/// is an answer to the save that asked rather than a line in a log file.
fn bind<R: Runtime>(
    app: &AppHandle<R>,
    hub: &Arc<Hub>,
    config: &RemoteConfig,
) -> Result<Listener, String> {
    let address = config.address.as_str();
    let port = config.port;
    let ip: IpAddr = address
        .trim()
        .parse()
        .map_err(|_| format!("{address} is not an address this machine can serve on"))?;
    let addr = SocketAddr::new(ip, port);

    let std_listener = StdTcpListener::bind(addr).map_err(|err| match err.kind() {
        std::io::ErrorKind::AddrInUse => {
            format!("port {port} is already in use — choose another one")
        }
        std::io::ErrorKind::AddrNotAvailable => {
            format!("{address} is not an address of this machine any more")
        }
        _ => err.to_string(),
    })?;
    std_listener
        .set_nonblocking(true)
        .map_err(|err| err.to_string())?;

    let (stop, _) = watch::channel(());
    let (key, _) = watch::channel::<Arc<str>>(config.key.trim().into());
    let (kick, _) = watch::channel(0u64);
    // A `oneshot` used for nothing but its drop: the sender goes with the
    // task, and the receiver resolving is how `released` learns the socket is
    // free.
    let (finished, wait_for_it) = tokio::sync::oneshot::channel::<()>();

    let server = Server {
        app: app.clone(),
        hub: Arc::clone(hub),
        key: key.subscribe(),
        kick: kick.subscribe(),
        stop: stop.subscribe(),
    };

    let mut shutdown = stop.subscribe();
    tauri::async_runtime::spawn(async move {
        // Moved in so it is dropped when this task returns, whichever way it
        // returns — including the early one below.
        let _finished = finished;
        let listener = match tokio::net::TcpListener::from_std(std_listener) {
            Ok(listener) => listener,
            Err(err) => {
                tracing::error!(error = %err, "the remote listener could not be adopted");
                return;
            }
        };

        let router = Router::new()
            .route("/ws", get(socket))
            // Before the fallback, which would otherwise answer it with "not
            // part of this build" — the asset route is a catch-all.
            .route("/auth", get(lock_state).post(unlock))
            .fallback(asset)
            .with_state(server);

        // With the peer's address, which is what the rate limit on `/auth`
        // counts wrong keys against.
        // Not `with_graceful_shutdown`. That waits for the connections it is
        // serving before it returns, and this listener's connections are
        // WebSockets an operator keeps open all evening — so the socket would
        // stay bound until the last browser had finished, which is exactly the
        // window in which rebinding reports "the port is already in use".
        // Dropping the serve future instead closes the listening socket at
        // once. The connections are tasks of their own and do not go with it,
        // and they are already watching the same channel — see `serve_client`,
        // which sends each browser a close frame and stops.
        tokio::select! {
            served = axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            ) => {
                if let Err(err) = served {
                    tracing::error!(error = %err, "the remote listener stopped");
                }
            }
            // The sender is dropped with the `Listener`, and a closed channel
            // is how that reaches here.
            _ = shutdown.changed() => {}
        }
    });

    tracing::info!(
        // Without the key. The log is the one copy of this that gets pasted
        // into a bug report.
        url = %url_for(address, port, ""),
        protected = !config.key.trim().is_empty(),
        "the LAN remote client is being served"
    );
    Ok(Listener {
        address: address.to_string(),
        port,
        key,
        kick,
        finished: wait_for_it,
        _stop: stop,
    })
}

/// What the routes are given.
struct Server<R: Runtime> {
    app: AppHandle<R>,
    hub: Arc<Hub>,
    /// The key as it stands now, already trimmed. Empty is an open listener,
    /// and every check below is written so that empty passes.
    ///
    /// A `watch` rather than a value, because replacing a key must not rebind
    /// the socket — see [`Listener::key`] — so the running server has to be
    /// able to see a new one.
    key: watch::Receiver<Arc<str>>,
    /// Watched by every client task: a change on it is the key having been
    /// replaced, and every browser let in with the old one goes.
    kick: watch::Receiver<u64>,
    /// Watched by every socket task. When the listener is dropped this closes,
    /// and the sockets close with it — which is what makes switching the
    /// remote off drop the connections it was serving rather than leaving them
    /// hanging on a server that is no longer accepting.
    stop: watch::Receiver<()>,
}

/// Hand-written rather than derived: `derive(Clone)` would ask the runtime
/// parameter to be `Clone` too, which it is not and does not need to be —
/// everything actually held here clones on its own.
impl<R: Runtime> Clone for Server<R> {
    fn clone(&self) -> Self {
        Self {
            app: self.app.clone(),
            hub: Arc::clone(&self.hub),
            key: self.key.clone(),
            kick: self.kick.clone(),
            stop: self.stop.clone(),
        }
    }
}

// --- Serving the page -----------------------------------------------------

/// The frontend: the very same one the app's own window is running.
///
/// **The dev server first, whenever there is one.** `tauri dev` serves the
/// app's window from Vite, and the asset resolver goes on answering from
/// whatever happens to be left in the build directory — so preferring the
/// resolver means the operator's window and the remote are two different
/// builds of this app driving one session. That is worse than not serving at
/// all, because it looks right: every fix lands in the window and none of them
/// reaches the browser, and the browser is where it is being tested. A
/// released app has no dev URL, so this costs it nothing.
///
/// Otherwise the bundle Tauri embedded, rather than a directory next to the
/// executable: there is one browser build, it is compiled into the app, and a
/// second copy on disk is a copy that can be a version behind.
async fn asset<R: Runtime>(State(server): State<Server<R>>, uri: Uri) -> Response {
    let path = uri.path();

    if let Some(dev) = server.app.config().build.dev_url.as_ref() {
        return proxy(dev.as_str(), dev_target(&uri)).await;
    }

    // A browser asked for `/?group=2` wants the page, and the frontend serves
    // one document for every route it has.
    let wanted = if path == "/" { "/index.html" } else { path };

    match server.app.asset_resolver().get(wanted.to_string()) {
        Some(found) => {
            let mime = HeaderValue::from_str(&found.mime_type)
                .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
            ([(header::CONTENT_TYPE, mime)], found.bytes).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not part of this build").into_response(),
    }
}

/// What a request looks like on the way to the dev server: the whole URI,
/// query string and all.
///
/// The query is not decoration there. Vite addresses a component's stylesheet
/// by it — `App.svelte?svelte&type=style&lang.css` — and forwarding only the
/// path answers that with the component's *JavaScript* instead. Nothing fails
/// loudly when it does: every module resolves, the page renders, and it comes
/// up with none of its component styles.
fn dev_target(uri: &Uri) -> &str {
    uri.path_and_query()
        .map_or(uri.path(), |whole| whole.as_str())
}

/// Passes a request through to the Vite dev server (development only).
///
/// Only ever reached in a checkout, because only a checkout has a dev URL —
/// and there it answers *everything*, the asset resolver included, for the
/// reason [`asset`] gives: the window and the remote have to be one build.
async fn proxy(dev: &str, path: &str) -> Response {
    let target = format!("{}{}", dev.trim_end_matches('/'), path);

    match reqwest::get(&target).await {
        Ok(response) => {
            let mime = response
                .headers()
                .get(header::CONTENT_TYPE)
                .cloned()
                .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream"));
            let status =
                StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            match response.bytes().await {
                Ok(body) => (status, [(header::CONTENT_TYPE, mime)], body).into_response(),
                Err(err) => (StatusCode::BAD_GATEWAY, err.to_string()).into_response(),
            }
        }
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            format!("the development server at {dev} did not answer: {err}"),
        )
            .into_response(),
    }
}

// --- The key --------------------------------------------------------------

/// What the browser is given once it has offered the right key.
///
/// Not the key itself: a cookie is readable by anything that can read the
/// browser's jar, and there is no reason for the key to be in two places when
/// one of them only ever has to be compared.
fn cookie_for(key: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(COOKIE_DOMAIN.as_bytes());
    hash.update(key.as_bytes());
    hex::encode(hash.finalize())
}

/// Whether two strings are the same, without telling the caller *where* they
/// first differ by how long the answer took.
///
/// The keys here are short and rate-limited, so a timing attack is not the
/// realistic way in. It costs one line to not have to reason about that.
fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |seen, (x, y)| seen | (x ^ y)) == 0
}

/// One cookie out of the header, by name.
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(found, _)| found.trim() == name)
        .map(|(_, value)| value.trim().to_string())
}

/// Whether this request may be served the session.
///
/// An empty key is an open listener, which is what every installation before
/// this one was and what a house network the operator controls may reasonably
/// stay.
fn unlocked(headers: &HeaderMap, key: &str) -> bool {
    key.is_empty() || cookie(headers, COOKIE).is_some_and(|held| same(&held, &cookie_for(key)))
}

/// Who has been guessing, and for how long they are made to stop.
#[derive(Default)]
struct Gate {
    attempts: Mutex<HashMap<IpAddr, Attempt>>,
}

struct Attempt {
    failures: u32,
    /// When the last wrong key came from this address. Both halves of the
    /// decision hang off it: whether the lockout has run out, and whether this
    /// address is still worth remembering.
    last: Instant,
}

impl Gate {
    /// Whether this address may offer a key at all.
    ///
    /// A poisoned lock answers yes. This is a speed bump in front of an
    /// optional key on a local network, and failing it closed would mean one
    /// panicked thread locks the operator out of their own show.
    fn open(&self, who: IpAddr) -> bool {
        let Ok(mut attempts) = self.attempts.lock() else {
            return true;
        };
        attempts.retain(|_, seen| seen.last.elapsed() < FORGET);
        !attempts
            .get(&who)
            .is_some_and(|seen| seen.failures >= MAX_FAILURES && seen.last.elapsed() < LOCKOUT)
    }

    fn failed(&self, who: IpAddr) {
        let Ok(mut attempts) = self.attempts.lock() else {
            return;
        };
        let seen = attempts.entry(who).or_insert(Attempt {
            failures: 0,
            last: Instant::now(),
        });
        // A lockout that has run out starts the count again rather than
        // leaving the address one wrong key away from another minute of it
        // forever.
        seen.failures = if seen.failures >= MAX_FAILURES && seen.last.elapsed() >= LOCKOUT {
            1
        } else {
            seen.failures + 1
        };
        seen.last = Instant::now();
    }

    fn passed(&self, who: IpAddr) {
        if let Ok(mut attempts) = self.attempts.lock() {
            attempts.remove(&who);
        }
    }
}

/// Whether this browser needs a key, and whether it already has one.
///
/// Asked by the page before it opens the socket, because a refused upgrade is
/// not something a browser will say anything useful about: `onclose` gives
/// 1006 and no status, so "wrong key" and "network gone" are the same event on
/// that path. One small request ahead of it is what tells them apart — and it
/// is asked again before every reconnect, so a key replaced mid-show turns
/// into the key prompt rather than into a page that retries forever.
async fn lock_state<R: Runtime>(State(server): State<Server<R>>, headers: HeaderMap) -> Response {
    let key = server.key.borrow().clone();
    let body = serde_json::json!({
        "required": !key.is_empty(),
        "unlocked": unlocked(&headers, &key),
    });
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        body.to_string(),
    )
        .into_response()
}

/// Trades a key for the cookie.
///
/// The body is the key and nothing else. There is no form and no JSON here on
/// purpose: this listener serves one page and one socket, and a parser is a
/// thing that can be fed something interesting by whoever can reach the port.
async fn unlock<R: Runtime>(
    State(server): State<Server<R>>,
    ConnectInfo(who): ConnectInfo<SocketAddr>,
    offered: String,
) -> Response {
    let key = server.key.borrow().clone();
    if key.is_empty() {
        // Nothing to offer. Answered as success rather than as an error: the
        // page asks in the one flow it has, and "you are in" is true.
        return StatusCode::NO_CONTENT.into_response();
    }

    if !server.hub.gate.open(who.ip()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "too many wrong keys from this machine — wait a minute",
        )
            .into_response();
    }

    if !same(offered.trim(), &key) {
        server.hub.gate.failed(who.ip());
        tracing::warn!(from = %who.ip(), "a wrong key was offered to the remote");
        return (StatusCode::FORBIDDEN, "that is not the key").into_response();
    }

    server.hub.gate.passed(who.ip());
    tracing::info!(from = %who.ip(), "a browser was let in to the remote");
    (
        StatusCode::NO_CONTENT,
        [(
            header::SET_COOKIE,
            // No `Secure`: this is http and a `Secure` cookie on an http page
            // is a cookie the browser throws away. `SameSite=Strict` is what
            // is actually load-bearing — with `from_this_app` it is the second
            // half of keeping somebody else's page off this socket.
            format!(
                "{COOKIE}={}; Path=/; Max-Age={COOKIE_MAX_AGE}; HttpOnly; SameSite=Strict",
                cookie_for(&key)
            ),
        )],
    )
        .into_response()
}

// --- The socket -----------------------------------------------------------

/// What a browser sends. One shape: a call, with the id it wants the answer
/// tagged with.
#[derive(Debug, Deserialize)]
struct Call {
    id: u64,
    #[serde(flatten)]
    command: crate::commands::Command,
}

/// The answer to one call. `ok` rather than an error field alone, so a
/// command that legitimately answers `null` is not read as a failure.
#[derive(Debug, Serialize)]
struct Answer {
    #[serde(rename = "type")]
    kind: &'static str,
    id: u64,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

async fn socket<R: Runtime>(
    upgrade: WebSocketUpgrade,
    State(server): State<Server<R>>,
    headers: HeaderMap,
) -> Response {
    if !from_this_app(&headers) {
        tracing::warn!(
            origin = ?headers.get(header::ORIGIN),
            "a page from somewhere else tried to open the remote socket"
        );
        return (
            StatusCode::FORBIDDEN,
            "this socket is only served to the page this app serves",
        )
            .into_response();
    }
    let key = server.key.borrow().clone();
    if !unlocked(&headers, &key) {
        tracing::warn!("the remote socket was asked for without the key");
        return (
            StatusCode::UNAUTHORIZED,
            "this session is served with a key — open the address from the QR code",
        )
            .into_response();
    }

    upgrade.on_upgrade(move |socket| serve_client(socket, server))
}

/// Whether the upgrade was asked for by the page this app itself served.
///
/// Not authentication — that is still to come, and this is no substitute for
/// it. It closes a different hole, and one the "anyone who reaches the chosen
/// interface" decision never covered: a WebSocket upgrade is not subject to
/// the same-origin policy, so without this *any* web page anybody on this
/// network opens — or any page at all, by resolving its own name to this
/// machine's address — could open this socket and start, stop and reconfigure
/// the session from the operator's own browser. The operator never reached
/// this app for that to happen.
///
/// The rule: a browser states the page's origin, and it has to be the address
/// this app is being served on. No origin at all is not a browser — `curl`, a
/// script, the tests — and that is left to the LAN-trust decision, because a
/// caller that can forge the header can forge anything else too.
fn from_this_app(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Some(asked) = origin
        .to_str()
        .ok()
        .and_then(|origin| origin.parse::<Uri>().ok())
    else {
        return false;
    };
    let Some(asked) = asked.authority() else {
        return false;
    };

    headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(|host| asked.as_str().eq_ignore_ascii_case(host))
}

/// One connected browser, for as long as it stays connected.
///
/// Calls and events share the socket and are handled in the same task: a call
/// is a round trip on a LAN and the events in the meantime are a handful of
/// bytes, so splitting the socket would buy ordering problems and no
/// throughput. What a slow client costs is a thinner stream of levels, not a
/// slower session — the pipeline never waits on this.
async fn serve_client<R: Runtime>(mut socket: WebSocket, server: Server<R>) {
    // The key was checked at the upgrade and has no further part to play: a
    // socket is either open or it is not.
    let Server {
        app,
        hub,
        mut stop,
        mut kick,
        ..
    } = server;
    // Marked seen at the start, so that what `changed` reports afterwards is a
    // key replaced while *this* browser was connected.
    kick.mark_unchanged();

    // Subscribed *before* the first frame is sent, so the events a client
    // would otherwise miss between connecting and asking for its snapshot are
    // already queued behind it. The snapshot the frontend then fetches carries
    // the overlap, and `mergeHistory` on the other side drops the duplicates.
    let mut events = hub.events.subscribe();
    let clients = Arc::clone(&hub.clients);
    let count = clients.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::info!(clients = count, "a remote client connected");
    announce(&app, &hub.status());

    // Thinned per client rather than centrally: two browsers on two links do
    // not have to agree about how far behind they are.
    let mut last_levels: Option<Instant> = None;

    loop {
        tokio::select! {
            // The listener was switched off. Said out loud rather than just
            // dropped, so the page shows "disconnected" instead of spending
            // its reconnect backoff working it out.
            _ = stop.changed() => {
                let _ = socket.send(Message::Close(None)).await;
                break;
            }

            // The key was replaced. This browser was let in with the old one
            // and has to offer the new one — which its page does by asking
            // `/auth` before it reconnects, and being sent to the key screen.
            _ = kick.changed() => {
                tracing::info!("a remote client was dropped because the key was replaced");
                let _ = socket.send(Message::Close(None)).await;
                break;
            }

            event = events.recv() => match event {
                Ok(frame) => {
                    if is_levels(&frame) {
                        let now = Instant::now();
                        if last_levels.is_some_and(|last| now.duration_since(last) < LEVELS_INTERVAL) {
                            continue;
                        }
                        last_levels = Some(now);
                    }
                    if socket.send(Message::Text(frame.as_ref().into())).await.is_err() {
                        break;
                    }
                }
                // Too far behind to catch up. Kept rather than dropped: the
                // frontend re-reads the snapshot on reconnect, and a client
                // that is one burst behind is better served by the next event
                // than by a blank page.
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "a remote client fell behind");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },

            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if let Some(answer) = answer(&app, &text).await {
                        if socket.send(Message::Text(answer.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Some(Ok(Message::Ping(payload))) => {
                    if socket.send(Message::Pong(payload)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(err)) => {
                    tracing::info!(error = %err, "a remote client dropped");
                    break;
                }
            },
        }
    }

    let count = clients.fetch_sub(1, Ordering::Relaxed).saturating_sub(1);
    tracing::info!(clients = count, "a remote client disconnected");
    announce(&app, &hub.status());
}

/// Whether a serialised frame is a level event.
///
/// On the string rather than on a typed value because the frames are
/// serialised once for every client and this is the one place that has to look
/// inside one. The prefix is fixed by [`EventFrame`]'s field order, so the
/// check is a comparison and not a parse.
fn is_levels(frame: &str) -> bool {
    frame
        .strip_prefix(FRAME_PREFIX)
        .and_then(|rest| rest.strip_prefix(EVENT_LEVELS))
        // The closing quote, so that an event whose name merely starts with
        // "levels" is not thinned along with it.
        .is_some_and(|rest| rest.starts_with('"'))
}

/// Runs one call and serialises its answer.
///
/// A frame that is not a call at all is answered with nothing — there is no id
/// to tag a complaint with, and a browser that sends rubbish is a browser with
/// a bug rather than an operator to inform.
async fn answer<R: Runtime>(app: &AppHandle<R>, text: &str) -> Option<String> {
    let call: Call = match serde_json::from_str(text) {
        Ok(call) => call,
        Err(err) => {
            tracing::warn!(error = %err, "a remote client sent something that is not a call");
            return None;
        }
    };

    let id = call.id;
    let answer = match crate::commands::dispatch(app, call.command).await {
        Ok(value) => Answer {
            kind: "result",
            id,
            ok: true,
            value: Some(value),
            error: None,
        },
        Err(message) => Answer {
            kind: "result",
            id,
            ok: false,
            value: None,
            error: Some(message),
        },
    };

    serde_json::to_string(&answer)
        .map_err(|err| tracing::error!(error = %err, "an answer could not be serialised"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sender thins levels by looking at the serialised frame, which only
    /// works while the frame starts the way [`EventFrame`] writes it.
    #[test]
    fn a_levels_frame_is_recognised_by_its_prefix() {
        let frame = serde_json::to_string(&EventFrame {
            kind: "event",
            event: EVENT_LEVELS,
            payload: serde_json::json!({ "streams": [] }),
        })
        .expect("serialising");

        assert!(is_levels(&frame));
    }

    /// Nothing else may be thinned — a transcript that mentions the word
    /// "levels" is still a transcript.
    #[test]
    fn a_transcript_is_never_thinned() {
        let frame = serde_json::to_string(&EventFrame {
            kind: "event",
            event: "transcript",
            payload: serde_json::json!({ "text": "levels" }),
        })
        .expect("serialising");

        assert!(!is_levels(&frame));
    }

    /// Off by default, on no address. A file written before P16 has no remote
    /// block at all, and reading its absence as "serve the network" is the one
    /// way this feature could surprise somebody.
    #[test]
    fn the_listener_is_off_until_it_is_asked_for() {
        let config: RemoteConfig =
            serde_json::from_str("{}").expect("an empty object is a default remote");
        assert!(!config.enabled);
        assert!(config.address.is_empty());
        assert_eq!(config.port, DEFAULT_PORT);
    }

    /// The query has to survive the trip to the dev server, and a plain path
    /// has to come through untouched.
    #[test]
    fn a_request_reaches_the_dev_server_whole() {
        let styles: Uri = "/src/App.svelte?svelte&type=style&lang.css"
            .parse()
            .expect("a uri");
        assert_eq!(
            dev_target(&styles),
            "/src/App.svelte?svelte&type=style&lang.css"
        );

        let plain: Uri = "/src/main.ts".parse().expect("a uri");
        assert_eq!(dev_target(&plain), "/src/main.ts");

        // A group opened in a tab of its own: the page is the same document,
        // and the query is what says which group it shows.
        let group: Uri = "/?group=2".parse().expect("a uri");
        assert_eq!(dev_target(&group), "/?group=2");
    }

    /// The key has to be *in* the code the operator points a phone at —
    /// scanning and then typing a password is the workflow this exists to
    /// avoid. In the fragment, so it never reaches a log.
    #[test]
    fn the_address_carries_the_key_in_its_fragment() {
        assert_eq!(
            url_for("192.168.1.20", 8787, "k7p4m2rq"),
            "http://192.168.1.20:8787/#k=k7p4m2rq"
        );
        // No key, and the address is what it always was.
        assert_eq!(
            url_for("192.168.1.20", 8787, ""),
            "http://192.168.1.20:8787/"
        );
        // A key somebody typed. A `#` in it would otherwise end the fragment
        // and hand the page half a key.
        assert_eq!(
            url_for("10.0.0.2", 8787, "hi there#"),
            "http://10.0.0.2:8787/#k=hi%20there%23"
        );
    }

    /// An empty key is an open listener — which is what every installation
    /// before this one was, and what a file written before it says.
    #[test]
    fn no_key_lets_everyone_in() {
        assert!(unlocked(&HeaderMap::new(), ""));
    }

    /// And a key means the cookie, or nothing.
    #[test]
    fn a_key_is_the_cookie_or_nothing() {
        let key = "k7p4m2rq";
        assert!(!unlocked(&HeaderMap::new(), key), "no cookie got in");

        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("other=1; {COOKIE}={}; last=2", cookie_for(key))
                .parse()
                .expect("a cookie header"),
        );
        assert!(unlocked(&headers, key), "the right cookie was refused");

        // The key itself is not the cookie: what is stored is derived from it,
        // so a jar that is read gives up the value and not the key.
        assert_ne!(cookie_for(key), key);

        // And the cookie for one key does not open another.
        assert!(!unlocked(&headers, "something else"));
    }

    /// Ten wrong keys and the address waits. Without this an eight-character
    /// key is a few minutes' work.
    #[test]
    fn guessing_runs_out() {
        let gate = Gate::default();
        let who: IpAddr = "192.168.1.55".parse().expect("an address");

        for _ in 0..MAX_FAILURES {
            assert!(gate.open(who), "locked out too early");
            gate.failed(who);
        }
        assert!(!gate.open(who), "the guessing was never stopped");

        // And somebody else on the same network is not caught by it.
        let other: IpAddr = "192.168.1.56".parse().expect("an address");
        assert!(gate.open(other));

        // Getting in clears it, so an operator who mistyped four times and
        // then scanned the code is not one wrong key away from a lockout.
        gate.passed(who);
        assert!(gate.open(who));
    }

    #[test]
    fn loopback_is_offered_last() {
        let found = interfaces();
        let first_loopback = found.iter().position(|i| i.loopback);
        let last_routable = found.iter().rposition(|i| !i.loopback);
        if let (Some(first), Some(last)) = (first_loopback, last_routable) {
            assert!(last < first, "a routable address has to come first");
        }
    }
}

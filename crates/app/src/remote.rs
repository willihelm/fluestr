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
//! **There is no authentication yet.** Anyone who reaches the chosen interface
//! can control the session. That is why the listener is off by default, why it
//! binds the one address the operator picked rather than `0.0.0.0`, and why
//! the URL is shown with the address in it. The transport is arranged so a
//! token check is one layer in front of [`socket`] when it is wanted, not a
//! redesign. What is *not* left to that decision is a page from somewhere
//! else driving the session out of the operator's own browser — see
//! [`from_this_app`], which a socket has to pass before it is upgraded.
//!
//! **A slow link must fall behind on meters and never on transcripts.** The
//! level meters are 20 Hz of numbers nobody reads a second later; the
//! transcripts are the product. So the per-client sender thins levels to
//! [`LEVELS_INTERVAL`] and lets everything else through untouched.

use std::net::{IpAddr, SocketAddr, TcpListener as StdTcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
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
    /// Off by default, and the operator turns it on for the evening. With no
    /// authentication in front of it, "not listening" is the safe state and
    /// has to be the one a fresh install is in.
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
            url: running.map(|l| url_for(&l.address, l.port)),
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

/// The URL to type into the other machine's browser.
fn url_for(address: &str, port: u16) -> String {
    format!("http://{address}:{port}/")
}

/// Brings the listener in line with the configuration, and says what happened.
///
/// Called on every `save_config` and once at startup, so the listener is a
/// consequence of the configuration rather than of a button — the same
/// arrangement as everything else the Settings page changes. A configuration
/// that asks for what is already running is a no-op, which matters because the
/// page saves through a debounce and typing a port would otherwise rebind
/// once per keystroke.
pub fn apply<R: Runtime>(app: &AppHandle<R>, config: &RemoteConfig) -> Status {
    let Some(state) = app.try_state::<crate::AppState>() else {
        return Status {
            running: false,
            url: None,
            clients: 0,
            error: Some("the application state is not ready".into()),
        };
    };
    let hub = Arc::clone(&state.remote);

    // The listener lock is taken and released inside this block, and nothing
    // inside it asks the hub for its status: that would take the same lock
    // again, on a mutex that is not reentrant.
    let changed = {
        let mut slot = match hub.listener.lock() {
            Ok(slot) => slot,
            Err(err) => {
                tracing::error!(error = %err, "the listener state is unreadable");
                return hub.status();
            }
        };

        let wanted = config.enabled && !config.address.trim().is_empty();
        let unchanged = match slot.as_ref() {
            Some(running) => {
                wanted && running.address == config.address && running.port == config.port
            }
            // Off and already off, which is every save on every machine that
            // never turns this on.
            None => !wanted,
        };
        // The page saves through a debounce, so typing a port arrives as four
        // configurations. Rebinding on each of them would take the listener
        // down three times for setups nobody asked to serve — and announcing a
        // status that has not moved would be an event per keystroke to every
        // view.
        if unchanged {
            false
        } else {
            // Down before up, always: rebinding a port the old listener still
            // holds fails, and "the port is already in use" would then be this
            // app complaining about itself.
            if slot.take().is_some() {
                tracing::info!("the LAN remote client stopped serving");
            }

            if wanted {
                match bind(app, &hub, &config.address, config.port) {
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
            } else {
                hub.clear_error();
            }
            true
        }
    };

    let status = hub.status();
    // Only when something actually happened: the status carries the client
    // count, and announcing it on every keystroke of the Settings page would
    // be an event per keystroke to every window.
    if changed {
        announce(app, &status);
    }
    status
}

/// Tells every view what the listener is doing.
///
/// Its own event rather than part of the configuration change that caused it:
/// whether the listener actually came up is not something the configuration
/// knows, and the client count moves on its own afterwards.
pub fn announce<R: Runtime>(app: &AppHandle<R>, status: &Status) {
    crate::broadcast(app, crate::EVENT_REMOTE_STATUS, status.clone());
}

/// Opens the socket and starts serving on it.
///
/// Binding happens here, synchronously, so that "the port is already in use"
/// is an answer to the save that asked rather than a line in a log file.
fn bind<R: Runtime>(
    app: &AppHandle<R>,
    hub: &Arc<Hub>,
    address: &str,
    port: u16,
) -> Result<Listener, String> {
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
    let server = Server {
        app: app.clone(),
        hub: Arc::clone(hub),
        stop: stop.subscribe(),
    };

    let mut shutdown = stop.subscribe();
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(std_listener) {
            Ok(listener) => listener,
            Err(err) => {
                tracing::error!(error = %err, "the remote listener could not be adopted");
                return;
            }
        };

        let router = Router::new()
            .route("/ws", get(socket))
            .fallback(asset)
            .with_state(server);

        let served = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                // The sender is dropped with the `Listener`, and a closed
                // channel is how that reaches here.
                let _ = shutdown.changed().await;
            })
            .await;
        if let Err(err) = served {
            tracing::error!(error = %err, "the remote listener stopped");
        }
    });

    tracing::info!(url = %url_for(address, port), "the LAN remote client is being served");
    Ok(Listener {
        address: address.to_string(),
        port,
        _stop: stop,
    })
}

/// What the routes are given.
struct Server<R: Runtime> {
    app: AppHandle<R>,
    hub: Arc<Hub>,
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
    let Server { app, hub, mut stop } = server;

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

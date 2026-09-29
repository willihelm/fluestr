//! Tauri backend.
//!
//! Holds the capture session and the worker pool, and forwards their output to
//! the UI as events, and owns the configuration file the Settings page is
//! saved to (P12), and the model manager's downloads (P11). What is here
//! carries every configured channel of one multichannel device through the
//! pipeline into the chat.
//!
//! Group and channel editing is not a command surface of its own: the
//! configuration is one file with one writer, moved whole by `load_config` and
//! `save_config` (P12). The one thing that is a command is changing a channel
//! of a session that is *already running* — `update_stream_config`, which the
//! file cannot express because the session is not the file.

pub mod activity;
pub mod alerts;
// Generation only, so it exists only where generation runs: the frontend's
// types are written by `cargo test` (P6).
#[cfg(test)]
mod bindings;
pub mod commands;
pub mod config;
// The IOKit reader on macOS; elsewhere a reader that never finds a GPU, which
// the resource meter already answers with "memory only" (P18).
#[cfg_attr(not(target_os = "macos"), path = "gpu_none.rs")]
pub mod gpu;
pub mod history;
pub mod models;
pub mod monitor;
pub mod outputs;
pub mod pool;
pub mod prompt;
pub mod remote;
pub mod replay;
pub mod resources;
pub mod rules;
pub mod session;
pub mod webhook;
pub mod worker;

use history::History;
use monitor::Monitor;
use remote::Hub;
use rules::Rules;
use serde::Serialize;
use session::{
    EventSink, LevelsEvent, OverloadEvent, Session, SessionConfig, SessionState, TranscriptEvent,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Event names on the bridge. Constants because the UI subscribes by string
/// and a typo there fails silently — nothing arrives, nothing complains.
pub const EVENT_TRANSCRIPT: &str = "transcript";
/// Plural: one event carries every stream's meter, so the cost of the meters
/// does not grow with the channel count.
pub const EVENT_LEVELS: &str = "levels";
pub const EVENT_OVERLOAD: &str = "overload";
pub const EVENT_SESSION_STATE: &str = "session-state";
/// How far a model download has got (P11).
pub const EVENT_MODEL_PROGRESS: &str = "model-progress";
/// What the app and its workers are costing the machine, once a second (P8).
pub const EVENT_RESOURCES: &str = "resources";
/// The chat was emptied. Carries no payload: every window empties its own
/// copy, and the one thing they need to agree on is *when*.
pub const EVENT_CLEARED: &str = "communication-cleared";
/// The configuration was written. Sent to every window, and read by the
/// detached group windows (P7): a group renamed, refilled or given its flash
/// setting on the Settings page has to reach the window that is showing it,
/// and that window has no other way to hear about it.
pub const EVENT_CONFIG: &str = "config-changed";
/// Whether the LAN remote client is being served, and to how many browsers
/// (P16). Its own event rather than part of the configuration change that
/// caused it: whether the listener actually came up is not something the
/// configuration knows, and the client count moves on its own afterwards.
pub const EVENT_REMOTE_STATUS: &str = "remote-status";
/// How the outbound queue is doing (P19): what has gone out, and what has been
/// given up on. Sent on every outcome rather than sampled, because the first
/// drop is the interesting one — a dead endpoint is otherwise the one failure
/// this app can have that nobody notices until a cue does not happen.
pub const EVENT_WEBHOOK_STATUS: &str = "webhook-status";
/// Older replay audio was dropped to make room (P22): the line through the
/// chat before which bubbles have lost their play button.
pub const EVENT_REPLAY_HORIZON: &str = "replay-horizon";
/// A channel started or stopped having something on its way to the chat —
/// somebody talking, or what they said being decoded (P23). Edge-triggered.
pub const EVENT_SPEECH_ACTIVITY: &str = "speech-activity";
/// What the shoutbox is doing, ten times a second while it runs (P26).
pub const EVENT_SHOUTBOX: &str = "shoutbox-status";

/// What a detached group window's label starts with, followed by the group's
/// position in the active profile.
///
/// The label is how the window knows which group it is: the frontend reads its
/// own label rather than a query string, so the answer is there before the
/// first paint and survives a reload of the webview. It is also what makes
/// opening the same group twice focus the window that is already open instead
/// of stacking a second one on it.
pub const GROUP_WINDOW_PREFIX: &str = "group-";

/// The label suffix of the all-channels window, which is the one detached view
/// that is not a group. Kept in step with `ALL_CHANNELS_VIEW` in
/// `ui/src/lib/windows.ts`.
///
/// A word and not a number, because every number is already a group's
/// position.
pub const GROUP_WINDOW_ALL: &str = "all";

/// Sends one event to every view there is.
///
/// Both transports, always: the Tauri bridge for the app's own windows, and
/// the hub for whatever browsers the LAN remote is serving (P16). One function
/// rather than two calls at each emit site, because the second of the two is
/// what gets forgotten — and a forgotten one is a remote that silently stops
/// receiving one kind of event while everything else keeps working.
pub(crate) fn broadcast<R: Runtime, S: Serialize + Clone>(
    app: &AppHandle<R>,
    event: &str,
    payload: S,
) {
    if let Some(state) = app.try_state::<AppState>() {
        state.remote.publish(event, &payload);
    }
    let _ = app.emit(event, payload);
}

/// [`broadcast`], for the few events a remote browser is shown less of.
///
/// The configuration and the listener's status carry things only the machine
/// may see — the key, the webhook destinations and their headers — so the
/// app's own windows get `local` and the hub gets `remote`. Still one call,
/// for [`broadcast`]'s reason.
pub(crate) fn broadcast_split<R: Runtime, L: Serialize + Clone, T: Serialize>(
    app: &AppHandle<R>,
    event: &str,
    local: L,
    remote: &T,
) {
    if let Some(state) = app.try_state::<AppState>() {
        state.remote.publish(event, remote);
    }
    let _ = app.emit(event, local);
}

/// Sends session output to every view.
///
/// The rules ride along (P19): this is the one place every view's copy of a
/// transcript comes from, so it is where they are evaluated. A detached group
/// window and the LAN remote (P16) then highlight the same characters as the
/// main chat, and are asked for the same reactions, because they are given the
/// same answer — not because three evaluations happen to agree. It is also the
/// only place that can fire an action exactly *once* per event, which is what
/// P19's outbound actions will need and what no window can promise.
struct TauriEvents<R: Runtime> {
    app: AppHandle<R>,
    rules: Arc<Mutex<Rules>>,
    /// Where a rule's outbound action goes (P19). Not an event: it leaves the
    /// building rather than reaching a view, and it must leave exactly once —
    /// which is the whole reason the rules are evaluated here and not in each
    /// window.
    webhooks: Arc<webhook::Sender>,
    /// What a view opened later is caught up from — see [`history`]. Written
    /// here because this is the one place every view's copy comes from, so it
    /// is also the only place that can record what they were all sent.
    history: Arc<Mutex<History>>,
}

impl<R: Runtime> EventSink for TauriEvents<R> {
    fn transcript(&self, mut event: TranscriptEvent) {
        // A poisoned lock here would cost every later transcript its rules,
        // and silently. The text still has to reach the chat — a rule that
        // does not fire is a worse chat, an event that is dropped is no chat
        // at all.
        match self.rules.lock() {
            Ok(mut rules) if !rules.is_empty() => {
                let outcome = rules.evaluate(
                    rules::Segment {
                        stream: event.stream_id,
                        label: &event.label,
                        text: &event.text,
                    },
                    Instant::now(),
                );
                event.alerts = outcome.alerts;
                event.actions = outcome.actions;
                event.flash_views = outcome.flash_views;
                // Queued, never awaited: an unreachable host is the normal
                // case in a venue with no route out, and a request made here
                // would stall every transcript for the length of its timeout.
                for request in outcome.webhooks {
                    self.webhooks.send(request);
                }
            }
            Ok(_) => {}
            Err(err) => tracing::error!(error = %err, "the rule list is unreadable"),
        }
        // Recorded with what the rules made of it, after the evaluation and
        // before the emit: a group window opened in ten minutes has to
        // highlight the same characters as the window that is open now.
        match self.history.lock() {
            Ok(mut history) => history.push(event.clone()),
            Err(err) => tracing::error!(error = %err, "the chat history is unreadable"),
        }
        broadcast(&self.app, EVENT_TRANSCRIPT, event);
    }

    fn levels(&self, event: LevelsEvent) {
        broadcast(&self.app, EVENT_LEVELS, event);
    }

    fn overload(&self, event: OverloadEvent) {
        broadcast(&self.app, EVENT_OVERLOAD, event);
    }

    fn state(&self, event: SessionState) {
        match self.history.lock() {
            Ok(mut history) => history.note_state(&event),
            Err(err) => tracing::error!(error = %err, "the chat history is unreadable"),
        }
        broadcast(&self.app, EVENT_SESSION_STATE, event);
    }

    fn horizon(&self, horizon: replay::Horizon) {
        broadcast(&self.app, EVENT_REPLAY_HORIZON, horizon);
    }

    fn activity(&self, event: activity::SpeechActivity) {
        // Recorded for the view opened while somebody is talking, and for the
        // remote that reconnects: a placeholder is only as good as its end,
        // and a window that missed the end must not keep it.
        match self.history.lock() {
            Ok(mut history) => history.note_activity(&event),
            Err(err) => tracing::error!(error = %err, "the chat history is unreadable"),
        }
        broadcast(&self.app, EVENT_SPEECH_ACTIVITY, event);
    }
}

/// Brings the outputs into line with the active profile of `config` (P22,
/// P26) — at startup and on every save, like the LAN remote.
pub(crate) fn apply_outputs<R: Runtime>(app: &AppHandle<R>, config: &config::Config) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let outputs = config
        .profiles
        .get(config.active)
        .map(|profile| profile.outputs.clone())
        .unwrap_or_default();
    let handle = app.clone();
    let emit: outputs::Emit = Arc::new(move |status| broadcast(&handle, EVENT_SHOUTBOX, status));
    if let Some(horizon) = state.outputs.apply(&outputs, &state.replay, emit) {
        broadcast(app, EVENT_REPLAY_HORIZON, horizon);
    }
}

/// Everything the commands work on.
///
/// `pub` because [`commands`] reaches it through the `AppHandle` rather than
/// through a Tauri command argument: the LAN remote's socket has a handle and
/// no command context, and one way of reaching this is better than two.
#[derive(Default)]
pub struct AppState {
    pub(crate) session: Mutex<Option<Session>>,
    /// The device opened only to be measured, when no session is running
    /// (P8).
    ///
    /// Beside the session rather than inside it because it is what fills the
    /// gap *around* one: an operator setting a channel's trim before a show
    /// has no session and needs the meters most. One device is opened by one
    /// thing at a time — [`start_session`] stops this first, and the UI starts
    /// it again once the session has ended.
    pub(crate) monitor: Mutex<Option<Monitor>>,
    /// Where the configuration file lives, resolved once at setup.
    ///
    /// Resolved there rather than per command because it depends on the app's
    /// identifier and on the platform, and a command that cannot answer where
    /// its own configuration is has nothing useful to do. `None` means the
    /// resolver refused — see [`run`]: the app still starts, and saving says
    /// why it cannot.
    config_path: Mutex<Option<PathBuf>>,
    /// Where downloaded models go. Resolved once at setup, like the
    /// configuration path and for the same reason. `None` means the platform
    /// would not say where the app may keep data, which makes downloading
    /// impossible and has to be said out loud rather than failing per click.
    models_dir: Mutex<Option<PathBuf>>,
    /// The cancel flag of each download in flight, by model id.
    ///
    /// Keyed by id rather than being a single flag because the speech model
    /// and the VAD are two different downloads an operator may reasonably
    /// start together — and cancelling one of them must not stop the other.
    pub(crate) downloads: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// The compiled rules (P19).
    ///
    /// Compiled once when they change rather than per transcript, and shared
    /// with every session's event sink through the `Arc` — so a rule written
    /// mid-show takes effect on the next segment without restarting anything.
    /// Behind a `Mutex` rather than an `RwLock` because evaluating one is a
    /// write: a rate-limited rule remembers when it last fired.
    pub(crate) rules: Arc<Mutex<Rules>>,
    /// The outbound queue a rule's webhook action goes onto (P19).
    ///
    /// Shared with every session's event sink through the `Arc`, like the
    /// rules themselves, and for the same reason: it outlives any one session,
    /// and its counters are what makes a dead endpoint visible across all of
    /// them. Drained by one task started in [`run`].
    pub(crate) webhooks: Arc<webhook::Sender>,
    /// The transcripts already emitted, and the session state around them.
    ///
    /// Shared with every session's event sink through the `Arc`, like the
    /// alert list, and for the same reason: it has to outlive any one session,
    /// because Start is non-destructive and the chat spans several of them.
    /// It is also what a reconnecting remote client is caught up from (P16).
    pub(crate) history: Arc<Mutex<History>>,
    /// The sidecar's language list, once it has been asked for.
    ///
    /// Cached because it is a table compiled into whisper.cpp — it cannot
    /// change while the app runs — and because fetching it means spawning a
    /// process, which should not happen every time Settings is opened.
    pub(crate) languages: Mutex<Option<Vec<whisper_protocol::Language>>>,
    /// The LAN remote client's listener and its broadcast channel (P16).
    ///
    /// Always here, even while the remote is switched off: [`broadcast`] emits
    /// through it on every event, and a state that had to be checked for
    /// existence first would be checked at every emit site.
    pub(crate) remote: Arc<Hub>,
    /// The audio behind the bubbles, for replaying (P22). Shared with every
    /// session, which fills it, and outliving them, like the history.
    pub(crate) replay: Arc<replay::ReplayStore>,
    /// Where audio leaves the app: the listen output and the shoutbox (P22,
    /// P26). Independent of any session.
    pub(crate) outputs: outputs::Engine,
}

impl AppState {
    /// The configuration path, or the reason there is none.
    pub(crate) fn config_path(&self) -> Result<PathBuf, String> {
        self.config_path
            .lock()
            .map_err(|_| "configuration path poisoned".to_string())?
            .clone()
            .ok_or_else(|| "no configuration directory on this system".to_string())
    }

    pub(crate) fn models_dir(&self) -> Result<PathBuf, String> {
        self.models_dir
            .lock()
            .map_err(|_| "model directory poisoned".to_string())?
            .clone()
            .ok_or_else(|| "no application data directory on this system".to_string())
    }

    /// The stored model choice. A configuration that cannot be read resolves
    /// as "nothing chosen", which lands on the default — the model picker is
    /// not the place to surface a broken configuration file, and `load_config`
    /// already did.
    pub(crate) fn selection(&self) -> models::Selection {
        self.config_path()
            .map(|path| config::load(&path).selection())
            .unwrap_or_default()
    }

    /// How many sidecars the operator asked to decode in parallel, or `None`
    /// for the default. Read from the file at the moment a session starts, for
    /// the same reason the model choice is: it is the setting as it stands
    /// now, not as it stood when the app came up.
    pub(crate) fn workers(&self) -> Option<usize> {
        self.config_path()
            .ok()
            .and_then(|path| config::load(&path).workers)
    }
}

// --- The three things that stay on the desktop (P16) ----------------------
//
// Not in [`commands`], because a browser has no equivalent for any of them and
// an arm answering "not on this transport" would suggest the remote is missing
// something. What is true is that a tab is not a window: the remote picks no
// files, flashes in the page and its tab title rather than asking the
// platform, and opens a group in a tab of its own.

/// Asks for a model file with the system's own file dialog.
///
/// `current` is what the field holds, and the dialog opens on it: a second
/// attempt after picking the wrong `.bin` should not start again at whatever
/// the operating system last remembered, which on a stick full of models is
/// the whole walk back through the directory tree.
///
/// `None` is a cancelled dialog, which is not an error and must not read as
/// one: it is the most likely outcome of opening a picker, and the path in
/// Settings stays exactly as it was.
///
/// `async` on purpose. A synchronous command runs on the main thread, which is
/// the thread the dialog has to be driven from — blocking it there is a hung
/// window rather than a picker.
#[tauri::command]
async fn pick_model_file<R: Runtime>(app: AppHandle<R>, current: Option<String>) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;

    let mut dialog = app
        .dialog()
        .file()
        // A whisper model is a `.bin`, and a dialog that offers every file on
        // the disk invites the one mistake this field can make. "All files"
        // stays available in the dialog itself for anything unusual.
        .add_filter("Whisper model", &["bin"]);

    // Both halves are checked against the disk before they are used. A typed
    // path that is half-finished or points at an unplugged stick is exactly
    // when somebody reaches for the button, and a dialog asked to open
    // somewhere that is not there opens somewhere unpredictable instead.
    if let Some(current) = current.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        let current = Path::new(current);
        if let Some(dir) = current.parent().filter(|dir| dir.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        if current.is_file() {
            if let Some(name) = current.file_name().and_then(|name| name.to_str()) {
                dialog = dialog.set_file_name(name);
            }
        }
    }

    dialog.blocking_pick_file().map(|file| file.to_string())
}

/// Asks the platform to draw attention to this window.
///
/// The window it was called from, not "the main window": a detached group
/// window is alerted for its own members, and it is the caller that knows
/// whether what arrived concerns it.
///
/// Deliberately not an audible alarm. An intercom room is already a room full
/// of speech, and a beep per matching word is the fastest way to have the
/// feature switched off. `urgent` picks between the two requests the platform
/// offers: an alert term keeps asking until the app is looked at, anything
/// else is a single bounce.
///
/// A group's flash-on-message asks for more than a bounce — the whole window
/// in dark orange — and that half of it is not here: no platform offers to
/// paint a window, so the page paints itself. See `flash.ts`. Which is also
/// why that half reaches a focused window and this one does not.
///
/// A focused window is left alone — the operator is already looking at it, and
/// macOS would bounce the dock icon of the app in front of them.
#[tauri::command]
fn request_attention<R: Runtime>(window: tauri::Window<R>, urgent: bool) -> Result<(), String> {
    if window.is_focused().unwrap_or(false) {
        return Ok(());
    }

    let kind = if urgent {
        tauri::UserAttentionType::Critical
    } else {
        tauri::UserAttentionType::Informational
    };
    window
        .request_user_attention(Some(kind))
        .map_err(|err| err.to_string())
}

/// Opens a group in a window of its own, or focuses the one already open.
///
/// A second WebView on the same frontend, backed by the same session state: it
/// starts no capture and no worker pool, so Start and Stop stay global and a
/// group window costs the machine a view rather than a second transcription of
/// the same audio.
///
/// `group` is the group's position in the active profile — the same position
/// the configuration file stores membership by. It travels in the window's
/// label rather than in a query string, so the window knows which group it is
/// before its first paint, and reopening the same group finds the window
/// instead of stacking another one on it.
///
/// `on_top` is the group's "keep this window in front" setting, applied at
/// creation so the window is born in front rather than jumping there a moment
/// after it appears. It is stored with the group, so it can also change while
/// the window is open — the window then applies it to itself.
#[tauri::command]
fn open_group_window<R: Runtime>(
    app: AppHandle<R>,
    group: Option<usize>,
    name: String,
    on_top: bool,
) -> Result<(), String> {
    // No position means every channel: all channels is a view of the same
    // shape as a group, but it is not one, so it has no place in the profile's
    // group list to be named after.
    let label = match group {
        Some(index) => format!("{GROUP_WINDOW_PREFIX}{index}"),
        None => format!("{GROUP_WINDOW_PREFIX}{GROUP_WINDOW_ALL}"),
    };
    // The group's own name first: this window is read from a Dock icon and a
    // window menu, where "Fluestr" is what every one of them says.
    let title = format!("{name} — Fluestr");

    if let Some(existing) = app.get_webview_window(&label) {
        // Already open: bring it forward and let it pick up a rename. Asking
        // for a group that is on screen behind three other windows is asking
        // to see it, not asking for a duplicate of it.
        let _ = existing.set_title(&title);
        let _ = existing.set_always_on_top(on_top);
        let _ = existing.unminimize();
        return existing.set_focus().map_err(|err| err.to_string());
    }

    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("index.html".into()))
        .title(title)
        .always_on_top(on_top)
        // Narrow and tall by default: a group is a handful of channels, and
        // these windows are put down the side of a screen next to whatever
        // else is being worked in.
        .inner_size(520.0, 780.0)
        .min_inner_size(360.0, 320.0)
        .build()
        .map(|_| ())
        .map_err(|err| {
            tracing::error!(error = ?err, label, "opening a group window failed");
            err.to_string()
        })
}

/// Closes every detached group window.
///
/// Called when the active profile changes. A group window is a view onto a
/// group of the profile that is running, and the stream ids its transcripts
/// carry are positions in *that* profile's channel list — carried over to
/// another production it would be filtering the right numbers against the
/// wrong channels. Switching profiles is a setup action, not something done
/// during a show, so closing the windows costs an operator a layout they were
/// about to rebuild anyway.
#[tauri::command]
fn close_group_windows<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    for (label, window) in app.webview_windows() {
        if label.starts_with(GROUP_WINDOW_PREFIX) {
            let _ = window.close();
        }
    }
    Ok(())
}

// --- Starting a session ---------------------------------------------------

/// Starts a session and returns its running state.
///
/// Blocks until the model is loaded, which on a machine's very first start
/// includes the one-off Metal shader compilation (~8 s). Returning the state
/// rather than only emitting it means the button that was pressed gets its own
/// answer, wherever it was pressed; the events carry what happens after that.
///
/// Lives here rather than in [`commands`] because it is the one command that
/// builds the event sink, which is this module's own type.
/// The sink both the session and the monitor report through.
///
/// One builder rather than one per caller: every view is caught up from the
/// history this writes and from the rules it evaluates, and a second sink
/// assembled somewhere else is how one of those quietly stops happening for
/// half the events.
pub(crate) fn events<R: Runtime>(app: &AppHandle<R>) -> Result<Arc<dyn EventSink>, String> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "the application state is not ready".to_string())?;
    Ok(Arc::new(TauriEvents {
        app: app.clone(),
        rules: Arc::clone(&state.rules),
        webhooks: Arc::clone(&state.webhooks),
        history: Arc::clone(&state.history),
    }))
}

pub(crate) fn start_session<R: Runtime>(
    app: &AppHandle<R>,
    config: SessionConfig,
) -> Result<SessionState, String> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "the application state is not ready".to_string())?;
    let mut slot = state.session.lock().map_err(|_| "session state poisoned")?;

    // A session whose pipeline has already ended — device unplugged, sidecar
    // lost — is still sitting in the slot, because the failure is discovered
    // on the processing thread and nothing there can reach this state. Left
    // alone it makes the next Start fail with "a session is already running"
    // while the UI, which saw the failure, offers Start rather than Stop, and
    // nothing but restarting the app gets out of it. Clearing it here is also
    // what joins its threads and releases its sidecar.
    if slot.as_ref().is_some_and(Session::is_finished) {
        tracing::info!("clearing the previous session, whose pipeline had ended");
        if let Some(finished) = slot.take() {
            finished.stop();
        }
    }

    if slot.is_some() {
        return Err("a session is already running".into());
    }

    // The meters of the Settings page have the device open. It is released
    // here rather than by the caller: the session is what has to have it, and
    // a Start that fails because the app itself was holding the interface
    // would be the app blaming the operator for its own housekeeping.
    if let Ok(mut monitor) = state.monitor.lock() {
        if let Some(monitor) = monitor.take() {
            monitor.stop();
        }
    }

    // Which model, decided before anything is opened. A session that starts,
    // captures audio and only then discovers it has nothing to transcribe with
    // is the silent malfunction P11 exists to remove — and the message has to
    // name the fix, because "no model" is not something an operator can act on.
    let models = match state.models_dir() {
        Ok(dir) => Some(models::resolve(&dir, &state.selection()).map_err(|err| {
            tracing::error!(error = ?err, "no usable model");
            err.to_string()
        })?),
        // No app directory: the model manager cannot work here, but the
        // development search still can, so this is not fatal on its own.
        Err(err) => {
            tracing::warn!(error = %err, "no model directory — falling back to the local search");
            None
        }
    };

    let events = events(app)?;
    match Session::start(
        config,
        events.clone(),
        models,
        state.workers(),
        Arc::clone(&state.replay),
    ) {
        Ok((session, running)) => {
            *slot = Some(session);
            // Announced as well as returned. The view that pressed Start
            // learns from the answer, but it is not the only view any more: a
            // detached group window (P7) and a browser (P16) have nothing but
            // the event, and without this they go on saying "not running"
            // through a running session — and, having no zero point for it,
            // time every line as "+0:12" instead of as a time of day.
            //
            // The sink is also what records the state in the history, so a
            // view opened later is caught up on it too.
            events.state(running.clone());
            Ok(running)
        }
        Err(err) => {
            // With the chain of causes, which the UI string flattens away.
            tracing::error!(error = ?err, "starting the session failed");
            let message = err.to_string();
            // Also as an event, so a failure is visible in the same place a
            // successful start is, rather than only in the caller's catch.
            events.state(SessionState::Failed {
                message: message.clone(),
            });
            Err(message)
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = AppState::default();
            // A missing configuration directory is not a reason to refuse to
            // start: the app transcribes perfectly well without ever saving a
            // profile, and an operator in front of a show is better served by
            // a running app that cannot remember than by no app at all. It is
            // reported, and every save says so again.
            match app.path().app_config_dir() {
                Ok(dir) => {
                    let path = dir.join(config::FILE_NAME);
                    tracing::info!(path = %path.display(), "configuration file");
                    // Before anything is served: the listener below keys its
                    // cookies with it.
                    state.remote.keep_secret_in(&dir.join(remote::SECRET_FILE));
                    *state.config_path.lock().expect("fresh state") = Some(path);
                }
                Err(err) => {
                    tracing::error!(error = %err, "no configuration directory — nothing will be saved");
                }
            }
            // Models go under the app's *data* directory, not its
            // configuration one: they are hundreds of megabytes of cache that
            // can always be fetched again, and on macOS that is the difference
            // between something the user's backup carries around forever and
            // something it can skip.
            match app.path().app_data_dir() {
                Ok(dir) => {
                    let dir = dir.join(models::DIR_NAME);
                    tracing::info!(path = %dir.display(), "model directory");
                    *state.models_dir.lock().expect("fresh state") = Some(dir);
                }
                Err(err) => {
                    tracing::error!(error = %err, "no data directory — models cannot be downloaded");
                }
            }
            app.manage(state);

            // The LAN remote client, if the configuration asks for one (P16).
            // Read from the file here rather than waiting for the UI's first
            // save, so that an operator who left it on last night finds it on
            // this morning without opening Settings. Off is the normal state
            // and costs a file read.
            let handle = app.handle().clone();
            if let Some(state) = handle.try_state::<AppState>() {
                if let Ok(path) = state.config_path() {
                    let wanted = config::load(&path).remote;
                    if wanted.enabled {
                        // Nothing is serving yet, so `apply` has nothing to
                        // wait for and this returns at once. Blocked on
                        // rather than spawned so that an app which comes up
                        // with the remote on is serving by the time the
                        // window opens.
                        tauri::async_runtime::block_on(remote::apply(&handle, &wanted));
                    }
                }
            }

            // The outputs, for the same reason as the remote above: a
            // shoutbox that was on last night is on this morning without
            // anybody opening Settings (P26).
            let handle = app.handle().clone();
            if let Some(state) = handle.try_state::<AppState>() {
                if let Ok(path) = state.config_path() {
                    apply_outputs(&handle, &config::load(&path));
                }
            }

            // The one task that drains the outbound queue (P19). Started for
            // as long as the app is open rather than per session, like the
            // resource sampler below: its counters span the sessions of an
            // evening, and a cue that failed during the last one is still the
            // thing an operator needs to see.
            let handle = app.handle().clone();
            if let Some(state) = handle.try_state::<AppState>() {
                webhook::Sender::start(std::sync::Arc::clone(&state.webhooks), handle.clone());
            }

            // The resource indicator runs for as long as the window does, not
            // only while a session does: an operator opening the app on a
            // machine that is already loaded should see that before they press
            // Start, and the figures after Stop are how the memory a session
            // held is seen to come back.
            let handle = app.handle().clone();
            resources::spawn(move |usage| {
                broadcast(&handle, EVENT_RESOURCES, usage);
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Everything both views share goes through the one command layer
            // (P16) — see [`commands`].
            commands::command,
            // What only a window can do.
            pick_model_file,
            request_attention,
            open_group_window,
            close_group_windows
        ])
        .run(tauri::generate_context!())
        .expect("failed to start the Tauri application");
}

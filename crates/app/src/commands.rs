//! The one command layer (P16).
//!
//! Every mutation the app offers exists exactly once, here. The Tauri bridge
//! is a thin wrapper that hands a request to [`dispatch`]; the LAN remote's
//! socket hands it the same request. There is no second implementation to keep
//! in agreement, so the browser peer cannot quietly fall behind the desktop as
//! commands are added — a command that is not in [`Command`] is missing from
//! both views at once, which is the kind of gap that shows up immediately.
//!
//! What is deliberately *not* here is the handful of things the browser has no
//! equivalent for: the native file picker, the platform's attention request,
//! and the detached group windows. Those stay `#[tauri::command]`s in
//! [`crate::run`], because putting them here would mean an enum arm that
//! answers "not on this transport" — a shape that says the remote is missing
//! something, when what is true is that a browser tab is not a window.
//!
//! The arguments arrive in camelCase because that is what a frontend sends,
//! and the command names are snake_case because that is what they were called
//! when they were Tauri commands: this layer was extracted underneath a
//! working UI, and renaming the wire at the same time would have made the
//! extraction impossible to check.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, Runtime};

use crate::alerts::Matcher;
use crate::config::Config;
use crate::models;
use crate::remote;
use crate::session::{self, SessionConfig, SessionState, StreamUpdate};
use crate::{AppState, EVENT_CLEARED, EVENT_CONFIG, EVENT_MODEL_PROGRESS};

/// The model choice the Settings page is currently showing.
///
/// Sent with the request rather than read from the configuration file: the
/// page saves through a debounce, so the file answers with what was selected a
/// second ago, and the "in use" banner would contradict the radio button next
/// to it.
#[derive(Debug, Clone, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "models.ts", rename = "ModelSelection")]
pub struct SelectionArg {
    pub model: Option<String>,
    pub vad_model: Option<String>,
    pub model_path: Option<String>,
}

impl From<SelectionArg> for models::Selection {
    fn from(arg: SelectionArg) -> Self {
        Self {
            speech: arg.model,
            vad: arg.vad_model,
            path: arg.model_path,
        }
    }
}

/// Everything either transport can ask for.
///
/// Adjacently tagged, so a call is `{"command": "save_config", "args": {…}}`
/// and a command without arguments is `{"command": "stop_session"}` — the
/// content may be left out for a unit variant, which is what keeps the
/// no-argument calls from having to send an empty object.
#[derive(Debug, Deserialize)]
#[serde(
    tag = "command",
    content = "args",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Command {
    AppVersion,
    LoadConfig,
    SaveConfig {
        config: Config,
    },
    ListModels {
        selection: Option<SelectionArg>,
    },
    DownloadModel {
        id: String,
    },
    CancelDownload {
        id: String,
    },
    RemoveModel {
        id: String,
    },
    ListDevices,
    ListLanguages,
    SetAlertTerms {
        terms: Vec<String>,
    },
    SessionSnapshot,
    ClearCommunication,
    StartSession {
        config: SessionConfig,
    },
    StopSession,
    /// Renames a running channel, or puts it onto another language, without
    /// stopping capture (P6).
    UpdateStreamConfig {
        update: StreamUpdate,
    },
    /// The addresses the LAN remote could be served on (P16).
    ListInterfaces,
    /// Whether it is being served, and to how many browsers.
    RemoteStatus,
}

/// Runs one command and answers with its result as JSON.
///
/// JSON rather than a typed result, because the two callers want different
/// things from it: the socket writes it to the wire, and the Tauri bridge
/// hands it back to a frontend that was going to parse JSON either way. A
/// typed answer would need an enum mirroring this one, which is the second
/// copy this module exists to avoid.
pub async fn dispatch<R: Runtime>(app: &AppHandle<R>, command: Command) -> Result<Value, String> {
    match command {
        Command::AppVersion => json(env!("CARGO_PKG_VERSION")),
        Command::LoadConfig => json(load_config(app)?),
        Command::SaveConfig { config } => json(save_config(app, config)?),
        Command::ListModels { selection } => json(list_models(app, selection)?),
        Command::DownloadModel { id } => json(download_model(app, id).await?),
        Command::CancelDownload { id } => json(cancel_download(app, &id)?),
        Command::RemoveModel { id } => json(remove_model(app, &id)?),
        Command::ListDevices => json(session::list_devices().map_err(|err| err.to_string())?),
        Command::ListLanguages => json(list_languages(app)?),
        Command::SetAlertTerms { terms } => json(set_alert_terms(app, terms)?),
        Command::SessionSnapshot => json(session_snapshot(app)?),
        Command::ClearCommunication => json(clear_communication(app)?),
        // On a blocking thread: starting a session loads the model, which on
        // a machine's first ever start is eight seconds of Metal shader
        // compilation. Held on the async runtime that would be eight seconds
        // in which no remote client is served.
        Command::StartSession { config } => {
            let app = app.clone();
            let started =
                tauri::async_runtime::spawn_blocking(move || crate::start_session(&app, config))
                    .await
                    .map_err(|err| format!("the session could not be started: {err}"))??;
            json(started)
        }
        Command::StopSession => json(stop_session(app)?),
        Command::UpdateStreamConfig { update } => json(update_stream_config(app, update)?),
        Command::ListInterfaces => json(remote::interfaces()),
        Command::RemoteStatus => json(remote_status(app)?),
    }
}

/// A result on its way back through either transport.
///
/// Failing to serialise an answer is a bug in a struct on this side, not
/// something the caller did — so it is logged with the type that could not be
/// written and reported as the flat string both transports carry.
fn json<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|err| {
        tracing::error!(error = %err, type_name = std::any::type_name::<T>(), "an answer could not be serialised");
        err.to_string()
    })
}

/// The state, or the reason there is none.
///
/// Every command starts here rather than taking a `tauri::State` argument: the
/// socket has an `AppHandle` and no Tauri command context, and one lookup that
/// both transports make is better than two ways of reaching the same value.
fn state<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::State<'_, AppState>, String> {
    app.try_state::<AppState>()
        .ok_or_else(|| "the application state is not ready".to_string())
}

/// The stored configuration, or the default one.
///
/// Infallible on purpose where it can be: a first start, an unreadable file
/// and a file from a newer version all come back as an empty configuration and
/// a log line rather than as an error the UI would have to render instead of
/// the app. What it cannot paper over is not knowing where to look, which is
/// the one case that also makes saving impossible and therefore has to be
/// visible.
fn load_config<R: Runtime>(app: &AppHandle<R>) -> Result<Config, String> {
    Ok(crate::config::load(&state(app)?.config_path()?))
}

/// Writes the whole configuration.
///
/// The whole of it, not a patch: it is a few kilobytes that a human edits at
/// human speed, and a partial update would need the two sides to agree on
/// which half is authoritative — which is exactly the drift the single file
/// exists to avoid. That is also what makes a remote save safe: it is
/// last-write-wins, and the side that lost is told.
fn save_config<R: Runtime>(app: &AppHandle<R>, config: Config) -> Result<(), String> {
    let path = state(app)?.config_path()?;
    // Through the same tidying the file gets on the way in, so that what is
    // compared below is two configurations in one form rather than one of each.
    let config = config.sanitised();

    // Already what the file says: no write, and above all no announcement.
    //
    // The announcement is the point. Every view adopts a configuration change
    // it did not make, and adopting is a state change, which schedules that
    // view's own save. A view that then writes back what it was given — an
    // older one, or one whose round trip is not quite exact — turns one save
    // into a conversation that nothing ends: the configuration bounces between
    // two peers several times a second, and every field on the Settings page
    // is rebuilt under the cursor on each pass. The views are careful not to
    // start that (see `agreed` in `App.svelte`), but care in one view is not a
    // property of the system. This is: a configuration that changes nothing
    // changes nothing, whoever sent it and whatever version they are running.
    if crate::config::load(&path) == config {
        // The listener still gets a look in, because it can disagree with a
        // file that has not changed: a bind that failed — the port taken, the
        // address gone — leaves the setting on and nothing served, and
        // returning here would mean the only way to try again is to change a
        // field and change it back. `apply` is a no-op when the listener
        // already matches, so this costs the ordinary save nothing.
        remote::apply(app, &config.remote);
        return Ok(());
    }

    crate::config::save(&path, &config).map_err(|err| {
        // With the chain of causes, which the UI string flattens away.
        tracing::error!(error = ?err, "saving the configuration failed");
        err.to_string()
    })?;

    // The listener follows the file, here and at startup, and nowhere else
    // (P16). A separate "enable the remote" command would be a second writer
    // of the same setting, and the two would disagree the first time a save
    // and a click landed together.
    remote::apply(app, &config.remote);

    // Announced to every view, which is how a detached group window (P7)
    // learns that its group was renamed, refilled or switched to another
    // profile — and how the local Settings page hears about a save made from
    // the LAN remote (P16). The window that saved recognises its own echo.
    //
    // After the write, not before: a view that reacted to a configuration the
    // disk then refused would be showing something nothing else agrees with.
    crate::broadcast(app, EVENT_CONFIG, config);
    Ok(())
}

// --- The model manager (P11) ---------------------------------------------

/// The catalogue, what of it is installed, and which model a session would
/// actually start with.
///
/// One command rather than three, because the three answers are only useful
/// together: "small is installed" means nothing without "and it is the one in
/// use", and the page would otherwise render a picker and its active state
/// from two round trips that can disagree.
fn list_models<R: Runtime>(
    app: &AppHandle<R>,
    selection: Option<SelectionArg>,
) -> Result<models::ModelsView, String> {
    let state = state(app)?;
    let selection = match selection {
        Some(arg) => arg.into(),
        None => state.selection(),
    };
    Ok(models::view(&state.models_dir()?, &selection))
}

/// Fetches one catalogue model, resuming an interrupted attempt.
///
/// Returns as soon as the download is *running*, not when it finishes: this is
/// minutes of work, and a command that blocked for it would freeze the window
/// and hit Tauri's own timeouts. Progress arrives as `model-progress` events,
/// which is also what a resumed or cancelled download reports through — and,
/// since those events reach the remote too, a download started in the browser
/// draws its bar on the local Settings page as well.
async fn download_model<R: Runtime>(app: &AppHandle<R>, id: String) -> Result<(), String> {
    let entry = models::entry(&id).ok_or_else(|| format!("no such model: {id}"))?;
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    // The lock is taken and dropped before anything is awaited: a guard held
    // across an await point is a guard held for as long as the download runs.
    let dir = {
        let state = state(app)?;
        let dir = state.models_dir()?;
        let mut downloads = state
            .downloads
            .lock()
            .map_err(|_| "download state poisoned".to_string())?;
        // Pressing the button twice — or pressing it locally while the remote
        // presses it too — must not start two writers on one part file. They
        // would interleave, and only the checksum at the end would notice,
        // after both had run to completion.
        if downloads.contains_key(&id) {
            return Err(format!("{id} is already downloading"));
        }
        downloads.insert(id.clone(), std::sync::Arc::clone(&cancel));
        dir
    };

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let progress_app = app.clone();
        let result = models::download(entry, &dir, cancel, move |progress| {
            crate::broadcast(&progress_app, EVENT_MODEL_PROGRESS, progress);
        })
        .await;

        // Cleared before the failure is reported, so that the retry the
        // operator is about to press is not refused as "already downloading".
        if let Some(state) = app.try_state::<AppState>() {
            if let Ok(mut downloads) = state.downloads.lock() {
                downloads.remove(&id);
            }
        }

        if let Err(err) = result {
            // Cancelling is not a failure and has already reported itself.
            if err.to_string() == "cancelled" {
                tracing::info!(id, "download cancelled");
                return;
            }
            tracing::error!(id, error = ?err, "downloading a model failed");
            crate::broadcast(
                &app,
                EVENT_MODEL_PROGRESS,
                models::Progress {
                    id,
                    received: 0,
                    total: entry.bytes,
                    stage: models::Stage::Failed,
                    message: Some(err.to_string()),
                },
            );
        }
    });

    Ok(())
}

/// Stops a download in flight. What has arrived is kept, so pressing download
/// again continues rather than starts over.
fn cancel_download<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<(), String> {
    let state = state(app)?;
    let downloads = state
        .downloads
        .lock()
        .map_err(|_| "download state poisoned".to_string())?;
    if let Some(cancel) = downloads.get(id) {
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

/// Deletes an installed model, and any half-finished download of it.
///
/// Worth having rather than leaving to the Finder: these are the largest files
/// the app ever writes, in a directory macOS does its best to hide — and the
/// remote operator has no Finder on this machine at all.
fn remove_model<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<(), String> {
    models::remove(&state(app)?.models_dir()?, id).map_err(|err| {
        tracing::error!(error = ?err, "removing a model failed");
        err.to_string()
    })
}

/// Every language the sidecar can be given, for the picker on the channel
/// setup.
///
/// Fetched from whisper.cpp rather than listed here, so that no language list
/// exists anywhere in this repository: adding one is a whisper.cpp version,
/// never a code change of ours.
fn list_languages<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<whisper_protocol::Language>, String> {
    let state = state(app)?;
    let mut cached = state
        .languages
        .lock()
        .map_err(|_| "language list poisoned")?;
    if let Some(languages) = cached.as_ref() {
        return Ok(languages.clone());
    }

    let languages = crate::worker::list_languages().map_err(|err| {
        tracing::error!(error = ?err, "asking the sidecar for its language list failed");
        err.to_string()
    })?;
    *cached = Some(languages.clone());
    Ok(languages)
}

// --- Alert terms (P17) ----------------------------------------------------

/// Compiles the alert list. Sent by the UI whenever it changes, and once at
/// startup with what the configuration file held.
///
/// The list is not read from the configuration file here, for the same reason
/// `list_models` takes its selection as an argument: Settings saves through a
/// debounce, so the file is a second behind the term that was just typed —
/// and a term that only starts working a second after it was entered is a term
/// nobody can tell is working.
fn set_alert_terms<R: Runtime>(app: &AppHandle<R>, terms: Vec<String>) -> Result<(), String> {
    let matcher = Matcher::new(terms);
    tracing::info!(terms = matcher.len(), "alert list compiled");
    *state(app)?
        .alerts
        .lock()
        .map_err(|_| "the alert list is poisoned".to_string())? = matcher;
    Ok(())
}

// --- The chat, and the views onto it (P7, P16) ----------------------------

/// The chat as it stands: every transcript still held, the sessions' zero
/// points, and the session state.
///
/// Asked for once by every view as it opens, and again by a remote client that
/// reconnects. The main window uses it to come back from a reload with its
/// chat intact; a detached group window uses it because it is opened in the
/// middle of a show and would otherwise start blank while the room is full of
/// things that were already said; a browser uses it for both reasons at once.
///
/// Bounded by the same in-memory history every other view is caught up from
/// (see [`crate::history::CAPACITY`]), so a client that slept longer than the
/// buffer resumes from what is left rather than from nothing.
fn session_snapshot<R: Runtime>(app: &AppHandle<R>) -> Result<crate::history::Snapshot, String> {
    Ok(state(app)?
        .history
        .lock()
        .map_err(|_| "the chat history is poisoned".to_string())?
        .snapshot())
}

/// Empties the chat — every transcript, in every group, from every session and
/// in every view.
///
/// A backend command rather than each view clearing itself, which is what it
/// was while there was only one window. With several views onto one chat, the
/// action has to happen in the one place they are all caught up from,
/// otherwise a group window or a browser would keep showing what the main
/// window was just told had been deleted.
///
/// Changes no configuration: channels, colours, groups and the vocabulary all
/// survive it. The confirmation is the caller's — see `ClearCommunication`.
fn clear_communication<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    state(app)?
        .history
        .lock()
        .map_err(|_| "the chat history is poisoned".to_string())?
        .clear();
    crate::broadcast(app, EVENT_CLEARED, ());
    Ok(())
}

fn stop_session<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let session = state(app)?
        .session
        .lock()
        .map_err(|_| "session state poisoned")?
        .take();
    if let Some(session) = session {
        session.stop();
    }
    Ok(())
}

/// Changes one channel of the session that is running.
///
/// A command rather than a restart, because the two things it can change —
/// the label and the language — are the two an operator gets wrong while the
/// room is already talking, and Stop is what they will not press to fix them.
/// What it cannot change is the device channel: the channel map is fixed when
/// capture opens, so a repatch is honestly a restart. See
/// [`session::StreamUpdate`].
///
/// Answers with the running state and announces it, so the Settings page that
/// asked, a detached group window and a browser on the LAN all take the new
/// mapping from the backend rather than each from its own copy of the setup.
///
/// Refused when nothing is running: there is no stream to update, and
/// silently succeeding would leave a page showing a change that took effect
/// nowhere. The *stored* setup is edited on the Settings page like always —
/// this is the live session, and the configuration file keeps its one writer.
fn update_stream_config<R: Runtime>(
    app: &AppHandle<R>,
    update: StreamUpdate,
) -> Result<SessionState, String> {
    let state = state(app)?;
    let session = state.session.lock().map_err(|_| "session state poisoned")?;
    let session = session
        .as_ref()
        .ok_or_else(|| "no session is running".to_string())?;
    session.update_stream(update).map_err(|err| {
        tracing::error!(error = ?err, "updating a running stream failed");
        err.to_string()
    })
}

fn remote_status<R: Runtime>(app: &AppHandle<R>) -> Result<remote::Status, String> {
    Ok(state(app)?.remote.status())
}

/// The Tauri half of the bridge: one command, carrying which command.
///
/// One entry point rather than sixteen `#[tauri::command]`s, so that the
/// desktop and the LAN remote reach [`dispatch`] the same way. Adding a
/// command is one arm of [`Command`] and one line of the frontend's typed
/// wrapper; there is nowhere left to add it to only one of the two views.
#[tauri::command]
pub async fn command<R: Runtime>(app: AppHandle<R>, request: Command) -> Result<Value, String> {
    dispatch(&app, request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape both transports send. Pinned by a test because it is the
    /// one thing the Tauri bridge and the LAN remote's socket have to agree on
    /// character for character, and neither of them would fail loudly: a
    /// misspelled command is a rejected call in the browser and a working app
    /// on the desktop, or the other way round.
    #[test]
    fn a_call_names_its_command_and_carries_its_arguments() {
        let call: Command = serde_json::from_str(
            r#"{"command":"set_alert_terms","args":{"terms":["Feuer","Arzt"]}}"#,
        )
        .expect("a call with arguments");

        match call {
            Command::SetAlertTerms { terms } => assert_eq!(terms, ["Feuer", "Arzt"]),
            other => panic!("read as {other:?}"),
        }
    }

    /// A command that takes nothing sends nothing. Without this the frontend
    /// would have to send an empty object for half the calls it makes, and the
    /// half that forgot would be rejected.
    #[test]
    fn a_command_without_arguments_needs_no_arguments() {
        assert!(matches!(
            serde_json::from_str::<Command>(r#"{"command":"stop_session"}"#),
            Ok(Command::StopSession)
        ));
    }

    /// camelCase on the wire, snake_case in Rust — the frontend is the one
    /// writing these, and it writes JavaScript.
    #[test]
    fn arguments_arrive_as_a_frontend_writes_them() {
        let call: Command = serde_json::from_str(
            r#"{"command":"list_models","args":{"selection":{"model":"small-q5_1","vadModel":null,"modelPath":null}}}"#,
        )
        .expect("a selection");

        match call {
            Command::ListModels {
                selection: Some(selection),
            } => assert_eq!(selection.model.as_deref(), Some("small-q5_1")),
            other => panic!("read as {other:?}"),
        }
    }

    /// The half of an update that is not being changed may be left out (P6).
    ///
    /// Both transports send this, and the browser one sends hand-written JSON
    /// over a socket: a shape that insisted on `"language": null` to rename a
    /// channel would be a rejected call there and a working one on the desktop.
    #[test]
    fn an_update_may_change_one_field_and_leave_the_other_alone() {
        let call: Command = serde_json::from_str(
            r#"{"command":"update_stream_config","args":{"update":{"streamId":2,"label":"Inspizienz"}}}"#,
        )
        .expect("an update carrying only a label");

        match call {
            Command::UpdateStreamConfig { update } => {
                assert_eq!(update.stream_id, 2);
                assert_eq!(update.label.as_deref(), Some("Inspizienz"));
                assert_eq!(update.language, None, "nothing said is nothing changed");
            }
            other => panic!("read as {other:?}"),
        }
    }

    /// A command nobody defined is a rejected call, not a silently ignored
    /// one: the browser peer is built on this layer, and a call that vanishes
    /// is a button that does nothing for no stated reason.
    #[test]
    fn an_unknown_command_is_refused() {
        assert!(serde_json::from_str::<Command>(r#"{"command":"delete_everything"}"#).is_err());
    }
}

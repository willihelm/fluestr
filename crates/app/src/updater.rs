//! Updating the app from its GitHub releases.
//!
//! The release workflow (`.github/workflows/release.yml`) attaches a
//! `latest.json` to every release, naming the version and, per platform, the
//! bundle and its signature. The plugin fetches that file from the endpoint in
//! `tauri.conf.json`, compares versions, and refuses any bundle whose signature
//! does not match the public key next to the endpoint — which is what makes it
//! safe to install something fetched over the network without asking.
//!
//! Two commands rather than the plugin's JavaScript package, for the reason
//! the dialog and the login item are commands: the frontend keeps one way of
//! talking to the backend. Desktop only — a browser on the LAN has no business
//! replacing the program on the machine it is looking at.

use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};

/// How far the download of an update has got. Sent to the app's own windows
/// only, never to a remote browser: it is about this machine.
pub const EVENT_UPDATE_PROGRESS: &str = "update-progress";

/// A newer release than the one running.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "updater.ts")]
pub struct AvailableUpdate {
    /// The release's version, without the `v` of its tag.
    pub version: String,
    /// The release notes, as written on GitHub. Absent when the release has
    /// none.
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "updater.ts")]
pub struct UpdateProgress {
    pub received: u64,
    /// Absent when the server sends no length — then there is a count and no
    /// bar.
    pub total: Option<u64>,
}

/// The update the last check found, kept so that installing it installs what
/// the button offered rather than whatever a second check would say.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

/// Asks GitHub whether there is a newer release.
///
/// Called once by the main window as it opens. `None` is "up to date" — and
/// also what a development build always answers, because the executable it
/// would replace is the one under `target/`.
#[tauri::command]
pub async fn check_for_update<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<AvailableUpdate>, String> {
    if cfg!(debug_assertions) {
        return Ok(None);
    }

    let update = app
        .updater()
        .map_err(|err| err.to_string())?
        .check()
        .await
        .map_err(|err| {
            // Offline at a venue is the normal case, not a failure worth more
            // than a line in the log: the button simply does not appear.
            tracing::info!(error = %err, "could not check for an update");
            err.to_string()
        })?;

    let available = update.as_ref().map(|update| AvailableUpdate {
        version: update.version.clone(),
        notes: update.body.clone().filter(|notes| !notes.trim().is_empty()),
    });
    if let Some(available) = &available {
        tracing::info!(version = %available.version, "an update is available");
    }
    *app.state::<Pending>()
        .0
        .lock()
        .map_err(|_| "update state poisoned")? = update;
    Ok(available)
}

/// Downloads the update the last check found, installs it and restarts.
///
/// A running session and the meters are stopped between the download and the
/// install, not before: the download takes a while and the show goes on
/// during it, but the install replaces the program the workers were started
/// from — and on Windows it ends this process itself, so stopping afterwards
/// would be too late for the workers to be shut down rather than orphaned.
///
/// Resolves only on failure; on success the app is gone before it could.
#[tauri::command]
pub async fn install_update<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let update = app
        .state::<Pending>()
        .0
        .lock()
        .map_err(|_| "update state poisoned")?
        .clone()
        .ok_or("there is no update to install — check again")?;

    let mut received: u64 = 0;
    let mut reported: u64 = 0;
    let bytes = update
        .download(
            |chunk, total| {
                received += chunk as u64;
                // About a hundred events for the whole download, rather than
                // one per network chunk.
                let step = total.map_or(512 * 1024, |total| (total / 100).max(1));
                if received - reported >= step || total == Some(received) {
                    reported = received;
                    let _ = app.emit(EVENT_UPDATE_PROGRESS, UpdateProgress { received, total });
                }
            },
            || {},
        )
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "downloading the update failed");
            err.to_string()
        })?;

    tracing::info!(version = %update.version, "installing the update");
    crate::commands::stop_session(&app)?;
    crate::commands::stop_monitor(&app)?;

    update.install(bytes).map_err(|err| {
        tracing::error!(error = %err, "installing the update failed");
        err.to_string()
    })?;

    app.restart()
}

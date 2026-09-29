//! The ticker window (P24): the latest lines of one view, in a strip that
//! stays on top.
//!
//! A detached view (P7) laid out for the least room rather than a new kind of
//! window: the same frontend, the same events, a label of its own so the page
//! knows before its first paint that it is a ticker. It is for the laptop at
//! FOH beside the DAW, where a full chat window costs space nobody has — which
//! is why it has no title bar, and why it has to stay above that DAW even when
//! the DAW is fullscreen.
//!
//! Where it was left is the screen's business and not the production's, so
//! the geometry is kept by the page that opens it, in `localStorage`, beside
//! the sidebar width and for the same reason; see `ui/src/lib/ticker.ts`. What
//! this side adds is the check that the place still exists: a ticker
//! remembered on a monitor that is no longer plugged in would open somewhere
//! nobody can see it, and a window with no title bar cannot be found and
//! dragged back.

use serde::Deserialize;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Runtime};

use crate::GROUP_WINDOW_ALL;

/// What a ticker window's label starts with, followed by the view's position
/// in the active profile or [`GROUP_WINDOW_ALL`] — the same suffix a detached
/// group window carries, because it names the same view. Kept in step with
/// `TICKER_WINDOW_PREFIX` in `ui/src/lib/windows.ts`.
pub const TICKER_WINDOW_PREFIX: &str = "ticker-";

/// The label of the main window: Tauri's default, which `tauri.conf.json`
/// does not override.
const MAIN_WINDOW: &str = "main";

/// One to three lines at the chat's size, across the width of half a laptop
/// screen: what fits under a DAW's transport bar without covering a track.
const DEFAULT_WIDTH: f64 = 640.0;
const DEFAULT_HEIGHT: f64 = 76.0;
/// One line, and enough of it to hold a label and a few words. Kept in step
/// with `MIN_WIDTH` and `MIN_HEIGHT` in `ui/src/lib/ticker.ts`, which holds the
/// page's own resize handles to it.
const MIN_WIDTH: f64 = 240.0;
const MIN_HEIGHT: f64 = 32.0;

/// How much of a remembered ticker has to lie on a screen for it to be put
/// back there, in physical pixels on each axis. Enough to grab: a window
/// with a corner on screen is a window that can be dragged back.
const GRAB: i64 = 48;

/// Where a ticker was, in physical pixels — what the window's own move and
/// resize events report, so nothing is converted on the way into storage and
/// nothing is lost to rounding on the way back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A monitor's area, in the same physical desktop coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Whether enough of `geometry` lies on one of `screens` to be grabbed.
///
/// On one screen rather than on their union: two monitors side by side with
/// different heights leave a gap in the union's bounding box, and a window
/// put into that gap is off every screen there is.
pub fn on_screen(geometry: Geometry, screens: &[Screen]) -> bool {
    if geometry.width == 0 || geometry.height == 0 {
        return false;
    }
    screens.iter().any(|screen| {
        let overlap = |start: i32, length: u32, from: i32, extent: u32| {
            let end = i64::from(start) + i64::from(length);
            let limit = i64::from(from) + i64::from(extent);
            end.min(limit) - i64::from(start).max(i64::from(from))
        };
        overlap(geometry.x, geometry.width, screen.x, screen.width) >= GRAB
            && overlap(geometry.y, geometry.height, screen.y, screen.height) >= GRAB
    })
}

fn screens<R: Runtime>(app: &AppHandle<R>) -> Vec<Screen> {
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|monitor| Screen {
            x: monitor.position().x,
            y: monitor.position().y,
            width: monitor.size().width,
            height: monitor.size().height,
        })
        .collect()
}

/// Opens a view as a ticker, or brings the one already open forward.
///
/// `group` is the view's position in the active profile, `None` for all
/// channels — the same addressing as `open_group_window`, and a group can have
/// both a window and a ticker open at once, because they are two things to
/// want of it. `geometry` is where this view's ticker was last left, if the
/// opening page remembers one.
///
/// Built hidden and shown once it is where it belongs, so a remembered ticker
/// does not appear at the default place first and jump.
#[tauri::command]
pub fn open_ticker_window<R: Runtime>(
    app: AppHandle<R>,
    group: Option<usize>,
    name: String,
    geometry: Option<Geometry>,
) -> Result<(), String> {
    let label = match group {
        Some(index) => format!("{TICKER_WINDOW_PREFIX}{index}"),
        None => format!("{TICKER_WINDOW_PREFIX}{GROUP_WINDOW_ALL}"),
    };
    // Never drawn — the window has no title bar — but read in the Window menu,
    // Mission Control and the task switcher, where it has to be told from the
    // group window of the same name.
    let title = format!("{name} — Ticker — Fluestr");

    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_title(&title);
        let _ = existing.unminimize();
        let _ = existing.show();
        return existing.set_focus().map_err(|err| err.to_string());
    }

    let window =
        tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("index.html".into()))
            .title(title)
            // No title bar: at one to three lines it would be a third of the
            // window. The page draws a grip to move it by.
            .decorations(false)
            .inner_size(DEFAULT_WIDTH, DEFAULT_HEIGHT)
            .min_inner_size(MIN_WIDTH, MIN_HEIGHT)
            // Resized by handles the page draws, never natively: the native
            // zone runs down the left edge too, where the grip is, and there
            // it would resize a window the operator meant to move.
            .resizable(false)
            // On macOS the level is set below instead: `always_on_top` there is
            // the floating level, which a fullscreen DAW's Space is above, and
            // setting both would leave the result to whichever landed last.
            .always_on_top(cfg!(not(target_os = "macos")))
            .visible_on_all_workspaces(true)
            .visible(false)
            .build()
            .map_err(|err| {
                tracing::error!(error = ?err, label, "opening a ticker failed");
                err.to_string()
            })?;

    match geometry {
        Some(geometry) if on_screen(geometry, &screens(&app)) => {
            let _ = window.set_size(PhysicalSize::new(geometry.width, geometry.height));
            let _ = window.set_position(PhysicalPosition::new(geometry.x, geometry.y));
        }
        Some(geometry) => {
            tracing::info!(
                ?geometry,
                label,
                "the ticker's last place is off every screen — opening it at the default"
            );
        }
        None => {}
    }

    #[cfg(target_os = "macos")]
    above_fullscreen(&window);

    window.show().map_err(|err| err.to_string())
}

/// Keeps a ticker above everything, fullscreen apps included.
///
/// Two things `always_on_top` does not do on macOS. A fullscreen app gets a
/// Space of its own, and a window only appears in it if it may join every
/// Space *and* is marked as an auxiliary of fullscreen windows. And the level:
/// `always_on_top` is the floating level, which is where a DAW's own plug-in
/// windows sit as well; the status level is above them, below the menu bar.
///
/// **Not yet checked against the DAW the house runs** — which is the risk the
/// plan names first (P24). If a fullscreen DAW still covers the ticker, what
/// is left is a panel window, which Tauri does not build.
#[cfg(target_os = "macos")]
fn above_fullscreen<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    use objc2_app_kit::{NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior};

    let Ok(pointer) = window.ns_window() else {
        tracing::warn!("no native window for the ticker — it stays at the default level");
        return;
    };
    // An address rather than the pointer: the closure has to be `Send`, and
    // it runs on the main thread, which is the only one AppKit may be touched
    // from — which is also the thread the window lives on.
    let address = pointer as usize;
    let _ = window.run_on_main_thread(move || {
        // SAFETY: the address came from the window this closure was queued
        // by, which is only torn down on this same thread — so it cannot be
        // closed between the two.
        let ns_window = unsafe { &*(address as *const NSWindow) };
        ns_window.setCollectionBehavior(
            ns_window.collectionBehavior()
                | NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        ns_window.setLevel(NSStatusWindowLevel);
    });
}

/// Brings the main window forward — what a click on a ticker line asks for,
/// the other half being the message the page sends it about *which* line.
///
/// A command rather than something the ticker does to the main window itself:
/// a window may place itself, and the capability that would let it place
/// another is one no view needs otherwise.
#[tauri::command]
pub fn focus_main_window<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let main = app
        .get_webview_window(MAIN_WINDOW)
        .ok_or_else(|| "the main window is not open".to_string())?;
    let _ = main.unminimize();
    let _ = main.show();
    main.set_focus().map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: Screen = Screen {
        x: 0,
        y: 0,
        width: 2880,
        height: 1800,
    };
    /// To the right of the laptop, shorter, and lower down.
    const SIDE: Screen = Screen {
        x: 2880,
        y: 600,
        width: 1920,
        height: 1080,
    };

    fn at(x: i32, y: i32) -> Geometry {
        Geometry {
            x,
            y,
            width: 1280,
            height: 150,
        }
    }

    #[test]
    fn a_ticker_on_a_screen_goes_back_there() {
        assert!(on_screen(at(100, 1600), &[LAPTOP]));
        assert!(on_screen(at(3000, 700), &[LAPTOP, SIDE]));
    }

    #[test]
    fn a_ticker_on_an_unplugged_screen_does_not() {
        assert!(!on_screen(at(3000, 700), &[LAPTOP]));
        assert!(!on_screen(at(100, 100), &[]));
    }

    // The gap in the bounding box of two screens of different heights: inside
    // the union's rectangle, on neither screen.
    #[test]
    fn the_gap_between_two_screens_is_off_screen() {
        assert!(!on_screen(at(3000, 100), &[LAPTOP, SIDE]));
    }

    #[test]
    fn a_corner_is_enough_and_a_sliver_is_not() {
        // 100 × 100 pixels of it on the laptop: enough to take hold of.
        assert!(on_screen(at(2780, 1700), &[LAPTOP]));
        // Ten pixels of it: technically on screen, practically lost.
        assert!(!on_screen(at(2870, 100), &[LAPTOP]));
        // Hanging off the top-left, into negative coordinates.
        assert!(on_screen(at(-1200, -100), &[LAPTOP]));
    }

    #[test]
    fn an_empty_geometry_is_nowhere() {
        let empty = Geometry {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        assert!(!on_screen(empty, &[LAPTOP]));
    }
}

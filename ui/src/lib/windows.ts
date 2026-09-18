// Which window this frontend is running in (P7).
//
// The app is one bundle mounted in several WebViews: the main window, and a
// detached window per group opened from the sidebar. What tells them apart is
// the window's **label**, which the backend builds from the group's position in
// the active profile — see `GROUP_WINDOW_PREFIX` in `crates/app/src/lib.rs`.
//
// The label rather than a query string or a hash, for two reasons. It is
// readable before the first paint, so a group window never renders the main
// view for a frame and then replaces it; and it is what Tauri identifies a
// window by, so asking for the same group twice focuses the window that is
// already open instead of stacking a second one on it.

import { getCurrentWindow } from "@tauri-apps/api/window";

/// Kept in step with `GROUP_WINDOW_PREFIX` on the Rust side. The two have to
/// agree or a group window comes up as a second main window.
export const GROUP_WINDOW_PREFIX = "group-";

/// The group a window label names, or `null` when it names no group.
///
/// Strict about what it accepts: anything that is not this prefix followed by a
/// non-negative integer is not a group window. A label that nearly matches
/// would otherwise open a view of group `NaN`, which renders as an empty chat
/// and looks like a group somebody emptied.
export const groupIndexOf = (label: string): number | null => {
  if (!label.startsWith(GROUP_WINDOW_PREFIX)) return null;

  const rest = label.slice(GROUP_WINDOW_PREFIX.length);
  if (!/^\d+$/.test(rest)) return null;

  return Number(rest);
};

/// The group this window is showing, or `null` for the main window.
///
/// Also `null` outside Tauri — `just dev-ui` serves the same bundle in a plain
/// browser, where there is no window to ask. The main view is the right answer
/// there: it is the one that works without a backend to talk to.
export const currentGroup = (): number | null => {
  try {
    return groupIndexOf(getCurrentWindow().label);
  } catch {
    return null;
  }
};

/// Keeps this window in front of other windows, or stops doing so.
///
/// The group's setting, applied by the window to itself: the backend already
/// applied it when it opened the window, and this is what a change made on the
/// Settings page while the window is open goes through.
///
/// Silent when it cannot, like `setWindowTitle` — outside Tauri there is no
/// window to put anywhere.
export const setWindowOnTop = (onTop: boolean) => {
  try {
    void getCurrentWindow()
      .setAlwaysOnTop(onTop)
      .catch(() => {});
  } catch {
    // No window to raise — see `currentGroup`.
  }
};

/// Renames this window.
///
/// A group window is read from the Dock and the window menu, where its title is
/// the only thing that tells it from the three others open beside it — so a
/// group renamed on the Settings page has to rename the window showing it. The
/// title is set once more from the backend when an already open window is
/// asked for again; this is the half that happens while nobody asked.
///
/// Silent when it cannot: outside Tauri there is no window, and a rename that
/// failed is a stale title rather than anything the operator can act on.
export const setWindowTitle = (title: string) => {
  try {
    void getCurrentWindow().setTitle(title).catch(() => {});
  } catch {
    // No window to name — see `currentGroup`.
  }
};

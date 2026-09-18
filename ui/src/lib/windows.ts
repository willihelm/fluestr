// Which view this frontend is (P7, P16).
//
// The app is one bundle mounted in several places: the main window, a detached
// window per group opened from the sidebar, and — over the LAN — a browser tab
// per group beside the browser's main tab.
//
// What tells them apart on the desktop is the window's **label**, which the
// backend builds from the group's position in the active profile; see
// `GROUP_WINDOW_PREFIX` in `crates/app/src/lib.rs`. The label rather than a
// query string, for two reasons: it is readable before the first paint, so a
// group window never renders the main view for a frame and then replaces it,
// and it is what Tauri identifies a window by, so asking for the same group
// twice focuses the window that is already open instead of stacking a second
// one on it.
//
// A browser has neither a label nor a second OS window, so the query string is
// what is left — `?group=2`. It buys back both properties in the browser's own
// terms: it is in the URL before the document is parsed, and a named
// `window.open` target reuses the tab that is already showing that group.

import { getCurrentWindow } from "@tauri-apps/api/window";
import { isRemote } from "./transport";

/// Kept in step with `GROUP_WINDOW_PREFIX` on the Rust side. The two have to
/// agree or a group window comes up as a second main window.
export const GROUP_WINDOW_PREFIX = "group-";

/// The query parameter a remote group tab carries. The browser's equivalent of
/// the window label, and named the same way for the same reason.
const GROUP_PARAM = "group";

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

/// The group a URL asks for, or `null` for the main view.
///
/// As strict as `groupIndexOf`, and for the same reason: `?group=all` is not a
/// group, and a view of group `NaN` is an empty chat that looks like a group
/// somebody emptied.
export const groupParamOf = (search: string): number | null => {
  const value = new URLSearchParams(search).get(GROUP_PARAM);
  if (value === null || !/^\d+$/.test(value)) return null;

  return Number(value);
};

/// The group this view is showing, or `null` for the main view.
///
/// The window label in the app, the query string in a browser. Also `null`
/// when neither says anything — `just dev-ui` serves the same bundle in a
/// plain browser with no backend behind it, and the main view is the right
/// answer there because it is the one that renders without one.
export const currentGroup = (): number | null => {
  if (isRemote) return groupParamOf(window.location.search);

  try {
    return groupIndexOf(getCurrentWindow().label);
  } catch {
    return null;
  }
};

/// The group tabs this page has opened, by group.
///
/// Kept so that switching profiles can close them, which is the one thing the
/// desktop's `close_group_windows` does that a browser cannot be asked to do
/// from the backend: a tab belongs to the browser, and only the page that
/// opened it holds a handle to it.
const tabs = new Map<number, Window>();

/// Opens a group in a tab of its own, or brings the one already open forward.
///
/// The browser's half of `openGroupWindow`. The target name carries the group,
/// so a second request for the same group lands in the same tab — the same
/// guarantee the window label gives on the desktop, by the same mechanism of
/// naming the thing after what it shows.
export const openGroupTab = (group: number, name: string) => {
  const existing = tabs.get(group);
  if (existing && !existing.closed) {
    existing.focus();
    return;
  }

  const opened = window.open(
    `${window.location.pathname}?${GROUP_PARAM}=${group}`,
    `${GROUP_WINDOW_PREFIX}${group}`,
  );
  // A blocked pop-up. Worth a line rather than a silent nothing: the operator
  // clicked something and a browser decided for them, and nothing on the page
  // would otherwise say so.
  if (!opened) {
    console.warn(`the browser refused to open a tab for ${name}`);
    return;
  }
  tabs.set(group, opened);
};

/// Closes every group tab this page opened.
///
/// Called when the active profile changes, like its desktop counterpart: the
/// stream ids a group view filters by are positions in the active profile's
/// channel list, so a tab carried over to another production would be
/// filtering the right numbers against the wrong channels.
export const closeGroupTabs = () => {
  for (const [, tab] of tabs) {
    if (!tab.closed) tab.close();
  }
  tabs.clear();
};

/// Keeps this window in front of other windows, or stops doing so.
///
/// The group's setting, applied by the window to itself: the backend already
/// applied it when it opened the window, and this is what a change made on the
/// Settings page while the window is open goes through.
///
/// Silent when it cannot, like `setWindowTitle` — outside Tauri there is no
/// window to put anywhere, and a browser tab cannot be put in front of the
/// windows around the browser.
export const setWindowOnTop = (onTop: boolean) => {
  if (isRemote) return;

  try {
    void getCurrentWindow()
      .setAlwaysOnTop(onTop)
      .catch(() => {});
  } catch {
    // No window to raise — see `currentGroup`.
  }
};

/// Whether this view can be pinned in front of the others at all.
///
/// One of the three things that stay on the desktop (P16). The pin is hidden
/// rather than disabled in a browser: a control that cannot do anything is
/// worse than no control, and there is nothing here for an operator to fix.
export const canPin = !isRemote;

/// Renames this view.
///
/// A group window is read from the Dock and the window menu, where its title is
/// the only thing that tells it from the three others open beside it — so a
/// group renamed on the Settings page has to rename the window showing it. The
/// title is set once more from the backend when an already open window is
/// asked for again; this is the half that happens while nobody asked.
///
/// In a browser the tab strip is the Dock, and the document title is what is
/// in it — which is also what the attention request flashes, so the two have
/// to be the same string. See `flash.ts`.
///
/// Silent when it cannot: outside Tauri there is no window, and a rename that
/// failed is a stale title rather than anything the operator can act on.
export const setWindowTitle = (title: string) => {
  if (isRemote) {
    document.title = title;
    return;
  }

  try {
    void getCurrentWindow()
      .setTitle(title)
      .catch(() => {});
  } catch {
    // No window to name — see `currentGroup`.
  }
};

// Which view this frontend is (P7, P16).
//
// The app is one bundle mounted in several places: the main window, a detached
// window per view opened from the sidebar, and — over the LAN — a browser tab
// per view beside the browser's main tab.
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
// A detachable view is a group or the all-channels view, and the label says
// which: `group-2` or `group-all`. All channels is not a group — it is what the
// chat shows when no group is selected — but it is a view of the same shape,
// and a window is the one place the difference stops mattering: what an
// operator wants on a second screen, or on a phone beside the desk, is as often
// everything as it is one group.
//
// A browser has neither a label nor a second OS window, so the query string is
// what is left — `?group=2`, `?group=all`, and `?ticker=2` and `?shoutbox` for
// the two pop-ups. It buys back both properties in the browser's own
// terms: it is in the URL before the document is parsed, and a named
// `window.open` target reuses the tab that is already showing that group.

import { getCurrentWindow } from "@tauri-apps/api/window";
import { isRemote } from "./transport";

/// Kept in step with `GROUP_WINDOW_PREFIX` on the Rust side. The two have to
/// agree or a group window comes up as a second main window.
export const GROUP_WINDOW_PREFIX = "group-";

/// How the all-channels view is spelled in a label and in a URL, and the one
/// value of [`GroupView`] that is not a group's position. Kept in step with
/// `GROUP_WINDOW_ALL` on the Rust side.
///
/// A word rather than a number, because every number is already a group: a
/// sentinel index would be one off-by-one away from being somebody's eighth
/// group, and this has to be readable in a URL a person can type.
export const ALL_CHANNELS_VIEW = "all" as const;

/// Which view a detached window or tab shows: one group, by its position in
/// the active profile, or every channel.
export type GroupView = number | typeof ALL_CHANNELS_VIEW;

/// The query parameter a remote group tab carries. The browser's equivalent of
/// the window label, and named the same way for the same reason.
const GROUP_PARAM = "group";

/// The view a window label names, or `null` when it names none — which is the
/// main window.
///
/// Strict about what it accepts: anything that is not this prefix followed by a
/// non-negative integer or `all` is not a group window. A label that nearly
/// matches would otherwise open a view of group `NaN`, which renders as an
/// empty chat and looks like a group somebody emptied.
export const groupIndexOf = (label: string): GroupView | null => {
  if (!label.startsWith(GROUP_WINDOW_PREFIX)) return null;

  return viewOf(label.slice(GROUP_WINDOW_PREFIX.length));
};

/// Kept in step with `TICKER_WINDOW_PREFIX` on the Rust side (P24). A ticker
/// is a view like a group window is, under a prefix of its own so the page can
/// tell which of the two layouts to mount — and followed by the same suffix,
/// because it names the same view.
export const TICKER_WINDOW_PREFIX = "ticker-";

/// The view a ticker window's label names, or `null` when it is not a ticker.
/// As strict as `groupIndexOf`, for its reason.
export const tickerIndexOf = (label: string): GroupView | null => {
  if (!label.startsWith(TICKER_WINDOW_PREFIX)) return null;

  return viewOf(label.slice(TICKER_WINDOW_PREFIX.length));
};

/// The query parameter a remote ticker carries — `?ticker=2`, `?ticker=all`.
/// The browser's equivalent of the ticker's label, like `GROUP_PARAM`.
const TICKER_PARAM = "ticker";

/// The view a URL asks for a ticker of, or `null` when it asks for none. As
/// strict as `groupParamOf`, for its reason.
export const tickerParamOf = (search: string): GroupView | null =>
  viewOf(new URLSearchParams(search).get(TICKER_PARAM));

/// The view this window is a ticker of, or `null` when it is none.
///
/// The window label in the app, the query string in a browser. A browser's
/// ticker is a small pop-up window rather than a strip that stays in front —
/// a page cannot put itself above the windows around the browser — but it is
/// the same least-room layout, which is what a second laptop or a phone
/// beside the desk is short of as well.
export const currentTicker = (): GroupView | null => {
  if (isRemote) return tickerParamOf(window.location.search);

  try {
    return tickerIndexOf(getCurrentWindow().label);
  } catch {
    return null;
  }
};

/// The label of the shoutbox window (P26), kept in step with
/// `SHOUTBOX_WINDOW` on the Rust side. One window and not one per view: there
/// is one shoutbox, the active profile's.
export const SHOUTBOX_WINDOW = "shoutbox";

/// Whether a URL asks for the shoutbox window: `?shoutbox`, the browser's
/// equivalent of the label, and named after it.
export const shoutboxParamOf = (search: string): boolean =>
  new URLSearchParams(search).has(SHOUTBOX_WINDOW);

/// Whether this window is the shoutbox window. The label in the app, the query
/// string in a browser, where it is a pop-up opened from the status bar.
export const isShoutboxWindow = (): boolean => {
  if (isRemote) return shoutboxParamOf(window.location.search);

  try {
    return getCurrentWindow().label === SHOUTBOX_WINDOW;
  } catch {
    return false;
  }
};

/// The view a URL asks for, or `null` for the main view.
///
/// As strict as `groupIndexOf`, and for the same reason: a view of group `NaN`
/// is an empty chat that looks like a group somebody emptied.
export const groupParamOf = (search: string): GroupView | null =>
  viewOf(new URLSearchParams(search).get(GROUP_PARAM));

/// The one reading of a label suffix or a query value, so that a window and
/// the tab showing the same thing cannot come to different conclusions about
/// what it spells.
const viewOf = (value: string | null): GroupView | null => {
  if (value === ALL_CHANNELS_VIEW) return ALL_CHANNELS_VIEW;
  if (value === null || !/^\d+$/.test(value)) return null;

  return Number(value);
};

/// The view this window is showing, or `null` for the main view.
///
/// The window label in the app, the query string in a browser. Also `null`
/// when neither says anything — `just dev-ui` serves the same bundle in a
/// plain browser with no backend behind it, and the main view is the right
/// answer there because it is the one that renders without one.
export const currentGroup = (): GroupView | null => {
  if (isRemote) return groupParamOf(window.location.search);

  try {
    return groupIndexOf(getCurrentWindow().label);
  } catch {
    return null;
  }
};

/// The tabs and pop-ups this page has opened, by their target name — which is
/// the window label they stand in for: `group-2`, `ticker-all`, `shoutbox`.
///
/// Kept so that switching profiles can close the views among them, which is
/// the one thing the desktop's `close_group_windows` does that a browser
/// cannot be asked to do from the backend: a tab belongs to the browser, and
/// only the page that opened it holds a handle to it. Kept as well so that
/// asking again focuses what is open: `window.open` on a name that is in use
/// would load the page in it again, from the start.
const tabs = new Map<string, Window>();

/// Opens `query` on this page under the target `name`, or brings forward
/// what is already open under it. `features` makes it a pop-up of that size
/// rather than a tab; a phone opens a tab either way.
const openTab = (name: string, query: string, title: string, features?: string): Window | null => {
  const existing = tabs.get(name);
  if (existing && !existing.closed) {
    existing.focus();
    return existing;
  }

  const opened = window.open(`${window.location.pathname}?${query}`, name, features);
  // A blocked pop-up. Worth a line rather than a silent nothing: the operator
  // clicked something and a browser decided for them, and nothing on the page
  // would otherwise say so.
  if (!opened) {
    console.warn(`the browser refused to open a window for ${title}`);
    return null;
  }
  tabs.set(name, opened);
  return opened;
};

/// Opens a group in a tab of its own, or brings the one already open forward.
///
/// The browser's half of `openGroupWindow`. The target name carries the group,
/// so a second request for the same group lands in the same tab — the same
/// guarantee the window label gives on the desktop, by the same mechanism of
/// naming the thing after what it shows.
export const openGroupTab = (group: GroupView, name: string) => {
  openTab(`${GROUP_WINDOW_PREFIX}${group}`, `${GROUP_PARAM}=${group}`, name);
};

/// Opens a view as a ticker in a pop-up of its own (P24), or brings the one
/// already open forward. The browser's half of `openTickerWindow`: `features`
/// carries where this view's ticker was last left — see `ticker.ts`.
export const openTickerTab = (view: GroupView, name: string, features: string) => {
  openTab(`${TICKER_WINDOW_PREFIX}${view}`, `${TICKER_PARAM}=${view}`, `the ${name} ticker`, features);
};

/// Opens the shoutbox in a pop-up of its own (P26), or brings it forward. The
/// size the app's shoutbox window opens at: the curve beside its fields.
export const openShoutboxTab = () => {
  openTab(SHOUTBOX_WINDOW, SHOUTBOX_WINDOW, "the shoutbox", "popup,width=760,height=520");
};

/// Closes every group tab and every ticker this page opened.
///
/// Called when the active profile changes, like its desktop counterpart: the
/// stream ids a view filters by are positions in the active profile's channel
/// list, so a tab carried over to another production would be filtering the
/// right numbers against the wrong channels. The shoutbox stays, as its window
/// does in the app: it follows the active profile and is no view of it.
export const closeGroupTabs = () => {
  for (const [name, tab] of tabs) {
    if (name === SHOUTBOX_WINDOW) continue;
    if (!tab.closed) tab.close();
    tabs.delete(name);
  }
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

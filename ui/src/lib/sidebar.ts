// How wide the chat's group sidebar is, and where that is remembered (P7).
//
// Remembered per screen rather than in `config.json`: the configuration is the
// production's setup and it reaches every view the LAN remote serves (P16), so
// a width stored there would let a phone in the auditorium decide how much of
// the desk's screen the sidebar takes. `localStorage` is the opposite — it
// belongs to the browser or webview that did the dragging and to no other.
//
// It is also the reason this is worth having at all. On the desk 12 rem is a
// reasonable guess; on a phone held in one hand it is a third of the screen
// spent on a list of eight names that is read once and then in the way.

/// Narrow enough to be nearly out of the way, wide enough that a row is still
/// a row: below this the names are ellipsis and the count, and an operator
/// dragging past it meant to close the sidebar, which is not what this does.
export const MIN_WIDTH = 96;

/// The largest share of the window the sidebar may take. A sidebar wider than
/// the chat is no longer a sidebar, and the chat is why the app is open — but
/// on a phone "most of the screen for a moment, to pick a group" is a real
/// thing to want, which is why this is not a half.
export const MAX_FRACTION = 0.6;

/// What it was before it could be dragged: 12 rem at the default font size.
/// Also where a double-click on the handle puts it back to.
export const DEFAULT_WIDTH = 192;

const KEY = "intercom2chat.sidebar-width";

/// The width brought inside what the current window can show.
///
/// Applied to a stored value as well as to a drag, because the window the
/// sidebar was measured in is not necessarily the one it is read back into: a
/// phone turned from landscape to portrait, or a window dragged to a smaller
/// screen, would otherwise restore a sidebar wider than the display.
export const clampWidth = (width: number, available: number): number => {
  if (!Number.isFinite(width)) return DEFAULT_WIDTH;

  return Math.min(maxWidth(available), Math.max(MIN_WIDTH, Math.round(width)));
};

/// The widest the sidebar may be in a window of this width — which is also
/// what the drag handle reports as the top of its range, so that what an
/// assistive technology reads out is the bound the drag is actually held to.
///
/// `Math.max` and not the fraction alone: a window narrower than twice the
/// minimum would otherwise produce an upper bound below the lower one, and the
/// sidebar would come back at a width nothing asked for.
export const maxWidth = (available: number): number =>
  Math.max(MIN_WIDTH, Math.round(available * MAX_FRACTION));

/// What a stored entry means, including the two ways there is none: never
/// written, and written by a version of this that stored something else.
///
/// Separate from the reading so that what the sidebar comes back at is decided
/// by a function with no browser in it — the storage call is three lines with
/// nothing to get wrong, and this is the part with the decisions.
export const readWidth = (stored: string | null, available: number): number =>
  clampWidth(stored === null ? DEFAULT_WIDTH : Number(stored), available);

/// The remembered width, or the default when there is none.
///
/// Storage is reached inside a `try`: a webview with storage disabled, or a
/// browser in a private window, throws on the *access* rather than answering
/// with nothing — and a chat that fails to render because it could not read a
/// sidebar width would be an absurd way to lose a session.
export const loadWidth = (available: number): number => {
  try {
    return readWidth(window.localStorage.getItem(KEY), available);
  } catch {
    return clampWidth(DEFAULT_WIDTH, available);
  }
};

/// Remembers a width for the next time this screen opens the chat. Silent when
/// it cannot, for the reason above: the sidebar is the right width now, which
/// is what the operator was actually asking for.
export const saveWidth = (width: number) => {
  try {
    window.localStorage.setItem(KEY, String(Math.round(width)));
  } catch {
    // No storage — the drag still took effect.
  }
};

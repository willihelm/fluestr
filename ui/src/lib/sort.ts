// Which end of the chat new lines arrive at, and who decides (P7).
//
// Two answers, deliberately, because there are two questions. **The default**
// is one setting for the whole app: it is stored in `config.json` beside the
// vocabulary, it reaches every view the LAN remote serves, and it is what a
// main window, a detached group window or a browser tab is *born* with. **The
// override** is this one view's — the desk reads the chat downwards while the
// stage manager's second screen may want the newest line under the eye at the
// top, and turning a window over is something done during a show, on the
// window in front of the operator.
//
// The override is kept in `localStorage`, like the sidebar width and for its
// reason: it is a property of the screen reading the chat, and one stored in
// the configuration would let a phone in the auditorium turn the desk's chat
// upside down. It is keyed per *view*, so the main window, each group window
// and the all-channels window each keep their own — which is the whole point
// of being able to override at all.

import type { StoredSortDirection } from "./bridge";
import type { GroupView } from "./windows";

/// Where the newest line is.
///
/// The stored type, re-exported under the name the UI uses: the default and an
/// override are the same choice made in two places, and two spellings of it
/// would be one conversion away from a window reading the opposite of what was
/// asked for.
export type SortDirection = StoredSortDirection;

/// What a configuration that has never held one is read as, and what the app
/// did before it could be turned over: newest at the bottom, which is what a
/// chat is. Kept in step with `SortDirection::default` on the Rust side.
export const DEFAULT_DIRECTION: SortDirection = "newest-bottom";

/// The other direction — what the toggle switches to, and how a distance from
/// the opposite edge is read.
export const flip = (direction: SortDirection): SortDirection =>
  direction === "newest-top" ? "newest-bottom" : "newest-top";

/// What a control says, in the words the manual test uses: where the newest
/// line is, not which way a list is sorted.
export const label = (direction: SortDirection): string =>
  direction === "newest-top" ? "Newest at top" : "Newest at bottom";

/// Where a view's override is kept, if it has one.
///
/// Per view rather than per screen: the main window and a group window on the
/// same screen are the two things an operator most wants to read in opposite
/// directions. The all-channels window is spelled the way it is everywhere
/// else — see `windows.ts` — so a key can be read by a human looking at
/// storage and matched to the window it belongs to.
export const overrideKey = (view: GroupView | null): string =>
  `intercom2chat.sort.${view === null ? "main" : `group-${view}`}`;

/// What a stored entry means, including the two ways there is none: never
/// written, and written by a version of this that stored something else.
///
/// Separate from the reading so that what a window opens with is decided by a
/// function with no browser in it — the storage call is three lines with
/// nothing to get wrong, and this is the part with the decision.
export const readOverride = (stored: string | null): SortDirection | null =>
  stored === "newest-top" || stored === "newest-bottom" ? stored : null;

/// This view's override, or `null` when it follows the app's default.
///
/// Storage is reached inside a `try`, like the sidebar's width: a webview with
/// storage disabled, or a browser in a private window, throws on the *access*
/// rather than answering with nothing — and a chat that fails to render
/// because it could not read a reading direction would be an absurd way to
/// lose a session.
export const loadOverride = (view: GroupView | null): SortDirection | null => {
  try {
    return readOverride(window.localStorage.getItem(overrideKey(view)));
  } catch {
    return null;
  }
};

/// Remembers this view's override. `null` removes the entry, giving the view
/// back to the app's default — nothing in the chat asks for that today, and it
/// is here so that forgetting an override is a removal rather than a third
/// value some later reader has to know about.
///
/// Silent when it cannot, for the reason above: the window is reading the way
/// they asked now, which is what they were actually asking for.
export const saveOverride = (view: GroupView | null, direction: SortDirection | null) => {
  try {
    if (direction === null) window.localStorage.removeItem(overrideKey(view));
    else window.localStorage.setItem(overrideKey(view), direction);
  } catch {
    // No storage — the override lasts as long as this window does.
  }
};

/// The bubbles of a page in the order this direction reads them.
///
/// The page itself is always taken from the newest end — the window is what
/// bounds the DOM, and which lines are *in* it is not a question of which way
/// round they are shown. This only turns the page over.
///
/// A copy, never a reverse in place: the array it is handed is derived state,
/// and reversing it would reorder the list every reader shares.
export const arrange = <T>(page: T[], direction: SortDirection): T[] =>
  direction === "newest-top" ? [...page].reverse() : page;

/// As much of a scroller as the two distances below need. An interface rather
/// than the element, so the arithmetic can be tested without a DOM.
export interface Extent {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

/// How far the viewport is from the end the newest line is at.
///
/// This is what "is the operator still following the chat" asks, and it is the
/// one piece of the follow behaviour that changes with the direction: at the
/// bottom in one direction, at the top in the other, and the rest of the chat
/// can then be written as if there were only one.
export const fromNewest = (extent: Extent, direction: SortDirection): number =>
  direction === "newest-top"
    ? Math.max(0, extent.scrollTop)
    : Math.max(0, extent.scrollHeight - extent.scrollTop - extent.clientHeight);

/// How far the viewport is from the end the history is at — the edge the next
/// page is added to, and therefore the one "reach for more before they get
/// there" watches.
export const fromOldest = (extent: Extent, direction: SortDirection): number =>
  fromNewest(extent, flip(direction));

/// The `scrollTop` that puts the newest line on screen. What "Jump to latest"
/// sets, and where the follow effect holds the view.
export const newestEdge = (extent: Extent, direction: SortDirection): number =>
  direction === "newest-top" ? 0 : extent.scrollHeight;

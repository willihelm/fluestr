// Whether a view shows the meter strip, and who decides.
//
// One answer, and it is the view's own: the strip is the tallest fixed thing
// above the chat, and a window parked in a corner of the desk — or a phone
// held in one hand — is often opened for the chat alone. Collapsing it is what
// makes those window sizes worth having.
//
// Kept in `localStorage`, per view, like the reading direction and for its
// reason: it is a property of the screen the chat is being read on, and a
// second window on the same desk may want the bars while this one does not.
// See `sort.ts` for the longer version of that argument.

import type { GroupView } from "./windows";

/// Where a view's answer is kept. Spelled like the sort override's key, so a
/// human reading storage can match both to the window they belong to.
export const collapseKey = (view: GroupView | null): string =>
  `fluestr.meters.${view === null ? "main" : `group-${view}`}`;

/// What a stored entry means. Only an explicit "collapsed" hides the strip:
/// anything else — never written, or written by a version of this that stored
/// something else — is a window that shows its meters, which is the state an
/// operator can see the way out of.
export const readCollapsed = (stored: string | null): boolean =>
  stored === "collapsed";

/// Whether this view opens with the strip collapsed.
///
/// Storage is reached inside a `try`, as everywhere else here: a webview with
/// storage disabled throws on the access rather than answering with nothing,
/// and losing the meters over it would be an absurd way to lose a session.
export const loadCollapsed = (view: GroupView | null): boolean => {
  try {
    return readCollapsed(window.localStorage.getItem(collapseKey(view)));
  } catch {
    return false;
  }
};

/// Remembers this view's answer. Silent when it cannot: the strip is the way
/// they just asked for it, which is what they were actually asking for.
export const saveCollapsed = (view: GroupView | null, collapsed: boolean) => {
  try {
    if (collapsed) window.localStorage.setItem(collapseKey(view), "collapsed");
    else window.localStorage.removeItem(collapseKey(view));
  } catch {
    // No storage — the choice lasts as long as this window does.
  }
};

// When a window asks to be looked at (P17).
//
// Lifted out of the main chat because there is now more than one window to ask
// for it. The main window and a detached group window watch different things —
// and they have to agree on *how* they count, or the same sentence flashes
// twice in one window and not at all in another.
//
// Two reasons, and they are not the same request:
//
//   - An **alert term** is urgent. The main window sees every one of them,
//     whichever group happens to be selected: it is the view of the whole
//     session, and a word that must not be missed must not be missed because
//     of what is on screen. A group window sees only its own members' — it is
//     not a view of the session and never claimed to be.
//   - A group's **flash-on-message** is the quieter request, and it follows the
//     view: only while that group is what the window shows.
//
// Counted per **bubble**, not per segment. One sentence arrives as three
// segments, and three flashes for one sentence is exactly the noise this
// feature must not add. What decides is whether the event that just arrived
// opened the last bubble, or brought the first alert term into it.

import type { Bubble } from "./bubbles";
import type { TranscriptEvent } from "./bridge";

/// What a window answers for.
export interface Watching {
  /// The streams whose alert terms this window is urgent about. `null` is
  /// every stream — the main window, which is the view of the whole session.
  alerts: Set<number> | null;
  /// The streams whose every new bubble this window flashes for, or `null`
  /// when the view has no flash setting to follow: the all-channels view, or a
  /// group with the setting off.
  flash: Set<number> | null;
}

/// Whether to ask, and how insistently. `null` is "do not ask".
///
/// `urgent` picks between the two requests the platform offers: an alert term
/// keeps asking until the app is looked at, a group's flash-on-message is a
/// single bounce. Never a sound, in either case — an intercom room is already
/// a room full of speech.
export const attentionFor = (
  event: TranscriptEvent,
  /// The last bubble of *this window's* chat, after the event was added. The
  /// event is in it either way; the question is what it did to it.
  last: Bubble | undefined,
  watching: Watching,
): { urgent: boolean } | null => {
  if (!last) return null;

  // The first alert term in this bubble: every range the bubble holds is one
  // this event brought. A later segment adding a second term to the same
  // bubble is the same sentence still being spoken, and it has already been
  // asked about.
  const urgent =
    event.alerts.length > 0 &&
    last.alerts.length === event.alerts.length &&
    (watching.alerts === null || watching.alerts.has(event.streamId));

  const flashing =
    last.parts === 1 && watching.flash !== null && watching.flash.has(event.streamId);

  return urgent || flashing ? { urgent } : null;
};

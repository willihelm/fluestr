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
//   - A group's **flash-on-message** is the loud one, and it follows the view:
//     only while that group is what the window shows. It is the whole window
//     in dark orange, asked for by the operator on the one view worth being
//     interrupted by — including the all-channels view, which carries the same
//     setting on the profile rather than on a group it does not have.
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
  /// The streams whose every new bubble floods this window in dark orange, or
  /// `null` when the view on screen has its flash setting off.
  ///
  /// A set either way, never "every stream": a view that floods for everything
  /// is the all-channels view with its setting on, and it knows its streams.
  /// The `null` here means "off", which is not what `alerts`'s `null` means —
  /// the two questions are different, and neither has a use for the other's
  /// shorthand.
  flash: Set<number> | null;
}

/// Whether to ask, and how. `null` is "do not ask".
///
/// `urgent` is an alert term: it picks the insisting one of the two requests
/// the platform offers, and keeps asking until the app is looked at. `loud` is
/// a group's flash-on-message: the whole window in dark orange, one pulse per
/// message. They are independent — a term matched on a group that flashes is
/// both — and never a sound, in either case: an intercom room is already a
/// room full of speech.
export const attentionFor = (
  event: TranscriptEvent,
  /// The last bubble of *this window's* chat, after the event was added. The
  /// event is in it either way; the question is what it did to it.
  last: Bubble | undefined,
  watching: Watching,
): { urgent: boolean; loud: boolean } | null => {
  if (!last) return null;

  // The first alert term in this bubble: every range the bubble holds is one
  // this event brought. A later segment adding a second term to the same
  // bubble is the same sentence still being spoken, and it has already been
  // asked about.
  const urgent =
    event.alerts.length > 0 &&
    last.alerts.length === event.alerts.length &&
    (watching.alerts === null || watching.alerts.has(event.streamId));

  const loud =
    last.parts === 1 && watching.flash !== null && watching.flash.has(event.streamId);

  return urgent || loud ? { urgent, loud } : null;
};

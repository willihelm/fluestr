// When a window asks to be looked at (P17, P19).
//
// What is asked for is no longer decided here: a rule's actions are worked out
// once, on the backend, for every view at the same time (`crates/app/src/
// rules.rs`). Two questions are left, and they are both properties of the
// *window* rather than of the rule:
//
//   - **does this window answer for it?** Two conditions for a flood, and
//     they are two different questions. The rule's *channels* say where the
//     words had to be said, and a window that is not showing that channel has
//     nothing to flood about. The rule's *views* say which windows it is for,
//     which is the question the channels cannot answer: a rule narrowed to
//     the stage manager's group used to flood every window that happened to
//     show one of its channels, the main window included, and narrowing the
//     trigger did nothing about it. A rule that names no view is for every
//     window showing the channel, which is what the flash always did.
//     An attention request is wider in the main window on purpose: that
//     window is the view of the whole session, so a rule that fires on a
//     channel the operator is not currently looking at still reaches them
//     there. A group window is not a view of the session and never claimed to
//     be.
//   - **has it been asked already?** Counted per **bubble**, not per segment.
//     One sentence arrives as three segments, and three flashes for one
//     sentence is exactly the noise this must not add. Bubbles are a view's
//     own — a channel outside a group does not interrupt a bubble in that
//     group's window — which is the other reason this question is answered
//     here and not on the backend.

import type { Bubble } from "./bubbles";
import type { TranscriptEvent } from "./bridge";
import type { Action } from "./generated/rules";
import { ALL_CHANNELS_VIEW, type GroupView } from "./windows";

/// What a window answers for.
export interface Watching {
  /// The streams whose attention requests this window makes. `null` is every
  /// stream — the main window, which is the view of the whole session.
  attention: Set<number> | null;
  /// The streams this window is currently showing. A window floods only for
  /// what is on screen. `null` is every stream: the all-channels view.
  shown: Set<number> | null;
  /// **Which view this window is**, as the rules address one: a group's
  /// position in the active profile, or `ALL_CHANNELS_VIEW`. The main window
  /// answers with whatever its sidebar has selected, because that is the view
  /// it currently is; a group window answers with the group it was opened
  /// for.
  ///
  /// The all-channels view is deliberately not "every group": a rule that
  /// names the stage manager's window means that window, and the view of
  /// everything is not it.
  view: GroupView;
}

/// How to ask the platform, or `null` for not at all.
///
/// `"urgent"` keeps asking until the app is looked at; `"once"` is a single
/// bounce. Never a sound, in either case: an intercom room is already a room
/// full of speech.
export type Ask = "urgent" | "once" | null;

/// Whether to ask, and how. `null` is "do nothing".
///
/// `loud` is the flood — the whole window in dark orange — and it is
/// independent of `ask`: a rule can ask for either, or for both, and the two
/// are the only reactions a window can make.
/// Whether this window is one of the ones the flash was addressed to.
///
/// `flashViews` is the union of the views every firing rule named, `null`
/// being "every window" — see `crates/app/src/rules.rs`. A window that is the
/// all-channels view is addressed only by that `null`: it is the view of
/// everything, not a stand-in for each group in turn.
const addressed = (event: TranscriptEvent, view: GroupView): boolean => {
  const views = event.flashViews ?? null;
  if (views === null) return true;
  return view !== ALL_CHANNELS_VIEW && views.includes(view);
};

export const attentionFor = (
  event: TranscriptEvent,
  /// The last bubble of *this window's* chat, after the event was added. The
  /// event is in it either way; the question is what it did to it.
  last: Bubble | undefined,
  watching: Watching,
): { ask: Ask; loud: boolean } | null => {
  if (!last) return null;

  const actions = event.actions ?? [];

  // Whether this event is the one that brought `action` into the bubble. The
  // bubble keeps every part's actions, so an action that occurs once came from
  // the segment that just arrived; a later segment asking for the same thing
  // is the same sentence still being spoken, and it has already been asked
  // about.
  const brought = (action: Action) =>
    actions.includes(action) && last.actions.filter((a) => a === action).length === 1;

  const answers = (streams: Set<number> | null) =>
    streams === null || streams.has(event.streamId);

  const ask: Ask =
    brought("urgentAttention") && answers(watching.attention)
      ? "urgent"
      : brought("attention") && answers(watching.attention)
        ? "once"
        : null;

  // Both questions, and in this order because they fail for different
  // reasons: `answers` is "this channel is not on screen here", `addressed`
  // is "this flash was meant for another window".
  const loud = brought("flash") && answers(watching.shown) && addressed(event, watching.view);

  return ask !== null || loud ? { ask, loud } : null;
};

import { describe, expect, it } from "vitest";
import { attentionFor } from "./attention";
import { toBubbles } from "./bubbles";
import type { Action, AlertMatch, TranscriptEvent } from "./bridge";
import { ALL_CHANNELS_VIEW } from "./windows";

let seq = 0;

/// A transcript as the backend hands it over, with the rules already applied:
/// the ranges to mark and the actions that were asked for (P19). What is being
/// tested here is only what a *window* then does with that.
const event = (
  streamId: number,
  text: string,
  actions: Action[] = [],
  alerts: AlertMatch[] = [],
  tStartMs = seq * 10_000,
  /// Which windows the flash is for, `null` being every window showing the
  /// channel — which is what an unaddressed rule compiles to.
  flashViews: number[] | null = null,
): TranscriptEvent => ({
  sessionId: 1,
  streamId,
  seq: seq++,
  label: `Stream ${streamId}`,
  tStartMs,
  tEndMs: tStartMs + 900,
  text,
  detectedLanguage: "de",
  decodeMs: 10,
  latencyMs: 20,
  alerts,
  actions,
  flashViews,
});

/// What the window would do, given the whole chat it is showing.
const after = (
  messages: TranscriptEvent[],
  watching: Parameters<typeof attentionFor>[2],
) => {
  const bubbles = toBubbles(messages);
  return attentionFor(messages[messages.length - 1], bubbles[bubbles.length - 1], watching);
};

/// The main window with nothing selected: it answers for every stream, it is
/// showing all of them, and the view it *is* is the view of everything.
const everything = { attention: null, shown: null, view: ALL_CHANNELS_VIEW };

const HIT: AlertMatch[] = [{ start: 0, end: 5 }];

describe("attentionFor", () => {
  it("says nothing about a line no rule fired on", () => {
    expect(after([event(0, "Vorhang in fünf")], everything)).toBeNull();
  });

  it("asks urgently when a rule asked it to", () => {
    expect(
      after([event(0, "Feuer im Schnürboden", ["highlight", "urgentAttention"], HIT)], everything),
    ).toEqual({ ask: "urgent", loud: false });
  });

  it("asks once for the quieter of the two requests", () => {
    expect(after([event(0, "Arzt", ["attention"])], everything)).toEqual({
      ask: "once",
      loud: false,
    });
  });

  // Urgent wins: a rule that keeps asking and a rule that asks once are one
  // window, and the louder request is the one that must not be swallowed.
  it("asks urgently when two rules ask differently on one segment", () => {
    expect(after([event(0, "Feuer", ["attention", "urgentAttention"])], everything)).toEqual({
      ask: "urgent",
      loud: false,
    });
  });

  // One sentence is three segments, and the rule fires on the one that carries
  // the word. The rest must not ask again.
  it("asks once per bubble, not once per segment", () => {
    const first = event(0, "Feuer im Schnürboden", ["urgentAttention"], HIT);
    const second = event(0, " wir räumen", [], [], first.tEndMs + 200);
    expect(after([first, second], everything)).toBeNull();
  });

  // A second match in the same breath is the same sentence still being spoken.
  it("stays quiet when a later segment fires the same rule into the same bubble", () => {
    const first = event(0, "Feuer im Schnürboden", ["urgentAttention"], HIT);
    const second = event(0, " Feuer", ["urgentAttention"], [{ start: 1, end: 6 }], first.tEndMs + 200);
    expect(after([first, second], everything)).toBeNull();
  });

  // …but an action this bubble has not asked for yet is a new request, even
  // in the middle of a sentence. Under P17 a flood could only ever come from
  // the segment that opened the bubble; a rule can fire on a word said three
  // segments in.
  it("flashes for a rule that fires partway through a bubble", () => {
    const first = event(0, "wir gehen jetzt", ["urgentAttention"], HIT);
    const second = event(0, " auf Cue fünf", ["flash"], [], first.tEndMs + 200);
    expect(after([first, second], everything)).toEqual({ ask: null, loud: true });
  });

  // A group window answers for its members and for nobody else.
  it("ignores an attention request for a channel the window does not show", () => {
    const watching = { attention: new Set([1]), shown: new Set([1]), view: 0 };
    expect(after([event(0, "Feuer", ["urgentAttention"], HIT)], watching)).toBeNull();
    expect(after([event(1, "Feuer", ["urgentAttention"], HIT)], watching)).toEqual({
      ask: "urgent",
      loud: false,
    });
  });

  it("floods for a rule that fired on a channel this view shows", () => {
    const watching = { attention: null, shown: new Set([0]), view: 0 };
    expect(after([event(0, "Vorhang", ["flash"])], watching)).toEqual({
      ask: null,
      loud: true,
    });
  });

  // The flood follows the view, which is the one thing about it that did not
  // change with P19: a window does not paint itself for something it is not
  // showing.
  it("does not flood for a channel this view does not show", () => {
    const watching = { attention: null, shown: new Set([1]), view: 0 };
    expect(after([event(0, "Vorhang", ["flash"])], watching)).toBeNull();
  });

  it("does not flood for a segment that only continues a bubble", () => {
    const first = event(0, "Vorhang", ["flash"]);
    const second = event(0, " in fünf", ["flash"], [], first.tEndMs + 200);
    expect(after([first, second], everything)).toBeNull();
  });

  // Both, because they are not alternatives: one keeps the platform asking,
  // the other paints the window. A segment that asks for both gets both.
  it("asks and floods when one segment fires rules that want each", () => {
    expect(after([event(0, "Feuer", ["flash", "urgentAttention"], HIT)], everything)).toEqual({
      ask: "urgent",
      loud: true,
    });
  });

  // Highlighting is in the text, not in the window: a rule that only marks its
  // word asks this window for nothing.
  it("says nothing for a rule that only highlights", () => {
    expect(after([event(0, "Feuer", ["highlight"], HIT)], everything)).toBeNull();
  });

  // The bug the addressing exists for. Narrowing a rule to a group used to
  // narrow only *when* it fired: every window showing the channel — the view
  // of everything included — still flooded, so restricting a rule to one
  // group looked like it did nothing at all.
  it("does not flood a window the flash was not addressed to", () => {
    const mds = { attention: new Set([0]), shown: new Set([0]), view: 1 };
    expect(after([event(0, "Test", ["flash"], [], undefined, [0])], mds)).toBeNull();
  });

  it("floods the window the flash names", () => {
    const stage = { attention: new Set([0]), shown: new Set([0]), view: 0 };
    expect(after([event(0, "Test", ["flash"], [], undefined, [0])], stage)).toEqual({
      ask: null,
      loud: true,
    });
  });

  // The view of everything is not a stand-in for each group in turn: a rule
  // that names the stage manager's window means that window.
  it("leaves the all-channels view out of an addressed flash", () => {
    expect(after([event(0, "Test", ["flash"], [], undefined, [0])], everything)).toBeNull();
  });

  // …but the main window *is* that view while it is showing that group, and
  // it flashes then. The window is which view it is, not which window it is.
  it("floods the main window while it is showing the group that was named", () => {
    const showingStage = { attention: null, shown: new Set([0]), view: 0 };
    expect(after([event(0, "Test", ["flash"], [], undefined, [0])], showingStage)).toEqual({
      ask: null,
      loud: true,
    });
  });

  // An unaddressed flash is the old behaviour and stays reachable.
  it("floods every window showing the channel when no window was named", () => {
    expect(after([event(0, "Vorhang", ["flash"])], everything)).toEqual({
      ask: null,
      loud: true,
    });
  });

  // Addressing is the flash's alone. An attention request is the app's, and
  // a window that answers for the channel still makes it.
  it("still asks for attention from a window the flash was not addressed to", () => {
    const mds = { attention: new Set([0]), shown: new Set([0]), view: 1 };
    expect(after([event(0, "Test", ["flash", "urgentAttention"], [], undefined, [0])], mds)).toEqual(
      { ask: "urgent", loud: false },
    );
  });

  it("says nothing when there is no chat yet", () => {
    expect(attentionFor(event(0, "Vorhang", ["flash"]), undefined, everything)).toBeNull();
  });
});

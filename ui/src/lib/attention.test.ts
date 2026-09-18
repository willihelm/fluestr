import { describe, expect, it } from "vitest";
import { attentionFor } from "./attention";
import { toBubbles } from "./bubbles";
import type { AlertMatch, TranscriptEvent } from "./bridge";

let seq = 0;

const event = (
  streamId: number,
  text: string,
  alerts: AlertMatch[] = [],
  tStartMs = seq * 10_000,
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
});

/// What the window would do, given the whole chat it is showing.
const after = (
  messages: TranscriptEvent[],
  watching: Parameters<typeof attentionFor>[2],
) => {
  const bubbles = toBubbles(messages);
  return attentionFor(messages[messages.length - 1], bubbles[bubbles.length - 1], watching);
};

const everything = { alerts: null, flash: null };

describe("attentionFor", () => {
  it("says nothing about an ordinary line", () => {
    expect(after([event(0, "Vorhang in fünf")], everything)).toBeNull();
  });

  it("is urgent about an alert term", () => {
    expect(after([event(0, "Feuer im Schnürboden", [{ start: 0, end: 5 }])], everything)).toEqual({
      urgent: true,
    });
  });

  // One sentence is three segments, and the term is only found once. The
  // second segment must not ask again.
  it("asks once per bubble, not once per segment", () => {
    const first = event(0, "Feuer im Schnürboden", [{ start: 0, end: 5 }]);
    const second = event(0, " wir räumen", [], first.tEndMs + 200);
    expect(after([first, second], everything)).toBeNull();
  });

  // A second term in the same breath is the same sentence still being spoken.
  it("stays quiet when a later segment brings another term into the same bubble", () => {
    const first = event(0, "Feuer im Schnürboden", [{ start: 0, end: 5 }]);
    const second = event(0, " Feuer", [{ start: 1, end: 6 }], first.tEndMs + 200);
    expect(after([first, second], everything)).toBeNull();
  });

  // A group window answers for its members and for nobody else.
  it("ignores an alert term on a channel the window does not show", () => {
    const watching = { alerts: new Set([1]), flash: null };
    expect(after([event(0, "Feuer", [{ start: 0, end: 5 }])], watching)).toBeNull();
    expect(after([event(1, "Feuer", [{ start: 0, end: 5 }])], watching)).toEqual({
      urgent: true,
    });
  });

  it("flashes for a new bubble from a group that asked for it", () => {
    const watching = { alerts: null, flash: new Set([0]) };
    expect(after([event(0, "Vorhang")], watching)).toEqual({ urgent: false });
  });

  it("does not flash for a segment that only continues a bubble", () => {
    const watching = { alerts: null, flash: new Set([0]) };
    const first = event(0, "Vorhang");
    const second = event(0, " in fünf", [], first.tEndMs + 200);
    expect(after([first, second], watching)).toBeNull();
  });

  it("does not flash for a channel outside the group", () => {
    const watching = { alerts: null, flash: new Set([1]) };
    expect(after([event(0, "Vorhang")], watching)).toBeNull();
  });

  // Urgency wins: an alert term in a flashing group is still the request that
  // keeps asking.
  it("is urgent when a bubble both opens and carries a term", () => {
    const watching = { alerts: null, flash: new Set([0]) };
    expect(after([event(0, "Feuer", [{ start: 0, end: 5 }])], watching)).toEqual({
      urgent: true,
    });
  });

  it("says nothing when there is no chat yet", () => {
    expect(attentionFor(event(0, "Vorhang"), undefined, everything)).toBeNull();
  });
});

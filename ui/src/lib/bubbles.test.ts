import { describe, expect, it } from "vitest";
import {
  MAX_CHARS,
  MAX_GAP_MS,
  MAX_SPAN_MS,
  audible,
  mergeHistory,
  placeholders,
  toBubbles,
} from "./bubbles";
import type { SpeechActivity, TranscriptEvent } from "./bridge";

// A transcript event with only the fields that decide grouping spelled out.
// Everything else is noise for these tests and would hide what each one is
// actually varying.
let seq = 0;
const event = (e: Partial<TranscriptEvent> & { tStartMs: number }): TranscriptEvent => ({
  sessionId: 1,
  streamId: 0,
  flashViews: null,
  seq: seq++,
  label: "Regie",
  tEndMs: e.tStartMs + 1000,
  text: "Standby",
  detectedLanguage: "de",
  decodeMs: 300,
  latencyMs: 330,
  alerts: [],
  actions: [],
  audio: true,
  ...e,
});

describe("toBubbles", () => {
  it("merges what one person said without a pause into one bubble", () => {
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000, text: "Standby für Cue fünf" }),
      event({ tStartMs: 1200, tEndMs: 2000, text: "dann fade out" }),
    ]);

    expect(bubbles).toHaveLength(1);
    expect(bubbles[0].text).toBe("Standby für Cue fünf dann fade out");
    expect(bubbles[0].parts).toBe(2);
    expect(bubbles[0].tEndMs).toBe(2000);
  });

  it("starts a new bubble after a real silence", () => {
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000 }),
      event({ tStartMs: 1000 + MAX_GAP_MS + 1 }),
    ]);

    expect(bubbles).toHaveLength(2);
  });

  it("measures the gap between speech, not between decodes", () => {
    // Exactly at the threshold still continues: the boundary belongs to the
    // side that keeps an utterance whole.
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000, latencyMs: 4000 }),
      event({ tStartMs: 1000 + MAX_GAP_MS }),
    ]);

    expect(bubbles).toHaveLength(1);
  });

  it("never merges across channels or across sessions", () => {
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000, streamId: 0 }),
      event({ tStartMs: 1100, tEndMs: 2000, streamId: 1 }),
      event({ tStartMs: 2100, sessionId: 2, streamId: 1 }),
    ]);

    expect(bubbles).toHaveLength(3);
  });

  it("does not grow a bubble that another channel has already interrupted", () => {
    // The whole reason merging only ever touches the *last* bubble: with eight
    // channels interleaved the list is read as a chronology, and a box that
    // grows above a later line breaks that.
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: 500, streamId: 0, text: "Standby" }),
      event({ tStartMs: 600, tEndMs: 900, streamId: 1, text: "verstanden" }),
      event({ tStartMs: 1000, streamId: 0, text: "und Go" }),
    ]);

    expect(bubbles).toHaveLength(3);
    expect(bubbles[0].text).toBe("Standby");
    expect(bubbles[2].text).toBe("und Go");
  });

  it("stops an open talkback from growing one unskimmable wall", () => {
    const long = "x".repeat(MAX_CHARS);
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000, text: long }),
      event({ tStartMs: 1100, text: "weiter" }),
    ]);

    expect(bubbles).toHaveLength(2);
  });

  it("caps how long one bubble may run", () => {
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: MAX_SPAN_MS }),
      event({ tStartMs: MAX_SPAN_MS + 100 }),
    ]);

    expect(bubbles).toHaveLength(2);
  });

  it("keeps every language a merged bubble was spoken in", () => {
    // DE/EN code switching is the normal case here, so a merged bubble
    // genuinely can be both and picking one would be a guess.
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000, detectedLanguage: "de" }),
      event({ tStartMs: 1100, tEndMs: 1800, detectedLanguage: "en" }),
      event({ tStartMs: 1900, detectedLanguage: "de" }),
    ]);

    expect(bubbles[0].languages).toEqual(["de", "en"]);
  });

  it("reports the freshness of the text at the bottom", () => {
    // The latency badge is read as "how current is what I am looking at",
    // which is the newest part's number and not the first one's.
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000, latencyMs: 900, decodeMs: 850 }),
      event({ tStartMs: 1100, latencyMs: 320, decodeMs: 300 }),
    ]);

    expect(bubbles[0].latencyMs).toBe(320);
    expect(bubbles[0].decodeMs).toBe(300);
  });

  it("keeps the key of the segment that opened the bubble", () => {
    // Svelte keys the list on it. If it followed the newest part instead,
    // every merge would re-key the box and throw away the DOM node — and with
    // it the scroll position the operator is reading.
    const first = event({ tStartMs: 0, tEndMs: 1000 });
    const before = toBubbles([first])[0].key;
    const after = toBubbles([first, event({ tStartMs: 1100 })])[0].key;

    expect(after).toBe(before);
  });

  it("does not leave a seam where a part was empty", () => {
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000, text: "Standby" }),
      event({ tStartMs: 1100, tEndMs: 1500, text: "   " }),
      event({ tStartMs: 1600, text: "Go" }),
    ]);

    expect(bubbles[0].text).toBe("Standby Go");
  });
});

describe("alert ranges through the merge", () => {
  it("moves a later segment's ranges to where its text landed", () => {
    // The ranges were matched against the segment. In the bubble that segment
    // sits behind everything already there and the space that joined them —
    // and a highlight one word out is worse than none.
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000, text: "Standby für Cue fünf" }),
      event({
        tStartMs: 1200,
        tEndMs: 2000,
        text: " Feuer im Saal",
        // As the backend matched them: against the untrimmed segment, whose
        // leading space whisper puts there.
        alerts: [{ start: 1, end: 6 }],
      }),
    ]);

    expect(bubbles).toHaveLength(1);
    const { text, alerts } = bubbles[0];
    expect(text.slice(alerts[0].start, alerts[0].end)).toBe("Feuer");
  });

  it("keeps the ranges of every part that had one", () => {
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000, text: "Feuer", alerts: [{ start: 0, end: 5 }] }),
      event({ tStartMs: 1200, tEndMs: 2000, text: "und ein Arzt", alerts: [{ start: 8, end: 12 }] }),
    ]);

    const { text, alerts } = bubbles[0];
    expect(alerts.map((a) => text.slice(a.start, a.end))).toEqual(["Feuer", "Arzt"]);
  });

  it("leaves a bubble without matches without ranges", () => {
    expect(toBubbles([event({ tStartMs: 0 })])[0].alerts).toEqual([]);
  });
});

// A window opening onto a chat that is already running (P7): the backend's
// history, and whatever arrived while it was being asked for.
describe("mergeHistory", () => {
  it("puts the history in front of what arrived live", () => {
    const history = [event({ tStartMs: 0, text: "eins" })];
    const live = [event({ tStartMs: 10_000, text: "zwei" })];
    expect(mergeHistory(history, live).map((e) => e.text)).toEqual(["eins", "zwei"]);
  });

  // The window subscribes before it asks, so a segment can be in both answers
  // — and two bubbles with one key throws and stops the list rendering.
  it("drops a segment the window already saw arrive", () => {
    const both = event({ tStartMs: 0, text: "eins" });
    expect(mergeHistory([both], [both])).toEqual([both]);
  });

  // `seq` repeats across sessions, so identity is all three parts of it.
  it("keeps two segments that share a sequence number across sessions", () => {
    const first = { ...event({ tStartMs: 0 }), sessionId: 1, streamId: 0, seq: 4 };
    const second = { ...event({ tStartMs: 0 }), sessionId: 2, streamId: 0, seq: 4 };
    expect(mergeHistory([first], [second])).toEqual([first, second]);
  });

  it("is the history alone when nothing arrived in between", () => {
    const history = [event({ tStartMs: 0 })];
    expect(mergeHistory(history, [])).toEqual(history);
  });
});

describe("replaying a bubble (P22)", () => {
  it("asks for every part of the bubble, in order", () => {
    const [bubble] = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000, seq: 4 }),
      event({ tStartMs: 1200, tEndMs: 2000, seq: 5 }),
    ]);
    expect(bubble.segments).toEqual([
      { sessionId: 1, streamId: 0, seq: 4 },
      { sessionId: 1, streamId: 0, seq: 5 },
    ]);
  });

  it("has no button if any part arrived without its audio", () => {
    const [bubble] = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000 }),
      event({ tStartMs: 1200, tEndMs: 2000, audio: false }),
    ]);
    expect(audible(bubble, null)).toBe(false);
  });

  it("loses its button once its first part is behind the horizon", () => {
    const [bubble] = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000 }),
      event({ tStartMs: 1200, tEndMs: 2000 }),
    ]);
    expect(audible(bubble, null)).toBe(true);
    expect(audible(bubble, { sessionId: 1, tEndMs: 1000 })).toBe(true);
    expect(audible(bubble, { sessionId: 1, tEndMs: 1001 })).toBe(false);
    // An earlier session's line says nothing about this one; a later one
    // says everything.
    expect(audible(bubble, { sessionId: 0, tEndMs: 99_999 })).toBe(true);
    expect(audible(bubble, { sessionId: 2, tEndMs: 0 })).toBe(false);
  });
});

describe("placeholders", () => {
  const talking = (a: Partial<SpeechActivity> & { sinceMs: number }): SpeechActivity => ({
    sessionId: 1,
    streamId: 0,
    label: "Regie",
    speaking: true,
    decoding: false,
    preview: "",
    ...a,
  });

  it("puts the indicator on the last bubble when the same speaker carries on", () => {
    const bubbles = toBubbles([event({ tStartMs: 0, tEndMs: 1000 })]);
    const a = talking({ sinceMs: 1000 + MAX_GAP_MS });
    expect(placeholders(bubbles, [a])).toEqual({ onLast: a, standalone: [] });
  });

  it("gives a channel a box of its own after a real pause", () => {
    const bubbles = toBubbles([event({ tStartMs: 0, tEndMs: 1000 })]);
    const a = talking({ sinceMs: 1001 + MAX_GAP_MS });
    expect(placeholders(bubbles, [a])).toEqual({ onLast: null, standalone: [a] });
  });

  it("never attaches to another channel's bubble, and orders boxes by who began first", () => {
    const bubbles = toBubbles([event({ tStartMs: 0, tEndMs: 1000, streamId: 2 })]);
    const later = talking({ streamId: 0, sinceMs: 1400 });
    const earlier = talking({ streamId: 1, sinceMs: 1200 });
    expect(placeholders(bubbles, [later, earlier])).toEqual({
      onLast: null,
      standalone: [earlier, later],
    });
  });

  it("does not attach to a bubble that is already full", () => {
    const bubbles = toBubbles([
      event({ tStartMs: 0, tEndMs: 1000, text: "x".repeat(MAX_CHARS) }),
    ]);
    expect(placeholders(bubbles, [talking({ sinceMs: 1100 })]).onLast).toBeNull();
  });

  it("does not attach across sessions", () => {
    const bubbles = toBubbles([event({ tStartMs: 0, tEndMs: 1000 })]);
    expect(placeholders(bubbles, [talking({ sessionId: 2, sinceMs: 1100 })]).onLast).toBeNull();
  });

  it("shows nothing when nobody is talking", () => {
    expect(placeholders([], [])).toEqual({ onLast: null, standalone: [] });
  });
});

// Turning transcript events into chat bubbles.
//
// A segment is what the VAD cut, not what a person said: one sentence usually
// arrives as two or three of them, and one bubble per segment shreds a
// sentence across three boxes with three timestamps. Speech that runs on
// belongs in one bubble.
//
// Merging is only ever *appending to the last bubble in the list*. A stream
// whose earlier bubble has meanwhile been followed by another channel's line
// starts a new one instead of growing upwards: with eight channels interleaved
// the list is read as a chronology, and a box that grows above a later line
// breaks that. The cost is a split bubble where a channel was interrupted,
// which is honest — someone else did speak in between.

import type { TranscriptEvent } from "./bridge";

/// Silence that ends a bubble. Below this the speaker simply carried on;
/// above it, the next segment is a new turn even from the same channel.
/// Measured between the previous segment's end and the next one's start, so
/// it is real silence, not decode latency.
export const MAX_GAP_MS = 1_500;

/// Ceilings on how far a bubble may run on. Without them a channel that is
/// never silent — an open talkback, a noisy line the VAD keeps triggering —
/// grows one unbounded wall of text that cannot be skimmed or timestamped.
export const MAX_CHARS = 320;
export const MAX_SPAN_MS = 30_000;

export interface Bubble {
  /// Stable across re-grouping: the identity of the segment that opened it.
  key: string;
  sessionId: number;
  streamId: number;
  label: string;
  tStartMs: number;
  tEndMs: number;
  text: string;
  /// Every language the parts were detected as, in order of first appearance.
  /// A list rather than one value because DE/EN code switching is the normal
  /// case here — a merged bubble genuinely can be both, and picking one of
  /// them to display would be a guess.
  languages: string[];
  /// Of the most recent part: how fresh the text at the bottom is, which is
  /// what the number is read for.
  decodeMs: number;
  latencyMs: number;
  parts: number;
}

const keyOf = (e: TranscriptEvent) => `${e.sessionId}-${e.streamId}-${e.seq}`;

const open = (e: TranscriptEvent): Bubble => ({
  key: keyOf(e),
  sessionId: e.sessionId,
  streamId: e.streamId,
  label: e.label,
  tStartMs: e.tStartMs,
  tEndMs: e.tEndMs,
  text: e.text.trim(),
  languages: e.detectedLanguage ? [e.detectedLanguage] : [],
  decodeMs: e.decodeMs,
  latencyMs: e.latencyMs,
  parts: 1,
});

/// Whether `e` continues `bubble` rather than starting a new one.
const continues = (bubble: Bubble, e: TranscriptEvent): boolean =>
  bubble.sessionId === e.sessionId &&
  bubble.streamId === e.streamId &&
  e.tStartMs - bubble.tEndMs <= MAX_GAP_MS &&
  bubble.tEndMs - bubble.tStartMs < MAX_SPAN_MS &&
  bubble.text.length < MAX_CHARS;

const append = (bubble: Bubble, e: TranscriptEvent): Bubble => {
  const next = e.text.trim();
  return {
    ...bubble,
    tEndMs: e.tEndMs,
    // A space, never a newline: the parts are one utterance, and the joins
    // should be invisible. Whisper's own leading space is trimmed off both
    // sides so the seam does not double up.
    text: next === "" ? bubble.text : `${bubble.text} ${next}`.trim(),
    languages:
      e.detectedLanguage && !bubble.languages.includes(e.detectedLanguage)
        ? [...bubble.languages, e.detectedLanguage]
        : bubble.languages,
    decodeMs: e.decodeMs,
    latencyMs: e.latencyMs,
    parts: bubble.parts + 1,
  };
};

/// Groups a chronological list of transcript events into bubbles.
///
/// Pure and recomputed from the whole list rather than kept as state, so there
/// is one definition of what a bubble is and no way for an incrementally
/// maintained copy to disagree with it. The cost is a pass over the session's
/// events per new event — the same order as the append the chat already does,
/// and it goes away with the virtualised list that P7 still owes.
export const toBubbles = (messages: TranscriptEvent[]): Bubble[] => {
  const bubbles: Bubble[] = [];

  for (const e of messages) {
    const last = bubbles[bubbles.length - 1];
    if (last && continues(last, e)) {
      bubbles[bubbles.length - 1] = append(last, e);
    } else {
      bubbles.push(open(e));
    }
  }

  return bubbles;
};

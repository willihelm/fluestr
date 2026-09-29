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

import { shift } from "./alerts";
import type { AlertMatch, ClipKey, Horizon, SpeechActivity, TranscriptEvent } from "./bridge";
import type { Action } from "./generated/rules";

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
  /// Where the highlights are in `text` (P17, P19), in UTF-16 code units.
  ///
  /// Carried through the merge rather than matched again here: the ranges came
  /// from the backend's one matcher, and a bubble is its segments joined, so
  /// each part's ranges move along by whatever was joined in front of them.
  alerts: AlertMatch[];
  /// What the rules asked for, across every part of the bubble (P19).
  ///
  /// Every part's actions, duplicates and all, rather than a set: what a
  /// window needs to know is not *whether* this bubble asked for a flood but
  /// whether the segment that just arrived is the one that asked first. One
  /// occurrence means it is — see `attention.ts`.
  actions: Action[];
  /// Of the most recent part: how fresh the text at the bottom is, which is
  /// what the number is read for.
  decodeMs: number;
  latencyMs: number;
  parts: number;
  /// Every segment the bubble is made of, in order — what its replay asks
  /// the backend for (P22).
  segments: ClipKey[];
  /// Whether every part's audio was held when it arrived. Whether it still
  /// is depends on the horizon — see `audible`.
  audio: boolean;
  /// When the first part ended: the oldest audio of the bubble, and so the
  /// first of it the store lets go of.
  firstEndMs: number;
}

/// What identifies one segment.
///
/// All three parts: `seq` starts again at zero with every session and is only
/// unique per stream, so `(streamId, seq)` repeats as soon as a second session
/// runs — and Start is non-destructive, so the chat holds lines from several of
/// them at once.
export const segmentKey = (e: TranscriptEvent) =>
  `${e.sessionId}-${e.streamId}-${e.seq}`;

const keyOf = segmentKey;

/// A segment's alert ranges, placed where its text lands in the bubble.
///
/// Two moves in one: the text is trimmed before it goes in — whisper puts a
/// space in front of everything it returns — and then it sits `at` characters
/// into the joined string. The ranges were matched against the untrimmed
/// segment, so they have to make both.
///
/// `?? []` because an event from a backend that predates P17 has no ranges at
/// all, and the chat is the wrong place to discover that.
const placed = (e: TranscriptEvent, at: number): AlertMatch[] =>
  shift(e.alerts ?? [], at - (e.text.length - e.text.trimStart().length));

const open = (e: TranscriptEvent): Bubble => ({
  key: keyOf(e),
  sessionId: e.sessionId,
  streamId: e.streamId,
  label: e.label,
  tStartMs: e.tStartMs,
  tEndMs: e.tEndMs,
  text: e.text.trim(),
  languages: e.detectedLanguage ? [e.detectedLanguage] : [],
  alerts: placed(e, 0),
  // `?? []` for `alerts`' reason: an event from a backend that predates P19
  // has no actions, and the chat is the wrong place to discover that.
  actions: [...(e.actions ?? [])],
  decodeMs: e.decodeMs,
  latencyMs: e.latencyMs,
  parts: 1,
  segments: [{ sessionId: e.sessionId, streamId: e.streamId, seq: e.seq }],
  // `?? false` for `alerts`' reason: a backend that predates P22 keeps no
  // audio at all.
  audio: e.audio ?? false,
  firstEndMs: e.tEndMs,
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
  // Where this segment's text lands in the joined string: after what is
  // already there and the single space between them. Nothing is appended for
  // an empty segment, so its ranges — there are none — do not move either.
  const offset = next === "" ? 0 : bubble.text.length + 1;
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
    alerts: [...bubble.alerts, ...placed(e, offset)],
    actions: [...bubble.actions, ...(e.actions ?? [])],
    decodeMs: e.decodeMs,
    latencyMs: e.latencyMs,
    parts: bubble.parts + 1,
    segments: [...bubble.segments, { sessionId: e.sessionId, streamId: e.streamId, seq: e.seq }],
    // All or nothing: half a bubble played as though it were the whole of
    // it would be worse than no button.
    audio: bubble.audio && (e.audio ?? false),
  };
};

/// Groups a chronological list of transcript events into bubbles.
///
/// Pure and recomputed from the whole list rather than kept as state, so there
/// is one definition of what a bubble is and no way for an incrementally
/// maintained copy to disagree with it. The cost is a pass over the session's
/// events per new event — the same order as the append the chat already does,
/// and unaffected by the chat rendering only a window of the result: the
/// grouping has to see the events before it can know which of them are the
/// last two hundred bubbles. Segments arrive at a speaking pace, so a pass
/// over an hour of them is still far below what one event's budget allows;
/// what would change that is loading a session from disk (P10), and the fix
/// there is to group once on load rather than to abandon recomputing.
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

/// The chat a window opens with: what the backend had, plus whatever arrived
/// while it was being asked for.
///
/// A window subscribes to the live events *before* it asks for the history,
/// because the other order drops whatever is said in between — and the gap is
/// exactly one round trip, at the moment a group window is opened because
/// something is happening. The cost of subscribing first is that a segment can
/// be in both answers, and a duplicate is not a cosmetic problem here: two
/// bubbles with one key throws and stops the list rendering entirely.
///
/// What arrived live wins its position, so the order stays the order the window
/// saw.
export const mergeHistory = (
  history: TranscriptEvent[],
  live: TranscriptEvent[],
): TranscriptEvent[] => {
  const seen = new Set(live.map(segmentKey));
  return [...history.filter((e) => !seen.has(segmentKey(e))), ...live];
};

/// Whether a bubble's audio can still be heard again (P22).
///
/// The store drops the oldest audio first across every channel, so what is
/// gone is exactly what ended before one line — the horizon. A bubble is
/// audible if its audio was kept when it arrived and its *first* part ended
/// on or after that line; one part gone is the whole bubble gone.
export const audible = (bubble: Bubble, horizon: Horizon | null): boolean => {
  if (!bubble.audio) return false;
  if (horizon === null) return true;
  if (bubble.sessionId !== horizon.sessionId) return bubble.sessionId > horizon.sessionId;
  return bubble.firstEndMs >= horizon.tEndMs;
};

/// Where the "speaking…" placeholders go (P23).
///
/// A channel whose words will join the last bubble — the same speaker carrying
/// on, by the rule `continues` applies to a segment — shows the indicator on
/// that bubble rather than in a box of its own. Every other active channel
/// gets a placeholder at the newest end, oldest activity first, which is
/// where and in which order their bubbles will land.
///
/// Judged by when the activity began rather than by the segment it will
/// become, which does not exist yet. The segment starts a pre-roll earlier
/// than the gate opened, so this errs towards a box of its own; the cost is a
/// placeholder whose text then lands one bubble up.
export const placeholders = (
  bubbles: Bubble[],
  active: SpeechActivity[],
): { onLast: SpeechActivity | null; standalone: SpeechActivity[] } => {
  const last = bubbles[bubbles.length - 1];
  const joins = (a: SpeechActivity) =>
    last !== undefined &&
    last.sessionId === a.sessionId &&
    last.streamId === a.streamId &&
    a.sinceMs - last.tEndMs <= MAX_GAP_MS &&
    last.tEndMs - last.tStartMs < MAX_SPAN_MS &&
    last.text.length < MAX_CHARS;

  const onLast = active.find(joins) ?? null;
  const standalone = active
    .filter((a) => a !== onLast)
    .sort((a, b) => a.sinceMs - b.sinceMs || a.streamId - b.streamId);
  return { onLast, standalone };
};

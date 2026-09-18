// Alert terms in the chat: the list an operator edits, and the pieces a view
// renders (P17).
//
// What is deliberately *not* here is the matching. The ranges arrive with the
// transcript event, matched once on the backend (`crates/app/src/alerts.rs`),
// so the main chat, a detached group window and the LAN viewer (P16) highlight
// the same characters. A second matcher here would agree with that one right
// up to the first term with an umlaut in it.
//
// The offsets are UTF-16 code units, which is what `slice` counts — so they
// are used exactly as they arrive.

import type { AlertMatch } from "./bridge";

/// The terms as the backend wants them: trimmed, without blanks, without
/// duplicates.
///
/// The editor keeps a row per term and an empty row is how a term is added, so
/// blanks are the normal state of the list being edited rather than a mistake
/// — dropping them here is what keeps a half-typed row out of the file and out
/// of the matcher.
export const cleanTerms = (terms: string[]): string[] => {
  const kept: string[] = [];
  for (const term of terms) {
    const trimmed = term.trim();
    if (trimmed === "") continue;
    // Case-insensitively, because the matcher is: "Feuer" and "feuer" are one
    // term, and showing both in the list suggests they are not.
    if (kept.some((k) => k.toLowerCase() === trimmed.toLowerCase())) continue;
    kept.push(trimmed);
  }
  return kept;
};

/// Whether two lists say the same thing, for deciding whether the compiled
/// list on the backend is still current. Order counts — it is cheaper to
/// recompile a reordered list than to explain when it does not matter.
export const sameTerms = (a: string[], b: string[]): boolean =>
  a.length === b.length && a.every((term, i) => term === b[i]);

/// One piece of a transcript on its way into the DOM.
export interface Piece {
  text: string;
  /// Whether this piece is an alert term and gets the highlight.
  alert: boolean;
}

/// Splits a line into highlighted and plain pieces.
///
/// Defensive about the ranges rather than trusting them: they were matched
/// against the text of *one segment*, and a bubble is several segments joined
/// (see `bubbles.ts`). A range that survived a re-grouping bug as an offset
/// into the wrong string must cost a highlight, never the line itself — the
/// text is why the chat exists.
export const pieces = (text: string, alerts: AlertMatch[]): Piece[] => {
  if (alerts.length === 0) return [{ text, alert: false }];

  const result: Piece[] = [];
  let at = 0;

  for (const { start, end } of alerts) {
    const from = Math.max(at, Math.min(start, text.length));
    const to = Math.max(from, Math.min(end, text.length));
    if (to === from) continue;

    if (from > at) result.push({ text: text.slice(at, from), alert: false });
    result.push({ text: text.slice(from, to), alert: true });
    at = to;
  }

  if (at < text.length) result.push({ text: text.slice(at), alert: false });

  return result;
};

/// The same ranges, moved along by `offset` code units.
///
/// Used when segments are joined into one bubble: the ranges were matched
/// against the segment, and the bubble is that segment somewhere in the
/// middle of a longer string.
export const shift = (alerts: AlertMatch[], offset: number): AlertMatch[] =>
  offset === 0 ? alerts : alerts.map((a) => ({ start: a.start + offset, end: a.end + offset }));

// Highlighted words in the chat: the words an operator writes, and the pieces
// a view renders (P17, P19).
//
// What is deliberately *not* here is the matching. The ranges arrive with the
// transcript event, matched once on the backend (`crates/app/src/alerts.rs`)
// by whichever rules asked to highlight, so the main chat, a detached group
// window and the LAN viewer (P16) highlight the same characters. A second
// matcher here would agree with that one right up to the first term with an
// umlaut in it.
//
// The offsets are UTF-16 code units, which is what `slice` counts — so they
// are used exactly as they arrive. Drawing them over a line is `search.ts`'
// job, together with the hits of whatever the operator is looking for (P9):
// the two kinds of highlight land on one string and are cut out of it in one
// pass.

import type { AlertMatch } from "./bridge";

/// Two words the compiled matcher would not tell apart
/// (`crates/app/src/alerts.rs`).
const same = (a: string, b: string): boolean =>
  a.normalize("NFC").toLowerCase() === b.normalize("NFC").toLowerCase();

/// A rule's words as the backend wants them: trimmed, without blanks, without
/// duplicates.
///
/// The editor's field is being typed into, so blanks and half-written repeats
/// are the normal state of a list being edited rather than a mistake —
/// dropping them here is what keeps them out of the file and out of the
/// matcher.
export const cleanTerms = (terms: string[]): string[] => {
  const kept: string[] = [];
  for (const term of terms) {
    const trimmed = term.trim();
    if (trimmed === "") continue;
    // Case-insensitively and normalised, because the matcher is both: "Feuer"
    // and "feuer" are one term, and so are the two spellings of "Bühne" that
    // differ only in whether the umlaut is its own character. Showing both in
    // the list suggests they do different things — and in the second case the
    // operator cannot even see which row is which.
    if (kept.some((k) => same(k, trimmed))) continue;
    kept.push(trimmed);
  }
  return kept;
};

/// The same ranges, moved along by `offset` code units.
///
/// Used when segments are joined into one bubble: the ranges were matched
/// against the segment, and the bubble is that segment somewhere in the
/// middle of a longer string.
export const shift = (alerts: AlertMatch[], offset: number): AlertMatch[] =>
  offset === 0 ? alerts : alerts.map((a) => ({ start: a.start + offset, end: a.end + offset }));

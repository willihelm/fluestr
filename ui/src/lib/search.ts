// Searching and filtering the chat (P9).
//
// **In this view and no other.** The chat is shared — the same lines are on the
// desk, in a detached group window and in every browser served over the LAN
// (P16) — but a search is one operator asking one question of it: "did anyone
// mention the fly bar", "show me only what Licht said". Pushing that onto the
// other screens would be putting one person's filter in front of everyone
// else's show. So there is no backend command and nothing is emitted: the
// state lives in the component, which is why `search_session` was decided
// against in P6.
//
// It costs nothing to keep it there, either. Every view is caught up from
// `session_snapshot` and then fed by `transcript`, so the whole chat is
// already in the page — searching it is reading an array this window owns, not
// a round trip to fetch a filtered copy of what it has.
//
// The offsets below are UTF-16 code units, like the alert ranges from the
// backend (P17), so both kinds of highlight are measured in the same unit and
// can be drawn over one string.

import type { AlertMatch } from "./bridge";

/// A combining mark — an umlaut written as its own character after the letter
/// it sits on. Not global, so `test` has no lastIndex to carry between calls.
const MARK = /\p{M}/u;

/// One term or one cluster of a line, as the search compares them: composed,
/// then lower-cased, then composed again — because lower-casing a composed
/// character can hand back a decomposed one.
///
/// The same rule the backend's alert matcher folds by
/// (`crates/app/src/alerts.rs`), deliberately: two matchers that disagree
/// about "Bühne" would mean the chat highlights a word for the house's alert
/// list and not for the operator looking for the same word.
const folded = (text: string): string =>
  text.normalize("NFC").toLowerCase().normalize("NFC");

/// A line folded for comparison, with the offset in the original that each
/// code unit of it came from.
///
/// The map is what makes the folding safe. Both normalisation and case mapping
/// can change a string's length — "Bu\u0308hne" is eleven units and folds to
/// ten — and the offsets found in the folded form are used to highlight the
/// original. So the fold is done one *cluster* at a time — a base character
/// with whatever marks sit on it — and every unit it produces remembers where
/// that cluster began. Then a hit in the folded text maps back exactly, however
/// much the fold moved.
///
/// `origin` has one entry per code unit plus a final one for the end of the
/// line, so the end of a match at the end of the text maps too.
const foldWithOrigins = (text: string): { lowered: string; origin: number[] } => {
  let lowered = "";
  const origin: number[] = [];
  let at = 0;
  let cluster = "";

  const flush = () => {
    if (cluster === "") return;
    lowered += folded(cluster);
    while (origin.length < lowered.length) origin.push(at);
    at += cluster.length;
    cluster = "";
  };

  // By code point, which is what `for...of` iterates a string by — a surrogate
  // pair must not be split down the middle of the character it spells.
  for (const ch of text) {
    if (cluster !== "" && !MARK.test(ch)) flush();
    cluster += ch;
  }
  flush();
  origin.push(text.length);

  return { lowered, origin };
};

/// Every occurrence of `query` in `text`, as ranges into `text`.
///
/// Case-insensitive and normalised, so "Bühne", "BÜHNE" and the decomposed
/// spelling of either are one word — which they are to the operator typing the
/// query, and which is what the house's alert list already does.
///
/// Non-overlapping and in order, which is what the renderer wants, and what
/// "three hits" should mean when someone searches for `aa` in `aaaa`.
///
/// A blank query has no occurrences rather than all of them: the box is empty
/// far more often than it is full, and a search that matched everything the
/// moment it was cleared would empty the chat under the "only matches" toggle.
///
/// Whole words are deliberately *not* required, which is where this parts
/// company with the alert matcher. An alert fires by itself and a false one
/// teaches the operator to ignore the next; a search is read by the person who
/// just typed it, so "Bühne" finding "Bühnenmeister" is the answer they wanted
/// and not a mistake they have to live with.
export const findAll = (text: string, query: string): AlertMatch[] => {
  const needle = folded(query.trim());
  if (needle === "") return [];

  const { lowered, origin } = foldWithOrigins(text);
  const found: AlertMatch[] = [];
  let at = 0;
  for (;;) {
    const start = lowered.indexOf(needle, at);
    if (start === -1) break;
    const end = start + needle.length;
    // Back through the map, so the range is an offset into the line as it will
    // be rendered rather than into the form it was compared in.
    found.push({ start: origin[start], end: origin[end] });
    at = end;
  }
  return found;
};

/// Whether a line matches at all. The filter's question, where the ranges are
/// not needed.
export const matches = (text: string, query: string): boolean =>
  findAll(text, query).length > 0;

/// One run of a line on its way into the DOM, and what is marked about it.
///
/// Two kinds at once, because they answer different questions and can land on
/// the same characters: an **alert** is a term the whole house agreed must not
/// be missed (P17), a **hit** is what this operator is looking for right now.
/// A word that is both stays both — dropping one of the marks would hide
/// either the standing rule or the current question, and the view has no basis
/// for deciding which.
export interface Piece {
  text: string;
  alert: boolean;
  hit: boolean;
}

/// Splits a line at every boundary of every range, and says what each piece is
/// inside of.
///
/// Defensive about the ranges rather than trusting them: alert ranges were
/// matched against the text of *one segment* and a bubble is several segments
/// joined (see `bubbles.ts`), so a range that survived a re-grouping bug is an
/// offset into the wrong string. A bad range must cost a highlight, never the
/// line itself — the text is why the chat exists. Every piece here comes from
/// `text` by `slice`, so the pieces always join back into exactly the line
/// that went in, whatever the ranges said.
export const split = (
  text: string,
  alerts: AlertMatch[],
  hits: AlertMatch[] = [],
): Piece[] => {
  if (alerts.length === 0 && hits.length === 0) {
    return [{ text, alert: false, hit: false }];
  }

  const clamp = (ranges: AlertMatch[]) =>
    ranges
      .map(({ start, end }) => ({
        start: Math.max(0, Math.min(start, text.length)),
        end: Math.max(0, Math.min(end, text.length)),
      }))
      .filter((r) => r.end > r.start);

  const inAlerts = clamp(alerts);
  const inHits = clamp(hits);

  // Every point at which something starts or stops being true, the ends of the
  // line included. Between two neighbouring boundaries nothing changes, so one
  // membership test per piece is enough — which is what lets the two kinds
  // overlap without the ranges having to be merged first.
  const edges = new Set<number>([0, text.length]);
  for (const { start, end } of [...inAlerts, ...inHits]) {
    edges.add(start);
    edges.add(end);
  }

  const bounds = [...edges].sort((a, b) => a - b);
  const covers = (ranges: AlertMatch[], from: number) =>
    ranges.some((r) => r.start <= from && from < r.end);

  const pieces: Piece[] = [];
  for (let i = 0; i < bounds.length - 1; i += 1) {
    const from = bounds[i];
    const to = bounds[i + 1];
    if (to === from) continue;
    pieces.push({
      text: text.slice(from, to),
      alert: covers(inAlerts, from),
      hit: covers(inHits, from),
    });
  }

  return pieces;
};

// Searching the chat this window is holding (P9).
//
// Two jobs, and the second is the one with teeth: finding the hits, and
// cutting a line into the pieces that get a highlight — which now has to carry
// the house's alert terms (P17) and this operator's search at the same time,
// over one string, without either of them being able to damage the text.

import { describe, expect, it } from "vitest";
import { findAll, matches, split } from "./search";

describe("findAll", () => {
  it("finds a word whatever case it was typed in", () => {
    // Nobody searching a live chat capitalises what they are looking for.
    expect(findAll("Feuer im Saal", "feuer")).toEqual([{ start: 0, end: 5 }]);
    expect(findAll("feuer im saal", "FEUER")).toEqual([{ start: 0, end: 5 }]);
  });

  it("finds a word inside a longer one", () => {
    // Spoken language with whisper's punctuation on it: somebody looking for
    // "Bühne" wants "Bühnenmeister" and "Bühne," as well. Requiring a whole
    // word would hide the line they are looking for behind a grammar rule.
    expect(findAll("Der Bühnenmeister kommt", "bühne")).toEqual([
      { start: 4, end: 9 },
    ]);
  });

  it("matches the two spellings of an umlaut against each other", () => {
    // One typed on a German keyboard, the other as whisper may return it.
    const composed = "Bühne frei";
    const decomposed = "Bühne frei".normalize("NFD");

    expect(findAll(composed, "bühne")).toHaveLength(1);
    expect(matches(decomposed, "bühne")).toBe(true);
  });

  it("highlights the right characters when the fold changed the length", () => {
    // The decoder emitted the umlaut as two characters and the operator typed
    // it as one, so the form that is compared is a unit shorter than the line
    // that is rendered. The range still has to name the word in the line.
    const text = "Die Bu\u0308hne ist frei".normalize("NFD");
    const [hit] = findAll(text, "bühne");

    expect(text.slice(hit.start, hit.end).normalize("NFC")).toBe("Bühne");
  });

  it("gives offsets that are UTF-16 units into the line as it was given", () => {
    // The ranges are handed to `slice`, and the folding that made the
    // comparison possible must not have moved anything: an umlaut before the
    // match is exactly where a mismatch would show.
    const text = "Bühne: Feuer im Saal";
    const [hit] = findAll(text, "feuer");

    expect(text.slice(hit.start, hit.end)).toBe("Feuer");
  });

  it("counts overlapping occurrences once", () => {
    // `aa` in `aaaa` is two, not three: the ranges are drawn on one string, so
    // they have to be able to sit next to each other.
    expect(findAll("aaaa", "aa")).toEqual([
      { start: 0, end: 2 },
      { start: 2, end: 4 },
    ]);
  });

  it("finds nothing for an empty query rather than everything", () => {
    // The box is empty far more often than it is full, and "matches
    // everything" under the only-matches toggle would empty the chat the
    // moment the query was cleared.
    expect(findAll("Feuer", "")).toEqual([]);
    expect(findAll("Feuer", "   ")).toEqual([]);
    expect(matches("Feuer", "")).toBe(false);
  });
});

describe("split", () => {
  it("leaves a line without marks in one piece", () => {
    expect(split("Standby für Cue fünf", [])).toEqual([
      { text: "Standby für Cue fünf", alert: false, hit: false },
    ]);
  });

  it("cuts the highlight out of the line around it", () => {
    // The offsets are UTF-16 units and the text has an umlaut before the
    // match, which is exactly where a byte offset would land in the wrong
    // place.
    const text = "Bühne: Feuer im Saal";

    expect(split(text, [{ start: 7, end: 12 }])).toEqual([
      { text: "Bühne: ", alert: false, hit: false },
      { text: "Feuer", alert: true, hit: false },
      { text: " im Saal", alert: false, hit: false },
    ]);
  });

  it("marks a line that is nothing but the term", () => {
    expect(split("Feuer", [{ start: 0, end: 5 }])).toEqual([
      { text: "Feuer", alert: true, hit: false },
    ]);
  });

  it("keeps every match of several", () => {
    const text = "Feuer und Arzt";

    expect(
      split(text, [
        { start: 0, end: 5 },
        { start: 10, end: 14 },
      ]).filter((p) => p.alert),
    ).toEqual([
      { text: "Feuer", alert: true, hit: false },
      { text: "Arzt", alert: true, hit: false },
    ]);
  });

  it("keeps a word that is both an alert term and a hit as both", () => {
    // The standing rule and the current question land on the same characters,
    // and the view has no basis for deciding which of the two to hide.
    const text = "Feuer im Saal";
    const marked = split(text, [{ start: 0, end: 5 }], findAll(text, "feuer"));

    expect(marked).toEqual([
      { text: "Feuer", alert: true, hit: true },
      { text: " im Saal", alert: false, hit: false },
    ]);
  });

  it("cuts a hit that overlaps an alert term into its parts", () => {
    // Searching for "euer im" across the end of the alert "Feuer": the piece
    // inside both is marked as both, and the rest as what it is.
    const text = "Feuer im Saal";
    const marked = split(text, [{ start: 0, end: 5 }], [{ start: 1, end: 8 }]);

    expect(marked).toEqual([
      { text: "F", alert: true, hit: false },
      { text: "euer", alert: true, hit: true },
      { text: " im", alert: false, hit: true },
      { text: " Saal", alert: false, hit: false },
    ]);
  });

  it("costs a nonsensical range its highlight and never the text", () => {
    // A range that outlived the string it was matched against must not be able
    // to swallow or duplicate the line: the text is why the chat exists.
    const text = "Feuer";
    const joined = (ranges: { start: number; end: number }[]) =>
      split(text, ranges)
        .map((p) => p.text)
        .join("");

    expect(joined([{ start: 3, end: 99 }])).toBe(text);
    expect(joined([{ start: 9, end: 12 }])).toBe(text);
    expect(joined([{ start: 4, end: 2 }])).toBe(text);
  });
});

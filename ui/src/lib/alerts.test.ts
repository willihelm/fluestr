import { describe, expect, it } from "vitest";
import { cleanTerms, pieces, sameTerms, shift } from "./alerts";

describe("cleanTerms", () => {
  it("drops the empty row a term is added through", () => {
    expect(cleanTerms(["Feuer", "  ", ""])).toEqual(["Feuer"]);
  });

  it("keeps one of a term entered twice in different cases", () => {
    // The matcher is case-insensitive, so two spellings are one term — a list
    // showing both would suggest they do different things.
    expect(cleanTerms(["Feuer", " feuer "])).toEqual(["Feuer"]);
  });
});

describe("sameTerms", () => {
  it("tells a changed list from an unchanged one", () => {
    expect(sameTerms(["Feuer"], ["Feuer"])).toBe(true);
    expect(sameTerms(["Feuer"], ["Feuer", "Arzt"])).toBe(false);
    expect(sameTerms(["Feuer", "Arzt"], ["Arzt", "Feuer"])).toBe(false);
  });
});

describe("pieces", () => {
  it("leaves a line without matches in one piece", () => {
    expect(pieces("Standby für Cue fünf", [])).toEqual([
      { text: "Standby für Cue fünf", alert: false },
    ]);
  });

  it("cuts the highlight out of the line around it", () => {
    // The offsets are UTF-16 units and the text has an umlaut before the
    // match, which is exactly where a byte offset would land in the wrong
    // place.
    const text = "Bühne: Feuer im Saal";
    const marked = pieces(text, [{ start: 7, end: 12 }]);

    expect(marked).toEqual([
      { text: "Bühne: ", alert: false },
      { text: "Feuer", alert: true },
      { text: " im Saal", alert: false },
    ]);
  });

  it("marks a line that is nothing but the term", () => {
    expect(pieces("Feuer", [{ start: 0, end: 5 }])).toEqual([
      { text: "Feuer", alert: true },
    ]);
  });

  it("keeps every match of several", () => {
    const text = "Feuer und Arzt";
    expect(
      pieces(text, [
        { start: 0, end: 5 },
        { start: 10, end: 14 },
      ]).filter((p) => p.alert),
    ).toEqual([
      { text: "Feuer", alert: true },
      { text: "Arzt", alert: true },
    ]);
  });

  it("costs a nonsensical range its highlight and never the text", () => {
    // A range that outlived the string it was matched against must not be
    // able to swallow or duplicate the line: the text is why the chat exists.
    const text = "Feuer";
    expect(pieces(text, [{ start: 3, end: 99 }]).map((p) => p.text).join("")).toBe(text);
    expect(pieces(text, [{ start: 9, end: 12 }]).map((p) => p.text).join("")).toBe(text);
    expect(pieces(text, [{ start: 4, end: 2 }]).map((p) => p.text).join("")).toBe(text);
  });
});

describe("shift", () => {
  it("moves the ranges along by what was joined before them", () => {
    expect(shift([{ start: 0, end: 5 }], 8)).toEqual([{ start: 8, end: 13 }]);
    expect(shift([{ start: 0, end: 5 }], 0)).toEqual([{ start: 0, end: 5 }]);
  });
});

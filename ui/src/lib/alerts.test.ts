import { describe, expect, it } from "vitest";
import { cleanTerms, shift } from "./alerts";

describe("cleanTerms", () => {
  it("drops the empty row a term is added through", () => {
    expect(cleanTerms(["Feuer", "  ", ""])).toEqual(["Feuer"]);
  });

  it("keeps one of a term entered twice in different cases", () => {
    // The matcher is case-insensitive, so two spellings are one term — a list
    // showing both would suggest they do different things.
    expect(cleanTerms(["Feuer", " feuer "])).toEqual(["Feuer"]);
  });

  it("keeps one of a term entered in both umlaut spellings", () => {
    // Two rows a keyboard can produce and nobody can tell apart on screen.
    // The backend compiles them to one term, so the list must show one.
    expect(cleanTerms(["B\u00fchne", "Bu\u0308hne"])).toEqual(["B\u00fchne"]);
  });
});

describe("shift", () => {
  it("moves the ranges along by what was joined before them", () => {
    expect(shift([{ start: 0, end: 5 }], 8)).toEqual([{ start: 8, end: 13 }]);
    expect(shift([{ start: 0, end: 5 }], 0)).toEqual([{ start: 0, end: 5 }]);
  });
});

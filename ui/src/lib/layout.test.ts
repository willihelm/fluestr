import { describe, expect, it } from "vitest";
import { isNarrow, NARROW_PX, NARROW_QUERY } from "./layout";

describe("isNarrow", () => {
  it("puts a phone on the narrow layout", () => {
    // Every phone in portrait, and most of them in landscape.
    expect(isNarrow(390)).toBe(true);
    expect(isNarrow(430)).toBe(true);
  });

  it("leaves a desk window on the desk layout", () => {
    expect(isNarrow(1440)).toBe(false);
  });

  // The boundary is part of the rule the CSS is written against: a window
  // exactly at the breakpoint matches `max-width`, so it must be narrow here
  // too, or the burger and the drawer it opens disagree at that one width.
  it("counts the breakpoint itself as narrow", () => {
    expect(isNarrow(NARROW_PX)).toBe(true);
    expect(isNarrow(NARROW_PX + 1)).toBe(false);
  });

  it("answers a window with no width with the layout that works at any", () => {
    expect(isNarrow(Number.NaN)).toBe(false);
  });
});

describe("NARROW_QUERY", () => {
  it("asks for the same width the rule is written in", () => {
    expect(NARROW_QUERY).toBe(`(max-width: ${NARROW_PX}px)`);
  });
});

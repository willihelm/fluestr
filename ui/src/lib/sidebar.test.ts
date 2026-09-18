import { describe, expect, it } from "vitest";
import {
  clampWidth,
  DEFAULT_WIDTH,
  MAX_FRACTION,
  maxWidth,
  MIN_WIDTH,
  readWidth,
} from "./sidebar";

describe("clampWidth", () => {
  it("leaves a sensible width alone", () => {
    expect(clampWidth(240, 1440)).toBe(240);
  });

  it("holds the sidebar to a share of the window", () => {
    expect(clampWidth(2000, 1000)).toBe(1000 * MAX_FRACTION);
  });

  it("keeps a row a row", () => {
    expect(clampWidth(10, 1440)).toBe(MIN_WIDTH);
    expect(clampWidth(-500, 1440)).toBe(MIN_WIDTH);
  });

  // A phone in portrait is narrow enough that the share falls below the
  // minimum. The minimum wins: the alternative is a sidebar at a width nobody
  // asked for, on the screen where this matters most.
  it("does not let the share push it below the minimum", () => {
    expect(clampWidth(DEFAULT_WIDTH, 100)).toBe(MIN_WIDTH);
  });

  it("answers a width that is not a number with the default", () => {
    expect(clampWidth(Number.NaN, 1440)).toBe(DEFAULT_WIDTH);
  });

  it("rounds to whole pixels", () => {
    expect(clampWidth(240.6, 1440)).toBe(241);
  });
});

describe("maxWidth", () => {
  it("is a share of the window", () => {
    expect(maxWidth(1000)).toBe(1000 * MAX_FRACTION);
  });

  // The bound the handle reports has to be one it can actually reach, or the
  // range read out to an assistive technology describes a different control.
  it("never falls below the minimum a row needs", () => {
    expect(maxWidth(100)).toBe(MIN_WIDTH);
    expect(clampWidth(10_000, 100)).toBe(maxWidth(100));
  });
});

describe("readWidth", () => {
  it("starts at the width the sidebar always had", () => {
    expect(readWidth(null, 1440)).toBe(DEFAULT_WIDTH);
  });

  it("comes back at the width it was left at", () => {
    expect(readWidth("300", 1440)).toBe(300);
  });

  // The window it is restored into is not the one it was measured in: a phone
  // turned to portrait, or a window dragged to a smaller screen.
  it("brings a stored width inside the window it is read into", () => {
    expect(readWidth("900", 1000)).toBe(1000 * MAX_FRACTION);
  });

  it("takes a corrupted entry for no entry", () => {
    expect(readWidth("wide", 1440)).toBe(DEFAULT_WIDTH);
  });
});

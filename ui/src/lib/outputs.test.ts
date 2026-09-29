import { describe, expect, it } from "vitest";
import { calibrationFor, curveTarget, decodeChannels, encodeChannels, outputChoices } from "./outputs";
import { restoreOutputs } from "./profiles";

describe("outputChoices", () => {
  it("offers the pairs first and every channel alone after them", () => {
    expect(outputChoices(4).map((c) => c.label)).toEqual(["1–2", "3–4", "1", "2", "3", "4"]);
    expect(outputChoices(1)).toEqual([{ channels: [0], label: "1" }]);
  });

  it("round-trips a route through the select's value", () => {
    expect(decodeChannels(encodeChannels([6, 7]))).toEqual([6, 7]);
    expect(decodeChannels("")).toEqual([]);
  });
});

describe("calibrationFor", () => {
  it("makes the page read what the meter reads", () => {
    // Uncalibrated the page shows −32 dBFS; the meter says 86 dB(A).
    expect(calibrationFor(86, -32, null)).toBe(118);
    // Already calibrated to 110 and showing 78: the same room, re-read.
    expect(calibrationFor(86, 78, 110)).toBe(118);
  });

  it("has nothing to calibrate against without a reading", () => {
    expect(calibrationFor(86, null, null)).toBeNull();
  });
});

describe("curveTarget", () => {
  it("is the same line with two corners as the backend's", () => {
    const shoutbox = {
      ...restoreOutputs().shoutbox,
      quietDb: -40,
      baseGainDb: -12,
      slopePercent: 100,
      maxGainDb: 0,
    };
    expect(curveTarget(shoutbox, -60)).toBe(-12);
    expect(curveTarget(shoutbox, -30)).toBe(-2);
    expect(curveTarget(shoutbox, -10)).toBe(0);
  });
});

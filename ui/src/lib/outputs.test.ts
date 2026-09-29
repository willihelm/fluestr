import { describe, expect, it } from "vitest";
import {
  calibrationFor,
  curveTarget,
  decodeChannels,
  encodeChannels,
  isShoutboxDefault,
  outputChoices,
  shoutboxDefaults,
} from "./outputs";
import { DEFAULT_OUTPUTS } from "./bridge";
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

describe("shoutboxDefaults", () => {
  it("resets the tuning and keeps the patch, the calibration and the mute", () => {
    const tuned = {
      ...structuredClone(DEFAULT_OUTPUTS.shoutbox),
      enabled: true,
      program: { device: "MADIface", deviceId: "madi", channel: 3 },
      calibrationDb: 117.5,
      muted: true,
      quietDb: 70,
      slopePercent: 120,
      trimDb: -6,
      windowMs: 8000,
    };
    expect(isShoutboxDefault(tuned)).toBe(false);
    const reset = shoutboxDefaults(tuned);
    expect(reset.quietDb).toBe(DEFAULT_OUTPUTS.shoutbox.quietDb);
    expect(reset.slopePercent).toBe(DEFAULT_OUTPUTS.shoutbox.slopePercent);
    expect(reset.trimDb).toBe(0);
    expect(reset.windowMs).toBe(DEFAULT_OUTPUTS.shoutbox.windowMs);
    expect(reset.enabled).toBe(true);
    expect(reset.program.channel).toBe(3);
    expect(reset.calibrationDb).toBe(117.5);
    expect(reset.muted).toBe(true);
    expect(isShoutboxDefault(reset)).toBe(true);
  });
});

describe("calibrationFor", () => {
  it("makes the page read what the meter reads", () => {
    // Uncalibrated the page shows −32 dBFS; the meter says 86 dB(A).
    expect(calibrationFor(86, -32, null)).toBe(118);
    // Already calibrated to 110 and showing 78: the same room, re-read.
    expect(calibrationFor(86, 78, 110)).toBe(118);
  });

  it("keeps a tenth and goes negative for a hot input", () => {
    // A 94 dB calibrator on a mic that shows −23.46 dBFS.
    expect(calibrationFor(94, -23.46, null)).toBe(117.5);
    // A hot input already reading above SPL: 96.34 shown at offset 0.
    expect(calibrationFor(94, 96.34, 0)).toBe(-2.3);
    // Re-calibrating on top of a negative offset.
    expect(calibrationFor(94, 93.7, -2.3)).toBe(-2);
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

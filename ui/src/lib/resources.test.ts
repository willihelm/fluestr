import { describe, expect, it } from "vitest";
import { BUSY, CRITICAL, detail, formatGpu, pressure } from "./resources";
import type { ResourceUsage } from "./bridge";

const usage = (over: Partial<ResourceUsage> = {}): ResourceUsage => ({
  gpu: 12,
  gpuCores: 16,
  ...over,
});

describe("resources", () => {
  it("shows the GPU share without decimals", () => {
    // It moves every second; a tenth of a percent is noise in a number that
    // only ever gets read at a glance.
    expect(formatGpu(41.6)).toBe("42 %");
    expect(formatGpu(0.2)).toBe("0 %");
  });

  it("never shows a negative share", () => {
    // The backend smooths the reading, and an exponential average fed a zero
    // after a spike can undershoot. "-0 %" would look like a bug in the app.
    expect(formatGpu(-0.4)).toBe("0 %");
  });

  it("warns before the machine is out of headroom, not once it is", () => {
    // The indicator exists to be seen before the decode queue starts dropping
    // segments — at 90 % of the GPU they are already going.
    expect(pressure(40)).toBe("calm");
    expect(pressure(BUSY)).toBe("busy");
    expect(pressure(CRITICAL)).toBe("critical");
    expect(BUSY).toBeLessThan(CRITICAL);
  });

  it("stays calm at the load an idle machine actually shows", () => {
    // Measured on an M1 Pro: the app open and idle smooths out around 40 %,
    // because the figure covers the whole machine. A threshold that called
    // that busy would make the indicator worthless on the first day.
    expect(pressure(40)).toBe("calm");
    expect(pressure(55)).toBe("calm");
  });

  it("says the GPU figure is the whole machine", () => {
    // Without this an operator sees a busy GPU on an idle app and goes looking
    // for a bug that is not there.
    expect(detail(usage())).toContain("whole machine");
    expect(detail(usage())).toContain("16 cores");
  });

  it("leaves the core count out when the GPU does not report one", () => {
    const text = detail(usage({ gpuCores: null }));
    expect(text).toContain("whole machine,");
    expect(text).not.toContain("cores");
  });
});

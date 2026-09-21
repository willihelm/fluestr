import { describe, expect, it } from "vitest";
import { BUSY, CRITICAL, detail, formatGpu, formatMemory, pressure, summary } from "./resources";
import type { ResourceUsage } from "./bridge";

const usage = (over: Partial<ResourceUsage> = {}): ResourceUsage => ({
  gpu: 12,
  memory: 2_100_000_000,
  totalMemory: 16_000_000_000,
  processes: 5,
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

  it("switches to gigabytes only once there is a whole one", () => {
    expect(formatMemory(742_000_000)).toBe("742 MB");
    expect(formatMemory(2_100_000_000)).toBe("2.1 GB");
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

  it("reads as one line", () => {
    expect(summary(usage({ gpu: 42 }))).toBe("GPU 42 % · RAM 2.1 GB");
  });

  it("drops the GPU half when the machine does not report one", () => {
    // Not a dash: a placeholder where a live figure belongs is the kind of
    // thing an operator learns to read as zero.
    expect(summary(usage({ gpu: null }))).toBe("RAM 2.1 GB");
  });

  it("says the GPU figure is the whole machine", () => {
    // Without this an operator sees a busy GPU on an idle app and goes looking
    // for a bug that is not there.
    expect(detail(usage())).toContain("whole machine");
    expect(detail(usage())).toContain("16 cores");
  });

  it("says that the workers are counted in the memory", () => {
    // Somebody comparing this with one row in Activity Monitor has to be able
    // to find out why the figures differ.
    expect(detail(usage())).toContain("5 processes");
    expect(detail(usage())).toContain("workers");
    expect(detail(usage())).toContain("16.0 GB");
  });

  it("counts one process in the singular", () => {
    expect(detail(usage({ processes: 1 }))).toContain("1 process");
  });

  it("leaves the total out when the machine would not say", () => {
    expect(detail(usage({ totalMemory: 0 }))).toContain("resident.");
  });

  it("says only what it knows on a machine without a GPU reading", () => {
    const text = detail(usage({ gpu: null }));
    expect(text).not.toContain("GPU");
    expect(text).toContain("5 processes");
  });

  it("leaves the core count out when the GPU does not report one", () => {
    const text = detail(usage({ gpuCores: null }));
    expect(text).toContain("whole machine,");
    expect(text).not.toContain("cores");
  });
});

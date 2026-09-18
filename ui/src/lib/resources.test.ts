import { describe, expect, it } from "vitest";
import { BUSY, CRITICAL, detail, formatCpu, formatMemory, pressure, summary } from "./resources";
import type { ResourceUsage } from "./bridge";

const usage = (over: Partial<ResourceUsage> = {}): ResourceUsage => ({
  cpu: 12,
  memory: 2_100_000_000,
  totalMemory: 16_000_000_000,
  processes: 5,
  cores: 8,
  ...over,
});

describe("resources", () => {
  it("shows the CPU share without decimals", () => {
    // It moves every second; a tenth of a percent is noise in a number that
    // only ever gets read at a glance.
    expect(formatCpu(41.6)).toBe("42 %");
    expect(formatCpu(0.2)).toBe("0 %");
  });

  it("never shows a negative share", () => {
    // The backend divides a sum of per-process figures by the core count, and
    // a process that exits mid-sample is the kind of thing that has produced
    // a negative reading before. "-0 %" would look like a bug in the app.
    expect(formatCpu(-0.4)).toBe("0 %");
  });

  it("switches to gigabytes only once there is a whole one", () => {
    expect(formatMemory(742_000_000)).toBe("742 MB");
    expect(formatMemory(2_100_000_000)).toBe("2.1 GB");
  });

  it("warns before the machine is out of headroom, not once it is", () => {
    // The indicator exists to be seen before the decode queue starts dropping
    // segments — at 90 % of every core they are already going.
    expect(pressure(40)).toBe("calm");
    expect(pressure(BUSY)).toBe("busy");
    expect(pressure(CRITICAL)).toBe("critical");
    expect(BUSY).toBeLessThan(CRITICAL);
  });

  it("reads as one line", () => {
    expect(summary(usage({ cpu: 42 }))).toBe("CPU 42 % · RAM 2.1 GB");
  });

  it("says that the workers are counted too", () => {
    // Somebody comparing this with one row in Activity Monitor has to be able
    // to find out why the figures differ.
    expect(detail(usage())).toContain("5 processes");
    expect(detail(usage())).toContain("workers");
    expect(detail(usage())).toContain("16.0 GB");
  });

  it("counts one process in the singular", () => {
    expect(detail(usage({ processes: 1 }))).toContain("1 process.");
  });

  it("leaves the total out when the machine would not say", () => {
    expect(detail(usage({ totalMemory: 0 }))).toContain("memory is resident.");
  });
});

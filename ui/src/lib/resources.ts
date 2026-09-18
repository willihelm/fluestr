// Formatting for the CPU and memory indicator (P8).
//
// Separate from the component because this is the part with decisions in it —
// how much precision a figure deserves, and where "busy" turns into "about to
// fall behind" — and those are worth pinning down in tests rather than reading
// out of a template.

import type { ResourceUsage } from "./bridge";

/// Above this share of the machine the indicator starts warning, below the
/// next one it is calm.
///
/// 75 % rather than 90: the point of the indicator is to be seen *before* the
/// decode queue starts dropping segments, and by the time a machine is at 90 %
/// of every core the segments are already going. See P5's overload counter for
/// what this is trying to give warning of.
export const BUSY = 75;
/// Above this, decoding is out of headroom and dropped segments are a matter
/// of time.
export const CRITICAL = 90;

export type Pressure = "calm" | "busy" | "critical";

export const pressure = (percent: number): Pressure => {
  if (percent >= CRITICAL) return "critical";
  if (percent >= BUSY) return "busy";
  return "calm";
};

/// A CPU share as it is shown: whole percent.
///
/// No decimal place. This figure moves every second and a tenth of a percent
/// is noise that makes the number impossible to read at a glance — which is
/// the only way it ever gets read.
export const formatCpu = (percent: number): string =>
  `${Math.round(Math.max(0, percent))} %`;

/// Bytes as gigabytes, or as megabytes while that would be a leading zero.
///
/// GB rather than GiB, and 1000 rather than 1024: this number is read next to
/// what macOS itself reports, and being consistent with Activity Monitor
/// matters more here than being consistent with the page size.
export const formatMemory = (bytes: number): string => {
  const mb = bytes / 1e6;
  if (mb < 1000) return `${Math.round(mb)} MB`;
  return `${(mb / 1000).toFixed(1)} GB`;
};

/// The whole line, e.g. `CPU 42 % · RAM 2.1 GB`.
export const summary = (usage: ResourceUsage): string =>
  `CPU ${formatCpu(usage.cpu)} · RAM ${formatMemory(usage.memory)}`;

/// What the tooltip says: where the figures come from, which is not obvious.
///
/// The decoding happens in sidecar processes, so "the app" in the indicator is
/// the app *and* its workers — an operator comparing it with one row in
/// Activity Monitor would otherwise conclude the indicator is wrong.
export const detail = (usage: ResourceUsage): string => {
  const processes =
    usage.processes === 1 ? "1 process" : `${usage.processes} processes`;
  const share =
    usage.totalMemory > 0
      ? ` of ${formatMemory(usage.totalMemory)}`
      : "";
  return (
    `Intercom2Chat and its whisper workers: ${processes}. ` +
    `CPU is the share of all ${usage.cores} cores; ` +
    `memory is resident${share}.`
  );
};

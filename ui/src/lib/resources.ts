// Formatting for the GPU indicator (P8).
//
// Separate from the component because this is the part with decisions in it —
// how much precision the figure deserves, and where "busy" turns into "about to
// fall behind" — and those are worth pinning down in tests rather than reading
// out of a template.

import type { ResourceUsage } from "./bridge";

/// Above this share of the GPU the indicator starts warning, below it the
/// machine is calm.
///
/// 75 % rather than 90: the point of the indicator is to be seen *before* the
/// decode queue starts dropping segments, and by the time the GPU is at 90 %
/// the segments are already going. See P5's overload counter for what this is
/// trying to give warning of.
///
/// Measured against the real thing on an M1 Pro: an idle machine with the app
/// open smooths out around 40 %, and eight channels decoding push it to 80 and
/// above. So 75 sits in the gap rather than in either population — which is
/// what keeps the indicator from crying wolf on an idle machine.
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

/// A GPU share as it is shown: whole percent.
///
/// No decimal place. This figure moves every second and a tenth of a percent
/// is noise that makes the number impossible to read at a glance — which is
/// the only way it ever gets read.
export const formatGpu = (percent: number): string =>
  `${Math.round(Math.max(0, percent))} %`;

/// What the tooltip says: where the figure comes from, which is not obvious.
///
/// The GPU figure is the *whole machine* — macOS reports no per-process share,
/// so a browser left open is in that number, and an operator who read it as
/// the app's own would go looking for a bug that is not there.
export const detail = (usage: ResourceUsage): string => {
  const cores = usage.gpuCores === null ? "" : ` (${usage.gpuCores} cores)`;
  return (
    `GPU is the whole machine${cores}, not just this app: macOS reports no ` +
    `per-process share, so anything else using the GPU is in this figure. ` +
    `Decoding runs on it, so it is what runs out first.`
  );
};

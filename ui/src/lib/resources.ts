// Formatting for the GPU and memory indicator (P8).
//
// Separate from the component because this is the part with decisions in it —
// how much precision a figure deserves, and where "busy" turns into "about to
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

/// The whole line, e.g. `GPU 42 % · RAM 2.1 GB`.
///
/// A machine that does not report its GPU drops that half rather than showing
/// a dash: half a strip is honest, and a placeholder where a live figure
/// belongs is the kind of thing an operator learns to read as zero.
export const summary = (usage: ResourceUsage): string => {
  const memory = `RAM ${formatMemory(usage.memory)}`;
  return usage.gpu === null
    ? memory
    : `GPU ${formatGpu(usage.gpu)} · ${memory}`;
};

/// What the tooltip says: where the figures come from, which is not obvious.
///
/// Two things have to be said, and neither is guessable from the strip. The
/// GPU figure is the *whole machine* — macOS reports no per-process share, so
/// a browser left open is in that number, and an operator who read it as the
/// app's own would go looking for a bug that is not there. The memory figure
/// is the opposite: only this app, but including its sidecars, so somebody
/// comparing it with one row in Activity Monitor has to know why it is larger.
export const detail = (usage: ResourceUsage): string => {
  const processes =
    usage.processes === 1 ? "1 process" : `${usage.processes} processes`;
  const share =
    usage.totalMemory > 0 ? ` of ${formatMemory(usage.totalMemory)}` : "";
  const memory =
    `Memory is Fluestr and its whisper workers — ${processes} — ` +
    `resident${share}.`;

  if (usage.gpu === null) return memory;

  const cores = usage.gpuCores === null ? "" : ` (${usage.gpuCores} cores)`;
  return (
    `GPU is the whole machine${cores}, not just this app: macOS reports no ` +
    `per-process share, so anything else using the GPU is in this figure. ` +
    `Decoding runs on it, so it is what runs out first. ` +
    memory
  );
};

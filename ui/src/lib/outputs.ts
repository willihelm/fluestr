// The logic behind the Audio output section (P22, P26) that is not a
// component: which channel choices a device offers, and the two calculations
// an engineer would otherwise do on a napkin — the calibration offset and the
// curve.

import { DEFAULT_OUTPUTS, SHOUTBOX_ESTIMATED_CALIBRATION_DB, type StoredShoutbox } from "./bridge";

export interface ChannelChoice {
  /// What the route stores, zero-based.
  channels: number[];
  /// What a patch bay says, one-based.
  label: string;
}

/// Every way to put a mono signal on a device: the pairs first, because
/// headphones are a pair, then every channel alone, because a speaker is not.
export const outputChoices = (count: number): ChannelChoice[] => {
  const pairs: ChannelChoice[] = [];
  for (let c = 0; c + 1 < count; c += 2) {
    pairs.push({ channels: [c, c + 1], label: `${c + 1}–${c + 2}` });
  }
  const singles: ChannelChoice[] = Array.from({ length: count }, (_, c) => ({
    channels: [c],
    label: `${c + 1}`,
  }));
  return [...pairs, ...singles];
};

/// The shoutbox's tuning back to the defaults — the curve, the speeds, the
/// gate, the ceiling, the averaging and the trim.
///
/// What is kept is what belongs to the room rather than to taste: the patch,
/// whether it is on, the calibration (measured, not chosen) and the mute (a
/// reset must never be what un-mutes talkback in the house).
export const shoutboxDefaults = (current: StoredShoutbox): StoredShoutbox => ({
  ...structuredClone(DEFAULT_OUTPUTS.shoutbox),
  enabled: current.enabled,
  program: current.program,
  reference: current.reference,
  output: current.output,
  calibrationDb: current.calibrationDb,
  muted: current.muted,
});

/// Whether the tuning is already at the defaults, so the reset is offered
/// only when it would change something.
export const isShoutboxDefault = (current: StoredShoutbox): boolean => {
  const defaults = shoutboxDefaults(current);
  return (Object.keys(defaults) as (keyof StoredShoutbox)[]).every(
    (key) => defaults[key] === current[key],
  );
};

/// A route's channels as one `<select>` value, and back.
export const encodeChannels = (channels: number[]): string => channels.join(",");
export const decodeChannels = (value: string): number[] =>
  value === "" ? [] : value.split(",").map((c) => Number(c));

/// The offset the room is read through, weighted dBFS to dB SPL: the
/// calibration, or the estimate until there is one. Mirrors
/// `config::Shoutbox::offset_db`.
export const offsetOf = (calibrationDb: number | null | undefined): number =>
  calibrationDb ?? SHOUTBOX_ESTIMATED_CALIBRATION_DB;

/// The calibration offset that makes the page read what the SPL meter beside
/// the mic reads.
///
/// `shownDb` is the level the page shows now, which already includes the
/// offset in force — the estimate, when `currentOffset` is `null`; `meterDb`
/// is what the meter says. `null` while there is no reading to calibrate
/// against. To a tenth, as a calibrator gives it, and negative for an input
/// that runs hotter than the room.
export const calibrationFor = (
  meterDb: number,
  shownDb: number | null,
  currentOffset: number | null,
): number | null => {
  if (shownDb === null || !Number.isFinite(meterDb)) return null;
  const uncalibrated = shownDb - offsetOf(currentOffset);
  return roundTenth(meterDb - uncalibrated);
};

/// Sets the calibration and keeps the curve where it was in the room.
///
/// A new offset changes what every room level on the page reads, the quiet
/// threshold's included; moved by the same amount, the threshold stays at the
/// same sound in the room, and the shoutbox behaves as it did — only the
/// numbers become the right ones. Whole dB, like the threshold is stored.
export const recalibrate = (shoutbox: StoredShoutbox, next: number | null) => {
  const shift = offsetOf(next) - offsetOf(shoutbox.calibrationDb);
  shoutbox.calibrationDb = next;
  shoutbox.quietDb = Math.round(shoutbox.quietDb + shift);
};

/// A level to one decimal, the resolution the calibration is stored at.
export const roundTenth = (db: number): number => Math.round(db * 10) / 10;

/// The gain the curve asks for at a room level — the same line with two
/// corners as `audio_core::shoutbox::Curve::target`, for drawing it.
export const curveTarget = (shoutbox: StoredShoutbox, levelDb: number): number => {
  const above = Math.max(0, levelDb - shoutbox.quietDb);
  const slope = Math.min(2, Math.max(0, shoutbox.slopePercent / 100));
  const max = Math.max(shoutbox.maxGainDb, shoutbox.baseGainDb);
  return Math.min(max, shoutbox.baseGainDb + slope * above);
};

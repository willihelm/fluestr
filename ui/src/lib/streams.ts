// The channel setup the operator edits, and its translation to what the
// backend wants.
//
// Two identifiers live here and they are not the same thing. `key` is stable
// for the lifetime of a row so Svelte keeps the input the operator is typing
// in when the list around it changes. The **stream id** is the row's position
// in the list and is assigned by the backend at start — it is what every event
// carries, and it only means anything while a session runs.

import { paletteColor } from "./palette";
import type { StreamConfig } from "./bridge";

export interface Channel {
  key: number;
  /// Zero-based device channel, as the backend wants it.
  channel: number;
  label: string;
  language: string;
  color: string;
}

let nextKey = 0;

/// A new row, preset to the first free device channel and the next palette
/// colour. Both are starting points the operator can overwrite; neither is
/// ever silently reassigned afterwards.
export const newChannel = (existing: Channel[], deviceChannels: number): Channel => {
  const taken = new Set(existing.map((c) => c.channel));
  let channel = 0;
  while (taken.has(channel) && channel < deviceChannels - 1) channel += 1;

  return {
    key: nextKey++,
    channel,
    label: `Channel ${channel + 1}`,
    language: "de",
    color: paletteColor(existing.length),
  };
};

/// What `start_session` is sent. The order is the order of the list, because
/// that is what defines the stream ids.
export const toConfig = (channels: Channel[]): StreamConfig[] =>
  channels.map((c) => ({
    channel: c.channel,
    label: c.label.trim() === "" ? `Channel ${c.channel + 1}` : c.label.trim(),
    language: c.language.trim().toLowerCase(),
  }));

/// Why the setup cannot be started, or `null` if it can.
///
/// Checked here rather than left to the backend so the operator is told before
/// the model spends seconds loading — and so the message can name the row.
/// The backend validates the same things again; this is the faster of two
/// answers, not the only one.
export const validate = (
  channels: Channel[],
  device: string,
  deviceChannels: number,
): string | null => {
  if (device === "") return "No input device selected.";
  if (channels.length === 0) return "Add at least one channel.";

  for (const c of channels) {
    if (c.channel < 0 || c.channel >= deviceChannels) {
      return `${c.label} is on channel ${c.channel + 1}, but the device has ${deviceChannels}.`;
    }
    if (c.language.trim() === "") {
      return `${c.label} has no language.`;
    }
  }

  // Allowed by the backend — mirroring one channel into two views is unusual
  // but not wrong — so this is not an error. It is almost always a mistake
  // during setup, though, and silently transcribing the same audio twice would
  // look like the app duplicating lines.
  return null;
};

/// Channels sharing a device channel. Worth showing as a warning; see
/// [`validate`] for why it is not an error.
export const duplicateChannels = (channels: Channel[]): number[] => {
  const seen = new Map<number, number>();
  for (const c of channels) seen.set(c.channel, (seen.get(c.channel) ?? 0) + 1);
  return [...seen.entries()].filter(([, n]) => n > 1).map(([channel]) => channel);
};

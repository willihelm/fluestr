// The channel setup the operator edits, and its translation to what the
// backend wants.
//
// Two identifiers live here and they are not the same thing. `key` is stable
// for the lifetime of a row so Svelte keeps the input the operator is typing
// in when the list around it changes. The **stream id** is the row's position
// in the list and is assigned by the backend at start — it is what every event
// carries, and it only means anything while a session runs.

import { paletteColor } from "./palette";
import { GAIN_DB_MAX, GAIN_DB_MIN, type ChannelSource, type StreamConfig } from "./bridge";

export interface Channel {
  key: number;
  /// Stable across restarts and windows, unlike `key`: what another instance
  /// reading along names this channel by (P29), and what a row showing a
  /// channel of another instance points at. Never shown.
  id: string;
  /// Set when this row is a channel another instance transcribes (P29). It
  /// is in the chat, the groups and the rules like any other row, and it has
  /// no input here: no device channel, no trim, no draft.
  source: ChannelSource | null;
  /// Zero-based device channel, as the backend wants it.
  channel: number;
  label: string;
  language: string;
  color: string;
  /// Input trim in whole decibels, applied by the backend before anything
  /// reads the audio. A desk does not hand out eight channels at one level,
  /// and whisper gives up on the quiet one long before a human would.
  gainDb: number;
  /// Muted by the operator: the channel is still captured and still metered,
  /// and nothing said on it is transcribed. Pressed on a beltpack left open
  /// in a noisy room, which is one channel filling the chat for everybody.
  ///
  /// Not the same thing as a channel the device does not have — see
  /// [`absentChannels`]. One is a decision, the other is an interface.
  muted: boolean;
  /// Gets a live draft while somebody talks on it (P28). Off for a channel
  /// whose drafts are not worth the GPU; its lines still arrive as before.
  preview: boolean;
}

/// A trim the backend will accept unchanged.
///
/// The range is the backend's, imported rather than restated. Applied here as
/// well as there for the reason every other bound in this frontend is: so the
/// knob stops where the pipeline stops, instead of showing a value that is
/// silently something else once it arrives.
export const clampGain = (db: number): number => {
  if (!Number.isFinite(db)) return 0;
  return Math.min(GAIN_DB_MAX, Math.max(GAIN_DB_MIN, Math.round(db)));
};

/// The language a setup starts on when nothing has chosen one — a new profile,
/// or one read from a file written before profiles had a default.
///
/// The only language named in the frontend, and a starting point rather than a
/// constraint: the picker is fed from whisper's own list (`listLanguages`), so
/// adding a language is never a change here. DE because that is the house this
/// was built for; one pick on the Settings page changes it for a profile.
export const DEFAULT_LANGUAGE = "de";

let nextKey = 0;

/// The device channel a row read from another instance is carried as (P29):
/// one no interface has, so the session keeps the row's place — and with it
/// every stream id after it — and sends it no audio.
export const NO_INPUT = 0xffff;

/// A fresh channel id: twelve hex digits, like the backend hands out to a
/// file that predates them.
export const newChannelId = (): string => {
  const bytes = new Uint8Array(6);
  globalThis.crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
};

/// Whether a row is an input of this machine rather than a channel read from
/// another instance.
export const isInput = (c: { source: ChannelSource | null }): boolean => c.source === null;

/// A row showing a channel of another instance (P29).
export const remoteChannel = (
  existing: Channel[],
  source: ChannelSource,
  label: string,
  color: string,
  language: string = DEFAULT_LANGUAGE,
): Channel => ({
  key: nextKey++,
  id: newChannelId(),
  source,
  channel: NO_INPUT,
  label,
  language,
  // The colour it has over there, so the same talkback looks the same on both
  // screens — unless a channel here already wears it.
  color: existing.some((c) => c.color === color) ? paletteColor(existing.length) : color,
  gainDb: 0,
  muted: false,
  preview: false,
});

/// A new row, preset to the first free channel and the next palette colour.
/// Both are starting points the operator can overwrite; neither is ever
/// silently reassigned afterwards.
///
/// Deliberately not clamped to the device's channel count: a setup may hold
/// more channels than the interface currently plugged in has, and those start
/// carried along silently rather than blocking the session — see
/// [`absentChannels`].
export const newChannel = (
  existing: Channel[],
  /// The profile's default language — what the picker above the channel list
  /// shows. Presetting a new row rather than leaving it on a fixed "de" is the
  /// whole point of that picker: eight channels of one production are
  /// overwhelmingly in one language, and setting it eight times is eight
  /// chances to miss one.
  language: string = DEFAULT_LANGUAGE,
): Channel => {
  const taken = new Set(existing.filter(isInput).map((c) => c.channel));
  let channel = 0;
  while (taken.has(channel)) channel += 1;

  return {
    key: nextKey++,
    id: newChannelId(),
    source: null,
    channel,
    label: `Channel ${channel + 1}`,
    language,
    color: paletteColor(existing.length),
    // Unity: a new channel is whatever the interface delivers until somebody
    // has heard it and decided otherwise.
    gainDb: 0,
    // Open, for the same reason: a channel is added in order to be heard.
    muted: false,
    // Drafted, like every channel until somebody decides otherwise.
    preview: true,
  };
};

/// A channel read back from the stored configuration (P12).
///
/// Everything but the key comes from the file; the key is handed out here,
/// because it identifies a row to Svelte for as long as the window is open and
/// means nothing outside it. That is also why it is not stored: it would be
/// noise in a file an operator may have to read, and a restored setup that
/// reused stale keys would collide with the ones a later `newChannel` hands
/// out.
export const restoreChannel = (
  // `muted` optional, unlike the rest: a file written before it existed does
  // not carry it, and the frontend reads those files as well as the backend
  // does.
  stored: Omit<Channel, "key" | "muted" | "preview" | "id" | "source"> & {
    muted?: boolean;
    preview?: boolean;
    id?: string;
    source?: ChannelSource | null;
  },
): Channel => ({
  ...stored,
  key: nextKey++,
  // The backend hands every channel an id on load, so this is for a file the
  // frontend reads some other way — a test, a hand-written fixture.
  id: stored.id || newChannelId(),
  source: stored.source ?? null,
  // A hand-edited file may hold a trim the backend would clamp anyway. Doing
  // it on the way in means the knob shows the value that is actually in force
  // rather than the one the file claims.
  gainDb: clampGain(stored.gainDb),
  // A file written before mute existed has no such field, and every channel
  // in it was open.
  muted: stored.muted ?? false,
  // The same for drafts: every channel was drafted.
  preview: stored.preview ?? true,
});

/// Every channel put onto one language.
///
/// The deliberate half of the profile's default: changing the picker presets
/// what is added next and leaves what is already there alone, because a setup
/// an operator built is not something to rewrite behind their back. Pressing
/// Apply is how they ask for exactly that — so it is one button and not a side
/// effect of the picker.
export const applyLanguage = (channels: Channel[], language: string): Channel[] =>
  channels.map((c) => (c.language === language ? c : { ...c, language }));

/// Which languages the setup actually uses, in the order the channels are in.
///
/// What the picker needs to say whether it still describes the setup below it:
/// one entry and it matches, several and the channels disagree with it.
export const usedLanguages = (channels: Channel[]): string[] => [
  ...new Set(channels.map((c) => c.language)),
];

/// What `start_session` is sent. The order is the order of the list, because
/// that is what defines the stream ids.
export const toConfig = (channels: Channel[]): StreamConfig[] =>
  channels.map((c) => ({
    // A row read from another instance keeps its place and gets no input.
    channel: isInput(c) ? c.channel : NO_INPUT,
    label: c.label.trim() === "" ? `Channel ${c.channel + 1}` : c.label.trim(),
    language: c.language.trim().toLowerCase(),
    gainDb: clampGain(c.gainDb),
    muted: c.muted,
    preview: isInput(c) && c.preview,
    id: c.id,
    remote: !isInput(c),
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
  /// Whether the named device is among the ones currently present. A stored
  /// profile (P12) may well name an interface that is not plugged in, and
  /// "not connected" is a different problem from "has no input channels" —
  /// one is solved with a cable, the other with a different device. Defaults
  /// to true so callers that only ever pass a device they picked from the
  /// list do not have to say so.
  connected = true,
): string | null => {
  // Rows read from another instance need no device and no session: only
  // this machine's own inputs are what a Start is for.
  channels = channels.filter(isInput);
  if (device === "") return "No input device selected.";
  if (!connected) return `${device} is not connected.`;
  if (channels.length === 0) return "Add at least one channel.";

  for (const c of channels) {
    if (c.language.trim() === "") {
      return `${c.label} has no language.`;
    }
  }

  // A channel the device does not have is carried along silently, not a
  // refusal to start: a setup built for the desk has to survive being carried
  // to a laptop with two inputs. Only a setup where *nothing* is left to
  // listen to is an error, because then starting would transcribe silence and
  // look like a broken app.
  if (absentChannels(channels, deviceChannels).length === channels.length) {
    return deviceChannels === 0
      ? "The selected device has no input channels."
      : `None of the ${channels.length} channels exists on this device; it has ${deviceChannels}.`;
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
  for (const c of channels.filter(isInput)) seen.set(c.channel, (seen.get(c.channel) ?? 0) + 1);
  return [...seen.entries()].filter(([, n]) => n > 1).map(([channel]) => channel);
};

/// The channels the device does not have, as indices into `channels` — the
/// stream ids, which is what the meters and the session state key on.
///
/// These start anyway and are carried along silently: they keep their id,
/// their colour and their place in the groups and receive no audio. Carrying
/// rather than dropping them is what keeps the ids stable, and rather than
/// refusing to start is what lets one saved setup serve both the desk and a
/// two-input laptop interface.
///
/// Deliberately not called "muted": that word belongs to the M beside the
/// meter, which an operator presses on a channel that *is* there. A silent
/// channel has to be traceable to one or the other.
export const absentChannels = (channels: Channel[], deviceChannels: number): number[] =>
  channels
    .map((c, streamId) =>
      isInput(c) && (c.channel < 0 || c.channel >= deviceChannels) ? streamId : -1,
    )
    .filter((streamId) => streamId >= 0);

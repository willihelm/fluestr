// Groups: named views onto a subset of the channels.
//
// Non-exclusive on purpose. A channel is not *in* one group the way a file is
// in one directory — the same talkback belongs in "Bühne" for the stage
// manager's view and in "Alles Licht" for the lighting one, and forcing a
// choice between them would make one of the two views wrong. So membership is
// a many-to-many relation, and removing a channel from a group removes it from
// that view and nothing else.
//
// Membership is by channel `key`, never by stream id. The stream id is the
// row's position in the channel list, so deleting the second of eight channels
// would silently re-point every group below it at a different channel. The key
// is stable for the lifetime of the row.

import type { Channel } from "./streams";

export interface Group {
  key: number;
  name: string;
  /// Channel keys, not stream ids. See the note above.
  members: number[];
}

/// The all-channels view, which is not a group and cannot be edited or
/// deleted: it is what the chat shows when no group is selected.
export const ALL_CHANNELS = null;

let nextKey = 0;

export const newGroup = (existing: Group[]): Group => ({
  key: nextKey++,
  name: `Group ${existing.length + 1}`,
  members: [],
});

/// A group read back from the stored configuration (P12).
///
/// The members arrive as channel keys, already translated from the positions
/// the file stores — see `profiles.ts`, which is the one place that
/// translation happens.
export const restoreGroup = (name: string, members: number[]): Group => ({
  key: nextKey++,
  name,
  members,
});

export const isMember = (group: Group, channel: Channel): boolean =>
  group.members.includes(channel.key);

/// Toggling membership is the only way it changes, and it is idempotent in
/// both directions — the UI offers a checkbox per channel, and a double event
/// from a click must not leave the group in the other state.
export const toggleMember = (group: Group, channel: Channel): Group => ({
  ...group,
  members: isMember(group, channel)
    ? group.members.filter((k) => k !== channel.key)
    : [...group.members, channel.key],
});

/// A channel deleted from the setup has to leave every group with it,
/// otherwise the groups accumulate keys pointing at nothing — invisible until
/// a later channel happens to be given a recycled key and appears in a group
/// nobody put it in.
export const forgetChannels = (groups: Group[], channels: Channel[]): Group[] => {
  const live = new Set(channels.map((c) => c.key));
  return groups.map((g) => ({ ...g, members: g.members.filter((k) => live.has(k)) }));
};

/// The stream ids a group shows, or `null` for "every stream".
///
/// Translating from channel keys to stream ids happens here and only here: the
/// stream id is the channel's position in the list, which is exactly how the
/// backend assigns it.
export const visibleStreams = (
  group: Group | typeof ALL_CHANNELS,
  channels: Channel[],
): Set<number> | null => {
  if (group === ALL_CHANNELS) return null;

  const streams = new Set<number>();
  channels.forEach((c, streamId) => {
    if (group.members.includes(c.key)) streams.add(streamId);
  });
  return streams;
};

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
  /// Whether a view of this group asks for the window's attention on every
  /// message, not only on an alert term (P17).
  ///
  /// Per group because that is the scope the question is asked at: the stage
  /// manager's group is worth looking up for whatever is said on it, the
  /// general talkback is not, and one setting for both would have to be off.
  /// Off by default — an app that starts by flashing at everything gets its
  /// attention requests ignored.
  flash: boolean;
  /// Whether this group's detached window (P7) stays in front of other
  /// windows.
  ///
  /// Set on that window itself, by the pin in its header, and stored here only
  /// so the next window this group opens comes up the same way — during a show
  /// "keep this one where I can see it" is asked of the window in front of the
  /// operator, not of a page in another one. Per group rather than a property
  /// of every group window, because a window that is always in front is a
  /// window covering something: the stage manager's view is worth that, the
  /// general talkback is not. Off by default for the same reason.
  onTop: boolean;
}

/// The all-channels view, which is not a group and cannot be edited or
/// deleted: it is what the chat shows when no group is selected.
export const ALL_CHANNELS = null;

let nextKey = 0;

export const newGroup = (existing: Group[]): Group => ({
  key: nextKey++,
  name: `Group ${existing.length + 1}`,
  members: [],
  flash: false,
  onTop: false,
});

/// A group read back from the stored configuration (P12).
///
/// The members arrive as channel keys, already translated from the positions
/// the file stores — see `profiles.ts`, which is the one place that
/// translation happens.
export const restoreGroup = (
  name: string,
  members: number[],
  flash: boolean = false,
  onTop: boolean = false,
): Group => ({
  key: nextKey++,
  name,
  members,
  flash,
  onTop,
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

/// The channels of one view, in stream order, for the chat's channel filter
/// (P9).
///
/// The stream id is the channel's position in the profile, which is how the
/// backend assigns it (`SessionConfig::streams`) and how every event names it.
/// `visible` is the group's set, or `null` for the all-channels view.
export const viewStreams = (
  channels: Channel[],
  visible: Set<number> | null,
): { streamId: number; label: string }[] =>
  channels
    .map((channel, streamId) => ({ streamId, label: channel.label }))
    .filter(({ streamId }) => visible === null || visible.has(streamId));

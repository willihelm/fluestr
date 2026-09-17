// Configuration profiles, and the translation between what the Settings page
// edits and what the configuration file holds (P12).
//
// A profile is one production's setup: an interface, a channel map with its
// labels, languages and colours, and the groups those channels are seen
// through. One engineer works several productions on the same machine — "Oper
// grosses Haus" is eight channels in five groups, "Probebühne" is three — and
// switching between them must not mean typing either of them in again.
//
// The vocabulary is deliberately *not* part of a profile. It is one global
// list, for the same reason it is session-wide rather than per channel: the
// jargon of a house outlives any one patch, and a copy per profile is a copy
// that drifts. See `vocabulary.ts` and P15.
//
// This module owns exactly one conversion, and owns it alone: group membership
// is a set of **channel keys** while the setup is being edited and a set of
// **positions** in the file. Both are right where they are. The key is stable
// while a row is edited, which is what stops deleting a channel from
// re-pointing every group below it; the position is what still means something
// after a restart, when every key is a number this window has never issued.

import { newChannel, restoreChannel, type Channel } from "./streams";
import { restoreGroup, type Group } from "./groups";
import { CONFIG_VERSION, type StoredConfig, type StoredProfile } from "./bridge";

export interface Profile {
  /// Stable for the lifetime of the window, like a channel's — it is what the
  /// switcher and Svelte identify a profile by, and it is not stored.
  key: number;
  name: string;
  device: string;
  channels: Channel[];
  groups: Group[];
}

let nextKey = 0;

/// A new, empty profile for the given device.
///
/// With one channel rather than none, for the same reason the app has always
/// started with one: an empty channel list is a dead end that the operator has
/// to work out how to leave.
export const newProfile = (existing: Profile[], device: string): Profile => ({
  key: nextKey++,
  name: `Profile ${existing.length + 1}`,
  device,
  channels: [newChannel([])],
  groups: [],
});

/// A profile as the file holds it. Membership translated from channel keys to
/// positions; a key that is not in the channel list is dropped, which is the
/// same thing `forgetChannels` does live.
export const toStored = (profile: Profile): StoredProfile => {
  const position = new Map(profile.channels.map((c, index) => [c.key, index]));

  return {
    name: profile.name,
    device: profile.device,
    channels: profile.channels.map((c) => ({
      channel: c.channel,
      label: c.label,
      language: c.language,
      color: c.color,
    })),
    groups: profile.groups.map((g) => ({
      name: g.name,
      // `flatMap` rather than a filter: a key with no position is a channel
      // that is no longer there, and dropping it is what `forgetChannels`
      // already does live.
      members: g.members.flatMap((key) => {
        const index = position.get(key);
        return index === undefined ? [] : [index];
      }),
    })),
  };
};

/// A profile read back from the file, with fresh keys.
///
/// A member position past the end of the channel list is dropped rather than
/// refused — the backend already does this on load, and doing it here too
/// means a file written by hand costs a group its member and not the operator
/// their setup.
export const fromStored = (stored: StoredProfile): Profile => {
  const channels = stored.channels.map(restoreChannel);

  return {
    key: nextKey++,
    name: stored.name,
    device: stored.device,
    channels,
    groups: stored.groups.map((g) =>
      restoreGroup(
        g.name,
        g.members.filter((p) => p >= 0 && p < channels.length).map((p) => channels[p].key),
      ),
    ),
  };
};

/// A copy under its own name, with its own keys.
///
/// Through the file's own shape on purpose: duplicating is exactly a save and
/// a load, so it goes through the same translation rather than a second one
/// that could disagree with it.
export const duplicate = (profile: Profile): Profile => ({
  ...fromStored(toStored(profile)),
  name: `${profile.name} (copy)`,
});

/// What `save_config` is sent.
///
/// The whole configuration, not a patch: it is a few kilobytes that a human
/// edits at human speed.
export const toStoredConfig = (
  profiles: Profile[],
  activeKey: number,
  vocabulary: string,
): StoredConfig => ({
  version: CONFIG_VERSION,
  // Stored as a position, like group membership and for the same reason: the
  // key means nothing to the next window.
  active: Math.max(
    0,
    profiles.findIndex((p) => p.key === activeKey),
  ),
  vocabulary,
  profiles: profiles.map(toStored),
});

/// What `load_config` returned, as the UI holds it.
///
/// `vocabulary: null` means the file has never held one, and the caller fills
/// in the default; an empty string is an operator who cleared the box, which
/// turns the prompt off and has to stay off.
export const fromStoredConfig = (
  stored: StoredConfig,
): { profiles: Profile[]; activeKey: number | null; vocabulary: string | null } => {
  const profiles = stored.profiles.map(fromStored);
  const active = profiles[stored.active] ?? profiles[0];

  return {
    profiles,
    activeKey: active?.key ?? null,
    vocabulary: stored.vocabulary,
  };
};

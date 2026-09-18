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

import { newChannel, restoreChannel, DEFAULT_LANGUAGE, type Channel } from "./streams";
import { restoreGroup, type Group } from "./groups";
import { CONFIG_VERSION, type StoredConfig, type StoredProfile } from "./bridge";

export interface Profile {
  /// Stable for the lifetime of the window, like a channel's — it is what the
  /// switcher and Svelte identify a profile by, and it is not stored.
  key: number;
  name: string;
  device: string;
  /// What a channel added to this profile starts on, and what the picker above
  /// the channel list shows. Every channel keeps its own language; this only
  /// decides what a new one is born with.
  language: string;
  channels: Channel[];
  groups: Group[];
}

let nextKey = 0;

/// A new, empty profile for the given device.
///
/// With one channel rather than none, for the same reason the app has always
/// started with one: an empty channel list is a dead end that the operator has
/// to work out how to leave.
export const newProfile = (
  existing: Profile[],
  device: string,
  /// Carried over from the profile being worked on rather than reset: an
  /// operator who sets up a second production in the same house is not
  /// switching language, and the one place a language is named by default
  /// stays `DEFAULT_LANGUAGE`.
  language: string = DEFAULT_LANGUAGE,
): Profile => ({
  key: nextKey++,
  name: `Profile ${existing.length + 1}`,
  device,
  language,
  channels: [newChannel([], language)],
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
    language: profile.language,
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
    // A file written before profiles had a default language has none, and
    // "" would make every channel added afterwards languageless.
    language: stored.language ?? DEFAULT_LANGUAGE,
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
  models: ModelChoice = {},
): StoredConfig => ({
  version: CONFIG_VERSION,
  // Stored as a position, like group membership and for the same reason: the
  // key means nothing to the next window.
  active: Math.max(
    0,
    profiles.findIndex((p) => p.key === activeKey),
  ),
  vocabulary,
  // `??` and not `||`: an empty manual path is a box the operator cleared,
  // which has to be stored as "no manual path" rather than as an empty string
  // the backend would then have to decide about.
  model: models.model ?? null,
  vadModel: models.vadModel ?? null,
  modelPath: models.modelPath?.trim() ? models.modelPath.trim() : null,
  profiles: profiles.map(toStored),
});

/// Which models the machine is set to use (P11).
///
/// Its own type rather than three loose arguments: it travels from the model
/// picker through `App.svelte` to the save, and three optional strings in a row
/// is how two of them end up swapped.
export interface ModelChoice {
  model?: string | null;
  vadModel?: string | null;
  modelPath?: string | null;
}

/// What `load_config` returned, as the UI holds it.
///
/// `vocabulary: null` means the file has never held one, and the caller fills
/// in the default; an empty string is an operator who cleared the box, which
/// turns the prompt off and has to stay off.
export const fromStoredConfig = (
  stored: StoredConfig,
): {
  profiles: Profile[];
  activeKey: number | null;
  vocabulary: string | null;
  models: ModelChoice;
} => {
  const profiles = stored.profiles.map(fromStored);
  const active = profiles[stored.active] ?? profiles[0];

  return {
    profiles,
    activeKey: active?.key ?? null,
    vocabulary: stored.vocabulary,
    // A file written before P11 has none of these, and `?? null` is what makes
    // that "never chose" — which resolves to the recommended model — rather
    // than `undefined` travelling back into the next save.
    models: {
      model: stored.model ?? null,
      vadModel: stored.vadModel ?? null,
      modelPath: stored.modelPath ?? null,
    },
  };
};

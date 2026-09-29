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

import { clampGain, newChannel, restoreChannel, DEFAULT_LANGUAGE, type Channel } from "./streams";
import { restoreGroup, type Group } from "./groups";
import { restoreRule, type Rule } from "./rules";
import { DEFAULT_DIRECTION, type SortDirection } from "./sort";
import {
  CONFIG_VERSION,
  DEFAULT_OUTPUTS,
  DEFAULT_TUNING as GENERATED_TUNING,
  MAX_SEGMENT_MS_MAX,
  MAX_SEGMENT_MS_MIN,
  MAX_WORKERS,
  REMOTE_DEFAULT_PORT,
  SILENCE_MS_MAX,
  SILENCE_MS_MIN,
  type DeviceOption,
  type StoredConfig,
  type StoredOutputs,
  type StoredProfile,
  type StoredRemote,
  type Tuning,
} from "./bridge";

/// What a configuration that has never held a remote setting is read as (P16):
/// off, on no address. Not a constant on the backend's side alone, because the
/// Settings page needs something to render before the first save.
export const NO_REMOTE: StoredRemote = {
  enabled: false,
  address: "",
  port: REMOTE_DEFAULT_PORT,
  key: "",
};

/// What a configuration that has never held the detection thresholds is read
/// as (P12): the constants the app compiled in before they were settings.
///
/// Beside `NO_REMOTE` and for its reason — the Settings page needs something
/// to render before the first save — and the numbers come from the backend's
/// own constants, so "the default" means one thing on both sides.
export const DEFAULT_TUNING: Tuning = GENERATED_TUNING;

/// Where audio leaves the app in a profile that never chose (P22, P26):
/// replays on the system output, no shoutbox. A fresh copy each time, because
/// a profile's outputs are edited in place and two profiles must not share
/// one object.
///
/// Laid over the defaults field by field, so a file written by a version
/// that had fewer of them comes back with the rest filled in rather than
/// `undefined` on the page.
export const restoreOutputs = (stored?: Partial<StoredOutputs> | null): StoredOutputs => {
  const base = structuredClone(DEFAULT_OUTPUTS);
  return {
    listen: {
      ...base.listen,
      ...(stored?.listen ?? {}),
      output: { ...base.listen.output, ...(stored?.listen?.output ?? {}) },
    },
    shoutbox: {
      ...base.shoutbox,
      ...(stored?.shoutbox ?? {}),
      program: { ...base.shoutbox.program, ...(stored?.shoutbox?.program ?? {}) },
      reference: { ...base.shoutbox.reference, ...(stored?.shoutbox?.reference ?? {}) },
      output: { ...base.shoutbox.output, ...(stored?.shoutbox?.output ?? {}) },
      calibrationDb: stored?.shoutbox?.calibrationDb ?? null,
    },
  };
};

/// Thresholds a session can actually be started with.
///
/// Applied on the way into the file rather than while typing, for the port's
/// reason: a number box being filled in goes through states that are not
/// numbers yet, and a field that fights them is a field nobody can fill in.
/// The backend clamps again — this is a page, not the only caller.
export const clampTuning = (tuning: Tuning): Tuning => ({
  silenceTimeoutMs: clampMs(
    tuning.silenceTimeoutMs,
    SILENCE_MS_MIN,
    SILENCE_MS_MAX,
    GENERATED_TUNING.silenceTimeoutMs,
  ),
  maxSegmentMs: clampMs(
    tuning.maxSegmentMs,
    MAX_SEGMENT_MS_MIN,
    MAX_SEGMENT_MS_MAX,
    GENERATED_TUNING.maxSegmentMs,
  ),
  speechThresholdPercent: Number.isFinite(tuning.speechThresholdPercent)
    ? Math.min(100, Math.max(0, Math.trunc(tuning.speechThresholdPercent)))
    : GENERATED_TUNING.speechThresholdPercent,
  // A file written before the switch existed has no such field, and the
  // default is on — the same answer the backend gives it.
  carryContext: typeof tuning.carryContext === "boolean"
    ? tuning.carryContext
    : GENERATED_TUNING.carryContext,
});

const clampMs = (value: number, min: number, max: number, fallback: number): number =>
  Number.isFinite(value) ? Math.min(max, Math.max(min, Math.trunc(value))) : fallback;

/// A port a browser can actually be pointed at.
///
/// Below 1024 needs privileges the app does not have and would fail to bind;
/// above 65535 is not a port. Applied on the way out of the Settings page
/// rather than while typing, because a field that fights the keystrokes of
/// somebody typing "8787" one digit at a time is a field nobody can fill in —
/// and applied again on the way into the file, because a number box that has
/// been cleared binds to `null`, which the backend reads as no port at all.
export const clampPort = (port: number): number => {
  if (!Number.isFinite(port)) return REMOTE_DEFAULT_PORT;
  return Math.min(65535, Math.max(1024, Math.trunc(port)));
};

/// A worker count a session can actually be started with.
///
/// Applied on the way into the file rather than while typing, for the reason
/// `clampPort` is: a box that fights somebody typing a number one digit at a
/// time is a box nobody can fill in. A cleared box is `null` — "never chose",
/// which is the default — and never zero, which is a pool with nothing in it.
export const clampWorkers = (workers: number | null): number | null => {
  if (workers === null || !Number.isFinite(workers)) return null;
  return Math.min(MAX_WORKERS, Math.max(1, Math.trunc(workers)));
};

export interface Profile {
  /// Stable for the lifetime of the window, like a channel's — it is what the
  /// switcher and Svelte identify a profile by, and it is not stored.
  key: number;
  name: string;
  /// The interface's display name, for reading. Kept beside the id because a
  /// device that is not plugged in still has to be nameable.
  device: string;
  /// The interface's stable id, which is what the profile is keyed on: it
  /// survives a rename and tells two units of the same model apart. Empty
  /// where it is not known yet — see `adoptDeviceIds`.
  deviceId: string;
  /// What a channel added to this profile starts on, and what the picker above
  /// the channel list shows. Every channel keeps its own language; this only
  /// decides what a new one is born with.
  language: string;
  channels: Channel[];
  groups: Group[];
  /// What fires on this profile's channels, and what it does (P19).
  ///
  /// On the profile rather than beside it, unlike the vocabulary and unlike
  /// the alert list it replaces: a trigger names channels and groups, and
  /// neither exists outside a profile. A shared list could hold "anything on
  /// channel 3", and channel 3 is a different channel in the next production.
  rules: Rule[];
  /// Where audio leaves the app (P22, P26): the listen output replays are
  /// heard on, and the shoutbox that follows the room. On the profile because
  /// both are patched on the interface it names. Held in the file's own
  /// shape: nothing in it is keyed.
  outputs: StoredOutputs;
}

let nextKey = 0;

/// A new, empty profile for the given device.
///
/// With one channel rather than none, for the same reason the app has always
/// started with one: an empty channel list is a dead end that the operator has
/// to work out how to leave.
export const newProfile = (
  existing: Profile[],
  /// Name and id together, as one argument rather than two adjacent strings:
  /// two positional strings in a row is how the id ends up in the name.
  device: { name: string; id: string },
  /// Carried over from the profile being worked on rather than reset: an
  /// operator who sets up a second production in the same house is not
  /// switching language, and the one place a language is named by default
  /// stays `DEFAULT_LANGUAGE`.
  language: string = DEFAULT_LANGUAGE,
): Profile => ({
  key: nextKey++,
  name: `Profile ${existing.length + 1}`,
  device: device.name,
  deviceId: device.id,
  language,
  channels: [newChannel([], language)],
  groups: [],
  // Quiet to begin with, for the reason a new group is: an app that starts by
  // flashing at everything gets its attention requests ignored.
  rules: [],
  outputs: restoreOutputs(),
});

/// A profile as the file holds it. Membership translated from channel keys to
/// positions; a key that is not in the channel list is dropped, which is the
/// same thing `forgetChannels` does live.
export const toStored = (profile: Profile): StoredProfile => {
  const position = new Map(profile.channels.map((c, index) => [c.key, index]));
  // Groups are stored by position too, for the same reason membership is: a
  // rule that names a group has to still name it after a restart, and the key
  // is a counter this window handed out.
  const groupPosition = new Map(profile.groups.map((g, index) => [g.key, index]));

  return {
    name: profile.name,
    device: profile.device,
    deviceId: profile.deviceId,
    language: profile.language,
    channels: profile.channels.map((c) => ({
      channel: c.channel,
      label: c.label,
      language: c.language,
      color: c.color,
      gainDb: clampGain(c.gainDb),
      muted: c.muted,
      description: c.description,
    })),
    groups: profile.groups.map((g) => ({
      name: g.name,
      onTop: g.onTop,
      // `flatMap` rather than a filter: a key with no position is a channel
      // that is no longer there, and dropping it is what `forgetChannels`
      // already does live.
      members: g.members.flatMap((key) => {
        const index = position.get(key);
        return index === undefined ? [] : [index];
      }),
    })),
    rules: profile.rules.map((rule) => ({
      name: rule.name,
      enabled: rule.enabled,
      words: rule.words,
      // `flatMap` for the members' reason: a key with no position names
      // something that is no longer there, and `forget` already drops it live.
      channels: rule.channels.flatMap((key) => {
        const index = position.get(key);
        return index === undefined ? [] : [index];
      }),
      // The windows the flash is for, by the same positions the group
      // windows are named by.
      views: rule.views.flatMap((key) => {
        const index = groupPosition.get(key);
        return index === undefined ? [] : [index];
      }),
      actions: rule.actions,
      // What the `webhook` action calls, stored with the rule and untouched
      // here: it holds no keys and no positions, so there is nothing to
      // translate. A rule edited on the LAN remote carries its destination
      // back out exactly as it came in — the remote has no box to change it
      // with (P19).
      webhook: rule.webhook,
      minIntervalMs: rule.minIntervalMs,
    })),
    // A copy, so the object the page is editing is not the one on its way to
    // the backend. Through JSON rather than `structuredClone`, which refuses
    // the reactive proxy the page holds it in; the shape is plain data.
    outputs: JSON.parse(JSON.stringify(profile.outputs)) as StoredOutputs,
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
  const groups = stored.groups.map((g) =>
    restoreGroup(
      g.name,
      g.members.filter((p) => p >= 0 && p < channels.length).map((p) => channels[p].key),
      // A file that predates the window setting opens its group windows like
      // any other window.
      g.onTop ?? false,
    ),
  );

  return {
    key: nextKey++,
    name: stored.name,
    device: stored.device,
    // A file written before stable ids has none. Empty rather than the name:
    // "not known yet" and "known to be this" are different states, and only
    // the first one may be overwritten by what is currently connected.
    deviceId: stored.deviceId ?? "",
    // A file written before profiles had a default language has none, and
    // "" would make every channel added afterwards languageless.
    language: stored.language ?? DEFAULT_LANGUAGE,
    channels,
    groups,
    // A file written before P19 has no rules; the backend's migration is what
    // fills them in from the alert terms and the flash flags, so by the time
    // one reaches here it has them. `?? []` is for the one case that migration
    // does not cover: a hand-written file.
    rules: (stored.rules ?? []).map((rule) =>
      restoreRule({
        name: rule.name,
        enabled: rule.enabled ?? true,
        words: rule.words ?? [],
        channels: (rule.channels ?? [])
          .filter((p) => p >= 0 && p < channels.length)
          .map((p) => channels[p].key),
        views: (rule.views ?? [])
          .filter((p) => p >= 0 && p < groups.length)
          .map((p) => groups[p].key),
        actions: rule.actions ?? [],
        webhook: rule.webhook ?? null,
        minIntervalMs: rule.minIntervalMs ?? 0,
      }),
    ),
    // A file written before P22 has none, which is no shoutbox and replays
    // on the system output.
    outputs: restoreOutputs(stored.outputs),
  };
};

/// The connected device a profile is set to, if it is there at all.
///
/// By id whenever the profile has one, and by name only when it does not. That
/// order is the whole point of the id: once a profile has one, a device whose
/// id is absent *is* absent — an interface that was renamed is still found,
/// and an identically named second unit is not mistaken for it.
///
/// The name fallback is for a profile that predates stable ids, and it can
/// only pick the first device of that name. That is exactly the ambiguity the
/// id exists to remove, and it lasts until the device is seen once.
export const connectedDevice = (
  devices: DeviceOption[],
  profile: { device: string; deviceId: string },
): DeviceOption | undefined =>
  profile.deviceId
    ? devices.find((d) => d.id === profile.deviceId)
    : devices.find((d) => d.name === profile.device);

/// Fills in the stable ids of profiles that have none, from what is connected.
///
/// Returns `null` when there was nothing to fill in, so the caller can tell a
/// migration from a no-op and only save in the first case.
///
/// This is the whole upgrade path: an operator with an existing setup plugs in
/// the interface they always use, and the profile quietly gains an id that
/// survives the next rename. Nothing is asked of them, and a profile whose
/// device is not connected is left alone rather than bound to whatever is.
export const adoptDeviceIds = (
  profiles: Profile[],
  devices: DeviceOption[],
): Profile[] | null => {
  let changed = false;

  const adopted = profiles.map((profile) => {
    if (profile.deviceId || !profile.device) return profile;

    const device = connectedDevice(devices, profile);
    if (!device) return profile;

    changed = true;
    // The name comes along: a device may have been renamed between the save
    // and now, and the profile should show what the operator sees today.
    return { ...profile, device: device.name, deviceId: device.id };
  });

  return changed ? adopted : null;
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
  /// The LAN remote client (P16). Beside the profiles like the model, and for
  /// the model's reason: which interface this machine can be reached on is a
  /// property of the machine and of the house it is standing in, not of the
  /// production being carried to it.
  remote: StoredRemote = NO_REMOTE,
  /// Which end of the chat new lines arrive at (P7). Beside the profiles like
  /// the vocabulary, and for its reason: how a house reads its chat outlives
  /// any one patch. A view may still override it for itself, and that override
  /// never comes through here — see `sort.ts`.
  chatSort: SortDirection = DEFAULT_DIRECTION,
  /// How the pipeline cuts and what it listens to (P12). Beside the profiles
  /// like the vocabulary, and for its reason: how hard the intercom is to hear
  /// is a property of the house, not of the production passing through it.
  tuning: Tuning = DEFAULT_TUNING,
  /// Whether the active profile's session starts the moment the app comes
  /// up. Beside the profiles for the remote's reason: whether this machine is
  /// attended when it is switched on is a property of where it stands, not of
  /// the production currently patched into it.
  autoStart: boolean = false,
  /// Whether the first-run setup guide has been walked, or left. Beside the
  /// profiles for the model's reason: it is a fact about this installation —
  /// whether the person in front of it has been shown the way through — and
  /// not about any production carried to it.
  setupDone: boolean = false,
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
  // Clamped here as well as in the box, because the box can be left in the
  // middle of a number: the window closing is the case the last-ditch save
  // exists for, and the backend would rather not be handed a `0`.
  workers: clampWorkers(models.workers ?? null),
  // Not the object as the page holds it: the port is a number box, and a box
  // an operator has cleared holds `null` until it is left. This save can
  // happen before that — the window closing is the case the last-ditch save
  // exists for — and `null` fails the backend's `u16` outright.
  // The key is trimmed on both sides — see `sanitise` in `config.rs`. A field
  // only one side tidies is a field that makes every save look like somebody
  // else's change, which is how the Settings page ends up rebuilding the box
  // being typed in.
  remote: { ...remote, port: clampPort(remote.port), key: (remote.key ?? "").trim() },
  chatSort,
  // Clamped here as well as in the boxes, for the port's reason: a box left
  // mid-number is exactly the state the last-ditch save catches.
  tuning: clampTuning(tuning),
  autoStart,
  setupDone,
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
  /// How many sidecars decode in parallel. `null` is "never chose", which is
  /// `DEFAULT_WORKERS`. It travels with the models rather than beside them
  /// because it is the same kind of setting and edited on the same page: what
  /// this machine can run, as opposed to what this production needs.
  workers?: number | null;
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
  remote: StoredRemote;
  chatSort: SortDirection;
  tuning: Tuning;
  autoStart: boolean;
  setupDone: boolean;
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
      workers: stored.workers ?? null,
    },
    // A file written before P16 has no remote block, and reading its absence
    // as anything but "off" is the one way this feature could start serving a
    // network nobody asked it to.
    remote: stored.remote ?? NO_REMOTE,
    // A file written before the chat could be turned over has no such field,
    // and what its absence meant is what the chat did then.
    chatSort: stored.chatSort ?? DEFAULT_DIRECTION,
    // A file written before these were settings has no such block, and what
    // its absence meant is the constants the app ran on then.
    tuning: stored.tuning ? clampTuning(stored.tuning) : DEFAULT_TUNING,
    // A file written before this existed describes a machine that had to be
    // started by hand, which is what its absence must keep meaning.
    autoStart: stored.autoStart ?? false,
    // A file written before the guide existed cannot say whether anybody was
    // ever shown it. `false` is the honest reading, and the app settles it at
    // startup rather than greeting a working installation with a welcome
    // screen — see `App.svelte`.
    setupDone: stored.setupDone ?? false,
  };
};

/// Carries the keys a view already holds onto the configuration it has just
/// read back (P16).
///
/// Keys are handed out per window and mean nothing outside it, so a
/// configuration read a second time — because another peer saved it — comes
/// back with all new ones. Two things break if they are simply taken on.
/// Everything the view keyed on is suddenly pointing at something that no
/// longer exists: the selected group falls back to the all-channels view, and
/// the active profile looks like a different profile, which is what closes the
/// detached group windows. And every keyed block on the Settings page is torn
/// down and rebuilt — which takes the focus out of the field being typed in,
/// on every save, which is every keystroke.
///
/// By **position**, which is what the file stores membership and the active
/// profile by anyway. A rename or a channel edit keeps every position and
/// therefore every key, so the page is updated in place and the cursor stays
/// where it was; adding, deleting or reordering shifts them, and the view
/// treats that as the setup change it is.
///
/// Group membership is expressed in channel keys, so carrying those over means
/// rewriting the membership to match. That is the whole of the work here, and
/// it is the reason this is one function rather than three.
export const reuseKeys = (fresh: Profile[], previous: Profile[]): Profile[] =>
  fresh.map((profile, index) => {
    const before = previous[index];
    if (!before) return profile;

    const channels = profile.channels.map((channel, position) => {
      const same = before.channels[position];
      return same ? { ...channel, key: same.key } : channel;
    });
    // Fresh key to carried-over key. A channel with no counterpart keeps the
    // key it was read with, and maps to itself.
    const carried = new Map(profile.channels.map((c, i) => [c.key, channels[i].key]));

    const groups = profile.groups.map((group, position) => {
      const same = before.groups[position];
      return {
        ...group,
        key: same ? same.key : group.key,
        members: group.members.map((member) => carried.get(member) ?? member),
      };
    });
    // The same map for groups, because a rule's flash points at them too
    // (P19): a rule left holding a fresh group key while the sidebar held the
    // carried one would be a flash that quietly stops reaching the window it
    // names.
    const carriedGroups = new Map(profile.groups.map((g, i) => [g.key, groups[i].key]));

    return {
      ...profile,
      key: before.key,
      channels,
      groups,
      rules: profile.rules.map((rule) => ({
        ...rule,
        channels: rule.channels.map((key) => carried.get(key) ?? key),
        views: rule.views.map((key) => carriedGroups.get(key) ?? key),
      })),
    };
  });

/// A configuration as a string that does not depend on the order its fields
/// happen to be written in.
///
/// What it is for is recognising the echo of one's own save (P16). Every save
/// is announced to every view including the one that caused it, and the
/// announcement has been through the backend — which serialises the struct in
/// its own field order and fills in what the file left out. Comparing the raw
/// JSON would call every one of those a foreign change, which is the same
/// thing as having no echo suppression at all.
export const canonical = (config: unknown): string =>
  JSON.stringify(config, (_key, value) =>
    value !== null && typeof value === "object" && !Array.isArray(value)
      ? Object.fromEntries(
          Object.entries(value as Record<string, unknown>).sort(([a], [b]) =>
            a < b ? -1 : a > b ? 1 : 0,
          ),
        )
      : value,
  );

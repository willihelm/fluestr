// What a stored profile has to survive (P12).
//
// Almost all of this is about one translation: group membership is a set of
// channel keys while the setup is edited and a set of positions in the file.
// Getting it wrong is not a crash — it is a group that silently watches the
// wrong channel after a restart, which is exactly the kind of failure nobody
// notices until the show.

import { describe, expect, it } from "vitest";
import {
  adoptDeviceIds,
  canonical,
  clampTuning,
  DEFAULT_TUNING,
  connectedDevice,
  duplicate,
  fromStored,
  fromStoredConfig,
  newProfile,
  restoreOutputs,
  reuseKeys,
  toStored,
  toStoredConfig,
  type Profile,
} from "./profiles";
import type { StoredConfig, Tuning } from "./bridge";
import {
  restoreChannel,
  remoteChannel,
  toConfig,
  validate,
  absentChannels,
  NO_INPUT,
  DEFAULT_LANGUAGE,
} from "./streams";
import { restoreGroup } from "./groups";
import { restoreRule } from "./rules";
import type { DeviceOption, StoredProfile } from "./bridge";

/// A file as an **older version wrote it**: the fields that version did not
/// have are simply missing.
///
/// A cast is needed because the types are generated from the Rust structs now
/// (P6), and those describe what the backend *sends* — which is always
/// complete, because `#[serde(default)]` fills every gap before the file
/// reaches the bridge. That is precisely why a short file has to be built by
/// hand to be tested at all. One helper, so that the deliberate incompleteness
/// is stated once rather than smuggled into ten fixtures.
const anOldFile = <T>(written: unknown): T => written as T;

/// Two channels and a group holding the second of them, built the way the
/// Settings page builds one — with keys, not positions.
const aProfile = (): Profile => {
  const channels = [
    restoreChannel({
      id: "regie",
      channel: 0,
      label: "Regie",
      language: "de",
      color: "#6ea8fe",
      gainDb: 0,
      muted: false,
      preview: true,
    }),
    // Trimmed and muted, so the round trip has values to lose.
    restoreChannel({
      id: "licht",
      channel: 3,
      label: "Licht",
      language: "en",
      color: "#5fd38d",
      gainDb: 9,
      muted: true,
      // Undrafted, so the round trip has that to lose too.
      preview: false,
    }),
  ];

  const groups = [restoreGroup("Bühne", [channels[1].key])];

  return {
    key: 99,
    name: "Grosses Haus",
    device: "MADIface",
    deviceId: "AppleUSBAudioEngine:RME:MADIface:2",
    language: "de",
    channels,
    groups,
    rules: [
      restoreRule({
        name: "Feuer",
        enabled: true,
        words: ["Feuer"],
        channels: [],
        views: [],
        actions: ["highlight", "urgentAttention"],
        webhook: null,
        minIntervalMs: 0,
      }),
      restoreRule({
        name: "Go auf der Bühne",
        enabled: true,
        words: ["Go"],
        channels: [channels[1].key],
        views: [groups[0].key],
        actions: ["flash"],
        webhook: null,
        minIntervalMs: 1_000,
      }),
    ],
    outputs: restoreOutputs(),
  };
};

const stored: StoredProfile = {
  name: "Grosses Haus",
  device: "MADIface",
  deviceId: "AppleUSBAudioEngine:RME:MADIface:2",
  language: "de",
  channels: [
    {
      id: "regie",
      source: null,
      channel: 0,
      label: "Regie",
      language: "de",
      color: "#6ea8fe",
      gainDb: 0,
      muted: false,
      preview: true,
    },
    {
      id: "licht",
      source: null,
      channel: 3,
      label: "Licht",
      language: "en",
      color: "#5fd38d",
      gainDb: 9,
      muted: true,
      preview: false,
    },
  ],
  groups: [{ name: "Bühne", onTop: false, members: [1] }],
  rules: [
    {
      name: "Feuer",
      enabled: true,
      words: ["Feuer"],
      channels: [],
      views: [],
      actions: ["highlight", "urgentAttention"],
      webhook: null,
      minIntervalMs: 0,
    },
    {
      name: "Go auf der Bühne",
      enabled: true,
      words: ["Go"],
      channels: [1],
      views: [0],
      actions: ["flash"],
      webhook: null,
      minIntervalMs: 1_000,
    },
  ],
  outputs: restoreOutputs(),
};

describe("toStored", () => {
  it("writes membership as positions, because keys mean nothing to the next window", () => {
    expect(toStored(aProfile())).toEqual(stored);
  });

  it("drops a member whose channel is gone rather than storing a dangling one", () => {
    // The same thing `forgetChannels` does live. A stored key pointing at
    // nothing would come back as a member of whatever later took that slot.
    const profile = aProfile();
    profile.groups[0].members = [profile.channels[1].key, 12345];

    expect(toStored(profile).groups[0].members).toEqual([1]);
  });
});

describe("restoreOutputs", () => {
  it("fills in what a file written before the outputs existed does not have", () => {
    const outputs = restoreOutputs(null);
    expect(outputs.shoutbox.enabled).toBe(false);
    expect(outputs.listen.keepMinutes).toBe(10);
    expect(outputs.listen.output.deviceId).toBe("");
  });

  it("keeps what was stored and fills in only what is missing", () => {
    const outputs = restoreOutputs({
      shoutbox: { ...restoreOutputs().shoutbox, enabled: true, trimDb: -4 },
    } as never);
    expect(outputs.shoutbox.enabled).toBe(true);
    expect(outputs.shoutbox.trimDb).toBe(-4);
    expect(outputs.listen.keepMinutes).toBe(10);
  });

  it("hands every profile an outputs object of its own", () => {
    const a = restoreOutputs();
    const b = restoreOutputs();
    a.shoutbox.muted = true;
    expect(b.shoutbox.muted).toBe(false);
  });
});

describe("fromStored", () => {
  it("gives every channel a key of this window's own", () => {
    // Keys are handed out per window and are not stored. Two profiles read
    // from the same file must not end up sharing them, or a group in one would
    // match a channel in the other.
    const first = fromStored(stored);
    const second = fromStored(stored);
    const keys = [...first.channels, ...second.channels].map((c) => c.key);

    expect(new Set(keys).size).toBe(keys.length);
  });

  it("points the group at the same channel it was pointed at before", () => {
    const profile = fromStored(stored);

    expect(profile.groups[0].members).toEqual([profile.channels[1].key]);
  });

  it("drops a member position past the end of the channel list", () => {
    // A hand-edited file costs a group its member, not the operator the setup.
    const broken = anOldFile<StoredProfile>({
      ...stored,
      groups: [{ name: "Bühne", members: [1, 9, -1] }],
    });
    const profile = fromStored(broken);

    expect(profile.groups[0].members).toEqual([profile.channels[1].key]);
  });
});

describe("the round trip", () => {
  it("brings back what was put in, down to colours and languages", () => {
    const before = aProfile();
    const after = fromStored(toStored(before));

    expect(after.name).toBe(before.name);
    expect(after.device).toBe(before.device);
    expect(after.channels.map(({ key, ...rest }) => rest)).toEqual(
      before.channels.map(({ key, ...rest }) => rest),
    );
    expect(after.groups[0].name).toBe(before.groups[0].name);
    // The keys differ, the channel they mean does not.
    expect(after.groups[0].members).toEqual([after.channels[1].key]);
  });

  it("survives a channel being deleted between two saves", () => {
    // The case positions are the risky choice for: delete the first channel
    // and the group must still mean the *same* channel at its new position,
    // not whatever moved into the old slot.
    const profile = fromStored(stored);
    profile.channels = profile.channels.slice(1);

    const again = fromStored(toStored(profile));
    expect(again.channels.map((c) => c.label)).toEqual(["Licht"]);
    expect(again.groups[0].members).toEqual([again.channels[0].key]);
  });
});

describe("duplicate", () => {
  it("copies the setup without sharing a single key with it", () => {
    // A copy that shared keys would have the original's groups follow edits to
    // the copy's channels.
    const before = aProfile();
    const copy = duplicate(before);

    expect(copy.name).toBe("Grosses Haus (copy)");
    expect(copy.channels.map((c) => c.label)).toEqual(["Regie", "Licht"]);
    expect(copy.groups[0].members).toEqual([copy.channels[1].key]);

    const shared = copy.channels.filter((c) => before.channels.some((b) => b.key === c.key));
    expect(shared).toEqual([]);
    expect(copy.key).not.toBe(before.key);
  });
});

describe("newProfile", () => {
  it("starts on the device it was created from, with one channel", () => {
    // An empty channel list is a dead end the operator has to work out how to
    // leave; a second setup on one machine is usually the same interface
    // patched differently.
    const created = newProfile([], { name: "MADIface", id: "...MADIface:2" });

    expect(created.device).toBe("MADIface");
    expect(created.deviceId).toBe("...MADIface:2");
    expect(created.channels).toHaveLength(1);
    expect(created.groups).toEqual([]);
  });

  it("carries the language over, down to the channel it starts with", () => {
    // Same argument as the device: a second production in the same house is
    // not a second language.
    const created = newProfile([], { name: "MADIface", id: "...MADIface:2" }, "en");

    expect(created.language).toBe("en");
    expect(created.channels[0].language).toBe("en");
  });

  it("falls back to the one default language when none is given", () => {
    expect(newProfile([], { name: "MADIface", id: "...MADIface:2" }).language).toBe(
      DEFAULT_LANGUAGE,
    );
  });
});

/// Two interfaces of the same model, which is the case a name cannot settle,
/// plus the built-in microphone.
const devices: DeviceOption[] = [
  {
    id: "BuiltInMicrophoneDevice",
    name: "MacBook Pro Microphone",
    channels: 1,
    defaultSampleRate: 48000,
    isDefault: true,
  },
  {
    id: "...Scarlett:1",
    name: "Scarlett 2i2",
    channels: 2,
    defaultSampleRate: 48000,
    isDefault: false,
  },
  {
    id: "...Scarlett:2",
    name: "Scarlett 2i2",
    channels: 2,
    defaultSampleRate: 48000,
    isDefault: false,
  },
];

describe("finding the profile's device", () => {
  it("goes by the id, so a renamed interface is still the same interface", () => {
    // The failure this replaces: an operator renames the desk in Audio MIDI
    // Setup and every profile silently reports its interface as missing.
    const found = connectedDevice(devices, {
      device: "Scarlett 2i2 (Bühne)",
      deviceId: "...Scarlett:2",
    });

    expect(found?.id).toBe("...Scarlett:2");
  });

  it("tells two identical interfaces apart", () => {
    const unit = (deviceId: string) =>
      connectedDevice(devices, { device: "Scarlett 2i2", deviceId });

    expect(unit("...Scarlett:1")?.id).toBe("...Scarlett:1");
    expect(unit("...Scarlett:2")?.id).toBe("...Scarlett:2");
  });

  it("reports a device whose id is not there as absent, name or no name", () => {
    // Deliberately *not* falling back to the name here. A profile that has an
    // id has been resolved once already, so a missing id means this device is
    // gone — and the second unit of the same model must not stand in for it.
    const found = connectedDevice(devices, {
      device: "Scarlett 2i2",
      deviceId: "...Scarlett:9",
    });

    expect(found).toBeUndefined();
  });

  it("falls back to the name only for a profile that has no id yet", () => {
    expect(connectedDevice(devices, { device: "Scarlett 2i2", deviceId: "" })?.id).toBe(
      "...Scarlett:1",
    );
  });
});

describe("adopting stable device ids", () => {
  it("gives a profile written before ids the id of the device it names", () => {
    const old = { ...aProfile(), device: "MacBook Pro Microphone", deviceId: "" };

    const adopted = adoptDeviceIds([old], devices);

    expect(adopted?.[0].deviceId).toBe("BuiltInMicrophoneDevice");
    // Everything else about the setup is untouched: this is a migration, not
    // an edit.
    expect(adopted?.[0].channels).toEqual(old.channels);
    expect(adopted?.[0].groups).toEqual(old.groups);
  });

  it("leaves a profile alone when its device is not connected", () => {
    // The office case. Binding the profile to whatever happens to be plugged
    // in would quietly re-point the setup at the built-in microphone.
    const old = { ...aProfile(), device: "MADIface", deviceId: "" };

    expect(adoptDeviceIds([old], devices)).toBeNull();
  });

  it("says nothing changed when every profile already has an id", () => {
    // What tells the caller not to write the file. A migration that reports
    // itself on every start is a save on every start.
    expect(adoptDeviceIds([aProfile()], devices)).toBeNull();
  });
});

describe("the configuration as a whole", () => {
  it("stores which profile was in use as a position", () => {
    const first = aProfile();
    const second = duplicate(first);
    const config = toStoredConfig([first, second], second.key, "Standby, Cue, Go.");

    expect(config.active).toBe(1);
    expect(config.vocabulary).toBe("Standby, Cue, Go.");
    expect(config.profiles).toHaveLength(2);
  });

  it("falls back to the first profile when the active one is not there", () => {
    const config = toStoredConfig([aProfile()], 4321, "");

    expect(config.active).toBe(0);
  });

  // The chat's reading direction (P7), stored once for the whole app beside
  // the vocabulary. A view may override it for itself, and that override is
  // the screen's own — it never travels through the configuration.
  it("stores the chat's reading direction", () => {
    const config = toStoredConfig([aProfile()], 1, "", {}, undefined, "newest-top");

    expect(config.chatSort).toBe("newest-top");
  });

  it("writes the chat downwards when nothing was chosen", () => {
    expect(toStoredConfig([aProfile()], 1, "").chatSort).toBe("newest-bottom");
  });

  // A file written before the chat could be turned over. What its absence
  // meant is what the chat did then, and reading it as anything else would
  // turn over every window of every setup that has one.
  it("reads a file with no direction as the chat it used to be", () => {
    const restored = fromStoredConfig(
      anOldFile<StoredConfig>({
        version: 1,
        active: 0,
        vocabulary: null,
        profiles: [],
      }),
    );

    expect(restored.chatSort).toBe("newest-bottom");
  });

  // The three detection thresholds (P12), stored once for the whole app
  // beside the vocabulary: how hard the intercom is to hear belongs to the
  // house, not to the production passing through it.
  it("stores the thresholds it was given", () => {
    const config = toStoredConfig([aProfile()], 1, "", {}, undefined, undefined, {
      silenceTimeoutMs: 350,
      maxSegmentMs: 4_000,
      speechThresholdPercent: 65,
      carryContext: false,
      livePreview: false,
      previewStepMs: 1_500,
    });

    expect(config.tuning).toEqual({
      silenceTimeoutMs: 350,
      maxSegmentMs: 4_000,
      speechThresholdPercent: 65,
      carryContext: false,
      livePreview: false,
      previewStepMs: 1_500,
    });
  });

  it("writes the defaults when nothing was chosen", () => {
    expect(toStoredConfig([aProfile()], 1, "").tuning).toEqual(DEFAULT_TUNING);
  });

  // A file written before the thresholds were settings. What its absence
  // meant is the constants the app ran on then, and reading it as anything
  // else would change the pipeline of every setup that has one.
  it("reads a file with no thresholds as the pipeline it used to be", () => {
    const restored = fromStoredConfig(
      anOldFile<StoredConfig>({
        version: 2,
        active: 0,
        vocabulary: null,
        profiles: [],
      }),
    );

    expect(restored.tuning).toEqual(DEFAULT_TUNING);
  });

  // The box is left mid-number when the window closes, which is exactly the
  // case the last-ditch save exists for. The backend clamps again; this is so
  // that what the page shows afterwards is what the audio is running on.
  it("brings a hand-edited threshold back into range", () => {
    const clamped = clampTuning({
      silenceTimeoutMs: 5,
      maxSegmentMs: 600_000,
      speechThresholdPercent: 250,
      carryContext: true,
      livePreview: true,
      previewStepMs: 50,
    });

    expect(clamped.silenceTimeoutMs).toBeGreaterThanOrEqual(150);
    expect(clamped.maxSegmentMs).toBeLessThanOrEqual(15_000);
    expect(clamped.speechThresholdPercent).toBe(100);
    expect(clamped.previewStepMs).toBe(300);
  });

  // A file written before the live preview existed (P28) gets it switched on,
  // the default, rather than off by a field that is missing.
  it("reads a file without the live preview switch as switched on", () => {
    const clamped = clampTuning({
      silenceTimeoutMs: 500,
      maxSegmentMs: 6_000,
      speechThresholdPercent: 50,
      carryContext: true,
    } as Tuning);

    expect(clamped.livePreview).toBe(true);
    expect(clamped.previewStepMs).toBe(DEFAULT_TUNING.previewStepMs);
  });

  it("falls back to the default rather than storing a box that is not a number", () => {
    const clamped = clampTuning({
      silenceTimeoutMs: Number.NaN,
      maxSegmentMs: Number.NaN,
      speechThresholdPercent: Number.NaN,
      carryContext: true,
      livePreview: true,
      previewStepMs: Number.NaN,
    });

    expect(clamped).toEqual(DEFAULT_TUNING);
  });

  it("comes back on the profile that was in use", () => {
    const restored = fromStoredConfig(
      anOldFile<StoredConfig>({
        version: 1,
        active: 1,
        vocabulary: "Standby.",
        profiles: [stored, { ...stored, name: "Probebühne" }],
      }),
    );

    expect(restored.profiles[1].key).toBe(restored.activeKey);
    expect(restored.profiles[1].name).toBe("Probebühne");
  });

  it("tells a vocabulary that was never set apart from one that was cleared", () => {
    // `null` keeps the default; an empty string is an operator who turned the
    // prompt off, and refilling it would turn it back on behind their back.
    const never = fromStoredConfig(
      anOldFile<StoredConfig>({ version: 1, active: 0, vocabulary: null, profiles: [] }),
    );
    const cleared = fromStoredConfig(
      anOldFile<StoredConfig>({ version: 1, active: 0, vocabulary: "", profiles: [] }),
    );

    expect(never.vocabulary).toBeNull();
    expect(cleared.vocabulary).toBe("");
  });

  it("keeps a file written before the model manager readable", () => {
    // P11 added three fields to the file. An operator upgrading has one
    // without them, and losing their profiles to a schema addition would be
    // the worst possible trade for a model picker.
    const restored = fromStoredConfig(
      anOldFile<StoredConfig>({
        version: 1,
        active: 0,
        vocabulary: "Standby.",
        profiles: [stored],
      }),
    );

    expect(restored.profiles).toHaveLength(1);
    // "Never chose", which resolves to the recommended model — not
    // `undefined`, which would travel straight back into the next save.
    expect(restored.models).toEqual({
      model: null,
      vadModel: null,
      modelPath: null,
      workers: null,
    });
  });

  it("keeps a file written before profiles had a default language readable", () => {
    // Same trade as above, and with a sharper failure: an empty string here
    // would travel into the picker and out again into every channel added
    // afterwards, which whisper cannot be given.
    const { language: _, ...withoutLanguage } = stored;

    const restored = fromStoredConfig(
      anOldFile<StoredConfig>({
        version: 1,
        active: 0,
        vocabulary: "Standby.",
        profiles: [withoutLanguage],
      }),
    );

    expect(restored.profiles[0].language).toBe(DEFAULT_LANGUAGE);
  });

  it("stores a cleared model path as nothing rather than as an empty string", () => {
    // The backend treats an empty manual path as "not set", and storing `""`
    // would leave it deciding that on every read instead of once here.
    const config = toStoredConfig([aProfile()], 1, "", {
      model: "small-q5_1",
      modelPath: "   ",
    });

    expect(config.model).toBe("small-q5_1");
    expect(config.modelPath).toBeNull();
    // Never chosen is stored as such, not dropped.
    expect(config.vadModel).toBeNull();
  });

  it("keeps the model choice out of the profiles", () => {
    // Which model a machine can run depends on the machine — its memory, its
    // GPU, the latency it can afford — not on the production. Carrying a
    // profile to a laptop must not carry a model that laptop cannot serve.
    const config = toStoredConfig([aProfile()], 1, "", { model: "large-v3-turbo-q5_0" });

    expect(config.model).toBe("large-v3-turbo-q5_0");
    for (const profile of config.profiles) {
      expect(profile).not.toHaveProperty("model");
    }
  });

  it("trims a pasted model path", () => {
    // Paths arrive from a Finder drag or a copy out of a terminal, and both
    // bring whitespace that turns a working path into a missing file.
    const config = toStoredConfig([aProfile()], 1, "", {
      modelPath: "  /Volumes/Stick/ggml-small-q5_1.bin\n",
    });

    expect(config.modelPath).toBe("/Volumes/Stick/ggml-small-q5_1.bin");
  });

  it("has no active profile when there are none at all", () => {
    // The first start, before the UI creates one.
    const empty = fromStoredConfig(
      anOldFile<StoredConfig>({ version: 1, active: 0, vocabulary: null, profiles: [] }),
    );

    expect(empty.profiles).toEqual([]);
    expect(empty.activeKey).toBeNull();
  });
});

describe("rules across the file", () => {
  // The window setting travels with the group: one whose window is meant to
  // stay in front and comes back behind everything is a view an operator
  // believes they can see and cannot.
  it("carries a group's window setting across a save and a load", () => {
    const profile = aProfile();
    profile.groups[0].onTop = true;

    expect(fromStored(toStored(profile)).groups[0].onTop).toBe(true);
  });

  it("opens the windows of a file written before that setting like any other", () => {
    expect(
      fromStored(
        anOldFile<StoredProfile>({ ...stored, groups: [{ name: "Bühne", members: [1] }] }),
      ).groups[0].onTop,
    ).toBe(false);
  });

  // The whole rule, because every part of it is load-bearing: a rule that
  // comes back without its channel fires everywhere, and one that comes back
  // without its limit floods.
  it("carries a rule across a save and a load", () => {
    const rule = fromStored(toStored(aProfile())).rules[1];

    expect(rule.words).toEqual(["Go"]);
    expect(rule.actions).toEqual(["flash"]);
    expect(rule.minIntervalMs).toBe(1_000);
  });

  // Positions on the way out, keys on the way in — the same translation group
  // membership gets, and for the same reason: a key is a counter this window
  // handed out.
  it("stores a rule's channels and views as positions and reads them back as keys", () => {
    const profile = aProfile();
    const written = toStored(profile).rules[1];

    expect(written.channels).toEqual([1]);
    expect(written.views).toEqual([0]);

    const read = fromStored(toStored(profile));
    expect(read.rules[1].channels).toEqual([read.channels[1].key]);
    expect(read.rules[1].views).toEqual([read.groups[0].key]);
  });

  it("drops a rule's channel whose channel is gone rather than storing a dangling one", () => {
    const profile = aProfile();
    profile.channels = [profile.channels[0]];

    expect(toStored(profile).rules[1].channels).toEqual([]);
  });

  it("reads a file written before rules existed as a profile without any", () => {
    const { rules: _, ...before } = stored;
    expect(fromStored(anOldFile<StoredProfile>(before)).rules).toEqual([]);
  });
});

// Adopting a configuration another peer wrote (P16). What matters is not that
// the values arrive — `fromStoredConfig` already does that — but that the
// keys this window has handed out survive it, because everything on screen is
// pointed at them: the selected group, and the profile identity that decides
// whether the detached group windows close.
describe("reuseKeys", () => {
  const profileWith = (name: string, groups: string[]): Profile => ({
    ...fromStored(
      anOldFile<StoredProfile>({
        name,
        device: "Scarlett",
        deviceId: "uid",
        language: "de",
        channels: [{ channel: 0, label: "Regie", language: "de", color: "#fff" }],
        groups: groups.map((g) => ({ name: g, members: [0] })),
      }),
    ),
  });

  it("keeps the keys of the channels too, and the membership that names them", () => {
    const before = [profileWith("Oper", ["Regie"])];
    const after = [profileWith("Oper", ["Regie"])];

    const [adopted] = reuseKeys(after, before);

    expect(adopted.channels.map((c) => c.key)).toEqual(before[0].channels.map((c) => c.key));
    // The group still names a channel that is actually in the list. Without
    // the remap it would be holding the key the fresh read handed out, which
    // nothing in the adopted profile has any more — a group that silently
    // watches nothing.
    expect(adopted.groups[0].members).toEqual([adopted.channels[0].key]);
  });

  it("keeps the keys of the profiles and groups in the same positions", () => {
    const before = [profileWith("Oper", ["Regie", "Ton"])];
    const after = [profileWith("Oper — umbenannt", ["Regie", "Ton"])];

    const [adopted] = reuseKeys(after, before);

    expect(adopted.key).toBe(before[0].key);
    expect(adopted.groups.map((g) => g.key)).toEqual(before[0].groups.map((g) => g.key));
    // The values are the new ones; only the identities are carried over.
    expect(adopted.name).toBe("Oper — umbenannt");
  });

  it("leaves a profile with no counterpart with the keys it was read with", () => {
    const before = [profileWith("Oper", [])];
    const after = [profileWith("Oper", []), profileWith("Probebühne", [])];

    const adopted = reuseKeys(after, before);

    expect(adopted[0].key).toBe(before[0].key);
    expect(adopted[1].key).toBe(after[1].key);
  });

  it("carries over as many groups as there are positions to match", () => {
    const before = [profileWith("Oper", ["Regie"])];
    const after = [profileWith("Oper", ["Regie", "Ton"])];

    const [adopted] = reuseKeys(after, before);

    expect(adopted.groups[0].key).toBe(before[0].groups[0].key);
    expect(adopted.groups[1].key).toBe(after[0].groups[1].key);
  });
});

// Recognising the echo of one's own save (P16). Every save is announced to
// every view including the one that caused it, and the announcement has been
// through the backend, which writes the same configuration in its own field
// order. A comparison that called that a foreign change would be no echo
// suppression at all — and the visible cost is the Settings page rebuilding
// the field being typed in, on every keystroke.
describe("canonical", () => {
  const config = toStoredConfig([], -1, "Standby, Cue, Go.");

  it("does not depend on the order the fields were written in", () => {
    const shuffled = Object.fromEntries(
      Object.entries(config).reverse(),
    ) as unknown as typeof config;

    expect(canonical(shuffled)).toBe(canonical(config));
  });

  it("still tells two different configurations apart", () => {
    expect(canonical({ ...config, vocabulary: "something else" })).not.toBe(
      canonical(config),
    );
  });

  // Nested, because the profiles are where the two sides are most likely to
  // disagree about order and where every real edit happens.
  it("reaches into the profiles", () => {
    const one = toStoredConfig([newProfile([], { name: "Scarlett", id: "uid" })], 0, "");
    const other = structuredClone(one);
    other.profiles[0] = Object.fromEntries(
      Object.entries(other.profiles[0]).reverse(),
    ) as unknown as (typeof other.profiles)[0];

    expect(canonical(other)).toBe(canonical(one));
  });
});

// The invariant the whole last-write-wins arrangement rests on (P16): reading
// a configuration and writing it straight back has to produce the same
// configuration. A view that adopts another peer's save and then writes back
// something *different* is a view that has started an argument nothing settles
// — the two bounce one configuration off each other several times a second,
// and every field on the Settings page is rebuilt under the cursor on each
// pass. A new field that `fromStoredConfig` drops, or that `toStoredConfig`
// spells differently, is exactly that bug, and this is what catches it.
describe("adopting a configuration", () => {
  const rich = (): StoredConfig => {
    const profile = newProfile([], { name: "Scarlett 18i20", id: "uid-1" });
    profile.name = "Oper grosses Haus";
    profile.channels = [
      { ...profile.channels[0], label: "Regie", channel: 0 },
      { ...profile.channels[0], key: 9_001, label: "Ton", channel: 3 },
    ];
    profile.groups = [
      {
        key: 9_100,
        name: "Regie",
        members: [profile.channels[0].key],
        onTop: true,
      },
    ];
    profile.rules = [
      {
        key: 9_200,
        name: "Feuer",
        enabled: true,
        words: ["Feuer", "Arzt"],
        channels: [],
        views: [profile.groups[0].key],
        actions: ["highlight", "urgentAttention"],
        webhook: null,
        minIntervalMs: 0,
      },
    ];

    return toStoredConfig(
      [profile],
      profile.key,
      "Standby, Cue, Go.",
      { model: "small-q5_1", vadModel: "silero-v5.1.2", modelPath: null, workers: 4 },
      { enabled: true, address: "192.168.178.27", port: 8787, key: "k7p4m2rq" },
      // Every one of the trailing fields set to something other than its
      // default, because that is the only way this test can see them: a field
      // `fromStoredConfig` drops but both sides default the same way round
      // trips perfectly and is still the bug described above.
      "newest-top",
      {
        silenceTimeoutMs: 350,
        maxSegmentMs: 4_000,
        speechThresholdPercent: 60,
        carryContext: false,
        livePreview: false,
        previewStepMs: 1_500,
      },
      true,
      true,
    );
  };

  it("and writing it straight back produces the same configuration", () => {
    const stored = rich();
    const read = fromStoredConfig(stored);
    const written = toStoredConfig(
      read.profiles,
      read.activeKey!,
      read.vocabulary ?? "",
      read.models,
      read.remote,
      read.chatSort,
      read.tuning,
      read.autoStart,
      read.setupDone,
    );

    expect(canonical(written)).toBe(canonical(stored));
  });

  // The port comes from a number box, and a number box that has been cleared
  // binds to `null`. The blur clamp normally catches it, but the save that
  // runs when the window is closed with the cursor still in the field does
  // not go through a blur — and `null` is what the backend refuses outright,
  // in exactly the save that exists as a last resort.
  it("writes a port the backend can read even when the field was cleared", () => {
    const profile = newProfile([], { name: "Probe", id: "probe" });
    const written = toStoredConfig([profile], profile.key, "", {}, {
      enabled: true,
      address: "192.168.178.27",
      port: null as unknown as number,
      key: "",
    });

    expect(written.remote?.port).toBe(8787);
  });

  it("holds a port to what can actually be bound", () => {
    const profile = newProfile([], { name: "Probe", id: "probe" });
    const low = toStoredConfig([profile], profile.key, "", {}, {
      enabled: true,
      address: "192.168.178.27",
      port: 80,
      key: "",
    });

    expect(low.remote?.port).toBe(1024);
  });

  // The worker count comes from a number box too, and the same last-ditch save
  // can catch it mid-edit. Zero is the one that matters: the pool refuses to
  // spawn with nothing in it, so a session that would have started does not.
  it("never writes a worker count the pool cannot spawn with", () => {
    const profile = newProfile([], { name: "Probe", id: "probe" });
    const write = (workers: number | null) =>
      toStoredConfig([profile], profile.key, "", { workers }).workers;

    expect(write(0)).toBe(1);
    expect(write(99)).toBe(8);
    expect(write(2.7)).toBe(2);
    // Cleared is "never chose" — the default — and not a number at all.
    expect(write(null)).toBeNull();
  });

  // Twice, because a translation can be stable on the second pass and not on
  // the first — which is the shape that makes a loop settle in testing and run
  // forever in front of an operator.
  it("is stable however many times it happens", () => {
    let stored = rich();
    for (let pass = 0; pass < 3; pass++) {
      const read = fromStoredConfig(stored);
      const next = toStoredConfig(
        read.profiles,
        read.activeKey!,
        read.vocabulary ?? "",
        read.models,
        read.remote,
        read.chatSort,
        read.tuning,
        read.autoStart,
        read.setupDone,
      );
      expect(canonical(next)).toBe(canonical(stored));
      stored = next;
    }
  });
});

describe("a channel read from another instance (P29)", () => {
  it("keeps its id and its source through the file", () => {
    const profile = aProfile();
    profile.channels = [
      ...profile.channels,
      remoteChannel(profile.channels, { server: "abc", channel: "x1" }, "Their Regie", "#123456"),
    ];
    const back = fromStored(toStored(profile));
    expect(back.channels.map((c) => c.id)).toEqual(profile.channels.map((c) => c.id));
    expect(back.channels[2].source).toEqual({ server: "abc", channel: "x1" });
    expect(back.channels[0].source).toBeNull();
  });

  it("is carried into a session without an input, and keeps every stream id after it", () => {
    const profile = aProfile();
    const remote = remoteChannel(profile.channels, { server: "abc", channel: "x1" }, "Theirs", "#123");
    const config = toConfig([profile.channels[0], remote, profile.channels[1]]);
    expect(config.map((s) => s.remote)).toEqual([false, true, false]);
    expect(config[1].channel).toBe(NO_INPUT);
    expect(config[2].channel).toBe(3);
    expect(config.map((s) => s.id)).toEqual([profile.channels[0].id, remote.id, "licht"]);
  });

  it("needs no device: only this machine's inputs are what a Start is for", () => {
    const remote = remoteChannel([], { server: "abc", channel: "x1" }, "Theirs", "#123");
    expect(validate([remote], "Desk", 8)).toBe("Add at least one channel.");
    expect(absentChannels([remote], 0)).toEqual([]);
  });
});

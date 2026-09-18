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
  connectedDevice,
  duplicate,
  fromStored,
  fromStoredConfig,
  newProfile,
  toStored,
  toStoredConfig,
  type Profile,
} from "./profiles";
import { restoreChannel, DEFAULT_LANGUAGE } from "./streams";
import { restoreGroup } from "./groups";
import type { DeviceOption, StoredProfile } from "./bridge";

/// Two channels and a group holding the second of them, built the way the
/// Settings page builds one — with keys, not positions.
const aProfile = (): Profile => {
  const channels = [
    restoreChannel({ channel: 0, label: "Regie", language: "de", color: "#6ea8fe" }),
    restoreChannel({ channel: 3, label: "Licht", language: "en", color: "#5fd38d" }),
  ];

  return {
    key: 99,
    name: "Grosses Haus",
    device: "MADIface",
    deviceId: "AppleUSBAudioEngine:RME:MADIface:2",
    language: "de",
    channels,
    groups: [restoreGroup("Bühne", [channels[1].key], false)],
  };
};

const stored: StoredProfile = {
  name: "Grosses Haus",
  device: "MADIface",
  deviceId: "AppleUSBAudioEngine:RME:MADIface:2",
  language: "de",
  channels: [
    { channel: 0, label: "Regie", language: "de", color: "#6ea8fe" },
    { channel: 3, label: "Licht", language: "en", color: "#5fd38d" },
  ],
  groups: [{ name: "Bühne", flash: false, onTop: false, members: [1] }],
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
    const broken = { ...stored, groups: [{ name: "Bühne", members: [1, 9, -1] }] };
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

  it("comes back on the profile that was in use", () => {
    const restored = fromStoredConfig({
      version: 1,
      active: 1,
      vocabulary: "Standby.",
      profiles: [stored, { ...stored, name: "Probebühne" }],
    });

    expect(restored.profiles[1].key).toBe(restored.activeKey);
    expect(restored.profiles[1].name).toBe("Probebühne");
  });

  it("tells a vocabulary that was never set apart from one that was cleared", () => {
    // `null` keeps the default; an empty string is an operator who turned the
    // prompt off, and refilling it would turn it back on behind their back.
    const never = fromStoredConfig({ version: 1, active: 0, vocabulary: null, profiles: [] });
    const cleared = fromStoredConfig({ version: 1, active: 0, vocabulary: "", profiles: [] });

    expect(never.vocabulary).toBeNull();
    expect(cleared.vocabulary).toBe("");
  });

  it("keeps a file written before the model manager readable", () => {
    // P11 added three fields to the file. An operator upgrading has one
    // without them, and losing their profiles to a schema addition would be
    // the worst possible trade for a model picker.
    const restored = fromStoredConfig({
      version: 1,
      active: 0,
      vocabulary: "Standby.",
      profiles: [stored],
    });

    expect(restored.profiles).toHaveLength(1);
    // "Never chose", which resolves to the recommended model — not
    // `undefined`, which would travel straight back into the next save.
    expect(restored.models).toEqual({ model: null, vadModel: null, modelPath: null });
  });

  it("keeps a file written before profiles had a default language readable", () => {
    // Same trade as above, and with a sharper failure: an empty string here
    // would travel into the picker and out again into every channel added
    // afterwards, which whisper cannot be given.
    const { language: _, ...withoutLanguage } = stored;

    const restored = fromStoredConfig({
      version: 1,
      active: 0,
      vocabulary: "Standby.",
      profiles: [withoutLanguage],
    });

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
    const empty = fromStoredConfig({ version: 1, active: 0, vocabulary: null, profiles: [] });

    expect(empty.profiles).toEqual([]);
    expect(empty.activeKey).toBeNull();
  });
});

describe("alert terms and flashing", () => {
  it("carries a group's flash setting across a save and a load", () => {
    // A group set to flash that comes back quiet is a view an operator
    // believes is watching them and is not.
    const profile = aProfile();
    profile.groups[0].flash = true;

    expect(fromStored(toStored(profile)).groups[0].flash).toBe(true);
  });

  it("keeps a file written before flashing existed quiet", () => {
    expect(fromStored(stored).groups[0].flash).toBe(false);
  });

  // The window setting travels the same way and for the same reason: a group
  // whose window is meant to stay in front and comes back behind everything is
  // a view an operator believes they can see and cannot.
  it("carries a group's window setting across a save and a load", () => {
    const profile = aProfile();
    profile.groups[0].onTop = true;

    expect(fromStored(toStored(profile)).groups[0].onTop).toBe(true);
  });

  it("opens the windows of a file written before that setting like any other", () => {
    expect(fromStored({ ...stored, groups: [{ name: "Bühne", members: [1] }] }).groups[0].onTop).toBe(
      false,
    );
  });

  it("stores the alert list without the editor's empty row", () => {
    const config = toStoredConfig([aProfile()], 99, "Standby", {}, ["Feuer", "  ", "Feuer"]);

    expect(config.alerts).toEqual(["Feuer"]);
  });

  it("reads a file from before the alert list as an empty one", () => {
    const { alerts } = fromStoredConfig({
      version: 1,
      active: 0,
      vocabulary: null,
      profiles: [stored],
    });

    expect(alerts).toEqual([]);
  });
});

// What a stored profile has to survive (P12).
//
// Almost all of this is about one translation: group membership is a set of
// channel keys while the setup is edited and a set of positions in the file.
// Getting it wrong is not a crash — it is a group that silently watches the
// wrong channel after a restart, which is exactly the kind of failure nobody
// notices until the show.

import { describe, expect, it } from "vitest";
import {
  duplicate,
  fromStored,
  fromStoredConfig,
  newProfile,
  toStored,
  toStoredConfig,
  type Profile,
} from "./profiles";
import { restoreChannel } from "./streams";
import { restoreGroup } from "./groups";
import type { StoredProfile } from "./bridge";

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
    channels,
    groups: [restoreGroup("Bühne", [channels[1].key])],
  };
};

const stored: StoredProfile = {
  name: "Grosses Haus",
  device: "MADIface",
  channels: [
    { channel: 0, label: "Regie", language: "de", color: "#6ea8fe" },
    { channel: 3, label: "Licht", language: "en", color: "#5fd38d" },
  ],
  groups: [{ name: "Bühne", members: [1] }],
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
    const created = newProfile([], "MADIface");

    expect(created.device).toBe("MADIface");
    expect(created.channels).toHaveLength(1);
    expect(created.groups).toEqual([]);
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

  it("has no active profile when there are none at all", () => {
    // The first start, before the UI creates one.
    const empty = fromStoredConfig({ version: 1, active: 0, vocabulary: null, profiles: [] });

    expect(empty.profiles).toEqual([]);
    expect(empty.activeKey).toBeNull();
  });
});

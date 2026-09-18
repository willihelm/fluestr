import { describe, expect, it } from "vitest";
import { forgetChannels, isMember, toggleMember, visibleStreams, type Group } from "./groups";
import type { Channel } from "./streams";

const channel = (key: number, label = `Channel ${key}`): Channel => ({
  key,
  channel: key,
  label,
  language: "de",
  color: "#6ea8fe",
});

const group = (key: number, members: number[]): Group => ({
  key,
  name: `Group ${key}`,
  members,
  flash: false,
  onTop: false,
});

describe("group membership", () => {
  it("lets one channel be in two groups at once", () => {
    // Non-exclusive on purpose: the same talkback belongs in the stage
    // manager's view and in the lighting one, and forcing a choice would make
    // one of the two wrong.
    const stage = group(0, [7]);
    const light = group(1, [7]);

    expect(isMember(stage, channel(7))).toBe(true);
    expect(isMember(light, channel(7))).toBe(true);
  });

  it("toggles in both directions and lands where it was", () => {
    const empty = group(0, []);
    const once = toggleMember(empty, channel(3));
    const twice = toggleMember(once, channel(3));

    expect(once.members).toEqual([3]);
    expect(twice.members).toEqual([]);
  });

  it("drops a deleted channel out of every group", () => {
    // Otherwise the groups accumulate keys pointing at nothing — invisible
    // until a later channel is given a recycled key and turns up in a group
    // nobody put it in.
    const groups = [group(0, [1, 2]), group(1, [2])];
    const left = forgetChannels(groups, [channel(1)]);

    expect(left[0].members).toEqual([1]);
    expect(left[1].members).toEqual([]);
  });
});

describe("visibleStreams", () => {
  const channels = [channel(10), channel(11), channel(12)];

  it("translates channel keys to the stream ids the events carry", () => {
    // The stream id is the position in the channel list, which is exactly how
    // the backend assigns it.
    expect(visibleStreams(group(0, [12, 10]), channels)).toEqual(new Set([0, 2]));
  });

  it("follows the channel a group means when the list is reordered", () => {
    // The point of keying membership on `key` rather than on position: delete
    // the first channel and the group must still show the same *channel*, at
    // its new stream id, instead of whatever moved into the old slot.
    const remaining = [channel(11), channel(12)];
    expect(visibleStreams(group(0, [12]), remaining)).toEqual(new Set([1]));
  });

  it("tells an empty group apart from no group at all", () => {
    // `null` is every stream; an empty set is a group with nothing in it, and
    // saying so beats silently showing everything.
    expect(visibleStreams(null, channels)).toBeNull();
    expect(visibleStreams(group(0, []), channels)).toEqual(new Set());
  });
});

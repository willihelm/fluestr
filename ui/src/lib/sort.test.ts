import { describe, expect, it } from "vitest";
import {
  arrange,
  DEFAULT_DIRECTION,
  flip,
  fromNewest,
  fromOldest,
  label,
  newestEdge,
  overrideKey,
  readOverride,
  type Extent,
} from "./sort";
import { ALL_CHANNELS_VIEW } from "./windows";

describe("readOverride", () => {
  // No entry is not a direction: it is the view following the app's default,
  // which is a different thing to be — the default can change under it.
  it("takes no entry for no override", () => {
    expect(readOverride(null)).toBeNull();
  });

  it("comes back the way the view was left", () => {
    expect(readOverride("newest-top")).toBe("newest-top");
    expect(readOverride("newest-bottom")).toBe("newest-bottom");
  });

  // A hand-edited entry, or one written by a version of this that stored
  // something else. The chat has to render either way, and following the
  // app's default is the answer that needs nothing else to be true.
  it("takes a corrupted entry for no override", () => {
    expect(readOverride("upwards")).toBeNull();
    expect(readOverride("")).toBeNull();
  });
});

describe("overrideKey", () => {
  // The main window and a group window on the same screen share one storage,
  // and being able to read them in opposite directions is the whole point of
  // an override.
  it("keeps every view's override apart", () => {
    const keys = [overrideKey(null), overrideKey(2), overrideKey(ALL_CHANNELS_VIEW)];
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("names a window the way the rest of the app spells it", () => {
    expect(overrideKey(2)).toContain("group-2");
    expect(overrideKey(ALL_CHANNELS_VIEW)).toContain("group-all");
  });
});

describe("flip", () => {
  it("has only the two directions", () => {
    expect(flip("newest-bottom")).toBe("newest-top");
    expect(flip("newest-top")).toBe("newest-bottom");
  });

  it("is its own inverse, which is what the toggle relies on", () => {
    expect(flip(flip("newest-top"))).toBe("newest-top");
  });
});

describe("label", () => {
  it("says where the newest line is", () => {
    expect(label("newest-top")).toBe("Newest at top");
    expect(label("newest-bottom")).toBe("Newest at bottom");
  });

  // Kept in step with `SortDirection::default` on the Rust side: a file with
  // no such field means what the chat did before there was one.
  it("defaults to the chat everyone has already used", () => {
    expect(DEFAULT_DIRECTION).toBe("newest-bottom");
  });
});

describe("arrange", () => {
  const page = [1, 2, 3];

  it("leaves a chat that reads downwards alone", () => {
    expect(arrange(page, "newest-bottom")).toEqual([1, 2, 3]);
  });

  it("shows the same bubbles in the inverse order", () => {
    expect(arrange(page, "newest-top")).toEqual([3, 2, 1]);
  });

  // The array handed in is derived state shared with the rest of the view —
  // reversing it in place would reorder the list every reader sees.
  it("never reverses in place", () => {
    const original = [...page];
    arrange(page, "newest-top");
    expect(page).toEqual(original);
  });
});

describe("the two edges", () => {
  // A viewport of 100 in a list of 500, scrolled 50 from the top: 350 from the
  // bottom.
  const extent: Extent = { scrollTop: 50, scrollHeight: 500, clientHeight: 100 };

  it("reads the newest edge from the end the newest line is at", () => {
    expect(fromNewest(extent, "newest-bottom")).toBe(350);
    expect(fromNewest(extent, "newest-top")).toBe(50);
  });

  it("reads the history from the other end", () => {
    expect(fromOldest(extent, "newest-bottom")).toBe(50);
    expect(fromOldest(extent, "newest-top")).toBe(350);
  });

  // A chat shorter than its viewport is at both edges at once. It must not
  // come out negative: a negative distance from the history edge would ask for
  // a page that is not there on every scroll event.
  it("never answers a negative distance", () => {
    const short: Extent = { scrollTop: 0, scrollHeight: 80, clientHeight: 300 };
    expect(fromNewest(short, "newest-bottom")).toBe(0);
    expect(fromOldest(short, "newest-top")).toBe(0);
  });

  it("puts the newest line on screen at the end it is at", () => {
    expect(newestEdge(extent, "newest-bottom")).toBe(500);
    expect(newestEdge(extent, "newest-top")).toBe(0);
  });
});

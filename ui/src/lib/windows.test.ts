import { describe, expect, it } from "vitest";
import {
  ALL_CHANNELS_VIEW,
  groupIndexOf,
  groupParamOf,
  shoutboxParamOf,
  tickerIndexOf,
  tickerParamOf,
} from "./windows";

describe("groupIndexOf", () => {
  it("reads the group out of a group window's label", () => {
    expect(groupIndexOf("group-0")).toBe(0);
    expect(groupIndexOf("group-12")).toBe(12);
  });

  it("reads the all-channels window out of its label", () => {
    // Not a group and so not a number, but a view a window can be opened on.
    expect(groupIndexOf("group-all")).toBe(ALL_CHANNELS_VIEW);
  });

  it("does not take the main window for a group", () => {
    expect(groupIndexOf("main")).toBeNull();
  });

  // A near miss has to be a miss: a view of group `NaN` renders as an empty
  // chat, which is indistinguishable from a group somebody emptied.
  it("refuses a label that is not the prefix and a number", () => {
    expect(groupIndexOf("group-")).toBeNull();
    expect(groupIndexOf("group-two")).toBeNull();
    // Near the one word that means something, which is the reason to be exact
    // about it rather than to look for a prefix.
    expect(groupIndexOf("group-alle")).toBeNull();
    expect(groupIndexOf("group-ALL")).toBeNull();
    expect(groupIndexOf("group-1.5")).toBeNull();
    expect(groupIndexOf("group--1")).toBeNull();
    expect(groupIndexOf("grouping-1")).toBeNull();
  });
});

// The browser's half of the same question (P16). A remote group view is a tab
// with `?group=2` rather than a window with a label, and it has to be exactly
// as strict: the two are the same view, and a query string is the half a
// person can type by hand.
describe("groupParamOf", () => {
  it("reads the group out of a remote group tab's URL", () => {
    expect(groupParamOf("?group=0")).toBe(0);
    expect(groupParamOf("?group=12")).toBe(12);
    expect(groupParamOf("?foo=1&group=3")).toBe(3);
    expect(groupParamOf("?group=all")).toBe(ALL_CHANNELS_VIEW);
  });

  it("takes a URL without one for the main view", () => {
    expect(groupParamOf("")).toBeNull();
    expect(groupParamOf("?foo=1")).toBeNull();
  });

  it("refuses anything that is not a non-negative integer", () => {
    expect(groupParamOf("?group=")).toBeNull();
    expect(groupParamOf("?group=two")).toBeNull();
    expect(groupParamOf("?group=1.5")).toBeNull();
    expect(groupParamOf("?group=-1")).toBeNull();
    expect(groupParamOf("?group=alle")).toBeNull();
  });
});

// A ticker (P24) names its view the way a group window does, under a prefix
// of its own — and the two must never be read as each other, or a ticker
// would come up as a full group window and the other way round.
describe("tickerIndexOf", () => {
  it("reads the view out of a ticker's label", () => {
    expect(tickerIndexOf("ticker-0")).toBe(0);
    expect(tickerIndexOf("ticker-7")).toBe(7);
    expect(tickerIndexOf("ticker-all")).toBe(ALL_CHANNELS_VIEW);
  });

  it("does not take another window for a ticker", () => {
    expect(tickerIndexOf("main")).toBeNull();
    expect(tickerIndexOf("group-2")).toBeNull();
    expect(groupIndexOf("ticker-2")).toBeNull();
  });

  it("is as strict as a group window's label", () => {
    expect(tickerIndexOf("ticker-")).toBeNull();
    expect(tickerIndexOf("ticker-two")).toBeNull();
    expect(tickerIndexOf("ticker-alle")).toBeNull();
    expect(tickerIndexOf("ticker--1")).toBeNull();
  });
});

// The browser's half of the two pop-ups (P16): a remote ticker is
// `?ticker=2`, the shoutbox `?shoutbox`.
describe("tickerParamOf", () => {
  it("reads the view out of a remote ticker's URL", () => {
    expect(tickerParamOf("?ticker=0")).toBe(0);
    expect(tickerParamOf("?ticker=all")).toBe(ALL_CHANNELS_VIEW);
  });

  it("takes a group tab or the main view for no ticker", () => {
    expect(tickerParamOf("?group=2")).toBeNull();
    expect(tickerParamOf("")).toBeNull();
    expect(tickerParamOf("?ticker=")).toBeNull();
    expect(tickerParamOf("?ticker=-1")).toBeNull();
  });
});

describe("shoutboxParamOf", () => {
  it("knows the shoutbox pop-up by its parameter", () => {
    expect(shoutboxParamOf("?shoutbox")).toBe(true);
    expect(shoutboxParamOf("?shoutbox=")).toBe(true);
    expect(shoutboxParamOf("")).toBe(false);
    expect(shoutboxParamOf("?group=shoutbox")).toBe(false);
  });
});

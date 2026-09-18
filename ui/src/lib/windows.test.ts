import { describe, expect, it } from "vitest";
import { groupIndexOf } from "./windows";

describe("groupIndexOf", () => {
  it("reads the group out of a group window's label", () => {
    expect(groupIndexOf("group-0")).toBe(0);
    expect(groupIndexOf("group-12")).toBe(12);
  });

  it("does not take the main window for a group", () => {
    expect(groupIndexOf("main")).toBeNull();
  });

  // A near miss has to be a miss: a view of group `NaN` renders as an empty
  // chat, which is indistinguishable from a group somebody emptied.
  it("refuses a label that is not the prefix and a number", () => {
    expect(groupIndexOf("group-")).toBeNull();
    expect(groupIndexOf("group-two")).toBeNull();
    expect(groupIndexOf("group-1.5")).toBeNull();
    expect(groupIndexOf("group--1")).toBeNull();
    expect(groupIndexOf("grouping-1")).toBeNull();
  });
});

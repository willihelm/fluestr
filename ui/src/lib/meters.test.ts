import { describe, expect, it } from "vitest";
import { collapseKey, readCollapsed } from "./meters";
import { ALL_CHANNELS_VIEW } from "./windows";

describe("readCollapsed", () => {
  // A window nobody has collapsed shows its meters. So does one whose entry
  // is unreadable: the shown state is the one an operator can see the way out
  // of, and a strip that is missing for no visible reason reads as a bug.
  it("shows the strip for no entry and for a corrupted one", () => {
    expect(readCollapsed(null)).toBe(false);
    expect(readCollapsed("")).toBe(false);
    expect(readCollapsed("true")).toBe(false);
  });

  it("hides it only for the entry it writes", () => {
    expect(readCollapsed("collapsed")).toBe(true);
  });
});

describe("collapseKey", () => {
  // Per view, which is the point: the desk keeps its bars while the group
  // window squeezed into the corner of the screen gives them up.
  it("gives every view its own entry", () => {
    expect(collapseKey(null)).toBe("intercom2chat.meters.main");
    expect(collapseKey(2)).toBe("intercom2chat.meters.group-2");
    expect(collapseKey(ALL_CHANNELS_VIEW)).toBe("intercom2chat.meters.group-all");
  });
});

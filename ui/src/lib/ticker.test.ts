import { describe, expect, it } from "vitest";
import {
  geometryKey,
  MIN_HEIGHT,
  MIN_WIDTH,
  popupFeatures,
  POPUP_HEIGHT,
  POPUP_WIDTH,
  readGeometry,
  resized,
} from "./ticker";
import { ALL_CHANNELS_VIEW } from "./windows";

describe("readGeometry", () => {
  it("reads back what was stored", () => {
    const stored = JSON.stringify({ x: -40, y: 1600, width: 1280, height: 150 });
    expect(readGeometry(stored)).toEqual({ x: -40, y: 1600, width: 1280, height: 150 });
  });

  it("takes nothing stored for the default place", () => {
    expect(readGeometry(null)).toBeNull();
  });

  // A ticker with no title bar that opens somewhere odd cannot be dragged
  // back by it, so anything doubtful is the default place instead.
  it("refuses what is not a whole geometry", () => {
    expect(readGeometry("not json")).toBeNull();
    expect(readGeometry("null")).toBeNull();
    expect(readGeometry("42")).toBeNull();
    expect(readGeometry(JSON.stringify({ x: 0, y: 0, width: 100 }))).toBeNull();
    expect(readGeometry(JSON.stringify({ x: 0.5, y: 0, width: 100, height: 50 }))).toBeNull();
    expect(readGeometry(JSON.stringify({ x: "0", y: 0, width: 100, height: 50 }))).toBeNull();
    expect(readGeometry(JSON.stringify({ x: 0, y: 0, width: 0, height: 50 }))).toBeNull();
    expect(readGeometry(JSON.stringify({ x: 0, y: 0, width: 100, height: -1 }))).toBeNull();
  });
});

describe("geometryKey", () => {
  // Per view: the stage manager's ticker and the all-channels one are two
  // strips in two places.
  it("keeps one place per view", () => {
    expect(geometryKey(2)).not.toBe(geometryKey(3));
    expect(geometryKey(ALL_CHANNELS_VIEW)).not.toBe(geometryKey(0));
  });
});

describe("resized", () => {
  const start = { x: 100, y: 1600, width: 1280, height: 150 };

  it("moves only the edge that is dragged", () => {
    expect(resized(start, "right", 40, 999, 2)).toEqual({ ...start, width: 1320 });
    expect(resized(start, "bottom", 999, 30, 2)).toEqual({ ...start, height: 180 });
    expect(resized(start, "bottom-right", 40, 30, 2)).toEqual({ ...start, width: 1320, height: 180 });
  });

  // The top edge grows the window upwards: its bottom stays where it is.
  it("keeps the bottom in place when the top is dragged", () => {
    const up = resized(start, "top", 0, -50, 2);
    expect(up).toEqual({ ...start, y: 1550, height: 200 });
    expect(up.y + up.height).toBe(start.y + start.height);

    const corner = resized(start, "top-right", 20, 50, 2);
    expect(corner).toEqual({ x: 100, y: 1650, width: 1300, height: 100 });
  });

  // At the minimum, a top edge dragged further down stops the window rather
  // than pushing it down the screen.
  it("stops at the minimum size, in physical pixels", () => {
    expect(resized(start, "right", -5000, 0, 2).width).toBe(MIN_WIDTH * 2);
    expect(resized(start, "bottom", 0, -5000, 1).height).toBe(MIN_HEIGHT);

    const squashed = resized(start, "top", 0, 5000, 2);
    expect(squashed.height).toBe(MIN_HEIGHT * 2);
    expect(squashed.y + squashed.height).toBe(start.y + start.height);
  });
});

// A remote ticker is a pop-up (P16), opened where the last one was left.
describe("popupFeatures", () => {
  it("opens at the app's default strip the first time", () => {
    expect(popupFeatures(null)).toBe(`popup,width=${POPUP_WIDTH},height=${POPUP_HEIGHT}`);
  });

  it("opens where the last one was left", () => {
    expect(popupFeatures({ x: 20, y: -900, width: 800, height: 90 })).toBe(
      "popup,width=800,height=90,left=20,top=-900",
    );
  });

  it("never opens smaller than a ticker can be", () => {
    expect(popupFeatures({ x: 0, y: 0, width: 10, height: 10 })).toBe(
      `popup,width=${MIN_WIDTH},height=${MIN_HEIGHT},left=0,top=0`,
    );
  });
});

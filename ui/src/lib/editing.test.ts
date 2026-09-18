// Which fields a save has to wait for.
//
// Getting this wrong is invisible in one direction and maddening in the other:
// a typed field left out means the label is rebuilt under the cursor on every
// save, and a discrete control wrongly included means a colour or a language
// is not stored until something else happens to take the focus.

import { describe, expect, it } from "vitest";
import { isEditable } from "./editing";

describe("isEditable", () => {
  it("counts the fields a label is typed into", () => {
    expect(isEditable({ tagName: "INPUT", type: "text" })).toBe(true);
    expect(isEditable({ tagName: "TEXTAREA" })).toBe(true);
    // The port on the remote settings, and the device channel number.
    expect(isEditable({ tagName: "INPUT", type: "number" })).toBe(true);
  });

  // A bare `<input>` is a text field; the attribute is what narrows it, and an
  // element read off the page may not carry one.
  it("takes an input with no type for a text field", () => {
    expect(isEditable({ tagName: "INPUT" })).toBe(true);
    expect(isEditable({ tagName: "input", type: "TEXT" })).toBe(true);
  });

  it("does not count the controls that are set in one act", () => {
    // The channel colour. Waiting for a focus change here would mean the
    // colour is not stored until the operator happens to click elsewhere.
    expect(isEditable({ tagName: "INPUT", type: "color" })).toBe(false);
    expect(isEditable({ tagName: "INPUT", type: "checkbox" })).toBe(false);
    expect(isEditable({ tagName: "SELECT" })).toBe(false);
    expect(isEditable({ tagName: "BUTTON" })).toBe(false);
  });

  it("is false for nothing focused at all", () => {
    expect(isEditable(null)).toBe(false);
    expect(isEditable(undefined)).toBe(false);
  });

  it("counts anything the page made editable in place", () => {
    expect(isEditable({ tagName: "DIV", isContentEditable: true })).toBe(true);
  });
});

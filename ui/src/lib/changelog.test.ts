import { describe, expect, it } from "vitest";
import { parseNotes } from "./changelog";

describe("parseNotes", () => {
  it("reads the notes the release script writes", () => {
    const notes = [
      "### New",
      "",
      "- Check for updates on start",
      "- Make settings editable while a session runs",
      "",
      "### Fixed",
      "",
      "- Compare the remote key without regard to case",
    ].join("\n");
    expect(parseNotes(notes)).toEqual([
      { kind: "heading", text: "New" },
      { kind: "item", text: "Check for updates on start" },
      { kind: "item", text: "Make settings editable while a session runs" },
      { kind: "heading", text: "Fixed" },
      { kind: "item", text: "Compare the remote key without regard to case" },
    ]);
  });

  // What GitHub generates when nobody wrote anything: a link the dialog has
  // no way to open, and so nothing to show.
  it("drops a bare changelog link", () => {
    const notes =
      "**Full Changelog**: https://github.com/willihelm/fluestr/compare/v0.1.0...v0.2.0";
    expect(parseNotes(notes)).toEqual([]);
    expect(parseNotes("https://example.com/notes")).toEqual([]);
  });

  it("takes the Markdown off what it keeps", () => {
    expect(parseNotes("* **Faster** start with [Parakeet](https://x.y) and `cargo`")).toEqual([
      { kind: "item", text: "Faster start with Parakeet and cargo" },
    ]);
  });

  it("keeps hand-written prose as text", () => {
    expect(parseNotes("First public release.\r\n\r\n## Notes\r\n- One")).toEqual([
      { kind: "text", text: "First public release." },
      { kind: "heading", text: "Notes" },
      { kind: "item", text: "One" },
    ]);
  });

  it("has nothing to say about no notes", () => {
    expect(parseNotes(null)).toEqual([]);
    expect(parseNotes("  \n\n")).toEqual([]);
  });
});

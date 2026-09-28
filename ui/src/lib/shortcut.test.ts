import { describe, expect, it } from "vitest";
import { isMac, shortcut } from "./shortcut";

describe("shortcut", () => {
  it("writes the Mac modifier as a symbol", () => {
    expect(shortcut("T", true)).toBe("⌘T");
    expect(shortcut(",", true)).toBe("⌘,");
  });

  it("writes Ctrl out elsewhere", () => {
    expect(shortcut("T", false)).toBe("Ctrl+T");
  });

  it("tells the platforms apart by user agent", () => {
    expect(isMac("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")).toBe(true);
    expect(isMac("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe(false);
  });
});

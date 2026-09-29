import { describe, expect, it } from "vitest";
import {
  CARRY_RESERVE,
  channelPromptWarning,
  estimatedTokens,
  lengthWarning,
  toConfig,
  TOKEN_LIMIT,
} from "./vocabulary";

describe("vocabulary", () => {
  it("turns an empty box into no prompt at all", () => {
    // Not an empty prompt: a cleared box has to really turn the vocabulary
    // off, which is how its effect on word error rate gets measured (P14).
    expect(toConfig("   ")).toBeNull();
    expect(toConfig(" Standby, Cue ")).toBe("Standby, Cue");
  });

  it("counts nothing for an empty box", () => {
    expect(estimatedTokens("")).toBe(0);
    expect(estimatedTokens("  \n ")).toBe(0);
  });

  it("says nothing about a prompt of a sane length", () => {
    expect(lengthWarning("Standby, Cue, Go, Fade out")).toBeNull();
  });

  it("warns before the limit, not at it", () => {
    // The estimate reads low on exactly the German compounds this list is
    // full of, so the warning has to come early enough to still be true.
    const nearly = "x".repeat(TOKEN_LIMIT * 4 * 0.9);
    expect(lengthWarning(nearly)).toContain("hallucination");
  });

  it("says which end of an over-long prompt is lost", () => {
    const tooLong = "x".repeat(TOKEN_LIMIT * 4 + 100);
    expect(lengthWarning(tooLong)).toContain("start");
  });

  it("says nothing about a channel without a description", () => {
    // The vocabulary box already speaks for the vocabulary alone.
    expect(channelPromptWarning("x".repeat(TOKEN_LIMIT * 4 * 2), "", true)).toBeNull();
  });

  it("counts a channel's description together with the vocabulary", () => {
    // Each fits on its own; the two together do not.
    const half = "x".repeat(TOKEN_LIMIT * 2);
    expect(lengthWarning(half)).toBeNull();
    expect(channelPromptWarning(half, half + "xxxxxxxx", false)).toContain("dropped");
  });

  it("keeps room for the previous sentence only while it is carried", () => {
    const vocabulary = "x".repeat(TOKEN_LIMIT * 4 * 0.5);
    const description = "x".repeat((TOKEN_LIMIT * 0.8 - TOKEN_LIMIT * 0.5 - CARRY_RESERVE / 2) * 4);
    expect(channelPromptWarning(vocabulary, description, false)).toBeNull();
    expect(channelPromptWarning(vocabulary, description, true)).toContain("previous sentence");
  });
});

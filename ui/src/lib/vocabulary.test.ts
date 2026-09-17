import { describe, expect, it } from "vitest";
import { estimatedTokens, lengthWarning, toConfig, TOKEN_LIMIT } from "./vocabulary";

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
});

import { describe, expect, it } from "vitest";
import { KEY_LENGTH, newKey } from "./key";

describe("the remote's key", () => {
  it("is eight characters an operator can read off a screen", () => {
    const key = newKey();
    expect(key).toHaveLength(KEY_LENGTH);
    // No `I`, `L`, `O` or `U`: the first three are what somebody reading a key
    // aloud gets wrong, and the fourth is so that no key spells anything.
    expect(key).toMatch(/^[0-9ABCDEFGHJKMNPQRSTVWXYZ]{8}$/);
  });

  it("is a different one every time", () => {
    const keys = new Set(Array.from({ length: 200 }, newKey));
    // Forty bits: two hundred draws colliding would mean the randomness is not
    // random, not that it was unlucky.
    expect(keys.size).toBe(200);
  });
});

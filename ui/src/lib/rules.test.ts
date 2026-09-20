import { describe, expect, it } from "vitest";
import {
  audienceOf,
  callsOut,
  compile,
  destinationOf,
  forget,
  newRule,
  newWebhook,
  parseWords,
  sameCompiled,
  triggerOf,
  type Rule,
} from "./rules";
import { restoreGroup, type Group } from "./groups";
import { restoreChannel, type Channel } from "./streams";

const channels: Channel[] = [
  restoreChannel({ channel: 0, label: "Regie", language: "de", color: "#6ea8fe", gainDb: 0 }),
  restoreChannel({ channel: 1, label: "Inspizienz", language: "de", color: "#5fd38d", gainDb: 0 }),
  restoreChannel({ channel: 2, label: "Licht", language: "de", color: "#f2b35e", gainDb: 0 }),
];

/// Two of them, because a view is addressed by its *position* and one group
/// cannot show that the second is not the first.
const groups: Group[] = [
  restoreGroup("Bühne", [channels[1].key, channels[2].key]),
  restoreGroup("Licht", [channels[2].key]),
];

const rule = (over: Partial<Rule> = {}): Rule => ({
  ...newRule([]),
  name: "Feuer",
  words: ["Feuer"],
  actions: ["highlight", "urgentAttention"],
  ...over,
});

describe("compile", () => {
  it("sends a rule with no channels as one that listens to every stream", () => {
    expect(compile([rule()], channels, groups)).toEqual([
      {
        name: "Feuer",
        words: ["Feuer"],
        streams: null,
        views: null,
        actions: ["highlight", "urgentAttention"],
        webhook: null,
        minIntervalMs: 0,
      },
    ]);
  });

  // The stream id is the channel's position in the profile, exactly as the
  // backend assigns it.
  it("resolves a channel to its stream id", () => {
    const compiled = compile([rule({ channels: [channels[2].key] })], channels, groups);
    expect(compiled[0].streams).toEqual([2]);
  });

  // The bug this separation exists for: a group says which window is told,
  // and it must not quietly widen or narrow *when* the rule fires.
  it("leaves the trigger alone when a rule names a view", () => {
    const compiled = compile(
      [rule({ actions: ["flash"], views: [groups[0].key] })],
      channels,
      groups,
    );
    expect(compiled[0].streams).toBeNull();
    expect(compiled[0].views).toEqual([0]);
  });

  it("resolves a view to the position its window is named by", () => {
    const compiled = compile(
      [rule({ actions: ["flash"], views: [groups[1].key, groups[0].key] })],
      channels,
      groups,
    );
    expect(compiled[0].views).toEqual([0, 1]);
  });

  // `null` is every window showing the channel, which is what the flash did
  // before it could be addressed at all.
  it("sends a flash nobody addressed as one for every window", () => {
    const compiled = compile([rule({ actions: ["flash"] })], channels, groups);
    expect(compiled[0].views).toBeNull();
  });

  // Not `null`, which would flood every window, and not an empty list, which
  // the wire cannot tell from "nobody".
  it("drops the flash from a rule addressed only to windows that are gone", () => {
    const compiled = compile(
      [rule({ actions: ["flash", "urgentAttention"], views: [9_999] })],
      channels,
      groups,
    );
    expect(compiled[0].actions).toEqual(["urgentAttention"]);
    expect(compiled[0].views).toBeNull();
  });

  it("leaves out a rule whose only action was a flash it cannot deliver", () => {
    expect(compile([rule({ actions: ["flash"], views: [9_999] })], channels, groups)).toEqual([]);
  });

  it("says nothing about views on a rule that does not flash", () => {
    const compiled = compile([rule({ views: [groups[0].key] })], channels, groups);
    expect(compiled[0].views).toBeNull();
  });

  it("leaves a rule that is switched off out altogether", () => {
    expect(compile([rule({ enabled: false })], channels, groups)).toEqual([]);
  });

  it("leaves out a rule with nothing to do", () => {
    expect(compile([rule({ actions: [] })], channels, groups)).toEqual([]);
  });

  // The trap the backend must never be handed: an empty stream list would have
  // to mean either "nothing" or "everything", and the rule the operator wrote
  // was the narrow one.
  it("leaves out a rule whose every named place is gone rather than widening it", () => {
    expect(compile([rule({ channels: [9_999] })], channels, groups)).toEqual([]);
  });

  it("drops highlighting from a rule with no words, because there is nothing to mark", () => {
    const compiled = compile(
      [rule({ words: [], actions: ["highlight", "flash"], channels: [channels[0].key] })],
      channels,
      groups,
    );
    expect(compiled[0].actions).toEqual(["flash"]);
  });

  it("leaves out a rule whose only action was highlighting it cannot do", () => {
    expect(compile([rule({ words: [], actions: ["highlight"] })], channels, groups)).toEqual([]);
  });

  // The editor's half-typed row has no business in the matcher, and the two
  // spellings of an umlaut are one word to the backend.
  it("cleans the words the way the matcher would", () => {
    const compiled = compile([rule({ words: ["Feuer", "  ", "feuer"] })], channels, groups);
    expect(compiled[0].words).toEqual(["Feuer"]);
  });
});

describe("sameCompiled", () => {
  it("sees no change in a rule that was only renamed on screen", () => {
    const before = compile([rule()], channels, groups);
    const after = compile([rule()], channels, groups);
    expect(sameCompiled(before, after)).toBe(true);
  });

  it("sees the change when a rule is narrowed to a channel", () => {
    const before = compile([rule()], channels, groups);
    const after = compile([rule({ channels: [channels[0].key] })], channels, groups);
    expect(sameCompiled(before, after)).toBe(false);
  });
});

describe("forget", () => {
  it("drops a channel that is no longer in the setup", () => {
    const kept = forget(
      [rule({ channels: [channels[0].key, 9_999] })],
      channels,
      groups,
    );
    expect(kept[0].channels).toEqual([channels[0].key]);
    expect(kept[0].enabled).toBe(true);
  });

  // The failure a rule has that a group's membership does not: with the last
  // of its places gone, the trigger reads as "anywhere", and a rule that grew
  // louder because something was deleted is exactly the surprise a show cannot
  // absorb.
  it("switches off a rule whose every named place is gone rather than widening it", () => {
    const kept = forget([rule({ channels: [9_999] })], channels, groups);
    expect(kept[0].channels).toEqual([]);
    expect(kept[0].enabled).toBe(false);
  });

  // The same failure from the other side: a flash meant for one window must
  // not become one that floods them all because the group was deleted.
  it("switches off a rule whose every named window is gone", () => {
    const kept = forget([rule({ actions: ["flash"], views: [9_999] })], channels, groups);
    expect(kept[0].views).toEqual([]);
    expect(kept[0].enabled).toBe(false);
  });

  it("leaves a rule that never named a place alone", () => {
    expect(forget([rule()], [], []) [0].enabled).toBe(true);
  });
});

describe("parseWords", () => {
  it("reads what the operator typed, without the gaps", () => {
    expect(parseWords(" Feuer , Arzt ,, ")).toEqual(["Feuer", "Arzt"]);
  });
});

describe("triggerOf", () => {
  it("says the rule back in words", () => {
    expect(triggerOf(rule({ channels: [channels[1].key] }), channels)).toBe(
      "Feuer, on Inspizienz",
    );
  });

  it("says so when a rule has no word condition at all", () => {
    expect(triggerOf(rule({ words: [], channels: [channels[0].key] }), channels)).toBe(
      "Anything said, on Regie",
    );
  });

  it("says so when a rule is not narrowed to anywhere", () => {
    expect(triggerOf(rule(), channels)).toBe("Feuer, on any channel");
  });
});

describe("audienceOf", () => {
  // The question only arises for a flash, and a line about windows on a rule
  // that does not flash is a line about nothing.
  it("says nothing about windows when the rule does not flash", () => {
    expect(audienceOf(rule(), groups)).toBeNull();
  });

  it("names the windows a flash was addressed to", () => {
    expect(audienceOf(rule({ actions: ["flash"], views: [groups[0].key] }), groups)).toBe(
      "Flashes Bühne",
    );
  });

  it("says what an empty list means, rather than leaving it blank", () => {
    expect(audienceOf(rule({ actions: ["flash"] }), groups)).toBe(
      "Flashes every window showing that channel",
    );
  });
});

// --- The outbound action (P19) --------------------------------------------
//
// The one action that leaves the machine, so the one with a payload of its
// own. What is tested here is the two halves being married: the box ticked
// without a destination is a rule being written, and a destination without
// the box is a rule that was switched off.

const calling = (over: Partial<Rule> = {}): Rule =>
  rule({
    actions: ["webhook"],
    webhook: { ...newWebhook(), url: "http://companion:8000/api/location/1/0/2/press" },
    ...over,
  });

describe("the webhook action", () => {
  it("compiles a rule whose only action leaves the building", () => {
    const compiled = compile([calling()], channels, groups);
    expect(compiled).toHaveLength(1);
    expect(compiled[0].actions).toEqual(["webhook"]);
    expect(compiled[0].webhook?.url).toBe("http://companion:8000/api/location/1/0/2/press");
  });

  // Ticking the box is not yet a webhook. A rule with nothing else to do and
  // no destination typed is a rule being written, exactly like one with no
  // actions at all.
  it("drops the action from a rule with no destination typed", () => {
    expect(compile([calling({ webhook: { ...newWebhook(), url: "  " } })], channels, groups))
      .toEqual([]);
    expect(compile([calling({ webhook: null })], channels, groups)).toEqual([]);
  });

  // Un-ticking the box switches the destination off without deleting it: the
  // rule is being silenced, not rewritten. Nothing may be sent meanwhile.
  it("sends no request for a rule whose action is un-ticked", () => {
    const compiled = compile([calling({ actions: ["highlight"] })], channels, groups);
    expect(compiled[0].webhook).toBeNull();
    expect(compiled[0].actions).toEqual(["highlight"]);
  });

  // The rest of the rule still narrows it. A cue nobody wanted on every
  // channel is the flood the trigger exists to prevent.
  it("keeps the trigger a webhook rule was narrowed with", () => {
    const compiled = compile(
      [calling({ channels: [channels[1].key], minIntervalMs: 2_000 })],
      channels,
      groups,
    );
    expect(compiled[0].streams).toEqual([1]);
    expect(compiled[0].minIntervalMs).toBe(2_000);
  });

  it("reads a rule that calls out back as its host", () => {
    expect(callsOut(calling())).toBe(true);
    expect(destinationOf(calling())).toBe("Calls POST companion:8000");
  });

  // A URL too malformed to parse is shown as it was typed: that is the state
  // it needs to be seen in.
  it("shows an unparseable destination as it was typed", () => {
    expect(destinationOf(calling({ webhook: { ...newWebhook(), url: "companion:8000" } })))
      .toBe("Calls POST companion:8000");
  });

  it("says nothing about a rule that does not call out", () => {
    expect(callsOut(rule())).toBe(false);
    expect(destinationOf(rule())).toBeNull();
  });

  // The body starts with the transcript out of it. The default must not be the
  // one that quietly ships what was said in a production off the premises.
  it("starts a new request with an empty body", () => {
    expect(newWebhook().body).toBe("");
    expect(newWebhook().method).toBe("post");
  });
});

// Rules: a trigger, and what it does (P19).
//
// One list replacing the two hard-wired reactions the app had. An alert term
// highlighted its word *and* asked for attention because that is what an alert
// term was; a view's "flash on every message" flooded the window because that
// is what the checkbox was called. Neither could be narrowed to a channel and
// neither could be combined, so "Feuer, but only on the stage manager's
// channel" could not be written at all. Here the two halves are separate: a
// trigger says when, an action says what.
//
// **A channel is an input; a group is a window.** They were one list of
// "places" once, and that is precisely what did not work: narrowing a rule to
// a group narrowed *when* it fired and said nothing about *who was told*, so
// every window showing the channel still flooded. The two questions are asked
// separately now — `channels` is where the words have to be said, `views` is
// which windows the flash reaches — and the editor keeps them apart because
// the app does.
//
// **What this file owns is the translation.** A rule is stored and edited in
// the operator's terms — words, channels, groups — and evaluated on the
// backend in the pipeline's terms: stream ids and group positions. Keys exist
// nowhere but in this window, so `compile` turns channels into stream ids and
// views into the positions a group window is addressed by, and the backend is
// left with one question per rule: these words (or any text) on these streams
// (or any stream), and — if it flashes — for these windows (or all of them).
//
// Like group membership, a rule holds **channel and group keys** while it is
// being edited and *positions* once it is stored: a key is stable while a list
// is edited, a position is what still means something after a restart. See
// `groups.ts` and `profiles.ts`, which is where that translation happens for
// both.

import { cleanTerms } from "./alerts";
import type { Group } from "./groups";
import type { Channel } from "./streams";
import type { Action, CompiledRule } from "./generated/rules";

export type { Action, CompiledRule };

/// Every action, in the order the editor offers them: what the rule does to
/// the text first, then what it does to the window, loudest last.
export const ACTIONS: { id: Action; title: string; hint: string }[] = [
  {
    id: "highlight",
    title: "Highlight the words",
    hint: "Marks them in the chat and rings the bubble. Needs words to mark.",
  },
  {
    id: "flash",
    title: "Flash the window",
    hint:
      "The whole window in dark orange, once — even when it is in front. " +
      "Which window is what the views say.",
  },
  {
    id: "attention",
    title: "Ask for attention",
    hint: "One bounce of an unfocused window.",
  },
  {
    id: "urgentAttention",
    title: "Ask urgently",
    hint: "Keeps asking until the app is looked at. What an alert term did.",
  },
];

export interface Rule {
  /// Stable for the lifetime of the window, like a channel's or a group's: it
  /// is what Svelte identifies a row by, and it is not stored.
  key: number;
  /// What the operator calls it. Allowed to be empty — the trigger is on
  /// screen anyway, and a name nobody needed would be a field invented to
  /// make a rule saveable.
  name: string;
  /// Off keeps a rule without deleting it. The show where the cue is not
  /// wanted is not the show where the rule is rewritten from memory.
  enabled: boolean;
  /// Words and phrases, matched whole-word and case-insensitively by the
  /// backend's one matcher. Empty is no word condition: "anything at all on
  /// channel 3".
  words: string[];
  /// **Where it has to be said**: channel keys, never stream ids — see the
  /// note above. Empty is every channel.
  channels: number[];
  /// **Which windows the flash reaches**: group keys, and not a second way of
  /// saying `channels`. A group is a view, so naming one here cannot change
  /// when the rule fires — it changes who is interrupted. Empty is every
  /// window that shows the channel, which is what the flash did before it
  /// could be addressed at all.
  views: number[];
  actions: Action[];
  /// At most one firing per this many milliseconds; zero is unthrottled.
  minIntervalMs: number;
}

let nextKey = 0;

export const newRule = (existing: Rule[]): Rule => ({
  key: nextKey++,
  name: `Rule ${existing.length + 1}`,
  // On, because a rule is written in order to be used: the switch is for
  // silencing one later, not for arming it now.
  enabled: true,
  words: [],
  channels: [],
  views: [],
  // Quiet to begin with in the two ways that interrupt, and useful in the one
  // that does not: a new rule with no trigger yet must not start by flooding
  // the window on every message said.
  actions: ["highlight"],
  minIntervalMs: 0,
});

/// A rule read back from the stored configuration, with its keys already
/// translated from the positions the file holds — see `profiles.ts`, which is
/// the one place that translation happens.
export const restoreRule = (stored: {
  name: string;
  enabled: boolean;
  words: string[];
  channels: number[];
  views: number[];
  actions: Action[];
  minIntervalMs: number;
}): Rule => ({ key: nextKey++, ...stored });

/// Whether a rule says *where it is said* at all. Empty is "any channel",
/// which is what "flash on every message" became.
export const isLocated = (rule: Rule): boolean => rule.channels.length > 0;

/// Whether a rule says *which windows* its flash is for. Empty is every
/// window that shows the channel.
export const isAddressed = (rule: Rule): boolean => rule.views.length > 0;

/// A channel or group deleted from the setup has to leave every rule with it.
///
/// The same thing `forgetChannels` does for group membership, with one
/// addition: a rule that loses *everything* it named — every channel, or
/// every window — is switched off rather than left to fire. Dropping the last
/// channel of "Feuer on the stage manager's channel" would otherwise turn a
/// rule the operator narrowed into the loudest one they have, and dropping
/// the last view would turn a flash meant for one window into one that floods
/// them all. A rule that grew louder because something was deleted is exactly
/// the surprise a show cannot absorb, so it stays in the list — visible, and
/// re-pointable — with its switch off.
export const forget = (rules: Rule[], channels: Channel[], groups: Group[]): Rule[] => {
  const liveChannels = new Set(channels.map((c) => c.key));
  const liveGroups = new Set(groups.map((g) => g.key));

  return rules.map((rule) => {
    const kept = {
      ...rule,
      channels: rule.channels.filter((k) => liveChannels.has(k)),
      views: rule.views.filter((k) => liveGroups.has(k)),
    };
    const lost =
      (isLocated(rule) && !isLocated(kept)) || (isAddressed(rule) && !isAddressed(kept));
    return lost ? { ...kept, enabled: false } : kept;
  });
};

/// The rules as the backend evaluates them (P19).
///
/// Four things happen here and nowhere else:
///
///   - channels become **stream ids**, which is the only thing the pipeline
///     names a channel by;
///   - views become **group positions**, which is what a group window is
///     addressed by (`group-3`) and the only handle on a view that means the
///     same thing in two windows. The backend never asks what one is: it
///     hands them back with the event, and each window compares them with the
///     view it is;
///   - a rule that cannot fire is left out — switched off, with nothing to
///     do, or narrowed to channels that are all gone. The backend must never
///     be handed an empty stream list, because "listens to nothing" and
///     "listens to everything" would then look alike;
///   - *highlight* is dropped from a rule with no words. There is nothing to
///     mark, and a rule that claimed to highlight and never did would look
///     like a broken matcher rather than a rule missing its words. *Flash* is
///     dropped on the same terms from a rule addressed only to windows that
///     are gone — for the reason an empty stream list is never sent.
export const compile = (rules: Rule[], channels: Channel[], groups: Group[]): CompiledRule[] => {
  // The stream id is the channel's position in the profile, exactly as the
  // backend assigns it — see `SessionConfig::streams`.
  const streamOf = new Map(channels.map((channel, streamId) => [channel.key, streamId]));
  // The group's position in the profile, which is how a group window is
  // named and how the file stores a group — see `windows.ts`.
  const positionOf = new Map(groups.map((group, position) => [group.key, position]));

  const compiled: CompiledRule[] = [];

  for (const rule of rules) {
    if (!rule.enabled) continue;

    const words = cleanTerms(rule.words);

    // Which windows the flash is for. `null` is every window showing the
    // channel; a rule addressed only to groups that have been deleted keeps
    // its other actions and loses this one, because an empty list on the wire
    // would be read as "nobody" and sending `null` instead would flood every
    // window — the two mistakes the addressing exists to prevent.
    let views: number[] | null = null;
    if (isAddressed(rule)) {
      const set = new Set<number>();
      for (const key of rule.views) {
        const position = positionOf.get(key);
        if (position !== undefined) set.add(position);
      }
      views = set.size === 0 ? null : [...set].sort((a, b) => a - b);
    }

    const reachable = !isAddressed(rule) || views !== null;
    const actions = rule.actions.filter(
      (a) => (a !== "highlight" || words.length > 0) && (a !== "flash" || reachable),
    );
    if (actions.length === 0) continue;

    let streams: number[] | null = null;
    if (isLocated(rule)) {
      const set = new Set<number>();
      for (const key of rule.channels) {
        const streamId = streamOf.get(key);
        if (streamId !== undefined) set.add(streamId);
      }
      if (set.size === 0) continue;
      streams = [...set].sort((a, b) => a - b);
    }

    compiled.push({
      name: rule.name,
      words,
      streams,
      views: actions.includes("flash") ? views : null,
      actions,
      minIntervalMs: Math.max(0, Math.trunc(rule.minIntervalMs) || 0),
    });
  }

  return compiled;
};

/// Whether two compiled lists say the same thing, for deciding whether the
/// list on the backend is still current.
///
/// By serialising, which is sound here and only here: `compile` builds every
/// object with the same fields in the same order and sorts the stream ids, so
/// two lists that mean the same thing are the same string. Recompiling costs
/// nothing, so a comparison that is occasionally too strict is the right way
/// round — it would resend, not miss a change.
export const sameCompiled = (a: CompiledRule[], b: CompiledRule[]): boolean =>
  JSON.stringify(a) === JSON.stringify(b);

// --- The editor's two conversions ----------------------------------------
//
// Words are one comma-separated field rather than a row each. A rule already
// has a name, a switch, two lists of places, four checkboxes and an interval;
// a growing column of one-word rows inside all that is what makes a rule look
// like a form to fill in. A comma is also how the operator says it out loud —
// "Feuer, Arzt" — and a phrase with a comma in it is not a thing anybody puts
// on an intercom.

export const parseWords = (text: string): string[] =>
  text
    .split(",")
    .map((word) => word.trim())
    .filter((word) => word !== "");

export const wordsText = (words: string[]): string => words.join(", ");

/// What a rule listens to, in a sentence, for the card that is collapsed.
///
/// Built from the setup rather than from the rule alone: a channel key means
/// nothing to read, and the point of the line is that a rule can be recognised
/// without opening it. Only the trigger — where the flash goes is a second
/// sentence, because it is a second question. See `audienceOf`.
export const triggerOf = (rule: Rule, channels: Channel[]): string => {
  const words = cleanTerms(rule.words);
  const places = rule.channels
    .map((key) => channels.find((c) => c.key === key)?.label)
    .filter(Boolean);

  const said = words.length === 0 ? "Anything said" : words.join(", ");
  return places.length === 0 ? `${said}, on any channel` : `${said}, on ${places.join(", ")}`;
};

/// Which windows the flash reaches, in a sentence — or `null` when the rule
/// does not flash and the question does not arise.
export const audienceOf = (rule: Rule, groups: Group[]): string | null => {
  if (!rule.actions.includes("flash")) return null;

  const named = rule.views.map((key) => groups.find((g) => g.key === key)?.name).filter(Boolean);
  return named.length === 0
    ? "Flashes every window showing that channel"
    : `Flashes ${named.join(", ")}`;
};

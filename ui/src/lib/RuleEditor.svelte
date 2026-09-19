<script lang="ts">
  // The rules: what fires, and what it does (P19).
  //
  // One editor replacing the two places a reaction used to be asked for — the
  // alert term list, and a checkbox on each group. Both were a trigger and an
  // action welded together, and neither could be narrowed: "Feuer" fired on
  // every channel or on none, and a group's flash fired on every word said or
  // on none. Here the two halves are written separately, so the rule an
  // operator actually wants — this word, on that channel — is one row.
  //
  // **Channels and groups are two questions, so they are two rows.** They
  // were one list of places, and the rule that read "Feuer, on the stage
  // manager's view" then flooded every window that showed one of those
  // channels — because a group named there narrowed *when* the rule fired and
  // never said anything about *which window* was told. A channel is an input
  // and belongs to the trigger; a group is a window and belongs to the flash.
  // Putting them in one row was the whole of that bug, so nothing here puts
  // them in one row again.
  //
  // A card per rule, and the whole rule on it. Triggers, conjunctions and
  // actions are a small language, and the way a small language goes unused is
  // a page that hides half of it behind a disclosure: the operator has to be
  // able to read what a rule does without opening anything. The line under the
  // name is the same rule said in words, which is what makes a list of six of
  // them scannable.
  //
  // Editable while a session runs, like the alert list it replaces and unlike
  // the channel map: rules are matched against finished text, so one written
  // in the middle of a show takes effect on the next segment — which is
  // exactly when somebody thinks of writing one.

  import {
    ACTIONS,
    audienceOf,
    newRule,
    parseWords,
    triggerOf,
    wordsText,
    type Action,
    type Rule,
  } from "./rules";
  import type { Group } from "./groups";
  import type { Channel } from "./streams";

  interface Props {
    rules: Rule[];
    /// The active profile's channels and groups: the inputs a trigger can
    /// name, and the windows a flash can be addressed to. Both belong to the
    /// profile, which is why the rules do too — a rule saying "channel 3"
    /// means a different channel in the next production.
    channels: Channel[];
    groups: Group[];
  }

  let { rules = $bindable(), channels, groups }: Props = $props();

  const add = () => {
    rules = [...rules, newRule(rules)];
  };

  const remove = (key: number) => {
    rules = rules.filter((r) => r.key !== key);
  };

  /// Toggling is the only way a list of keys changes, and it is idempotent in
  /// both directions: the UI offers a checkbox, and a double event from one
  /// click must not leave the rule in the other state.
  const toggle = <T,>(list: T[], value: T): T[] =>
    list.includes(value) ? list.filter((v) => v !== value) : [...list, value];

  /// The interval as the box shows it: seconds, because a rule that fires at
  /// most once a second is how an operator says it, and four zeroes in a
  /// millisecond box is how a typo becomes an evening without a cue.
  const seconds = (ms: number): number => Math.round(ms / 100) / 10;

  const setSeconds = (rule: Rule, value: string) => {
    const parsed = Number.parseFloat(value);
    rule.minIntervalMs = Number.isFinite(parsed) && parsed > 0 ? Math.round(parsed * 1000) : 0;
  };

  /// Whether this rule can ask for the action at all. Highlighting needs words
  /// to highlight; the box is disabled rather than hidden, because "why can I
  /// not tick this" is answered by a label that is still there.
  const offered = (rule: Rule, action: Action): boolean =>
    action !== "highlight" || parseWords(wordsText(rule.words)).length > 0;

  /// Whether to ask which windows the flash is for. Only when it flashes:
  /// a row of group chips on a rule that does not flash is a question about
  /// nothing, and it is the question that used to be confused with the
  /// trigger.
  const flashes = (rule: Rule): boolean => rule.actions.includes("flash");
</script>

<section class="panel">
  <header>
    <h2>Rules</h2>
    <span class="count">{rules.length} {rules.length === 1 ? "rule" : "rules"}</span>
    <button class="action" onclick={add}>Add rule</button>
  </header>

  <p class="hint">
    A rule is a trigger and what it does. Words are matched whole and in any capitalisation,
    so <em>Cue</em> does not fire on <em>rescue</em> — add the compound if you need it. Leave
    the words empty for every message, leave the channels empty for every channel, and
    fill in both for the rule that could not be written before: this word, on that channel.
    <strong>A channel is an input, a group is a window:</strong> channels say where it has to
    be said, groups say which window flashes. Every new message is marked in the chat whatever
    these say.
  </p>

  {#if rules.length === 0}
    <p class="hint empty">No rules. Nothing is highlighted and nothing flashes.</p>
  {:else}
    <ul>
      {#each rules as rule (rule.key)}
        <li class="rule" class:off={!rule.enabled}>
          <div class="top">
            <label class="switch" title="Keep this rule without letting it fire">
              <input type="checkbox" bind:checked={rule.enabled} />
              <span class="sr">Enabled</span>
            </label>
            <input
              type="text"
              class="name"
              bind:value={rule.name}
              placeholder="What this is for"
              aria-label="Rule name"
            />
            <button class="remove" onclick={() => remove(rule.key)} title="Delete this rule">
              ×
            </button>
          </div>

          <!-- The rule said back in words. The one line that makes a list of
               rules readable without opening each of them — two lines when it
               flashes, because who is told is a second thing to know. -->
          <p class="reads">{triggerOf(rule, channels)}</p>
          {#if audienceOf(rule, groups)}
            <p class="reads">{audienceOf(rule, groups)}</p>
          {/if}

          <div class="field">
            <span class="legend">Words</span>
            <input
              type="text"
              value={wordsText(rule.words)}
              oninput={(e) => (rule.words = parseWords(e.currentTarget.value))}
              placeholder="Feuer, Arzt — or leave empty for anything said"
              aria-label="Words that trigger this rule"
            />
          </div>

          <!-- Where it has to be said: input channels, and nothing else. -->
          {#if channels.length > 0}
            <div class="field">
              <span class="legend">On</span>
              <div class="chips">
                {#each channels as channel (channel.key)}
                  <label
                    class="chip channel"
                    class:picked={rule.channels.includes(channel.key)}
                    style="--channel: {channel.color}"
                  >
                    <input
                      type="checkbox"
                      checked={rule.channels.includes(channel.key)}
                      onchange={() => (rule.channels = toggle(rule.channels, channel.key))}
                    />
                    <span class="dot"></span>
                    <span>{channel.label}</span>
                  </label>
                {/each}
                <span class="aside">
                  {rule.channels.length === 0 ? "any channel" : "input channels"}
                </span>
              </div>
            </div>
          {/if}

          <div class="field">
            <span class="legend">Then</span>
            <div class="actions">
              {#each ACTIONS as action (action.id)}
                <label
                  class="action-item"
                  class:unavailable={!offered(rule, action.id)}
                  title={action.hint}
                >
                  <input
                    type="checkbox"
                    checked={rule.actions.includes(action.id)}
                    disabled={!offered(rule, action.id)}
                    onchange={() => (rule.actions = toggle(rule.actions, action.id))}
                  />
                  <span>{action.title}</span>
                </label>
              {/each}
            </div>
          </div>

          <!-- And who is told. A second row under the actions rather than
               beside the channels, because it qualifies one action and says
               nothing about when the rule fires. -->
          {#if flashes(rule) && groups.length > 0}
            <div class="field">
              <span class="legend">Flash in</span>
              <div class="chips">
                {#each groups as group (group.key)}
                  <label class="chip" class:picked={rule.views.includes(group.key)}>
                    <input
                      type="checkbox"
                      checked={rule.views.includes(group.key)}
                      onchange={() => (rule.views = toggle(rule.views, group.key))}
                    />
                    <span>{group.name}</span>
                  </label>
                {/each}
                <span class="aside">
                  {rule.views.length === 0
                    ? "every window showing that channel"
                    : "group windows — and the main window while it shows that group"}
                </span>
              </div>
            </div>
          {/if}

          <label class="limit">
            <span>At most once every</span>
            <input
              type="number"
              min="0"
              step="0.5"
              value={seconds(rule.minIntervalMs)}
              oninput={(e) => setSeconds(rule, e.currentTarget.value)}
            />
            <span>seconds — 0 for no limit</span>
          </label>
        </li>
      {/each}
    </ul>

    <p class="hint foot">
      A rule with a channel and no words fires on <em>every</em> message of that channel,
      which on a busy talkback is several a minute. That is what the limit is for, and a
      rule inside its limit does nothing at all — including highlighting. A flash with no
      group named reaches every window showing that channel, the main window included.
    </p>
  {/if}
</section>

<style>
  .hint {
    margin: 0 0 0.6rem;
    font-size: 0.8rem;
    color: var(--text-dim);
    max-width: 44rem;
  }

  .foot {
    margin: 0.6rem 0 0;
  }

  .empty {
    margin-bottom: 0;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    max-width: 44rem;
  }

  .rule {
    border: 1px solid var(--border);
    border-radius: 0.4rem;
    padding: 0.55rem;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  /* A rule that is off is still readable — it is kept in order to be read and
     switched back on — but it must not look like one that is firing. */
  .off .reads,
  .off .field,
  .off .limit {
    opacity: 0.5;
  }

  .top {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .name {
    flex: 1;
    font-weight: 600;
  }

  .reads {
    margin: 0;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .field {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }

  .legend {
    flex: none;
    width: 3rem;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }

  .field input[type="text"] {
    flex: 1;
  }

  .chips,
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }

  .chip,
  .action-item {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.8rem;
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0.15rem 0.55rem;
    cursor: pointer;
  }

  .chip.picked {
    border-color: var(--accent);
  }

  /* What an empty row means, where the row is. Said here rather than in the
     paragraph above the list, because the answer differs per rule. */
  .aside {
    align-self: center;
    font-size: 0.75rem;
    color: var(--text-dim);
  }

  .action-item.unavailable {
    opacity: 0.45;
    cursor: default;
  }

  .dot {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 50%;
    background: var(--channel);
    flex: none;
  }

  .limit {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .limit input {
    width: 4.5rem;
  }

  .switch {
    display: flex;
    align-items: center;
  }

  /* Visually hidden, still read out: the checkbox has no visible label
     because the name field is right beside it. */
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  input[type="text"],
  input[type="number"] {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.25rem 0.4rem;
    font: inherit;
    font-size: 0.85rem;
    min-width: 0;
  }

  button {
    background: var(--accent);
    color: #12141a;
    border: none;
    border-radius: 0.3rem;
    padding: 0.3rem 0.8rem;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .remove {
    background: transparent;
    color: var(--text-dim);
    font-size: 1.1rem;
    line-height: 1;
    padding: 0.2rem 0.45rem;
  }

  .remove:hover {
    color: #ff6b6b;
  }
</style>

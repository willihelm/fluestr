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
  //
  // **The webhook is written on the machine and nowhere else.** Every other
  // field on this card is editable from the LAN remote, because the remote is
  // a peer (P16). A destination is not: the remote's key is a shared lock
  // rather than an identity, and an arbitrary URL writable from a browser form
  // means anyone who got in can point crew speech off-site. So the
  // URL, the method, the headers and the body exist only in the app's own
  // window; the remote sees *that* a rule calls out, can switch it off, and
  // can edit its trigger. Everything below that branches on `isRemote` is that
  // one decision — see P19 in `docs/plan.md`.

  import {
    ACTIONS,
    audienceOf,
    callsOut,
    destinationOf,
    METHODS,
    newRule,
    newWebhook,
    parseWords,
    PLACEHOLDERS,
    triggerOf,
    wordsText,
    type Action,
    type Rule,
  } from "./rules";
  import { isRemote } from "./transport";
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

  /// Whether this rule can ask for the action at all. The box is disabled
  /// rather than hidden, because "why can I not tick this" is answered by a
  /// label that is still there.
  ///
  /// Two reasons it cannot. Highlighting needs words to highlight. And a
  /// webhook cannot be *started* from the remote — but one that already exists
  /// can always be un-ticked there, because switching a cue off in a hurry is
  /// the one thing a browser at the other end of the house is for.
  const offered = (rule: Rule, action: Action): boolean => {
    if (action === "highlight") return parseWords(wordsText(rule.words)).length > 0;
    if (action === "webhook" && isRemote) return rule.webhook !== null;
    return true;
  };

  /// Ticking an action, with the one that carries a payload handled here.
  ///
  /// Ticking *call a webhook* on the machine opens an empty request to fill
  /// in. Un-ticking leaves it in place rather than deleting it: the rule is
  /// being switched off, not rewritten, and an operator who un-ticks the box
  /// during a show wants the destination back afterwards. Nothing is sent
  /// while the box is clear — see `compile`.
  const tickAction = (rule: Rule, action: Action) => {
    rule.actions = toggle(rule.actions, action);
    if (action === "webhook" && rule.actions.includes(action) && rule.webhook === null) {
      rule.webhook = newWebhook();
    }
  };

  /// Whether to show the request. Only on the machine, and only when the rule
  /// asks for one: a URL box on a rule that does not call out is a question
  /// about nothing.
  const writesTheRequest = (rule: Rule): boolean =>
    !isRemote && rule.actions.includes("webhook") && rule.webhook !== null;

  const addHeader = (rule: Rule) => {
    if (!rule.webhook) return;
    rule.webhook = { ...rule.webhook, headers: [...rule.webhook.headers, { name: "", value: "" }] };
  };

  const removeHeader = (rule: Rule, index: number) => {
    if (!rule.webhook) return;
    rule.webhook = {
      ...rule.webhook,
      headers: rule.webhook.headers.filter((_, i) => i !== index),
    };
  };

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
    these say. A rule can also <strong>call a webhook</strong> — one HTTP request per matching
    bubble, written here on the machine and sent from it.
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
          <!-- The third sentence, and the only one about something outside
               this machine. On the collapsed card it is the host and nothing
               else: which box this reaches is the question, and a Companion
               press URL with a page, a row and a column in it is longer than
               the rest of the card. -->
          {#if destinationOf(rule)}
            <p class="reads out">{destinationOf(rule)}</p>
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
                    onchange={() => tickAction(rule, action.id)}
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

          <!-- The request, on the machine only. Four boxes and no preset: the
               next thing somebody wants to call is a media server, a light
               desk's own gateway, or something bought after this was written,
               and a catalogue of endpoints would make each of those a
               release. The Companion lines under the URL are a hint to copy,
               not a mode to pick. -->
          {#if writesTheRequest(rule) && rule.webhook}
            <div class="request">
              <div class="field">
                <span class="legend">Call</span>
                <select bind:value={rule.webhook.method} aria-label="HTTP method">
                  {#each METHODS as method (method)}
                    <option value={method}>{method.toUpperCase()}</option>
                  {/each}
                </select>
                <input
                  type="url"
                  class="url"
                  bind:value={rule.webhook.url}
                  placeholder="http://companion:8000/api/location/1/0/2/press"
                  aria-label="Webhook URL"
                />
              </div>

              <p class="hint example">
                Bitfocus Companion, which most houses already have, is a fan-out point: a
                button there reaches every device Companion has a module for — MIDI among
                them. Two to copy:
              </p>
              <pre class="example">POST http://&lt;companion-host&gt;:8000/api/location/1/0/2/press
POST http://&lt;companion-host&gt;:8000/api/custom-variable/last_call/value
     body: &#123;&#123;text&#125;&#125;</pre>

              <div class="field headers">
                <span class="legend">Headers</span>
                <div class="rows">
                  {#each rule.webhook.headers as header, i (i)}
                    <div class="header-row">
                      <input
                        type="text"
                        bind:value={header.name}
                        placeholder="Content-Type"
                        aria-label="Header name"
                      />
                      <input
                        type="text"
                        bind:value={header.value}
                        placeholder="application/json"
                        aria-label="Header value"
                      />
                      <button
                        class="remove"
                        onclick={() => removeHeader(rule, i)}
                        title="Delete this header"
                      >
                        ×
                      </button>
                    </div>
                  {/each}
                  <button class="small" onclick={() => addHeader(rule)}>Add header</button>
                </div>
              </div>

              <div class="field">
                <span class="legend">Body</span>
                <textarea
                  bind:value={rule.webhook.body}
                  rows="2"
                  placeholder="Empty — what was said stays here unless you put it in"
                  aria-label="Request body"
                ></textarea>
              </div>

              <p class="hint">
                The URL, every header value and the body are templates.
                {#each PLACEHOLDERS as placeholder, i (placeholder.name)}<code
                  >{placeholder.name}</code
                >&nbsp;is {placeholder.means}{i < PLACEHOLDERS.length - 1 ? ", " : "."}{/each}
                A value put in the URL is percent-encoded, and one put in a body is escaped as
                JSON when a <code>Content-Type</code> header says JSON. <strong>The body starts
                empty on purpose:</strong> putting <code>&#123;&#123;text&#125;&#125;</code> in
                it sends what was said in this production to that address, so it is a thing to
                do deliberately.
              </p>

              <p class="hint">
                One request per matching bubble, from this machine, with a three-second timeout
                and one retry — then it is dropped and counted beside the meters. Use the limit
                below on a rule with a channel trigger: it is what keeps a busy talkback from
                becoming a stream of requests.
              </p>
            </div>
          {:else if isRemote && callsOut(rule)}
            <!-- The remote sees that the rule calls out and can switch it off.
                 It has no box to change the destination with, and that is the
                 whole of P19's decision — the remote's key says who may reach
                 the session, not which of the people holding it is typing, and
                 an arbitrary URL writable from a browser form is a different
                 proposition from the remote that decision was made for. -->
            <p class="hint out">
              This rule calls a webhook. The request is set up on the machine running the
              session — untick the action above to stop it from here.
            </p>
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
    margin: 0 0 1rem;
    font-size: 0.8rem;
    color: var(--text-dim);
    max-width: 44rem;
  }

  .foot {
    margin: 1rem 0 0;
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
    gap: 1rem;
    max-width: 44rem;
  }

  .rule {
    border: 1px solid var(--border);
    border-radius: 0.4rem;
    padding: 0.85rem 0.9rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  /* A rule that is off is still readable — it is kept in order to be read and
     switched back on — but it must not look like one that is firing. */
  .off .reads,
  .off .field,
  .off .request,
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
    margin: -0.35rem 0 0;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .field {
    display: flex;
    align-items: baseline;
    gap: 0.75rem;
  }

  .legend {
    flex: none;
    width: 4.5rem;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }

  .field input[type="text"],
  .field input[type="url"] {
    flex: 1;
  }

  .chips,
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
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

  /* The one line about something outside this machine, and the note the
     remote gets in place of the boxes. Dimmer than an error and not an
     accent: it is a fact about the rule, not a warning. */
  .reads.out,
  .hint.out {
    color: var(--text-dim);
    font-style: italic;
  }

  .request {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 0.75rem 0.8rem;
    border: 1px dashed var(--border);
    border-radius: 0.3rem;
  }

  .request .url {
    flex: 1;
  }

  .rule .hint {
    margin: 0;
  }

  /* The two lines to copy. Monospaced because they are a URL and a body, and
     scrollable rather than wrapped: a broken URL is harder to read than one
     that runs off the edge. */
  pre.example {
    margin: 0;
    padding: 0.35rem 0.5rem;
    background: var(--bg);
    border-radius: 0.3rem;
    font-size: 0.72rem;
    color: var(--text-dim);
    overflow-x: auto;
  }

  .headers {
    align-items: flex-start;
  }

  .rows {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    flex: 1;
    min-width: 0;
  }

  .header-row {
    display: flex;
    gap: 0.3rem;
  }

  .header-row input {
    flex: 1;
    min-width: 0;
  }

  .request textarea {
    flex: 1;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.25rem 0.4rem;
    font-family: inherit;
    font-size: 0.85rem;
    resize: vertical;
    min-width: 0;
  }

  .request select {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.25rem 0.3rem;
    font: inherit;
    font-size: 0.8rem;
  }

  .request code {
    font-size: 0.72rem;
    color: var(--text);
  }

  .small {
    align-self: flex-start;
    background: transparent;
    color: var(--text-dim);
    border: 1px solid var(--border);
    font-weight: 400;
    font-size: 0.75rem;
    padding: 0.15rem 0.5rem;
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
  input[type="url"],
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
    color: var(--on-accent);
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
    color: var(--danger);
  }
</style>

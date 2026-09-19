<script lang="ts">
  // How every view onto the session is shown, for the whole app (P7).
  //
  // One setting so far — which end of the chat new lines arrive at — and the
  // section is named after the thing rather than after it: a window is a view
  // of which channels are shown, how they are read and what it does when
  // something arrives, and each of those is the same kind of answer. "Chat"
  // would have to be renamed by the second one.
  //
  // The default and not the reading direction: every chat window carries a
  // toggle in its own bar, and turning one over there is what an operator does
  // during a show. This is the answer for the windows that have not been
  // turned — the one opened next, the ones opened after a restart, and every
  // browser the remote serves.
  //
  // Beside the vocabulary in scope, and for its reason: how a house reads its
  // chat outlives any one patch, so it is stored once in `config.json` rather
  // than per profile. A view that wants the other direction overrides it for
  // itself, and that override is the screen's own — see `sort.ts`.

  import { label, type SortDirection } from "./sort";

  interface Props {
    sort: SortDirection;
  }

  let { sort = $bindable() }: Props = $props();

  // The two, in the order they are on screen: the chat as everyone has always
  // had it first, and the turned one under it.
  const choices: SortDirection[] = ["newest-bottom", "newest-top"];

  const note = (direction: SortDirection) =>
    direction === "newest-top"
      ? "The newest line is at the top, where the eye already is, and the history runs downwards."
      : "The chat reads downwards and follows the bottom edge, like every other chat.";
</script>

<section class="panel">
  <header>
    <h2>View</h2>
  </header>

  <p class="hint">
    Which end of the chat new lines arrive at. Every window opens this way — including
    the browsers on the remote — and any one of them can be turned over on its own from
    the control beside its search box.
  </p>

  <!-- A radio group rather than a checkbox, because neither direction is the
       absence of the other: "newest at bottom" is a choice an operator makes,
       and a box labelled "newest at top" that is off says nothing about what
       the chat then does. -->
  <div class="choices" role="radiogroup" aria-label="Where new lines arrive">
    {#each choices as choice (choice)}
      <label class="choice" class:on={sort === choice}>
        <input
          type="radio"
          name="chat-sort"
          checked={sort === choice}
          onchange={() => (sort = choice)}
        />
        <span class="title">
          {label(choice)}
          <span class="arrow" aria-hidden="true">{choice === "newest-top" ? "↑" : "↓"}</span>
        </span>
        <span class="note">{note(choice)}</span>
      </label>
    {/each}
  </div>
</section>

<style>
  .hint {
    margin: 0 0 0.6rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .choices {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  /* A card each rather than a row of radios, so the sentence that says what
     the direction does is part of the thing being chosen instead of a legend
     under it. */
  .choice {
    flex: 1 1 15rem;
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.1rem 0.5rem;
    align-items: baseline;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 0.4rem;
    padding: 0.5rem 0.6rem;
    cursor: pointer;
  }

  .choice.on {
    border-color: var(--accent);
  }

  .choice input {
    grid-row: span 2;
    align-self: center;
    margin: 0;
  }

  .title {
    font-size: 0.9rem;
    font-weight: 600;
  }

  .note {
    grid-column: 2;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .arrow {
    color: var(--text-dim);
    font-weight: 400;
  }
</style>

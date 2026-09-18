<script lang="ts">
  // The chat's left sidebar: which view of the channels is on screen.
  //
  // Selection only — groups are created and filled on the Settings page. The
  // sidebar is read during a show, when the question is "show me just the
  // stage" and not "let me reorganise my groups", and a rename box under the
  // cursor at that moment is a liability.
  //
  // "All channels" is always first and cannot be deleted: it is the state the
  // chat is in when no group is selected, not a group that happens to contain
  // everything.
  //
  // The one thing a row does besides select is open the group in a window of
  // its own (P7) — the second half of "show me just the stage", for the stage
  // that has to stay visible while the operator works in another view.

  import { openGroupWindow } from "./bridge";
  import type { Group } from "./groups";
  import type { Channel } from "./streams";

  interface Props {
    groups: Group[];
    channels: Channel[];
    /// `null` is the all-channels view.
    selected: number | null;
  }

  let { groups, channels, selected = $bindable() }: Props = $props();

  const colorsOf = (group: Group) =>
    channels.filter((c) => group.members.includes(c.key)).map((c) => c.color);

  /// Opens a group in its own window, by its **position** — which is what the
  /// configuration file stores membership by, and the only handle on a group
  /// that means the same thing in another window. The key is stable for this
  /// window's lifetime and meaningless outside it.
  ///
  /// Asking for a group that is already open focuses that window instead of
  /// stacking a second one on it; the backend decides that, because it is the
  /// side that knows which windows exist.
  const detach = (index: number, group: Group) => {
    openGroupWindow(index, group.name, group.onTop).catch((e) =>
      console.warn("the group window did not open:", e),
    );
  };
</script>

<nav>
  <h2>Views</h2>

  <button class="view" class:active={selected === null} onclick={() => (selected = null)}>
    <span class="name">All channels</span>
    <span class="count">{channels.length}</span>
  </button>

  <!-- A row is a button and carries a second one, so it is a div rather than a
       nested pair: a button inside a button is invalid and only one of the two
       would ever be clicked. -->
  {#each groups as group, index (group.key)}
    {@const colors = colorsOf(group)}
    <div class="row">
      <button
        class="view"
        class:active={selected === group.key}
        onclick={() => (selected = group.key)}
      >
        <span class="name">{group.name}</span>
        <!-- The member channels' colours, which is how a group is recognised at
             a glance — the same colours carry the bubbles in the chat. -->
        <span class="dots">
          {#each colors as color}
            <span class="dot" style="background: {color}"></span>
          {/each}
        </span>
        <span class="count">{colors.length}</span>
      </button>
      <button
        class="detach"
        onclick={() => detach(index, group)}
        title="Open {group.name} in its own window"
        aria-label="Open {group.name} in its own window"
      >
        ⧉
      </button>
    </div>
  {/each}

  {#if groups.length === 0}
    <p class="empty">Groups are made on the Settings page.</p>
  {/if}
</nav>

<style>
  nav {
    width: 12rem;
    flex: none;
    border-right: 1px solid var(--border);
    background: var(--surface);
    padding: 0.5rem;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  h2 {
    margin: 0 0 0.3rem 0.35rem;
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }

  /* The row the two buttons share. The second one only appears under the
     cursor: it is reached for now and then, and eight of them down the side of
     a live chat is eight things competing with the group names. It stays
     available to the keyboard, where hover means nothing. */
  .row {
    display: flex;
    align-items: center;
  }

  .detach {
    flex: none;
    visibility: hidden;
    background: transparent;
    color: var(--text-dim);
    border: none;
    border-radius: 0.3rem;
    padding: 0.35rem 0.4rem;
    font: inherit;
    font-size: 0.85rem;
    line-height: 1;
    cursor: pointer;
  }

  .row:hover .detach,
  .detach:focus-visible {
    visibility: visible;
  }

  .detach:hover {
    color: var(--text);
    background: var(--bg);
  }

  .view {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    width: 100%;
    background: transparent;
    color: var(--text);
    border: none;
    border-radius: 0.3rem;
    padding: 0.35rem 0.5rem;
    font: inherit;
    font-size: 0.85rem;
    text-align: left;
    cursor: pointer;
  }

  .view:hover {
    background: var(--bg);
  }

  .view.active {
    background: var(--bg);
    box-shadow: inset 2px 0 0 var(--accent);
  }

  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dots {
    display: flex;
    gap: 0.12rem;
  }

  .dot {
    width: 0.4rem;
    height: 0.4rem;
    border-radius: 50%;
  }

  .count {
    color: var(--text-dim);
    font-size: 0.75rem;
    min-width: 1rem;
    text-align: right;
  }

  .empty {
    margin: 0.4rem 0.35rem;
    font-size: 0.78rem;
    color: var(--text-dim);
  }
</style>

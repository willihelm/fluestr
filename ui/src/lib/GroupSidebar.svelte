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
</script>

<nav>
  <h2>Views</h2>

  <button class="view" class:active={selected === null} onclick={() => (selected = null)}>
    <span class="name">All channels</span>
    <span class="count">{channels.length}</span>
  </button>

  {#each groups as group (group.key)}
    {@const colors = colorsOf(group)}
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

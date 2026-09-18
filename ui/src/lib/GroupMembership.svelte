<script lang="ts">
  // Which channels are in which group, on the Settings page.
  //
  // A grid rather than a per-group list: with eight channels and a handful of
  // groups the question being answered is usually "where does this channel
  // show up", and that is a column, not a row. Membership is non-exclusive, so
  // a channel may be ticked in any number of groups — nothing here enforces a
  // single home, because there isn't one.
  //
  // Editable while a session runs, unlike the channel map. A group is a view,
  // not part of the capture configuration: an operator watching eight channels
  // live is exactly who needs to narrow them down to three.

  import { isMember, newGroup, toggleMember, type Group } from "./groups";
  import type { Channel } from "./streams";

  interface Props {
    groups: Group[];
    channels: Channel[];
    /// The flash setting of the all-channels view, which is not a group and so
    /// has nowhere in `groups` to keep one — it lives on the profile. Edited
    /// here anyway, because this is where the question is asked of every other
    /// view and an operator looking for it looks here.
    allChannelsFlash: boolean;
  }

  let { groups = $bindable(), channels, allChannelsFlash = $bindable() }: Props = $props();

  const add = () => {
    groups = [...groups, newGroup(groups)];
  };

  const remove = (key: number) => {
    // Deletes the view, never the channels in it.
    groups = groups.filter((g) => g.key !== key);
  };

  const toggle = (group: Group, channel: Channel) => {
    groups = groups.map((g) => (g.key === group.key ? toggleMember(g, channel) : g));
  };
</script>

<section class="panel">
  <header>
    <h2>Groups</h2>
    <span class="count">{groups.length} {groups.length === 1 ? "group" : "groups"}</span>
    <button class="action" onclick={add}>Add group</button>
  </header>

  <p class="hint">
    A group is a view onto some of the channels, selected in the sidebar of the chat.
    A channel can be in any number of groups; removing it from one leaves it in the
    others and in the setup. Every new message is marked in the chat whatever these
    say; flashing adds the loud request — the whole window in dark orange on every
    message of that view, not only on an alert term. For the one view worth being
    interrupted by.
  </p>

  <!-- Above the groups, and outside the grid: the all-channels view is not a
       group — it cannot be renamed, deleted or given members — and a card
       among the others would invite all three. -->
  <label class="all-channels">
    <input type="checkbox" bind:checked={allChannelsFlash} />
    <span>Flash on every message in <strong>All channels</strong></span>
  </label>

  {#if groups.length === 0}
    <p class="empty">No groups yet. The chat shows every channel.</p>
  {:else if channels.length === 0}
    <p class="empty">Add a channel first — there is nothing to put in a group.</p>
  {:else}
    <div class="grid">
      {#each groups as group (group.key)}
        <div class="group">
          <div class="name">
            <input type="text" bind:value={group.name} aria-label="Group name" />
            <button class="remove" onclick={() => remove(group.key)} title="Delete this group">
              ×
            </button>
          </div>

          <!-- Under the name and above the members, because it is a property
               of the group and not of any channel in it. -->
          <label class="flash">
            <input type="checkbox" bind:checked={group.flash} />
            <span>Flash on every message</span>
          </label>

          <div class="members">
            {#each channels as channel (channel.key)}
              <label style="--channel: {channel.color}">
                <input
                  type="checkbox"
                  checked={isMember(group, channel)}
                  onchange={() => toggle(group, channel)}
                />
                <span class="dot"></span>
                <span class="channel-label">{channel.label}</span>
              </label>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .hint,
  .empty {
    margin: 0 0 0.4rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .all-channels {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.8rem;
    color: var(--text-dim);
    margin-bottom: 0.6rem;
  }

  .all-channels strong {
    color: var(--text);
    font-weight: 600;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(15rem, 1fr));
    gap: 0.6rem;
  }

  .group {
    border: 1px solid var(--border);
    border-radius: 0.4rem;
    padding: 0.5rem;
  }

  .name {
    display: flex;
    gap: 0.3rem;
    align-items: center;
    margin-bottom: 0.4rem;
  }

  .name input {
    flex: 1;
  }

  .flash {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.8rem;
    color: var(--text-dim);
    margin-bottom: 0.4rem;
  }

  .members {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .members label {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.85rem;
    color: var(--text);
  }

  .dot {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 50%;
    background: var(--channel);
    flex: none;
  }

  .channel-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  input[type="text"] {
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

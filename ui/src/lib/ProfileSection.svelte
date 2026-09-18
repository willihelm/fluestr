<script lang="ts">
  // The profiles: one production's setup each — interface, channel map, groups
  // — and its own section on the Settings page (P12).
  //
  // It used to be a strip across the top of the page, visible whichever
  // section was open. That put the *rarest* action on the page in front of the
  // most frequent ones: a profile is picked once at the start of a show and
  // then never touched, while the channel map is edited all evening. Worse,
  // New / Duplicate / Delete sat under the cursor the whole time, next to the
  // fields they would discard.
  //
  // So it is a section like the others, and what stays visible instead is the
  // *name*: the rail heads the sections that belong to a profile with it. That
  // is the part an operator has to be able to see while editing — which setup
  // am I changing — and it is a label, not four buttons.
  //
  // A list rather than a dropdown, now that there is room for one: with three
  // setups on a machine the question "which one am I on, and what else is
  // there" is answered by looking rather than by opening a menu, and each row
  // can carry its own name field and its own Duplicate and Delete.
  //
  // Locked while a session runs, like the channel map and for the same reason:
  // the capture configuration is fixed once capture has opened, and switching
  // profiles under a running session would mean the meters, the colours and the
  // groups no longer describe what is being recorded.

  import { duplicate, newProfile, type Profile } from "./profiles";

  interface Props {
    profiles: Profile[];
    activeKey: number | null;
    locked: boolean;
  }

  let { profiles = $bindable(), activeKey = $bindable(), locked }: Props = $props();

  let active = $derived(profiles.find((p) => p.key === activeKey) ?? null);

  const add = () => {
    // Inherits the device of the profile being left: a second setup on the
    // same machine is usually the same interface patched differently.
    const created = newProfile(profiles, active?.device ?? "");
    profiles = [...profiles, created];
    activeKey = created.key;
  };

  const copy = (profile: Profile) => {
    const created = duplicate(profile);
    profiles = [...profiles, created];
    activeKey = created.key;
  };

  // No confirmation, because there is an undo that costs nothing to explain:
  // the profile is only really gone once the file is written, and the operator
  // is holding the app. What is guarded is the *last* profile — deleting it
  // would leave the Settings page with nothing to edit and no obvious way
  // back.
  //
  // Deleting the profile that is not the active one leaves the active one
  // alone; deleting the active one falls back to the first of what is left,
  // because a page with no active profile has nothing to show in Audio or
  // Groups.
  const remove = (profile: Profile) => {
    if (profiles.length < 2) return;
    const left = profiles.filter((p) => p.key !== profile.key);
    profiles = left;
    if (activeKey === profile.key) activeKey = left[0].key;
  };
</script>

<section class="panel">
  <header>
    <h2>Profiles</h2>
    <span class="count">
      {profiles.length}
      {profiles.length === 1 ? "profile" : "profiles"}
    </span>
    <button class="action" onclick={add} disabled={locked}>New profile</button>
  </header>

  <p class="hint">
    A profile is one production's setup: the interface, the channel map with its
    labels, languages and colours, and the groups those channels are seen through.
    The vocabulary and the installed models are not in it — they are shared by every
    profile and by the machine.
  </p>

  <ul class="list">
    {#each profiles as profile (profile.key)}
      <li class="row" class:selected={profile.key === activeKey}>
        <label class="pick">
          <input
            type="radio"
            name="profile"
            value={profile.key}
            bind:group={activeKey}
            disabled={locked}
            aria-label="Use {profile.name}"
          />
        </label>

        <input
          class="name"
          type="text"
          bind:value={profile.name}
          aria-label="Profile name"
        />

        <span class="about">
          {profile.channels.length}
          {profile.channels.length === 1 ? "channel" : "channels"}{profile.groups.length >
          0
            ? `, ${profile.groups.length} ${profile.groups.length === 1 ? "group" : "groups"}`
            : ""}
          {#if profile.device}
            · {profile.device}
          {/if}
        </span>

        <button onclick={() => copy(profile)} disabled={locked}>Duplicate</button>
        <button
          class="remove"
          onclick={() => remove(profile)}
          disabled={locked || profiles.length < 2}
          title={profiles.length < 2
            ? "The last profile cannot be deleted — there would be nothing to configure."
            : "Delete this profile"}
        >
          Delete
        </button>
      </li>
    {/each}
  </ul>

  {#if locked}
    <p class="note">
      Locked while the session runs: the capture configuration is fixed once capture
      has opened.
    </p>
  {/if}
</section>

<style>
  .hint,
  .note {
    margin: 0 0 0.5rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .note {
    margin: 0.5rem 0 0;
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    border: 1px solid var(--border);
    border-radius: 0.4rem;
    padding: 0.35rem 0.5rem;
    flex-wrap: wrap;
  }

  /* The same tint the rail uses for the open section, so "this is the one in
     use" looks the same wherever the page says it. */
  .row.selected {
    background: rgba(110, 168, 254, 0.12);
    border-color: var(--accent);
  }

  .pick {
    display: flex;
    align-items: center;
  }

  .name {
    flex: 1;
    min-width: 9rem;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.9rem;
  }

  /* What the row holds, so a list of three setups can be told apart without
     selecting each one in turn and reading the Audio section. */
  .about {
    font-size: 0.75rem;
    color: var(--text-dim);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  button {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.6rem;
    font: inherit;
    font-size: 0.8rem;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .remove:not(:disabled):hover {
    border-color: #ff6b6b;
    color: #ff6b6b;
  }
</style>

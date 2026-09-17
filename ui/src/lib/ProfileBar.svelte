<script lang="ts">
  // The profile switcher at the top of the Settings page (P12).
  //
  // A profile is one production's setup — interface, channel map, groups — and
  // an engineer works several of them on the same machine. The bar is the
  // first thing on the page on purpose: everything below it belongs to
  // whichever profile is selected here, and a page that let you edit eight
  // channels before revealing which setup you were editing would be a page
  // that loses somebody's patch.
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
    /// Why the last save did not happen, or `null`. Shown here rather than
    /// swallowed: an operator who builds a setup believing it is stored and
    /// finds it gone before the next show has been failed twice.
    saveError: string | null;
  }

  let {
    profiles = $bindable(),
    activeKey = $bindable(),
    locked,
    saveError,
  }: Props = $props();

  let active = $derived(profiles.find((p) => p.key === activeKey) ?? null);

  const add = () => {
    // Inherits the device of the profile being left: a second setup on the
    // same machine is usually the same interface patched differently.
    const created = newProfile(profiles, active?.device ?? "");
    profiles = [...profiles, created];
    activeKey = created.key;
  };

  const copy = () => {
    if (!active) return;
    const created = duplicate(active);
    profiles = [...profiles, created];
    activeKey = created.key;
  };

  // No confirmation, because there is an undo that costs nothing to explain:
  // the profile is only really gone once the file is written, and the operator
  // is holding the app. What is guarded is the *last* profile — deleting it
  // would leave the Settings page with nothing to edit and no obvious way
  // back.
  const remove = () => {
    if (!active || profiles.length < 2) return;
    const left = profiles.filter((p) => p.key !== active.key);
    profiles = left;
    activeKey = left[0].key;
  };
</script>

<section class="panel scope">
  <header>
    <h2>Profile</h2>
    <span class="count">
      {profiles.length}
      {profiles.length === 1 ? "profile" : "profiles"}
    </span>
  </header>

  <div class="row">
    <label class="pick">
      <span>Active</span>
      <select bind:value={activeKey} disabled={locked}>
        {#each profiles as profile (profile.key)}
          <option value={profile.key}>{profile.name}</option>
        {/each}
      </select>
    </label>

    {#if active}
      <label class="name">
        <span>Name</span>
        <input type="text" bind:value={active.name} aria-label="Profile name" />
      </label>
    {/if}

    <div class="actions">
      <button onclick={add} disabled={locked}>New</button>
      <button onclick={copy} disabled={locked || active === null}>Duplicate</button>
      <button
        class="remove"
        onclick={remove}
        disabled={locked || profiles.length < 2}
        title={profiles.length < 2
          ? "The last profile cannot be deleted — there would be nothing to configure."
          : "Delete this profile"}
      >
        Delete
      </button>
    </div>
  </div>

  <p class="hint">
    A profile holds the input device, the channel map with its colours, and the groups.
    The vocabulary is not in it: it is one list for every profile, because the terms of a
    house outlive any one patch.
  </p>

  {#if saveError}
    <p class="error">Not saved: {saveError}</p>
  {/if}
</section>

<style>
  .row {
    display: flex;
    align-items: flex-end;
    gap: 0.6rem;
    flex-wrap: wrap;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: 0.7rem;
    color: var(--text-dim);
  }

  .pick {
    min-width: 12rem;
  }

  .name {
    min-width: 12rem;
  }

  select,
  input {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.9rem;
  }

  .actions {
    display: flex;
    gap: 0.3rem;
  }

  button {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.35rem 0.7rem;
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

  .hint {
    margin: 0.5rem 0 0;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .error {
    margin: 0.35rem 0 0;
    font-size: 0.8rem;
    color: #ff6b6b;
  }
</style>

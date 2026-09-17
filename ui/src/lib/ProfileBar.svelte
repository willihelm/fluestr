<script lang="ts">
  // The profile switcher at the top of the Settings page (P12).
  //
  // A profile is one production's setup — interface, channel map, groups — and
  // an engineer works several of them on the same machine. The bar spans the
  // top of the page on purpose: the sections it governs are named under it in
  // the rail, and a page that let you edit eight channels before revealing
  // which setup you were editing would be a page that loses somebody's patch.
  //
  // A strip rather than a card, and without the paragraph that used to explain
  // what a profile holds. It is not a sixth question of the same kind as the
  // sections below — it *selects what they are about* — and the rail says what
  // belongs to it by listing those sections under its name.
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

<header class="bar">
  <label class="pick">
    <span>Profile</span>
    <select bind:value={activeKey} disabled={locked} aria-label="Active profile">
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

  {#if saveError}
    <!-- Never swallowed: an operator who builds a setup believing it is stored
         and finds it gone before the next show has been failed twice. -->
    <p class="error">Not saved: {saveError}</p>
  {:else if locked}
    <p class="note">Locked while the session runs.</p>
  {/if}
</header>

<style>
  /* The same height and hairline as the two bars above it, so the three read
     as one header rather than as a card that happens to sit at the top. */
  .bar {
    display: flex;
    align-items: flex-end;
    gap: 0.6rem;
    flex-wrap: wrap;
    padding: 0.55rem 1rem;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: 0.7rem;
    color: var(--text-dim);
  }

  .pick,
  .name {
    min-width: 11rem;
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

  .note,
  .error {
    margin: 0 0 0.25rem auto;
    font-size: 0.78rem;
  }

  .note {
    color: var(--text-dim);
  }

  .error {
    color: #ff6b6b;
  }
</style>

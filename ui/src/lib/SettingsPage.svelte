<script lang="ts">
  // Settings: everything about how the session is *configured*, away from the
  // live view. Input device and channel mapping, the one active vocabulary,
  // and group membership.
  //
  // What is deliberately not here is Start/Stop: it is session-wide and stays
  // in the header, reachable from either page, because an operator who has to
  // find the right page before they can stop capture has already lost.
  //
  // Since P12 everything on this page is stored and read back at startup, and
  // the device, the channel map and the groups belong to the *profile*
  // selected at the top. The vocabulary does not — it is one list for every
  // profile, which is why it is bound here and not inside the profile.

  import ProfileBar from "./ProfileBar.svelte";
  import StreamSetup from "./StreamSetup.svelte";
  import VocabularyEditor from "./VocabularyEditor.svelte";
  import GroupMembership from "./GroupMembership.svelte";
  import type { Profile } from "./profiles";
  import type { DeviceOption } from "./bridge";

  interface Props {
    devices: DeviceOption[];
    profiles: Profile[];
    activeKey: number | null;
    vocabulary: string;
    deviceChannels: number;
    locked: boolean;
    saveError: string | null;
  }

  let {
    devices,
    profiles = $bindable(),
    activeKey = $bindable(),
    vocabulary = $bindable(),
    deviceChannels,
    locked,
    saveError,
  }: Props = $props();

  // Mutated in place rather than reassigned: it is an element of the
  // `profiles` array, so writing to its fields is what makes the edit part of
  // the profile — and part of what gets saved.
  let active = $derived(profiles.find((p) => p.key === activeKey) ?? null);

  // A stored device that is not plugged in right now. Kept and shown rather
  // than silently replaced with whatever is connected: the profile describes
  // the production, and an operator who opens the app in the office without
  // the desk must find their setup intact, not quietly re-pointed at the
  // built-in microphone.
  let missing = $derived(
    active !== null && active.device !== "" && !devices.some((d) => d.name === active.device),
  );
</script>

<div class="page">
  <ProfileBar bind:profiles bind:activeKey {locked} {saveError} />

  {#if active}
    <section class="panel">
      <header>
        <h2>Input</h2>
      </header>
      <label>
        <span>Device</span>
        <select bind:value={active.device} disabled={locked}>
          {#each devices as option (option.name)}
            <option value={option.name}>{option.name} ({option.channels} ch)</option>
          {/each}
          {#if missing}
            <!-- Selectable, because it is what the profile says and
                 deselecting it would throw that away. -->
            <option value={active.device}>{active.device} — not connected</option>
          {/if}
        </select>
      </label>
      <p class="hint">
        One multichannel interface for every stream. Two devices would mean two clocks,
        and no common timebase to order one chat by.
      </p>
    </section>

    <StreamSetup bind:channels={active.channels} {deviceChannels} {locked} />
    <VocabularyEditor bind:vocabulary {locked} />
    <GroupMembership bind:groups={active.groups} channels={active.channels} />
  {/if}
</div>

<style>
  /* The gap between the cards is what does the separating, so it is the one
     measurement here that matters: wide enough to read as a break between two
     questions, not so wide that the page becomes a scroll. The page's own
     background shows through it — see `.panel` in app.css. */
  .page {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.9rem;
    padding: 0.9rem 1rem 1.5rem;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: 0.7rem;
    color: var(--text-dim);
    max-width: 28rem;
  }

  select {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.9rem;
  }

  .hint {
    margin: 0.4rem 0 0;
    font-size: 0.8rem;
    color: var(--text-dim);
  }
</style>

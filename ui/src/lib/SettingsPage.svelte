<script lang="ts">
  // Settings: everything about how the session is *configured*, away from the
  // live view. Input device and channel mapping, the one active vocabulary,
  // and group membership.
  //
  // What is deliberately not here is Start/Stop: it is session-wide and stays
  // in the header, reachable from either page, because an operator who has to
  // find the right page before they can stop capture has already lost.
  //
  // P12 still owes persistence for all of it — today the setup lives as long
  // as the window does.

  import StreamSetup from "./StreamSetup.svelte";
  import VocabularyEditor from "./VocabularyEditor.svelte";
  import GroupMembership from "./GroupMembership.svelte";
  import type { Channel } from "./streams";
  import type { Group } from "./groups";
  import type { DeviceOption } from "./bridge";

  interface Props {
    devices: DeviceOption[];
    device: string;
    channels: Channel[];
    groups: Group[];
    vocabulary: string;
    deviceChannels: number;
    locked: boolean;
  }

  let {
    devices,
    device = $bindable(),
    channels = $bindable(),
    groups = $bindable(),
    vocabulary = $bindable(),
    deviceChannels,
    locked,
  }: Props = $props();
</script>

<div class="page">
  <section class="device">
    <header>
      <h2>Input</h2>
    </header>
    <label>
      <span>Device</span>
      <select bind:value={device} disabled={locked}>
        {#each devices as option (option.name)}
          <option value={option.name}>{option.name} ({option.channels} ch)</option>
        {/each}
      </select>
    </label>
    <p class="hint">
      One multichannel interface for every stream. Two devices would mean two clocks,
      and no common timebase to order one chat by.
    </p>
  </section>

  <StreamSetup bind:channels {deviceChannels} {locked} />
  <VocabularyEditor bind:vocabulary {locked} />
  <GroupMembership bind:groups {channels} />
</div>

<style>
  .page {
    flex: 1;
    overflow-y: auto;
  }

  .device {
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    padding: 0.5rem 1rem 0.75rem;
  }

  header {
    margin-bottom: 0.4rem;
  }

  h2 {
    margin: 0;
    font-size: 0.85rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
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

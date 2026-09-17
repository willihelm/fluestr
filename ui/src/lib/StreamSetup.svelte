<script lang="ts">
  // The channel setup: one row per intercom channel, with the device channel
  // it is patched to, its label, its language and its colour. Vocabulary is
  // not here — it is one global list for every channel, edited beside this on
  // the Settings page (P15).
  //
  // Lives on the Settings page rather than above the chat, so the live view is
  // only the live view. P12 still owes making it a saved preset — an engineer
  // does not want to remap eight channels before every show. Editing is locked
  // while a session runs, because the channel map is fixed when capture
  // opens.

  import { newChannel, duplicateChannels, mutedChannels, type Channel } from "./streams";

  interface Props {
    channels: Channel[];
    deviceChannels: number;
    locked: boolean;
  }

  let { channels = $bindable(), deviceChannels, locked }: Props = $props();

  let duplicates = $derived(duplicateChannels(channels));
  // Not an error either: a row the current device has no channel for starts
  // muted, so one setup serves the desk and the laptop interface alike.
  let muted = $derived(new Set(mutedChannels(channels, deviceChannels)));

  const add = () => {
    channels = [...channels, newChannel(channels)];
  };

  const remove = (key: number) => {
    channels = channels.filter((c) => c.key !== key);
  };
</script>

<section>
  <header>
    <h2>Channels</h2>
    <span class="count">{channels.length} of {deviceChannels} on the device</span>
    <!-- Not capped at the device's channel count: the setup describes the
         production, not whatever interface happens to be plugged in now. -->
    <button class="add" onclick={add} disabled={locked}>Add channel</button>
  </header>

  <div class="rows">
    {#each channels as channel, index (channel.key)}
      <div class="row" class:muted={muted.has(index)} style="--channel: {channel.color}">
        <span class="dot" title="Stream {index}"></span>

        <label class="channel">
          <span>Ch</span>
          <!-- Counted from 1, the way channels are numbered on an interface
               and in a patch list. The backend stays zero-based; the
               conversion happens here and nowhere else. -->
          <input
            type="number"
            min="1"
            value={channel.channel + 1}
            oninput={(e) => (channel.channel = Number(e.currentTarget.value) - 1)}
            disabled={locked}
          />
        </label>

        <label class="label">
          <span>Label</span>
          <input type="text" bind:value={channel.label} disabled={locked} />
        </label>

        <label class="language">
          <span>Lang</span>
          <input
            type="text"
            size="3"
            bind:value={channel.language}
            disabled={locked}
            title="ISO 639-1 code. The list comes from the sidecar in a later step — nothing is hard-coded to de/en."
          />
        </label>

        <label class="colour">
          <span>Colour</span>
          <input type="color" bind:value={channel.color} disabled={locked} />
        </label>

        {#if muted.has(index)}
          <span class="tag" title="The selected device has only {deviceChannels} channels, so this one receives no audio. It keeps its place, its colour and its groups.">muted</span>
        {/if}

        <button class="remove" onclick={() => remove(channel.key)} disabled={locked}
          title="Remove this channel">×</button>
      </div>
    {/each}

    {#if channels.length === 0}
      <p class="empty">No channels yet. Add one to start transcribing.</p>
    {/if}
  </div>

  {#if muted.size > 0}
    <!-- Said once, in full, rather than only implied by the badges: an
         operator who sees a channel stay silent for a whole show has to be
         able to find out here why, and the reason is the device, not the
         setup. -->
    <p class="warn">
      {muted.size} of {channels.length} channels are above the {deviceChannels} the selected
      device has; they start muted and receive no audio.
    </p>
  {/if}

  {#if duplicates.length > 0}
    <!-- Allowed, not forbidden: two views of one channel is a legitimate if
         unusual setup. But during setup it is nearly always a slip, and the
         symptom — every line appearing twice — does not point at its cause. -->
    <p class="warn">
      Channel {duplicates.map((c) => c + 1).join(", ")} is used by more than one stream;
      it will be transcribed once per stream.
    </p>
  {/if}
</section>

<style>
  section {
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    padding: 0.5rem 1rem 0.75rem;
  }

  header {
    display: flex;
    align-items: baseline;
    gap: 0.75rem;
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

  .count {
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .add {
    margin-left: auto;
  }

  .rows {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .row {
    display: flex;
    align-items: flex-end;
    gap: 0.5rem;
  }

  .dot {
    width: 0.6rem;
    height: 0.6rem;
    border-radius: 50%;
    background: var(--channel);
    margin-bottom: 0.45rem;
    flex: none;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: 0.7rem;
    color: var(--text-dim);
  }

  .label {
    flex: 1;
  }

  /* Dimmed, not hidden: a muted row is still part of the setup, and it has to
     be editable — pointing it at a channel the device has is the fix. */
  .row.muted {
    opacity: 0.55;
  }

  .tag {
    align-self: center;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: #f2b544;
    border: 1px solid currentColor;
    border-radius: 0.25rem;
    padding: 0 0.3rem;
  }

  .empty {
    color: var(--text-dim);
    font-size: 0.85rem;
    margin: 0.25rem 0;
  }

  .warn {
    color: #f2b544;
    font-size: 0.8rem;
    margin: 0.5rem 0 0;
  }

  input {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.25rem 0.4rem;
    font: inherit;
    font-size: 0.85rem;
  }

  input[type="number"] {
    width: 3.5rem;
  }

  input[type="color"] {
    width: 2.2rem;
    height: 1.8rem;
    padding: 0.1rem;
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

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .remove {
    background: transparent;
    color: var(--text-dim);
    font-size: 1.1rem;
    line-height: 1;
    padding: 0.25rem 0.5rem;
  }

  .remove:hover:not(:disabled) {
    color: #ff6b6b;
  }
</style>

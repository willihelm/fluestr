<script lang="ts">
  // The channel setup: one row per intercom channel, with the device channel
  // it is patched to, its label, its language and its colour. Vocabulary is
  // not here — it is one global list for every channel, edited beside this on
  // the Settings page (P15).
  //
  // Lives on the Settings page rather than above the chat, so the live view is
  // only the live view. It belongs to the profile selected above it and is
  // stored with it (P12) — an engineer does not remap eight channels before
  // every show. Editing is locked while a session runs, because the channel
  // map is fixed when capture opens.

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

<section class="panel">
  <header>
    <h2>Channels</h2>
    <span class="count">{channels.length} of {deviceChannels} on the device</span>
    <!-- Not capped at the device's channel count: the setup describes the
         production, not whatever interface happens to be plugged in now. -->
    <button class="action" onclick={add} disabled={locked}>Add channel</button>
  </header>

  <div class="rows">
    <!-- The field names once, over the columns, rather than over every row.
         Eight channels meant eight copies of "Ch  Label  Lang  Colour", which
         is four times as much text as the setup itself and hid the one thing
         the section is scanned for: which label sits on which channel. The
         inputs keep the names as `aria-label`, so nothing is lost to a screen
         reader. -->
    {#if channels.length > 0}
      <div class="head" aria-hidden="true">
        <span></span>
        <span>Ch</span>
        <span>Label</span>
        <span>Lang</span>
        <span>Colour</span>
        <span></span>
      </div>
    {/if}

    {#each channels as channel, index (channel.key)}
      <div class="row" class:muted={muted.has(index)} style="--channel: {channel.color}">
        <span class="dot" title="Stream {index}"></span>

        <!-- Counted from 1, the way channels are numbered on an interface and
             in a patch list. The backend stays zero-based; the conversion
             happens here and nowhere else. -->
        <input
          type="number"
          min="1"
          aria-label="Device channel"
          value={channel.channel + 1}
          oninput={(e) => (channel.channel = Number(e.currentTarget.value) - 1)}
          disabled={locked}
        />

        <input type="text" aria-label="Label" bind:value={channel.label} disabled={locked} />

        <input
          type="text"
          aria-label="Language"
          bind:value={channel.language}
          disabled={locked}
          title="ISO 639-1 code. The list comes from the sidecar in a later step — nothing is hard-coded to de/en."
        />

        <input type="color" aria-label="Colour" bind:value={channel.color} disabled={locked} />

        <div class="end">
          <button class="remove" onclick={() => remove(channel.key)} disabled={locked}
            title="Remove this channel">×</button>
        </div>
      </div>
    {/each}

    {#if channels.length === 0}
      <p class="empty">No channels yet. Add one to start transcribing.</p>
    {/if}
  </div>

  {#if muted.size > 0}
    <!-- Said once, in full, rather than only implied by the greyed rows: an
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
  .rows {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  /* One template for the heading and every row, so the columns line up and
     stay lined up — the whole point of naming the fields once. */
  .head,
  .row {
    display: grid;
    grid-template-columns: 0.6rem 4rem minmax(6rem, 1fr) 3.5rem 2.6rem 2rem;
    align-items: center;
    gap: 0.5rem;
  }

  .head {
    font-size: 0.7rem;
    color: var(--text-dim);
  }

  .dot {
    width: 0.6rem;
    height: 0.6rem;
    border-radius: 50%;
    background: var(--channel);
  }

  .end {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    justify-content: flex-end;
    min-width: 0;
  }

  /* Dimmed, not hidden or badged: a muted row is still part of the setup,
     and it has to be editable — pointing it at a channel the device has is
     the fix. */
  .row.muted {
    opacity: 0.55;
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

  input {
    min-width: 0;
  }

  input[type="color"] {
    width: 100%;
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

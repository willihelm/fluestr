<script lang="ts">
  // The channel setup: one row per intercom channel, with the device channel
  // it is patched to, its label, its language, its colour and its vocabulary
  // prompt.
  //
  // Still in the chat view. P7's target interaction model puts this on a
  // dedicated Settings page and leaves the chat to the chat, and P12 makes it
  // a saved preset — an engineer does not want to remap eight channels before
  // every show. Editing is locked while a session runs, because the channel
  // map is fixed when capture opens.

  import { newChannel, duplicateChannels, type Channel } from "./streams";

  interface Props {
    channels: Channel[];
    deviceChannels: number;
    locked: boolean;
  }

  let { channels = $bindable(), deviceChannels, locked }: Props = $props();

  let duplicates = $derived(duplicateChannels(channels));

  const add = () => {
    channels = [...channels, newChannel(channels, deviceChannels)];
  };

  const remove = (key: number) => {
    channels = channels.filter((c) => c.key !== key);
  };
</script>

<section>
  <header>
    <h2>Channels</h2>
    <span class="count">{channels.length} of {deviceChannels} on the device</span>
    <button class="add" onclick={add} disabled={locked || channels.length >= deviceChannels}>
      Add channel
    </button>
  </header>

  <div class="rows">
    {#each channels as channel, index (channel.key)}
      <div class="row" style="--channel: {channel.color}">
        <span class="dot" title="Stream {index}"></span>

        <label class="channel">
          <span>Ch</span>
          <!-- Counted from 1, the way channels are numbered on an interface
               and in a patch list. The backend stays zero-based; the
               conversion happens here and nowhere else. -->
          <input
            type="number"
            min="1"
            max={deviceChannels}
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

        <label class="prompt">
          <span>Vocabulary</span>
          <input type="text" bind:value={channel.prompt} disabled={locked} />
        </label>

        <button class="remove" onclick={() => remove(channel.key)} disabled={locked}
          title="Remove this channel">×</button>
      </div>
    {/each}

    {#if channels.length === 0}
      <p class="empty">No channels yet. Add one to start transcribing.</p>
    {/if}
  </div>

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

  .prompt {
    flex: 1;
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

<script lang="ts">
  import type { TranscriptEvent } from "./bridge";

  interface Props {
    messages: TranscriptEvent[];
    /// Channel colour per stream id. A line belongs to a channel, and with
    /// eight of them interleaved the colour is what makes that readable at a
    /// glance — faster than reading eight labels.
    colors: Map<number, string>;
  }

  let { messages, colors }: Props = $props();

  const colorOf = (streamId: number) => colors.get(streamId) ?? "var(--accent)";

  let container: HTMLDivElement | undefined = $state();
  // Follow behaviour: stay pinned to the bottom, but not while the user is
  // reading further up. Still to come in P7: a proper "N new messages" button
  // and a virtualised list underneath — an hour across eight streams is tens
  // of thousands of nodes.
  let following = $state(true);

  const onScroll = () => {
    if (!container) return;
    const distance =
      container.scrollHeight - container.scrollTop - container.clientHeight;
    following = distance < 80;
  };

  $effect(() => {
    // Touch the length so the effect re-runs on every new message.
    messages.length;
    if (following && container) {
      container.scrollTop = container.scrollHeight;
    }
  });

  const clock = (ms: number) => {
    const total = Math.floor(ms / 1000);
    const minutes = Math.floor(total / 60);
    const seconds = total % 60;
    return `${minutes}:${String(seconds).padStart(2, "0")}`;
  };
</script>

<div class="chat" bind:this={container} onscroll={onScroll}>
  {#if messages.length === 0}
    <p class="empty">Nothing transcribed yet. Speak into one of the channels.</p>
  {/if}

  <!-- Keyed on all three, not on `streamId + seq`: those repeat across
       sessions, and a duplicate key throws and stops the list rendering
       entirely — which is what "nothing appears after Stop and Start" looked
       like. -->
  {#each messages as message (message.sessionId + "-" + message.streamId + "-" + message.seq)}
    <article style="--channel: {colorOf(message.streamId)}">
      <header>
        <span class="label">{message.label || "Stream " + message.streamId}</span>
        <span class="time">{clock(message.tStartMs)}</span>
        {#if message.detectedLanguage}
          <span class="lang">{message.detectedLanguage}</span>
        {/if}
        <span class="latency" title="Decode {message.decodeMs} ms of {message.latencyMs} ms total">
          {(message.latencyMs / 1000).toFixed(1)} s
        </span>
      </header>
      <p>{message.text}</p>
    </article>
  {/each}
</div>

{#if !following}
  <button
    class="jump"
    onclick={() => {
      following = true;
      if (container) container.scrollTop = container.scrollHeight;
    }}
  >
    Jump to latest
  </button>
{/if}

<style>
  .chat {
    flex: 1;
    overflow-y: auto;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .empty {
    color: var(--text-dim);
    margin: auto;
  }

  article {
    background: var(--surface);
    border: 1px solid var(--border);
    /* The channel's colour down the left edge: with eight channels
       interleaved this is what groups the lines, and it survives being read
       out of the corner of an eye while doing something else. */
    border-left: 3px solid var(--channel);
    border-radius: 0.5rem;
    padding: 0.5rem 0.75rem;
    max-width: 48rem;
  }

  header {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    font-size: 0.8rem;
    color: var(--text-dim);
    margin-bottom: 0.15rem;
  }

  .label {
    color: var(--channel);
    font-weight: 600;
  }

  .lang,
  .latency {
    font-variant-numeric: tabular-nums;
  }

  .latency {
    margin-left: auto;
  }

  article p {
    margin: 0;
  }

  .jump {
    position: absolute;
    bottom: 1.5rem;
    left: 50%;
    transform: translateX(-50%);
    background: var(--accent);
    color: #12141a;
    border: none;
    border-radius: 1rem;
    padding: 0.35rem 0.9rem;
    cursor: pointer;
    font-weight: 600;
  }
</style>

<script lang="ts">
  import type { TranscriptEvent } from "./bridge";
  import { toBubbles, type Bubble } from "./bubbles";

  interface Props {
    messages: TranscriptEvent[];
    /// Channel colour per stream id. A line belongs to a channel, and with
    /// eight of them interleaved the colour is what makes that readable at a
    /// glance — faster than reading eight labels.
    colors: Map<number, string>;
  }

  let { messages, colors }: Props = $props();

  // Consecutive segments of the same channel are one bubble; see
  // [`toBubbles`] for where a bubble ends.
  let bubbles = $derived(toBubbles(messages));

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
    // Touch the message count, not the bubble count: a segment appended to
    // the last bubble leaves the number of bubbles unchanged while the box
    // still grows, and the view has to follow that too.
    messages.length;
    if (following && container) {
      container.scrollTop = container.scrollHeight;
    }
  });

  const latencyTitle = (bubble: Bubble) =>
    bubble.parts === 1
      ? `Decode ${bubble.decodeMs} ms of ${bubble.latencyMs} ms total`
      : `Decode ${bubble.decodeMs} ms of ${bubble.latencyMs} ms total, last of ${bubble.parts} segments`;

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

  <!-- Keyed on the identity of the segment that opened the bubble. On all
       three parts, not on `streamId + seq`: those repeat across sessions, and
       a duplicate key throws and stops the list rendering entirely — which is
       what "nothing appears after Stop and Start" looked like. -->
  {#each bubbles as bubble (bubble.key)}
    <article style="--channel: {colorOf(bubble.streamId)}">
      <header>
        <span class="label">{bubble.label || "Stream " + bubble.streamId}</span>
        <span class="time">{clock(bubble.tStartMs)}</span>
        {#each bubble.languages as language}
          <span class="lang">{language}</span>
        {/each}
        <span class="latency" title={latencyTitle(bubble)}>
          {(bubble.latencyMs / 1000).toFixed(1)} s
        </span>
      </header>
      <p>{bubble.text}</p>
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

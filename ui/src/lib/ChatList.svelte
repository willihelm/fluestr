<script lang="ts">
  import type { TranscriptEvent } from "./bridge";

  interface Props {
    messages: TranscriptEvent[];
  }

  let { messages }: Props = $props();

  let container: HTMLDivElement | undefined = $state();
  // Follow behaviour: stay pinned to the bottom, but not while the user is
  // reading further up. P7 turns this into a proper "N new messages" button
  // and puts a virtualised list underneath — an hour across eight streams is
  // tens of thousands of nodes.
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
    <p class="empty">Nothing transcribed yet. Speak into the selected channel.</p>
  {/if}

  {#each messages as message (message.streamId + "-" + message.seq)}
    <article>
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
    color: var(--accent);
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

<script lang="ts">
  // Clear all communication: the one action that empties the chat.
  //
  // Start and Stop deliberately never clear anything, so this is the only way
  // transcripts go away — which is why it is explicit, confirmed, and offered
  // only while capture is stopped. Clearing under a running session would race
  // the transcripts still arriving, and the operator would be left unsure
  // whether what reappeared was old or new.
  //
  // It clears transcripts across every group and session. It changes no
  // configuration: channels, colours, groups and vocabulary all survive it.
  //
  // The confirmation is a panel and not `window.confirm`, because a modal the
  // WebView owns blocks the event loop the live meters are drawn from.

  interface Props {
    /// Disabled while a session runs, with the reason shown.
    running: boolean;
    empty: boolean;
    clear: () => void;
  }

  let { running, empty, clear }: Props = $props();

  let asking = $state(false);

  // A running session or an emptied chat both make the open panel stale; it
  // must not stay open offering an action that no longer applies.
  $effect(() => {
    if (running || empty) asking = false;
  });

  const confirm = () => {
    asking = false;
    clear();
  };
</script>

{#if asking}
  <div class="ask" role="alertdialog" aria-label="Clear all communication">
    <span>
      Delete every transcript, in every group and from every session? The channel,
      group and vocabulary setup is kept. This cannot be undone.
    </span>
    <button class="danger" onclick={confirm}>Delete everything</button>
    <button class="quiet" onclick={() => (asking = false)}>Cancel</button>
  </div>
{:else}
  <button
    class="quiet"
    onclick={() => (asking = true)}
    disabled={running || empty}
    title={running
      ? "Stop the session first — clearing while transcripts arrive would leave it unclear what was cleared."
      : "Delete every transcript. Configuration is kept."}
  >
    Clear all communication
  </button>
{/if}

<style>
  .ask {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.8rem;
    color: var(--text);
  }

  button {
    border: none;
    border-radius: 0.3rem;
    padding: 0.3rem 0.7rem;
    font: inherit;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
  }

  .quiet {
    background: transparent;
    color: var(--text-dim);
    border: 1px solid var(--border);
  }

  .quiet:hover:not(:disabled) {
    color: var(--text);
  }

  .quiet:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .danger {
    background: #ff6b6b;
    color: #12141a;
  }
</style>

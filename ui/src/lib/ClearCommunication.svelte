<script lang="ts">
  // Clear all communication: the one action that empties the chat.
  //
  // Start and Stop deliberately never clear anything, so this is the only way
  // transcripts go away — which is why it is explicit and confirmed. It is
  // offered at all times, a running session included: a chat that has filled
  // up mid-show is exactly when it is needed, and the lines that arrive after
  // the press are the ones the operator wants left.
  //
  // It clears transcripts across every group and session. It changes no
  // configuration: channels, colours, groups and vocabulary all survive it.
  //
  // The confirmation is a panel and not `window.confirm`, because a modal the
  // WebView owns blocks the event loop the live meters are drawn from.

  interface Props {
    /// Disabled only when there is nothing to delete.
    empty: boolean;
    clear: () => void;
  }

  let { empty, clear }: Props = $props();

  let asking = $state(false);

  // An emptied chat makes the open panel stale; it must not stay open
  // offering an action that no longer applies.
  $effect(() => {
    if (empty) asking = false;
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
    disabled={empty}
    title="Delete every transcript. Configuration is kept."
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
    /* The question is two lines long on a phone, and the two answers must not
       be pushed off the end of the bar by it: they wrap under it instead.
       Asked in the status bar rather than in a sheet over the chat, which is
       what makes the wrap the whole of what this needs. */
    flex-wrap: wrap;
    justify-content: flex-end;
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

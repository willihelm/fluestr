<script lang="ts">
  // Clear all: the one action that empties the chat.
  //
  // A bin and no words, in the header beside the meters: the status bar it
  // used to share with the session line is gone, and the header has room for
  // a glyph where it has none for a label. The words come back when it is
  // armed, because a question has to be read.
  //
  // Start and Stop deliberately never clear anything, so this is the only way
  // transcripts go away — which is why it is confirmed. It is offered at all
  // times, a running session included: a chat that has filled up mid-show is
  // exactly when it is needed, and the lines that arrive after the press are
  // the ones the operator wants left.
  //
  // It clears transcripts across every group and session. It changes no
  // configuration: channels, colours, groups and vocabulary all survive it.
  //
  // **The confirmation is the button.** One press arms it — it turns red and
  // asks — and a second within five seconds does it. Not `window.confirm`,
  // because a modal the WebView owns blocks the event loop the live meters
  // are drawn from; and no longer a panel beside it, because the panel
  // wrapped the bar it sat in onto a second row on a phone and moved everything
  // under it at the moment the operator was reading a question. Two presses
  // in the same place is the same protection in none of the space.
  //
  // It disarms itself rather than waiting: an armed button left over from a
  // press five minutes ago would fire on what its owner thinks is the first
  // click of two.

  interface Props {
    /// Disabled only when there is nothing to delete.
    empty: boolean;
    clear: () => void;
  }

  let { empty, clear }: Props = $props();

  /// How long the armed button waits for the second press.
  ///
  /// Long enough to read the word and move a finger, short enough that the
  /// operator who walked away is back to a button that cannot delete
  /// anything on one press.
  const ARM_MS = 5_000;

  let armed = $state(false);

  $effect(() => {
    if (!armed) return;
    const timer = setTimeout(() => (armed = false), ARM_MS);
    return () => clearTimeout(timer);
  });

  // An emptied chat makes an armed button stale: it must not stay armed
  // offering an action that no longer applies.
  $effect(() => {
    if (empty) armed = false;
  });

  const press = () => {
    if (!armed) {
      armed = true;
      return;
    }
    armed = false;
    clear();
  };
</script>

<button
  class="quiet"
  class:armed
  onclick={press}
  disabled={empty}
  title={armed
    ? "Press again to delete every transcript. Configuration is kept."
    : "Delete every transcript. Configuration is kept."}
  aria-label={armed
    ? "Confirm: delete every transcript, in every group and from every session"
    : "Clear all communication"}
>
  {#if armed}
    Sure?
  {:else}
    <!-- Drawn, for the reason the burger and the cog are: it takes its
         weight and colour from the row it is in at every font size. -->
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
      <g fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">
        <path d="M2.5 4h11M6 4V2.6h4V4M3.8 4l.7 9.4h7l.7-9.4M6.6 6.6v4.4M9.4 6.6v4.4" />
      </g>
    </svg>
  {/if}
</button>

<style>
  button {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 1.75rem;
    min-width: 1.75rem;
    border: none;
    border-radius: 0.3rem;
    padding: 0 0.4rem;
    font: inherit;
    font-size: 0.8rem;
    font-weight: 600;
    line-height: 1;
    cursor: pointer;
    white-space: nowrap;
  }

  /* Armed it grows leftwards only: its right edge is held by what sits
     after it, so the second press lands where the first did. */
  .quiet {
    background: transparent;
    color: var(--text-dim);
    border: 1px solid transparent;
  }

  .quiet:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--border);
    background: var(--bg);
  }

  .quiet:disabled {
    opacity: 0.5;
    cursor: default;
  }

  /* Armed: the same red the app's other destructive controls use, so what the
     second press does is read off the colour before the word. */
  .armed,
  .armed:hover:not(:disabled) {
    background: #ff6b6b;
    color: #12141a;
    border-color: #ff6b6b;
  }
</style>

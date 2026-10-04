<script lang="ts">
  // Clear all: the one action that empties the chat.
  //
  // Labelled with the two words and not with the whole sentence it is named
  // after. It sits in a bar that already has to fit the session line beside
  // it, and "all" has exactly one meaning in a window that only holds
  // transcripts.
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
  // wrapped the status bar onto a second row on a phone and moved everything
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
  {armed ? "Sure?" : "Clear all"}
</button>

<style>
  button {
    border: none;
    border-radius: 0.3rem;
    padding: 0.3rem 0.7rem;
    font: inherit;
    font-size: 0.8rem;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    /* The armed state changes the width of the label as well as its colour.
       Held at the wider of the two so the bar beside it does not shift under
       the second press — which would be a press landing on something that
       moved. */
    min-width: 5.5rem;
    text-align: center;
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

  /* Armed: the same red the app's other destructive controls use, so what the
     second press does is read off the colour before the word. */
  .armed,
  .armed:hover:not(:disabled) {
    background: var(--danger);
    color: var(--on-accent);
    border-color: var(--danger);
  }
</style>

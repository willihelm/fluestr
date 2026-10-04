<script lang="ts">
  // The solo of one channel, as the S beside the M on every desk: the input
  // is heard on this machine's listen output — the headphones replays go to.
  //
  // Latching, like the talk keys of an intercom panel: pressing another S
  // adds that channel, and each stays in until it is pressed again.
  //
  // Lit yellow, the colour solo is on a desk, so it is not mistaken for the
  // red M next to it.

  import { solo, toggleSolo } from "./solo.svelte";

  interface Props {
    streamId: number;
    /// What is being soloed, for the screen reader.
    label: string;
    disabled?: boolean;
  }

  let { streamId, label, disabled = false }: Props = $props();

  let on = $derived(solo.streams.has(streamId));
  let failed = $derived(solo.error?.streamId === streamId ? solo.error.message : null);
</script>

<button
  class="solo"
  class:on
  class:failed={failed !== null}
  type="button"
  {disabled}
  aria-label="Solo {label}"
  aria-pressed={on}
  title={failed !== null
    ? `${label} could not be soloed: ${failed}`
    : on
      ? `${label} is on the headphones — press again to take it off`
      : `Solo ${label} — hear it on the headphones, beside any other soloed channel`}
  onclick={() => toggleSolo(streamId)}
>
  S
</button>

<style>
  /* The M's shape, so the two read as a pair. */
  .solo {
    width: 1.5rem;
    height: 1.5rem;
    flex: none;
    padding: 0;
    display: grid;
    place-items: center;
    font-size: 0.7rem;
    font-weight: 700;
    line-height: 1;
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }

  .solo:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--text-dim);
  }

  .solo:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .solo.on {
    background: #f1c40f;
    border-color: #f1c40f;
    color: #1a1a1a;
  }

  /* The press that did not take: the reason is in the tooltip. */
  .solo.failed {
    border-color: #c0392b;
    color: #ff6b6b;
  }

  .solo:disabled {
    opacity: 0.45;
    cursor: default;
  }

  /* As the M: square, with a fingertip's target around it. */
  @media (pointer: coarse) {
    .solo {
      min-height: 0;
      width: 1.7rem;
      height: 1.7rem;
      position: relative;
    }

    .solo::after {
      content: "";
      position: absolute;
      inset: calc((1.7rem - var(--touch)) / 2);
    }
  }
</style>

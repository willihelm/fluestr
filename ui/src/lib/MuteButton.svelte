<script lang="ts">
  // The mute of one channel, as the M every desk has.
  //
  // One letter and not the word, because of where it sits: in a meter row
  // that already carries a label, a knob and a bar, and on a phone in a
  // column 2.8 rem wide. M is what the button is called on every intercom
  // panel and every mixing desk in the building, so it is the one label that
  // needs no reading.
  //
  // Lit red when it is doing something. A mute that looks like the rest of
  // the strip is a mute that is discovered at the end of a show, by somebody
  // asking why that channel never said anything.

  interface Props {
    /// Whether the channel is muted. Read, never written back: the owner of
    /// the setup applies the change, because it is also the one that stores it
    /// and pushes it into the running session.
    muted: boolean;
    /// What is being muted, for the screen reader. The visible name is beside
    /// it and would be read twice otherwise.
    label: string;
    disabled?: boolean;
    onchange: (muted: boolean) => void;
  }

  let { muted, label, disabled = false, onchange }: Props = $props();
</script>

<!-- `aria-pressed` rather than a checkbox: it is a button that stays in, which
     is what a toggle button is, and it keeps the name "Mute" instead of
     announcing a tick box called M. -->
<button
  class="mute"
  class:on={muted}
  type="button"
  {disabled}
  aria-label="Mute {label}"
  aria-pressed={muted}
  title={muted
    ? `${label} is muted — it is still metered, and nothing said on it is transcribed`
    : `Mute ${label} — the meter keeps moving, the transcription stops`}
  onclick={() => onchange(!muted)}
>
  M
</button>

<style>
  .mute {
    /* Square and small: it sits between a name and a bar, and a button as
       wide as its row's label would be the thing the eye lands on first in a
       strip that is read for its bars. */
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

  .mute:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--text-dim);
  }

  .mute:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  /* Red and filled, not merely tinted: this is the state somebody has to be
     able to spot in a strip of eight while a show is running. */
  .mute.on {
    background: #c0392b;
    border-color: #c0392b;
    color: #fff;
  }

  .mute:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>

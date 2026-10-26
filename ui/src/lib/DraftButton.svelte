<script lang="ts">
  // Whether one channel gets a live draft (P28), as a D beside its M.
  //
  // Lit when the channel is *excluded*, like the M is lit when it is muted:
  // drafting is what every channel does, and the key that stands out is the
  // one where somebody decided otherwise. Struck through so the lit state
  // reads as "no drafts" and not as "drafts on".
  //
  // For a channel whose drafts are not worth the GPU — every draft decodes
  // everything said so far again, so a position that talks at length keeps
  // the workers busy for as long as it talks. Its lines still arrive.

  interface Props {
    /// Whether the channel is drafted. Read, never written back, for the M's
    /// reason: the owner of the setup stores it and pushes it into the
    /// running session.
    preview: boolean;
    /// Which channel, for the screen reader.
    label: string;
    disabled?: boolean;
    onchange: (preview: boolean) => void;
  }

  let { preview, label, disabled = false, onchange }: Props = $props();
</script>

<button
  class="draft"
  class:off={!preview}
  type="button"
  {disabled}
  aria-label="No drafts for {label}"
  aria-pressed={!preview}
  title={preview
    ? `Stop drafting ${label} — its lines still arrive, without a draft ahead of them`
    : `${label} gets no drafts — its lines still arrive`}
  onclick={() => onchange(!preview)}
>
  D
</button>

<style>
  /* The M's geometry, so the keys line up; see `MuteButton`. */
  .draft {
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

  .draft:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--text-dim);
  }

  .draft:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .draft.off {
    background: var(--text-dim);
    border-color: var(--text-dim);
    color: var(--bg, #111);
    text-decoration: line-through;
  }

  .draft:disabled {
    opacity: 0.45;
    cursor: default;
  }

  @media (pointer: coarse) {
    .draft {
      min-height: 0;
      width: 1.7rem;
      height: 1.7rem;
      position: relative;
    }

    .draft::after {
      content: "";
      position: absolute;
      inset: calc((1.7rem - var(--touch)) / 2);
    }
  }
</style>

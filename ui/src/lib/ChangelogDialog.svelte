<script lang="ts">
  // What a newer release brings, and the button that installs it.
  //
  // A dialog in the middle of the window rather than a popover off the
  // header: the notes are a list worth reading, and the decision it asks for
  // restarts the app. Opened by itself once per release (see `UpdateButton`),
  // and from the header's button whenever it is asked for again.

  import { installUpdate, onUpdateProgress, type AvailableUpdate } from "./bridge";
  import { parseNotes } from "./changelog";

  interface Props {
    update: AvailableUpdate;
    /// Whether a session is running, so the dialog can say it will stop.
    running: boolean;
    onclose: () => void;
  }

  let { update, running, onclose }: Props = $props();

  const blocks = $derived(parseNotes(update.notes));

  let installing = $state(false);
  let error = $state<string | null>(null);
  /// 0–1 while the download's length is known; `null` before the first
  /// chunk and when the server does not say.
  let progress = $state<number | null>(null);

  const close = () => {
    if (!installing) onclose();
  };

  const install = async () => {
    installing = true;
    error = null;
    progress = null;
    const unlisten = await onUpdateProgress(({ received, total }) => {
      progress = total ? Math.min(1, received / total) : null;
    });
    try {
      // Resolves only by failing: on success the app restarts underneath it.
      await installUpdate();
    } catch (e) {
      error = String(e);
    } finally {
      unlisten();
      installing = false;
    }
  };
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === "Escape") close();
  }}
/>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="scrim" onclick={close}></div>

<div class="dialog" role="dialog" aria-modal="true" aria-labelledby="changelog-title">
  <header>
    <span class="badge" aria-hidden="true">
      <svg viewBox="0 0 16 16" width="16" height="16">
        <path
          d="M8 2v8M4.5 6.5 8 10l3.5-3.5M3 13.5h10"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
    </span>
    <div>
      <span class="eyebrow">Update available</span>
      <h2 id="changelog-title">Fluestr {update.version}</h2>
      <span class="current">You have {update.current}</span>
    </div>
  </header>

  <div class="notes">
    {#if blocks.length === 0}
      <p class="empty">This release has no notes.</p>
    {:else}
      {#each blocks as block, i (i)}
        {#if block.kind === "heading"}
          <h3>{block.text}</h3>
        {:else if block.kind === "item"}
          <p class="item">{block.text}</p>
        {:else}
          <p>{block.text}</p>
        {/if}
      {/each}
    {/if}
  </div>

  <footer>
    {#if installing}
      <div class="progress">
        <div class="bar" aria-hidden="true">
          <span
            class:unknown={progress === null}
            style="width: {progress === null ? 100 : progress * 100}%"
          ></span>
        </div>
        <span class="dim">
          {progress === null
            ? "Downloading…"
            : `Downloading ${Math.round(progress * 100)} %`}
        </span>
      </div>
    {:else if error}
      <span class="error">The update failed: {error}</span>
    {:else}
      <span class="dim">
        {running
          ? "Installing stops the running session and restarts Fluestr."
          : "Installing restarts Fluestr."}
      </span>
    {/if}
    <div class="actions">
      <button type="button" class="quiet" onclick={close} disabled={installing}>Later</button>
      <button type="button" class="primary" onclick={install} disabled={installing}>
        {error ? "Try again" : "Install and restart"}
      </button>
    </div>
  </footer>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgba(0, 0, 0, 0.55);
  }

  .dialog {
    position: fixed;
    z-index: 41;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: column;
    width: min(32rem, calc(100vw - 2rem));
    max-height: min(36rem, calc(100dvh - 2rem));
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    box-shadow: 0 1.5rem 3rem rgba(0, 0, 0, 0.45);
  }

  header {
    display: flex;
    align-items: center;
    gap: 0.85rem;
    padding: 1.1rem 1.25rem 0.9rem;
    border-bottom: 1px solid var(--border);
  }

  .badge {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 2.4rem;
    height: 2.4rem;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-radius: 0.5rem;
  }

  .eyebrow {
    display: block;
    font-size: 0.7rem;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--accent);
  }

  h2 {
    margin: 0.1rem 0;
    font-size: 1.15rem;
    font-weight: 600;
  }

  .current {
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .notes {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.4rem 1.25rem 1rem;
    font-size: 0.875rem;
    line-height: 1.45;
  }

  h3 {
    margin: 0.9rem 0 0.35rem;
    font-size: 0.72rem;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }

  p {
    margin: 0.3rem 0;
  }

  /* A list drawn from paragraphs, so a stray item before the first heading
     needs no list around it. The bullet is the accent, the text the body's. */
  .item {
    position: relative;
    padding-left: 1rem;
  }

  .item::before {
    content: "";
    position: absolute;
    left: 0.15rem;
    top: 0.55em;
    width: 0.35rem;
    height: 0.35rem;
    border-radius: 50%;
    background: var(--accent);
  }

  .empty {
    margin-top: 0.9rem;
    color: var(--text-dim);
  }

  footer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.85rem 1.25rem;
    font-size: 0.8rem;
    border-top: 1px solid var(--border);
  }

  .dim {
    color: var(--text-dim);
  }

  .error {
    color: #ff6b6b;
  }

  .progress {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 0.3rem;
    min-width: 10rem;
  }

  .bar {
    height: 0.3rem;
    overflow: hidden;
    background: var(--bg);
    border-radius: 999px;
  }

  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.2s;
  }

  .bar span.unknown {
    animation: breathe 1.2s ease-in-out infinite;
  }

  @keyframes breathe {
    50% {
      opacity: 0.3;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .bar span.unknown {
      animation: none;
    }
  }

  .actions {
    display: flex;
    gap: 0.5rem;
    margin-left: auto;
  }

  .actions button {
    height: 2rem;
    padding: 0 0.8rem;
    font: inherit;
    font-size: 0.85rem;
    font-weight: 600;
    border-radius: 0.3rem;
    cursor: pointer;
    white-space: nowrap;
  }

  .quiet {
    color: var(--text-dim);
    background: transparent;
    border: 1px solid var(--border);
  }

  .quiet:hover:not(:disabled) {
    color: var(--text);
    background: var(--bg);
  }

  .primary {
    color: var(--bg);
    background: var(--accent);
    border: 1px solid var(--accent);
  }

  .primary:hover:not(:disabled) {
    filter: brightness(1.08);
  }

  .actions button:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>

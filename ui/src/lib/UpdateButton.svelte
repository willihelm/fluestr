<script lang="ts">
  // A newer release is out. Shown beside the app's name only when there is
  // one — the main window asks GitHub once as it opens — and silent otherwise:
  // an app that is up to date has nothing to say about it.
  //
  // Two presses, not one: the first opens what the release is and what
  // installing it will do, the second installs. Installing restarts the app,
  // and during a show that also ends the session, which is not something a
  // stray click in the header should do.

  import { installUpdate, onUpdateProgress, type AvailableUpdate } from "./bridge";

  interface Props {
    update: AvailableUpdate;
    /// Whether a session is running, so the popover can say it will stop.
    running: boolean;
  }

  let { update, running }: Props = $props();

  let open = $state(false);
  let installing = $state(false);
  let error = $state<string | null>(null);
  /// 0–1 while the download's length is known; `null` before the first
  /// chunk and when the server does not say.
  let progress = $state<number | null>(null);
  let root = $state<HTMLElement>();
  let button = $state<HTMLElement>();
  let popover = $state<HTMLElement>();
  let place = $state({ top: 0, left: 0 });

  const MARGIN = 8;

  // Placed like the status dot's popover, fixed to the window and clamped
  // into it — see `StatusDot.svelte`.
  const position = () => {
    if (!button || !popover) return;
    const at = button.getBoundingClientRect();
    const width = popover.offsetWidth;
    const left = Math.min(at.left - 4, window.innerWidth - width - MARGIN);
    place = { top: at.bottom + 6, left: Math.max(MARGIN, left) };
  };

  $effect(() => {
    if (!open || !popover) return;
    position();
    window.addEventListener("resize", position);
    return () => window.removeEventListener("resize", position);
  });

  // Closed by a press outside and by Escape — but not while installing, when
  // the popover is the only place the progress is shown.
  $effect(() => {
    if (!open || installing) return;
    const away = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node)) open = false;
    };
    const escape = (e: KeyboardEvent) => {
      if (e.key === "Escape") open = false;
    };
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("keydown", escape);
    };
  });

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

  const label = $derived(
    installing
      ? progress === null
        ? "Downloading…"
        : `Downloading ${Math.round(progress * 100)} %`
      : `Update to ${update.version}`,
  );
</script>

<span class="update" bind:this={root}>
  <button
    bind:this={button}
    type="button"
    class="chip"
    aria-expanded={open}
    title="Fluestr {update.version} is available"
    onclick={() => (open = !open)}
  >
    <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
      <path
        d="M8 2v8M4.5 6.5 8 10l3.5-3.5M3 13.5h10"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
    <span>{label}</span>
  </button>

  {#if open}
    <div
      class="popover"
      role="dialog"
      aria-label="Update"
      bind:this={popover}
      style="top: {place.top}px; left: {place.left}px"
    >
      <strong>Fluestr {update.version} is available</strong>
      {#if update.notes}
        <pre class="notes">{update.notes}</pre>
      {/if}
      <span class="dim">
        {running
          ? "Installing stops the running session and restarts the app."
          : "Installing restarts the app."}
      </span>
      {#if installing}
        <div class="bar" aria-hidden="true">
          <span
            class:unknown={progress === null}
            style="width: {progress === null ? 100 : progress * 100}%"
          ></span>
        </div>
      {/if}
      {#if error}
        <span class="error">The update failed: {error}</span>
      {/if}
      <div class="actions">
        <button type="button" onclick={() => (open = false)} disabled={installing}>
          Later
        </button>
        <button type="button" class="primary" onclick={install} disabled={installing}>
          {installing ? label : "Install and restart"}
        </button>
      </div>
    </div>
  {/if}
</span>

<style>
  .update {
    position: relative;
    display: inline-flex;
    flex: none;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.15rem 0.55rem;
    font-size: 0.75rem;
    white-space: nowrap;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
    border-radius: 999px;
    cursor: pointer;
  }

  .chip:hover,
  .chip[aria-expanded="true"] {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }

  .popover {
    position: fixed;
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    width: min(22rem, calc(100vw - 16px));
    padding: 0.6rem 0.75rem;
    font-size: 0.8rem;
    line-height: 1.35;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 0.4rem;
    box-shadow: 0 0.4rem 1.2rem rgb(0 0 0 / 0.35);
  }

  .popover strong {
    font-weight: 600;
  }

  /* GitHub's notes are Markdown; shown as written rather than rendered, which
     reads fine for a list of changes and needs no parser. */
  .notes {
    max-height: 12rem;
    margin: 0;
    overflow: auto;
    font: inherit;
    white-space: pre-wrap;
    color: var(--text-dim);
  }

  .dim {
    color: var(--text-dim);
  }

  .error {
    color: #ff6b6b;
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
    justify-content: flex-end;
    gap: 0.4rem;
  }

  .actions button {
    height: 1.75rem;
    padding: 0 0.6rem;
    font: inherit;
    font-size: 0.8rem;
    font-weight: 600;
    color: var(--text-dim);
    background: transparent;
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    cursor: pointer;
    white-space: nowrap;
  }

  .actions button:hover:not(:disabled) {
    color: var(--text);
    background: var(--bg);
  }

  .actions .primary,
  .actions .primary:hover:not(:disabled) {
    color: var(--bg);
    background: var(--accent);
    border-color: var(--accent);
  }

  .actions button:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>

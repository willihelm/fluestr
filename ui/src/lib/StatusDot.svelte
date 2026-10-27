<script lang="ts" module>
  export type Tone = "ok" | "busy" | "warn" | "error" | "idle";
</script>

<script lang="ts">
  // The session's state as one dot beside the app's name.
  //
  // It replaces the status bar that used to sit under the header: a whole row
  // spent, during a show, on a sentence that says "running" for three hours.
  // The colour is what is glanced at; the sentence is what is asked for, and
  // it is behind the dot — as its tooltip under a mouse, and as a small
  // popover on a press, because a phone on the remote page has no hover.
  //
  // What goes wrong is not left to the dot: a failed start or a lost
  // connection is also said in the notice layer over the chat (see
  // `Notices.svelte`). The dot is red as well so the two agree.

  interface Props {
    tone: Tone;
    /// One or two words, for the popover's heading and the screen reader.
    label: string;
    /// The whole sentence: the tooltip and the popover's body.
    detail: string;
  }

  let { tone, label, detail }: Props = $props();

  let open = $state(false);
  let root = $state<HTMLElement>();
  let button = $state<HTMLElement>();
  let popover = $state<HTMLElement>();

  /// Where the popover goes, in viewport pixels. Fixed to the window and
  /// clamped into it, rather than hung off the dot: on a phone the dot sits
  /// mid-header, and a popover starting there runs off the right edge.
  let place = $state({ top: 0, left: 0 });

  /// The gap kept between the popover and the window's edges.
  const MARGIN = 8;

  const position = () => {
    if (!button || !popover) return;
    const dot = button.getBoundingClientRect();
    const width = popover.offsetWidth;
    const left = Math.min(dot.left - 4, window.innerWidth - width - MARGIN);
    place = { top: dot.bottom + 6, left: Math.max(MARGIN, left) };
  };

  $effect(() => {
    if (!open || !popover) return;
    position();
    window.addEventListener("resize", position);
    return () => window.removeEventListener("resize", position);
  });

  // Closed by any press outside it and by Escape, the way every other popover
  // in a desktop app behaves — a popover that waits for its own button to be
  // pressed again is one left standing over the chat.
  $effect(() => {
    if (!open) return;
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
</script>

<span class="status" bind:this={root}>
  <button
    bind:this={button}
    type="button"
    class="dot {tone}"
    title={open ? undefined : detail}
    aria-label="Status: {label}. {detail}"
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    <span class="ring" aria-hidden="true"><span class="pip"></span></span>
  </button>

  {#if open}
    <div
      class="popover"
      role="status"
      bind:this={popover}
      style="top: {place.top}px; left: {place.left}px"
    >
      <strong class={tone}>{label}</strong>
      <span>{detail}</span>
    </div>
  {/if}
</span>

<style>
  .status {
    position: relative;
    display: inline-flex;
    flex: none;
  }

  /* A target the size of a fingertip around a pip the size of a full stop:
     the pip is what is read, the padding is what is pressed. The button
     itself draws nothing — on a touch screen `app.css` makes every button
     44 px tall, and a button that drew its own circle became an oval. The
     circle is the ring inside it, which keeps its shape at any height. */
  .dot {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.6rem;
    min-width: 1.6rem;
    padding: 0;
    background: transparent;
    border: none;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }

  .ring {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 1.6rem;
    height: 1.6rem;
    border: 1px solid transparent;
    border-radius: 50%;
  }

  .dot:hover .ring,
  .dot[aria-expanded="true"] .ring {
    background: var(--bg);
    border-color: var(--border);
  }

  .pip {
    width: 0.6rem;
    height: 0.6rem;
    border-radius: 50%;
    background: currentColor;
  }

  .ok {
    color: #5fd38d;
  }

  .busy {
    color: #e8c36a;
  }

  .warn {
    color: #e8c36a;
  }

  .error {
    color: #ff6b6b;
  }

  .idle {
    color: var(--text-dim);
  }

  /* Running gets a halo, so "on" is told from "off" by more than a hue — the
     green and the grey are close for anybody who does not see green well. */
  .dot.ok .pip {
    box-shadow: 0 0 0 0.18rem color-mix(in srgb, currentColor 28%, transparent);
  }

  /* Loading the model breathes: the one state in which nothing else on the
     window moves, and the one in which the operator wonders whether it hung. */
  .dot.busy .pip {
    animation: breathe 1.2s ease-in-out infinite;
  }

  @keyframes breathe {
    50% {
      opacity: 0.3;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .dot.busy .pip {
      animation: none;
    }
  }

  .popover {
    /* Placed by `position` in the script. */
    position: fixed;
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    width: max-content;
    max-width: min(24rem, calc(100vw - 16px));
    padding: 0.5rem 0.7rem;
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
</style>

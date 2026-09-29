<script lang="ts">
  // A view as a ticker (P24): its latest lines in a strip that stays on top.
  //
  // For the laptop at FOH beside the DAW, where a full chat window costs space
  // nobody has. A detached view like `GroupWindow` — the same backend, the same
  // events, the same view addressing — laid out for the least room: no title
  // bar, no meters, no search, no controls. The text wraps or is cut off at
  // the far edge; it never scrolls. A click on a line brings the main window
  // forward at that line, which is where anything more than reading happens.
  //
  // It answers to the rules' view addressing as the view it shows (P19): a
  // flash meant for the stage manager's group floods the ticker showing that
  // group and no other. It never asks the platform for attention — it is
  // already the window in front of everything.
  //
  // Where it is, it remembers per view and per screen; see `ticker.ts`.

  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { PhysicalPosition, PhysicalSize } from "@tauri-apps/api/dpi";
  import { visibleStreams, type Group } from "./groups";
  import { fromStoredConfig } from "./profiles";
  import { arrange, DEFAULT_DIRECTION, type SortDirection } from "./sort";
  import { mergeHistory, toBubbles, type Bubble } from "./bubbles";
  import { attentionFor } from "./attention";
  import { LINES, resized, saveGeometry, type Edge, type Geometry } from "./ticker";
  import { ALL_CHANNELS_VIEW, setWindowTitle, type GroupView } from "./windows";
  import type { Channel } from "./streams";
  import {
    loadConfig,
    onCleared,
    onConfigChanged,
    onSessionState,
    onTranscript,
    requestAttention,
    revealInMain,
    sessionSnapshot,
    type SessionState,
    type StoredConfig,
    type TranscriptEvent,
  } from "./bridge";

  interface Props {
    /// The view's position in the active profile, or `ALL_CHANNELS_VIEW`.
    /// Read from the window label; see `windows.ts`.
    view: GroupView;
  }

  let { view }: Props = $props();

  const ALL_CHANNELS_NAME = "All channels";

  let showsAll = $derived(view === ALL_CHANNELS_VIEW);
  // Every channel of the active profile, for `GroupWindow`'s reason: the
  // stream id is the position in this list.
  let channels = $state<Channel[]>([]);
  let target = $state<Group | null>(null);
  let known = $state(false);
  let failure = $state<string | null>(null);
  /// The app's reading direction. The ticker has no control to turn it over
  /// with — it has no controls at all — so it reads the way the Settings page
  /// says every view is born reading.
  let chatSort = $state<SortDirection>(DEFAULT_DIRECTION);
  let messages = $state<TranscriptEvent[]>([]);
  let status = $state<SessionState | { state: "idle" }>({ state: "idle" });

  let name = $derived(showsAll ? ALL_CHANNELS_NAME : (target?.name ?? "Group"));
  let colors = $derived(new Map(channels.map((c, streamId) => [streamId, c.color])));
  let visible = $derived(
    showsAll
      ? new Set(channels.map((_, streamId) => streamId))
      : target === null
        ? new Set<number>()
        : visibleStreams(target, channels)!,
  );
  let bubbles = $derived(toBubbles(messages.filter((m) => visible.has(m.streamId))));
  let lines = $derived(arrange(bubbles.slice(-LINES), chatSort));
  let newestTop = $derived(chatSort === "newest-top");

  const colorOf = (streamId: number) => colors.get(streamId) ?? "var(--accent)";

  /// Reads the view out of a stored configuration — always the *active*
  /// profile, for `GroupWindow`'s reason — and renames the window with it.
  const apply = (stored: StoredConfig) => {
    const { profiles, activeKey, chatSort: sort } = fromStoredConfig(stored);
    chatSort = sort;
    const profile = profiles.find((p) => p.key === activeKey) ?? null;
    channels = profile?.channels ?? [];
    target = showsAll ? null : (profile?.groups[view as number] ?? null);
    known = true;
    // Never drawn — there is no title bar — but it is what the Window menu and
    // Mission Control call this window, so it follows a rename.
    if (showsAll || target) setWindowTitle(`${name} — Ticker — Fluestr`);
  };

  /// The flood, and only the flood: a rule addressed to this view, on a
  /// channel it shows (P19). `attention` is empty because a ticker never asks
  /// the platform to be looked at.
  const attend = (event: TranscriptEvent) => {
    const asking = attentionFor(event, bubbles[bubbles.length - 1], {
      attention: new Set(),
      shown: visible,
      view,
    });
    if (!asking?.loud) return;
    requestAttention(null, true).catch(() => {});
  };

  /// Sends the main window to this line. The newest segment of the bubble,
  /// because that is the one the main window is most certain to still hold
  /// in its own bubble of the same words.
  const reveal = (bubble: Bubble) => {
    const segment = bubble.segments[bubble.segments.length - 1];
    revealInMain(view, segment).catch((e) => console.warn("the main window did not come up:", e));
  };

  const close = () => {
    getCurrentWindow()
      .close()
      .catch((e) => console.warn("the ticker did not close:", e));
  };

  $effect(() => {
    // Subscribed before the history is asked for, for `GroupWindow`'s reason.
    const subscriptions = [
      onTranscript((event) => {
        messages = [...messages, event];
        if (visible.has(event.streamId)) attend(event);
      }),
      onSessionState((event) => {
        status = event;
      }),
      onCleared(() => {
        messages = [];
      }),
      onConfigChanged(apply),
    ];

    Promise.all(subscriptions).catch((e) => {
      failure = `event bridge unavailable: ${e}`;
    });

    const first = async () => {
      apply(await loadConfig());
      const snapshot = await sessionSnapshot();
      messages = mergeHistory(snapshot.transcripts, messages);
      if (status.state === "idle" && snapshot.state) status = snapshot.state;
    };
    first().catch((e) => (failure = String(e)));

    return () => {
      for (const subscription of subscriptions) {
        subscription.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  });

  // Where this ticker is, remembered as it is moved and resized. The two
  // events each carry half of it, so the other half is read once at the start
  // — from the window, not from storage: a place the backend refused as off
  // screen is not where the window is.
  // --- Resizing, by every edge but the left one ---------------------------
  //
  // The window is not natively resizable, so the page draws the handles. Not
  // for looks: a borderless window's native resize zone runs down every edge,
  // the left one included, and there it sits on the grip — the outer pixels
  // of what should only ever move the window would resize it instead. macOS
  // offers no native resize that a page can start, so this is done by hand:
  // the pointer is captured, and every move sets the size (and, for the top
  // edge, the position) measured from where the drag began.

  let resizing: {
    edge: Edge;
    start: Geometry;
    scale: number;
    x: number;
    y: number;
    pending: Geometry | null;
  } | null = null;

  const startResize = async (event: PointerEvent, edge: Edge) => {
    if (event.button !== 0 || place === null) return;
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const { screenX: x, screenY: y } = event;
    const scale = await getCurrentWindow().scaleFactor();
    resizing = { edge, start: place, scale, x, y, pending: null };
  };

  const moveResize = (event: PointerEvent) => {
    if (resizing === null) return;
    const drag = resizing;
    const first = drag.pending === null;
    drag.pending = resized(
      drag.start,
      drag.edge,
      (event.screenX - drag.x) * drag.scale,
      (event.screenY - drag.y) * drag.scale,
      drag.scale,
    );
    // One window update per frame, whatever the pointer's rate: each is a
    // round trip to the backend, and a queue of them is a window that trails
    // the pointer.
    if (first) requestAnimationFrame(() => resizeTo(drag));
  };

  const resizeTo = (drag: NonNullable<typeof resizing>) => {
    const next = drag.pending;
    drag.pending = null;
    if (next === null) return;
    const me = getCurrentWindow();
    void me.setSize(new PhysicalSize(next.width, next.height)).catch(() => {});
    // Only the top edge moves the window: it grows upwards from a bottom
    // that stays put.
    if (drag.edge.startsWith("top")) {
      void me.setPosition(new PhysicalPosition(next.x, next.y)).catch(() => {});
    }
  };

  const endResize = (event: PointerEvent) => {
    if (resizing === null) return;
    const handle = event.currentTarget as HTMLElement;
    if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
    resizing = null;
  };

  const EDGES: Edge[] = ["top", "right", "bottom", "top-right", "bottom-right"];

  /// Where the window is right now, in physical pixels, or `null` until it
  /// has been read. Plain rather than state: nothing renders from it.
  let place: Geometry | null = null;

  $effect(() => {
    const me = getCurrentWindow();
    const keep = (next: Partial<Geometry>) => {
      if (place === null) return;
      place = { ...place, ...next };
      saveGeometry(view, place);
    };

    Promise.all([me.outerPosition(), me.innerSize()])
      .then(([position, size]) => {
        place = { x: position.x, y: position.y, width: size.width, height: size.height };
      })
      .catch((e) => console.warn("the ticker cannot tell where it is:", e));

    const listeners = [
      me.onMoved(({ payload }) => keep({ x: payload.x, y: payload.y })),
      me.onResized(({ payload }) => keep({ width: payload.width, height: payload.height })),
    ];
    return () => {
      for (const listener of listeners) listener.then((unlisten) => unlisten()).catch(() => {});
    };
  });
</script>

<main>
  <!-- What the window is moved by: without a title bar, something has to be
       the title bar. A strip down the edge rather than across the top,
       because the top is a line of text the ticker cannot spare. -->
  <div class="grip" data-tauri-drag-region title="{name} — drag to move"></div>

  <div class="lines" class:newest-top={newestTop}>
    {#if failure}
      <p class="note error" data-tauri-drag-region>{failure}</p>
    {:else if known && !showsAll && target === null}
      <p class="note" data-tauri-drag-region>This group is no longer in the active profile.</p>
    {:else if lines.length === 0}
      <p class="note" data-tauri-drag-region>
        {name} · {status.state === "running" ? "nothing said yet" : "not running"}
      </p>
    {:else}
      {#each lines as bubble (bubble.key)}
        <button
          class="line"
          style="--channel: {colorOf(bubble.streamId)}"
          onclick={() => reveal(bubble)}
          title="Show this in the main window"
        >
          <span class="label">{bubble.label || "Stream " + bubble.streamId}</span>
          <span class="text"><span>{bubble.text}</span></span>
        </button>
      {/each}
    {/if}
  </div>

  {#each EDGES as edge}
    <div
      class="resize {edge}"
      aria-hidden="true"
      onpointerdown={(e) => void startResize(e, edge)}
      onpointermove={moveResize}
      onpointerup={endResize}
      onpointercancel={endResize}
    ></div>
  {/each}

  <button class="close" onclick={close} title="Close the ticker" aria-label="Close the ticker">
    <svg viewBox="0 0 12 12" width="9" height="9" aria-hidden="true">
      <path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
    </svg>
  </button>
</main>

<style>
  main {
    display: flex;
    height: 100vh;
    overflow: hidden;
    background: var(--surface);
    position: relative;
  }

  .grip {
    flex: none;
    width: 0.6rem;
    cursor: grab;
    background:
      radial-gradient(circle, var(--text-dim) 0.07rem, transparent 0.08rem) center / 0.3rem 0.3rem
        repeat-y;
    opacity: 0.5;
    border-right: 1px solid var(--border);
  }

  .grip:hover {
    opacity: 0.9;
  }

  /* Anchored at the newest edge and cut off at the other. Overflow on the
     start side of a flex container is clipped and cannot be scrolled to,
     which is exactly "never scrolls": a taller window shows more of the
     older lines, a one-line window shows the newest. */
  .lines {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    overflow: hidden;
    padding: 0.2rem 1.4rem 0.2rem 0.4rem;
  }

  .lines.newest-top {
    justify-content: flex-start;
  }

  .line {
    flex: none;
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: 0.5rem;
    align-items: start;
    width: 100%;
    background: transparent;
    color: var(--text);
    border: none;
    border-left: 3px solid var(--channel);
    border-radius: 0;
    padding: 0.05rem 0 0.05rem 0.4rem;
    font: inherit;
    font-size: 0.95rem;
    line-height: 1.35;
    text-align: left;
    cursor: pointer;
  }

  .line:hover {
    background: var(--bg);
  }

  .label {
    color: var(--channel);
    font-weight: 600;
    white-space: nowrap;
    max-width: 9rem;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* A long bubble shows its end, which is what was said last: at most three
     lines of it, and never more than the window has room for, so the label
     beside it is never pushed off. */
  .text {
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    max-height: min(calc(3 * 1.35em), calc(100vh - 0.5rem));
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  .note {
    margin: 0;
    color: var(--text-dim);
    font-size: 0.85rem;
    line-height: 1.35;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .error {
    color: #ff6b6b;
  }

  /* The resize handles: thin strips along the top, right and bottom edges
     and the two right-hand corners, over the content so a line under them is
     not clicked by a resize. Invisible — the cursor is what says what they
     do. None on the left: that edge is the grip. */
  .resize {
    position: absolute;
    z-index: 1;
  }

  .resize.top,
  .resize.bottom {
    left: 0.6rem;
    right: 0.6rem;
    height: 5px;
    cursor: ns-resize;
  }

  .resize.top {
    top: 0;
  }

  .resize.bottom {
    bottom: 0;
  }

  .resize.right {
    top: 0.6rem;
    bottom: 0.6rem;
    right: 0;
    width: 5px;
    cursor: ew-resize;
  }

  .resize.top-right,
  .resize.bottom-right {
    right: 0;
    width: 0.6rem;
    height: 0.6rem;
  }

  .resize.top-right {
    top: 0;
    cursor: nesw-resize;
  }

  .resize.bottom-right {
    bottom: 0;
    cursor: nwse-resize;
  }

  /* Out of the way until the pointer is over the ticker: the window is read
     far more often than it is closed. */
  .close {
    position: absolute;
    z-index: 2;
    top: 0.35rem;
    right: 0.45rem;
    display: flex;
    align-items: center;
    visibility: hidden;
    background: var(--bg);
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.2rem;
    line-height: 0;
    cursor: pointer;
  }

  main:hover .close,
  .close:focus-visible {
    visibility: visible;
  }

  .close:hover {
    color: var(--text);
  }
</style>

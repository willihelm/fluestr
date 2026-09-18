<script lang="ts">
  // The chat's left sidebar: which view of the channels is on screen.
  //
  // Selection only — groups are created and filled on the Settings page. The
  // sidebar is read during a show, when the question is "show me just the
  // stage" and not "let me reorganise my groups", and a rename box under the
  // cursor at that moment is a liability.
  //
  // "All channels" is always first and cannot be deleted: it is the state the
  // chat is in when no group is selected, not a group that happens to contain
  // everything.
  //
  // The one thing a row does besides select is open the view in a window of
  // its own (P7) — the second half of "show me just the stage", for the stage
  // that has to stay visible while the operator works in another view. All
  // channels detaches like the rest: it is not a group, but it is a view, and
  // on a phone beside the desk the whole chat in its own tab is the common
  // case rather than the exception.
  //
  // The sidebar's width is draggable and remembered per screen; see
  // `sidebar.ts` for why it is not in the configuration.

  import { untrack } from "svelte";
  import { openGroupWindow } from "./bridge";
  import type { Group } from "./groups";
  import { clampWidth, DEFAULT_WIDTH, loadWidth, maxWidth, MIN_WIDTH, saveWidth } from "./sidebar";
  import type { Channel } from "./streams";
  import { ALL_CHANNELS_VIEW, type GroupView } from "./windows";

  interface Props {
    groups: Group[];
    channels: Channel[];
    /// `null` is the all-channels view.
    selected: number | null;
  }

  let { groups, channels, selected = $bindable() }: Props = $props();

  const ALL_CHANNELS_NAME = "All channels";

  /// How far the drag handle moves the edge per arrow key. Coarse enough to
  /// get somewhere in a few presses, fine enough to land on a width: the
  /// keyboard is how this is reached without a pointer, not a second-class
  /// version of it.
  const STEP = 16;

  let width = $state(DEFAULT_WIDTH);
  let dragging = $state(false);
  /// The widest the handle may currently go, which the window's own width
  /// decides. Kept beside the value so the ARIA range describes the same
  /// bounds the drag is held to.
  let most = $state(DEFAULT_WIDTH);

  const colorsOf = (group: Group) =>
    channels.filter((c) => group.members.includes(c.key)).map((c) => c.color);

  /// Opens a view in a window of its own, by the **position** the
  /// configuration file stores membership by — the only handle on a group that
  /// means the same thing in another window — or `ALL_CHANNELS_VIEW`, which is
  /// the view that has no position because it is not a group. The key is
  /// stable for this window's lifetime and meaningless outside it.
  ///
  /// Asking for a view that is already open focuses that window instead of
  /// stacking a second one on it; the backend decides that, because it is the
  /// side that knows which windows exist.
  const detach = (view: GroupView, name: string, onTop: boolean) => {
    openGroupWindow(view, name, onTop).catch((e) =>
      console.warn("the group window did not open:", e),
    );
  };

  /// Brings the width inside what this window can show, and remembers it.
  ///
  /// The one way the width is ever set, so the stored value and the rendered
  /// one cannot disagree about the bounds.
  const resize = (next: number) => {
    most = maxWidth(window.innerWidth);
    width = clampWidth(next, window.innerWidth);
    saveWidth(width);
  };

  // The remembered width, clamped to the window it is being restored into —
  // a phone turned to portrait since the drag is the ordinary case, not an
  // edge one. Also re-clamped whenever the window changes size, because a
  // sidebar that is 60 % of a landscape screen is most of a portrait one.
  $effect(() => {
    // `width` is read through `untrack` because this effect installs a
    // listener rather than following the value: tracked, every pixel of a drag
    // would tear the listener down, read storage again and rebuild it.
    const fit = () => {
      most = maxWidth(window.innerWidth);
      width = clampWidth(untrack(() => width), window.innerWidth);
    };

    width = loadWidth(window.innerWidth);
    fit();

    window.addEventListener("resize", fit);
    return () => window.removeEventListener("resize", fit);
  });

  /// Pointer events rather than mouse events: one set of handlers then covers
  /// the finger on a phone and the mouse on the desk, and pointer capture
  /// keeps a fast drag that leaves the handle behind still attached to it.
  const grab = (event: PointerEvent) => {
    // Otherwise a drag on a touch screen scrolls the page it started on, and
    // on the desk it selects the text either side of the handle.
    event.preventDefault();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    dragging = true;
  };

  const drag = (event: PointerEvent) => {
    if (!dragging) return;
    // Measured from the left edge of the row the sidebar sits in and not from
    // the window's: they are the same today, and a layout that ever put
    // anything to the left of it would otherwise move the edge away from the
    // pointer that is dragging it.
    const left = (event.currentTarget as HTMLElement).parentElement?.getBoundingClientRect().left;
    resize(event.clientX - (left ?? 0));
  };

  const release = (event: PointerEvent) => {
    if (!dragging) return;
    dragging = false;
    (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
  };

  const nudge = (event: KeyboardEvent) => {
    const by =
      event.key === "ArrowLeft" ? -STEP : event.key === "ArrowRight" ? STEP : null;
    if (by === null) return;

    event.preventDefault();
    resize(width + by);
  };
</script>

<nav style="width: {width}px">
  <h2>Views</h2>

  <!-- A row is a button and carries a second one, so it is a div rather than a
       nested pair: a button inside a button is invalid and only one of the two
       would ever be clicked. -->
  <div class="row">
    <button class="view" class:active={selected === null} onclick={() => (selected = null)}>
      <span class="name">{ALL_CHANNELS_NAME}</span>
      <span class="count">{channels.length}</span>
    </button>
    <button
      class="detach"
      onclick={() => detach(ALL_CHANNELS_VIEW, ALL_CHANNELS_NAME, false)}
      title="Open {ALL_CHANNELS_NAME} in its own window"
      aria-label="Open {ALL_CHANNELS_NAME} in its own window"
    >
      ⧉
    </button>
  </div>

  {#each groups as group, index (group.key)}
    {@const colors = colorsOf(group)}
    <div class="row">
      <button
        class="view"
        class:active={selected === group.key}
        onclick={() => (selected = group.key)}
      >
        <span class="name">{group.name}</span>
        <!-- The member channels' colours, which is how a group is recognised at
             a glance — the same colours carry the bubbles in the chat. -->
        <span class="dots">
          {#each colors as color}
            <span class="dot" style="background: {color}"></span>
          {/each}
        </span>
        <span class="count">{colors.length}</span>
      </button>
      <button
        class="detach"
        onclick={() => detach(index, group.name, group.onTop)}
        title="Open {group.name} in its own window"
        aria-label="Open {group.name} in its own window"
      >
        ⧉
      </button>
    </div>
  {/each}

  {#if groups.length === 0}
    <p class="empty">Groups are made on the Settings page.</p>
  {/if}
</nav>

<!-- The edge, as something to take hold of. A separator rather than a slider,
     because what it moves is the boundary between two panes and not a value —
     and a focusable one, since a trackpad is not the only way an operator
     arrives at this app. Double-click puts it back where it started, which is
     the way out of a drag that went somewhere silly.

     The two warnings below are the window-splitter pattern meeting a rule
     written for static separators: ARIA says a separator that moves *is*
     focusable and does answer keys, and the alternatives are worse — a button
     announces itself as one and says nothing about the range, and a slider is
     a value where this is a boundary. Suppressed with the reason rather than
     the role changed to whatever passes. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="grip"
  class:dragging
  role="separator"
  aria-orientation="vertical"
  aria-label="Sidebar width"
  aria-valuenow={width}
  aria-valuemin={MIN_WIDTH}
  aria-valuemax={most}
  onpointerdown={grab}
  onpointermove={drag}
  onpointerup={release}
  onpointercancel={release}
  onkeydown={nudge}
  tabindex="0"
  ondblclick={() => resize(DEFAULT_WIDTH)}
></div>

<style>
  nav {
    flex: none;
    border-right: 1px solid var(--border);
    background: var(--surface);
    padding: 0.5rem;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  h2 {
    margin: 0 0 0.3rem 0.35rem;
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }

  /* Wider than the line it sits on, and pulled back over it: a one-pixel
     border is a target nobody hits, and a finger is not a pointer at all.
     `touch-action` is what stops a drag here from scrolling the chat behind
     it. */
  .grip {
    flex: none;
    border: none;
    padding: 0;
    width: 0.55rem;
    margin-left: -0.3rem;
    margin-right: -0.25rem;
    position: relative;
    z-index: 1;
    cursor: col-resize;
    touch-action: none;
    background: transparent;
  }

  /* A finger is not a pointer: the same edge has to be wide enough to be hit
     without aiming, which is the case this was built for. Still pulled back
     over the border, so what grows is the target and not the layout. */
  @media (pointer: coarse) {
    .grip {
      width: 1.4rem;
      margin-left: -0.75rem;
      margin-right: -0.65rem;
    }
  }

  /* Lit only while it is being used or aimed at: the sidebar already has an
     edge, and a second permanent line beside it would read as a second
     border. */
  .grip:hover,
  .grip:focus-visible,
  .grip.dragging {
    background: var(--accent);
    opacity: 0.55;
    outline: none;
  }

  /* The second button only appears under the cursor: it is reached for now and
     then, and eight of them down the side of a live chat is eight things
     competing with the group names. It stays available to the keyboard, where
     hover means nothing. */
  .row {
    display: flex;
    align-items: center;
  }

  .detach {
    flex: none;
    visibility: hidden;
    background: transparent;
    color: var(--text-dim);
    border: none;
    border-radius: 0.3rem;
    padding: 0.35rem 0.4rem;
    font: inherit;
    font-size: 0.85rem;
    line-height: 1;
    cursor: pointer;
  }

  .row:hover .detach,
  .detach:focus-visible {
    visibility: visible;
  }

  /* A touch screen has no hover to reveal it with, and a control that is only
     ever reachable by hovering is no control at all there. */
  @media (hover: none) {
    .detach {
      visibility: visible;
    }
  }

  .detach:hover {
    color: var(--text);
    background: var(--bg);
  }

  .view {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    width: 100%;
    min-width: 0;
    background: transparent;
    color: var(--text);
    border: none;
    border-radius: 0.3rem;
    padding: 0.35rem 0.5rem;
    font: inherit;
    font-size: 0.85rem;
    text-align: left;
    cursor: pointer;
  }

  .view:hover {
    background: var(--bg);
  }

  .view.active {
    background: var(--bg);
    box-shadow: inset 2px 0 0 var(--accent);
  }

  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dots {
    display: flex;
    gap: 0.12rem;
    /* At a narrow width the names are what has to survive, not the dots: the
       colours repeat in every bubble in the chat, the name does not. */
    overflow: hidden;
  }

  .dot {
    flex: none;
    width: 0.4rem;
    height: 0.4rem;
    border-radius: 50%;
  }

  .count {
    color: var(--text-dim);
    font-size: 0.75rem;
    min-width: 1rem;
    text-align: right;
  }

  .empty {
    margin: 0.4rem 0.35rem;
    font-size: 0.78rem;
    color: var(--text-dim);
  }
</style>

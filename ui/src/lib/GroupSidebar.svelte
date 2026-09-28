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
  // At the foot of it, ruled off from the views, is the way into Settings.
  // Not one of the rows above it and never selected: those say what the chat
  // is showing, this opens a dialog over the window and comes back. It lives
  // here because this is the app's one list of places, and because the header
  // is the Start button's bar and nothing should compete with that during a
  // show.
  //
  // The sidebar's width is draggable and remembered per screen; see
  // `sidebar.ts` for why it is not in the configuration.
  //
  // On a narrow window it is not a column at all. A phone in the auditorium
  // (P16) cannot spare a third of its width for a list that is read once and
  // then in the way, so the sidebar becomes a drawer over the chat, opened
  // from the burger in the header and closed again by the choice that was made
  // in it. Same rows, same markup — what changes is whether it takes space
  // from the chat or covers it for a moment.

  import { untrack } from "svelte";
  import { openGroupWindow } from "./bridge";
  import { shortcut } from "./shortcut";
  import type { Group } from "./groups";
  import { watchNarrow } from "./layout";
  import { clampWidth, DEFAULT_WIDTH, loadWidth, maxWidth, MIN_WIDTH, saveWidth } from "./sidebar";
  import type { Channel } from "./streams";
  import { ALL_CHANNELS_VIEW, type GroupView } from "./windows";

  interface Props {
    groups: Group[];
    channels: Channel[];
    /// `null` is the all-channels view.
    selected: number | null;
    /// Whether the drawer is showing. Only ever true on a narrow window, where
    /// the header's burger is what binds to it; on the desk the sidebar is
    /// simply there and this says nothing.
    open?: boolean;
    /// Opens the Settings dialog, from the button at the foot of the list.
    opensettings: () => void;
  }

  let {
    groups,
    channels,
    selected = $bindable(),
    open = $bindable(false),
    opensettings,
  }: Props = $props();

  /// Whether this window is too narrow for a column beside the chat.
  let narrow = $state(false);
  $effect(() =>
    watchNarrow((next) => {
      narrow = next;
      // A window that grew back to desk width while the drawer was open: the
      // sidebar is a column again, and a drawer state left standing would come
      // back the next time the window is made narrow, over a chat nobody asked
      // to have covered.
      if (!next) open = false;
    }),
  );

  /// Selecting a view, which on a phone is also the end of the errand the
  /// drawer was opened for. Anything else — a second tap to dismiss what is
  /// now covering the chat one was just brought to — is a step the operator
  /// has to take during a show for no reason.
  const pick = (view: number | null) => {
    selected = view;
    open = false;
  };

  const ALL_CHANNELS_NAME = "All channels";

  /// Where the cog's teeth sit, in degrees. Eight of them, which is as many
  /// as read as teeth rather than as a ring at 14 px.
  const TEETH = [0, 45, 90, 135, 180, 225, 270, 315];

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

<!-- The dimmed chat behind an open drawer, and the way out of it: a tap
     anywhere on what is covered puts it away, which is what every drawer on a
     phone does and what a thumb reaches for first. Only rendered while the
     drawer is open, so on the desk there is nothing over the chat to click
     through. A div with a click handler rather than a button, because it is
     the *backdrop* — the button that does this with a keyboard is the burger
     it was opened from, and Escape below covers the rest. -->
{#if narrow && open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={() => (open = false)}></div>
{/if}

<svelte:window
  onkeydown={(event) => {
    if (event.key === "Escape" && open) open = false;
  }}
/>

<!-- The width is dragged on the desk and is the drawer's own on a phone: a
     remembered 96 px column would be a drawer too narrow to read a group name
     in, and the drag handle it was set with is not even rendered there. -->
<nav
  class:drawer={narrow}
  class:open
  inert={narrow && !open}
  aria-label="Views"
  style={narrow ? "" : `width: ${width}px`}
>
  <h2>Views</h2>

  <!-- A row is a button and carries a second one, so it is a div rather than a
       nested pair: a button inside a button is invalid and only one of the two
       would ever be clicked. -->
  <div class="row">
    <button class="view" class:active={selected === null} onclick={() => pick(null)}>
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
        onclick={() => pick(group.key)}
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
    <p class="empty">Groups are made in Settings.</p>
  {/if}

  <!-- Pushed to the bottom and ruled off, so it reads as the thing that is
       not a view. On a phone it also closes the drawer it was pressed in:
       what it opens is about to stand over that drawer anyway. -->
  <div class="foot">
    <button
      class="settings"
      onclick={() => {
        opensettings();
        open = false;
      }}
      aria-haspopup="dialog"
      title="Settings ({shortcut(',')})"
    >
      <span>Settings</span>
      <!-- Drawn rather than an emoji, for the reason the detach glyph and the
           burger are: it has to take its weight and its colour from the row it
           is in, at every font size and in either theme. Teeth as blocks
           around a ring, not spokes radiating from a hub — spokes are a sun,
           which is what the first attempt at this looked like. -->
      <svg class="cog" viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
        <g fill="currentColor">
          {#each TEETH as angle}
            <rect x="7.2" y="1" width="1.6" height="2.8" rx="0.4" transform="rotate({angle} 8 8)" />
          {/each}
        </g>
        <circle cx="8" cy="8" r="4.5" fill="none" stroke="currentColor" stroke-width="1.6" />
        <circle cx="8" cy="8" r="1.6" fill="none" stroke="currentColor" stroke-width="1.2" />
      </svg>
    </button>
  </div>
</nav>

<!-- The edge, as something to take hold of. A separator rather than a slider,
     because what it moves is the boundary between two panes and not a value —
     and a focusable one, since a trackpad is not the only way an operator
     arrives at this app. Double-click puts it back where it started, which is
     the way out of a drag that went somewhere silly.

     Not rendered at all on a narrow window: the sidebar is a drawer there,
     with a width of its own and no boundary between two panes to move.

     The two warnings below are the window-splitter pattern meeting a rule
     written for static separators: ARIA says a separator that moves *is*
     focusable and does answer keys, and the alternatives are worse — a button
     announces itself as one and says nothing about the range, and a slider is
     a value where this is a boundary. Suppressed with the reason rather than
     the role changed to whatever passes. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
{#if !narrow}
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
{/if}

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

  /* The foot of the list, wherever the list ends: `margin-top: auto` puts it
     at the bottom edge of a short sidebar and directly under the last group
     of a long one, and the rule above it is what says it is not one of
     them. */
  .foot {
    margin-top: auto;
    padding-top: 0.4rem;
    border-top: 1px solid var(--border);
  }

  /* A row in the shape of the views above it, in the dim colour the app uses
     for what is furniture rather than content — it is reached on purpose,
     once, and must not draw the eye off the group that is flashing. The word
     at the left where every row above it starts, the cog at the right edge
     where those rows carry their channel count — the two columns the sidebar
     already has, rather than a third position halfway along. */
  .settings {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    width: 100%;
    background: transparent;
    color: var(--text-dim);
    border: none;
    border-radius: 0.3rem;
    padding: 0.35rem 0.5rem;
    font: inherit;
    font-size: 0.85rem;
    text-align: left;
    cursor: pointer;
  }

  .settings:hover {
    background: var(--bg);
    color: var(--text);
  }

  /* Against the right edge, in the column the counts are in. `flex: none`
     because a flex item with no explicit size would otherwise be stretched to
     the line height. */
  .cog {
    flex: none;
    margin-left: auto;
  }

  /* --- The drawer (narrow) ----------------------------------------------

     Over the chat rather than beside it, and over the chat *only*: the pane
     it is positioned against is the one holding the transcripts, so the
     header bars above stay lit and usable — including the burger it was
     opened from, which is the control an operator who opened it by accident
     reaches for. A drawer that covered the whole window would take Start and
     Stop with it, which is the one pair that must never be behind a scrim.

     A transform rather than `display`, so it slides rather than appears and
     the eye can follow where it came from and where it goes back to. */
  .drawer {
    position: absolute;
    z-index: 20;
    top: 0;
    bottom: 0;
    left: 0;
    /* Wide enough for a group name and its dots, never more than most of a
       phone: the chat behind it has to stay visible as the thing being
       returned to. */
    width: min(17rem, 78vw);
    /* The home indicator is at the bottom of the drawer, not of the window's
       header — so this is where it is padded out of, and the top is left at
       the nav's own padding because the bars above already cleared the
       notch. */
    padding-bottom: max(0.5rem, var(--safe-bottom));
    padding-left: max(0.5rem, var(--safe-left));
    box-shadow: 0 0 2rem rgba(0, 0, 0, 0.5);
    transform: translateX(-100%);
    transition: transform 160ms ease-out;
  }

  .drawer.open {
    transform: translateX(0);
  }

  /* A closed drawer is `inert`, so nothing in it can be tabbed into or read
     out while it is off screen. `visibility` finishes the job for the moment
     between the two: a transform alone leaves it composited beside the window
     and a horizontal scroll could find it. */
  .drawer[inert] {
    visibility: hidden;
    transition:
      transform 160ms ease-out,
      visibility 0s linear 160ms;
  }

  .backdrop {
    position: absolute;
    inset: 0;
    z-index: 15;
    background: rgba(0, 0, 0, 0.45);
  }

  /* The slide is the one thing here that is decoration rather than
     information: opened and closed are told apart by the drawer being there
     at all. */
  @media (prefers-reduced-motion: reduce) {
    .drawer {
      transition: none;
    }

    .drawer[inert] {
      transition: none;
    }
  }

  /* Rows a thumb can hit, in the layout where the pointer is a thumb. The
     desk keeps its compact list: eight 44 px rows is a sidebar that scrolls
     on a screen with room for all of them. */
  .drawer .view {
    padding: 0.55rem 0.5rem;
    font-size: 0.95rem;
  }

  .drawer .detach {
    padding: 0.55rem 0.6rem;
  }

  .drawer .settings {
    padding: 0.55rem 0.5rem;
    font-size: 0.95rem;
  }
</style>

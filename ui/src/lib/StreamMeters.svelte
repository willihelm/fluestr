<script lang="ts">
  // One meter per configured stream, side by side.
  //
  // On screen whether or not a session runs: the same `levels` event arrives
  // from the monitor before Start (P8), and a strip that appeared with the
  // session would move the chat under it twice a show. A bar with nothing
  // arriving stands still, which is what a quiet line looks like anyway.
  //
  // They arrive in a single `levels` event, so all the bars on screen are the
  // same age — which is the point of bundling them: eight meters read together
  // should have been measured together.
  //
  // Two shapes, one per room. On the desk each stream is a row — a label and a
  // bar that fills rightwards — laid out in as many columns as the window has
  // space for. On a phone (P16) that grid collapses to one column, and eight
  // stacked rows are most of the screen above a chat that is why the page is
  // open at all. There the meters turn: a strip of narrow columns filling
  // upwards, scrolled sideways if there are more than fit, in the height of a
  // single desk row.
  //
  // Either shape can be folded away entirely, down to the handle that brings
  // it back: the strip is the tallest fixed thing above the chat, and a window
  // parked in the corner of the desk is usually open for the chat alone. What
  // survives the fold is the health counts — a strip hiding the fact that
  // segments are being dropped would be worse than no strip.

  import LevelMeter from "./LevelMeter.svelte";
  import GainKnob from "./GainKnob.svelte";
  import MuteButton from "./MuteButton.svelte";
  import SoloButton from "./SoloButton.svelte";
  import { canSolo, watchSolo } from "./solo.svelte";
  import { watchNarrow } from "./layout";
  import { loadCollapsed, saveCollapsed } from "./meters";
  import { currentGroup } from "./windows";
  import type { Channel } from "./streams";
  import type { StreamLevel, WebhookStatus } from "./bridge";

  interface Props {
    /// *Every* configured channel, never a filtered list: the stream id is the
    /// position in this array, exactly as the backend assigns it, so a
    /// pre-filtered list would silently shift every meter onto the wrong
    /// stream. Which of them to draw is `visible`.
    channels: Channel[];
    levels: Map<number, StreamLevel>;
    /// Segments the decode queue gave up on, per stream id.
    overloaded: Map<number, number>;
    /// The stream ids the selected group shows, or `null` for all of them.
    visible: Set<number> | null;
    /// The stream ids the device has no channel for. They are shown rather
    /// than hidden: a meter that is simply missing reads as a bug, while a
    /// silent one reads as a channel nobody is talking on — and neither is
    /// what happened.
    ///
    /// Not the same as a muted channel, which is on the device and is being
    /// metered — that one is `channel.muted`, and it has a lit M beside it
    /// saying who silenced it.
    absent: Set<number>;
    /// How the outbound queue is doing (P19), or `null` before the first
    /// answer has arrived.
    ///
    /// Not per stream, unlike everything else in this strip: a rule's webhook
    /// is the session's and not a channel's. It sits at the end of the strip
    /// rather than in a panel of its own because a dead endpoint is a health
    /// problem, and this is where an operator already looks for one — a cue
    /// that silently stopped arriving is otherwise discovered by the cue not
    /// happening.
    webhooks: WebhookStatus | null;
    /// Sets a channel's input trim, or `null` where this view does not own the
    /// setup — a detached group window and the browser peer both render this
    /// strip, and neither writes the configuration file. The knob is left out
    /// entirely there rather than shown and ignored.
    ///
    /// Here rather than only on the Settings page because this is where a trim
    /// is actually set: against the meter beside it, while the channel is
    /// being spoken on. The Settings page keeps its own knob for the setup
    /// being built before anybody is talking.
    gain?: ((streamId: number, db: number) => void) | null;
    /// Mutes and unmutes a channel, or `null` in the views that do not own the
    /// setup — the same rule the knob follows, and the same reason: a detached
    /// group window and the browser peer render this strip and neither writes
    /// the configuration file.
    ///
    /// Here rather than only on the Settings page because this is when it is
    /// pressed: during a show, on the channel that is filling the chat with
    /// the room it is standing in.
    mute?: ((streamId: number, muted: boolean) => void) | null;
    /// Whether to show the S that puts a channel on this machine's
    /// headphones. Unlike the M and the knob it writes no setup, so every app
    /// window may have it — only a browser on the LAN may not, having no
    /// business in the host's headphones.
    listen?: boolean;
  }

  let {
    channels,
    levels,
    overloaded,
    visible,
    absent,
    webhooks,
    gain = null,
    mute = null,
    listen = canSolo,
  }: Props = $props();

  $effect(() => {
    if (listen) watchSolo();
  });

  /// Matched here rather than in CSS alone, because the orientation is not
  /// only a matter of styling: `LevelMeter` has to be told which way round it
  /// is drawing, and a media query cannot set a prop. The CSS below carries
  /// the same breakpoint for the parts it *can* do on its own.
  let narrow = $state(false);
  $effect(() => watchNarrow((next) => (narrow = next)));

  const empty = (streamId: number): StreamLevel => ({
    streamId,
    rms: 0,
    peak: 0,
    dropped: 0,
  });

  /// Which view this window is, for the storage key. Read once, like the
  /// chat's reading direction: a window does not become another view.
  const view = currentGroup();

  /// Whether the bars are folded away. This view's own answer, remembered
  /// across a restart — a window sized for the chat alone should still be that
  /// window tomorrow.
  let collapsed = $state(loadCollapsed(view));

  const fold = (next: boolean) => {
    collapsed = next;
    saveCollapsed(view, next);
  };

  /// Segments the decode queue gave up on across the channels this view shows.
  /// Only for the folded handle: expanded, each count sits on the meter of the
  /// stream it happened on, which is the more useful place for it.
  let dropped = $derived(
    channels.reduce(
      (total, _channel, streamId) =>
        visible === null || visible.has(streamId)
          ? total + (overloaded.get(streamId) ?? 0)
          : total,
      0,
    ),
  );
</script>

<div class="strip" class:folded={collapsed}>
  <!-- The handle, on the strip rather than in the window header: it is the
       thing directly above the chat that a window is being made shorter by.
       A full-width row across the top of the band, folded or not: it is the
       one control here that is not about a channel, and beside the bars it
       both sat among the knobs and moved — a left column when the bars were
       there, the whole band when they were not. A handle that is in a
       different place depending on what it did last is one that has to be
       looked for every time. -->
  <button
    class="fold"
    onclick={() => fold(!collapsed)}
    aria-expanded={!collapsed}
    aria-label="Level meters"
    title={collapsed ? "Show the meters" : "Hide the meters"}
  >
    <span class="chevron" aria-hidden="true">{collapsed ? "\u25b8" : "\u25be"}</span>
    <!-- Named whether or not the bars are showing, unlike the counts beside
         it: a row with a chevron and nothing else in it is a control that
         says only that something can be opened, not what. -->
    <span class="what">Meters</span>
    {#if dropped > 0}
      <!-- Folded, this is the only place the count is said at all. -->
      <span class="error folded-only" title="The decode queue was full and the oldest segments were dropped: {dropped}">
        {dropped}&nbsp;not transcribed
      </span>
    {/if}
    {#if webhooks && webhooks.dropped > 0}
      <span class="error folded-only" title="Webhook requests given up on — the queue was full, or the endpoint did not answer{webhooks.lastError ? `: ${webhooks.lastError}` : ''}">
        {webhooks.dropped}&nbsp;webhooks dropped
      </span>
    {/if}
  </button>

  {#if !collapsed}
  <div class="meters" class:vertical={narrow}>
    {#each channels as channel, streamId (channel.key)}
      {#if visible === null || visible.has(streamId)}
      {@const level = levels.get(streamId) ?? empty(streamId)}
      <div
        class="stream"
        class:absent={absent.has(streamId)}
        class:muted={channel.muted}
        style="--channel: {channel.color}"
      >
        <span class="name">{channel.label}</span>
        {#if mute}
          <!-- Before the knob: mute is pressed in a hurry and trim is set
               slowly, so the one that is reached for under pressure sits next
               to the name it belongs to. -->
          <MuteButton
            muted={channel.muted}
            label={channel.label}
            disabled={absent.has(streamId)}
            onchange={(next) => mute(streamId, next)}
          />
        {/if}
        {#if listen}
          <SoloButton
            {streamId}
            label={channel.label}
            disabled={absent.has(streamId)}
          />
        {/if}
        {#if gain}
          <!-- Before the bar, so the reading stays the widest thing in the row
               and the knob sits with the name of what it trims. -->
          <GainKnob
            value={channel.gainDb}
            label={channel.label}
            onchange={(db) => gain(streamId, db)}
          />
        {/if}
        <LevelMeter
          rms={level.rms}
          peak={level.peak}
          dropped={level.dropped}
          vertical={narrow}
        />
        {#if (overloaded.get(streamId) ?? 0) > 0}
          <span class="error" title="The decode queue was full and the oldest segments were dropped: {overloaded.get(streamId)}">
            <!-- As with the dropped count: the number stays, the sentence it is
                 the subject of goes to the tooltip when there is no room. -->
            {overloaded.get(streamId)}<span class="word">&nbsp;not transcribed</span>
          </span>
        {/if}
      </div>
      {/if}
    {/each}

    {#if webhooks && webhooks.dropped > 0}
      <div class="stream outbound">
        <span
          class="error"
          title="Webhook requests given up on — the queue was full, or the endpoint did not answer{webhooks.lastError
            ? `: ${webhooks.lastError}`
            : ''}"
        >
          {webhooks.dropped}<span class="word">&nbsp;webhooks dropped</span>
        </span>
      </div>
    {/if}
  </div>
  {/if}
</div>

<style>
  /* The handle and the bars it folds, as one band across the top of the chat:
     the border below and the surface behind it belong to the band rather than
     to the meters, so folding them away leaves a strip that still reads as the
     edge of the chat and not as a stray button. */
  .strip {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  /* A row across the top of the band, in the same place and at the same width
     whether the bars below it are showing or not. Its contents are pushed to
     the left, where the eye starting on the band already is. */
  .fold {
    display: flex;
    align-items: center;
    justify-content: flex-start;
    gap: 0.5rem;
    flex: none;
    width: 100%;
    padding: 0.15rem 1rem;
    padding-left: max(1rem, var(--safe-left));
    padding-right: max(1rem, var(--safe-right));
    background: none;
    border: none;
    color: var(--text-dim);
    font: inherit;
    font-size: 0.8rem;
    line-height: 1;
    text-align: left;
    cursor: pointer;
  }

  .fold:hover {
    color: var(--text);
  }

  /* Sized against the strip rather than against the text beside it: it is the
     one thing in the band that is aimed at with a mouse in a dark room, and at
     the size of the label it was a target the width of a full stop. The row it
     is in is the target now, but the arrow is still what is aimed at. */
  .chevron {
    font-size: 1.2rem;
    line-height: 1;
  }

  .what {
    font-weight: 600;
  }

  /* The counts are said here only when there is nothing else in the band to
     say them: expanded, each one sits on the meter it happened on. */
  .fold .folded-only {
    display: none;
  }

  .folded .fold .folded-only {
    display: inline;
  }

  .meters {
    display: grid;
    /* Wide enough for a name, an M, an S, a knob and a bar that is still worth
       reading. Below this the bar is what gives way, and a meter narrower
       than its own label is not a meter. */
    grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
    gap: 0.35rem 1rem;
    padding: 0 1rem 0.5rem;
    padding-left: max(1rem, var(--safe-left));
    padding-right: max(1rem, var(--safe-right));
    /* Under the handle, across the whole band. */
    min-width: 0;
  }

  .stream {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    /* A grid cell's contents may shrink below what they would like to be.
       Without it the "not transcribed" note — which cannot wrap, because it
       belongs on the bar's line — pushes out of its column and prints over
       the name of the next channel. */
    min-width: 0;
    font-size: 0.8rem;
    /* The meter takes its colour from the channel, so a bar can be found
       without reading the label next to it. */
    --accent: var(--channel);
  }

  /* Dimmed rather than labelled: a stream the device has no channel for
     keeps its meter, which simply stays at rest, and the greying says it
     receives no audio. */
  .stream.absent {
    opacity: 0.55;
  }

  /* A muted stream is *not* dimmed. Its meter is the whole point — it goes on
     showing that somebody is talking on a channel nothing is being
     transcribed from — and the lit M is what says why. The name is struck
     through instead, which is a statement rather than an absence. */
  .stream.muted .name {
    text-decoration: line-through;
    text-decoration-thickness: 1px;
  }

  /* The one cell that is not a channel. It has no meter, so it is only ever
     on screen when there is something wrong to say. */
  .stream.outbound {
    justify-content: flex-start;
  }

  .name {
    color: var(--channel);
    font-weight: 600;
    min-width: 4rem;
    max-width: 7rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Truncated rather than allowed to overrun the column: the whole sentence
     is in the tooltip, and a count that has run over the channel beside it
     costs more than the word it lost. */
  .error {
    color: var(--danger);
    white-space: nowrap;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* --- The strip (narrow) ------------------------------------------------

     Driven by the class the script sets rather than by the media query alone,
     so the bars and the `LevelMeter`s inside them can never be in different
     orientations: one decision, in `layout.ts`, applied to both.

     A row that scrolls sideways rather than one that wraps: a second line of
     meters is the thing this layout exists to avoid, and a house with twelve
     channels on a phone is a swipe, not a screenful. */
  .meters.vertical {
    display: flex;
    grid-template-columns: none;
    gap: 0 0.5rem;
    padding: 0 0.75rem 0.4rem;
    padding-left: max(0.75rem, var(--safe-left));
    padding-right: max(0.75rem, var(--safe-right));
    overflow-x: auto;
    /* A sideways drag here is the strip being scrolled; the page behind it
       must not go anywhere. */
    overscroll-behavior-x: contain;
  }

  .vertical .stream {
    flex: none;
    flex-direction: column-reverse;
    align-items: center;
    gap: 0.25rem;
    width: 2.8rem;
    /* A minimum rather than a height: a stream with a dropped count carries a
       line the others do not, and the row stretches every column to the
       tallest so the bars still start and end level. */
    min-height: 4.2rem;
  }

  /* `column-reverse`, so the label written first in the markup lands *under*
     the bar — which is where a channel name goes on every desk, and it keeps
     the document order the screen reader follows saying the name before the
     reading it belongs to. */
  .vertical .name {
    min-width: 0;
    max-width: 100%;
    font-size: 0.65rem;
    line-height: 1.2;
    text-align: center;
  }

  .vertical .error {
    font-size: 0.65rem;
    line-height: 1;
  }

  .vertical .error .word {
    display: none;
  }

  /* The same gutter the rest of the phone layout keeps, plus whatever the
     rounded corner takes. */
  @media (max-width: 45rem) {
    .fold {
      padding-left: max(0.75rem, var(--safe-left));
      padding-right: max(0.75rem, var(--safe-right));
    }
  }
</style>

<script lang="ts">
  // One meter per running stream, side by side.
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

  import LevelMeter from "./LevelMeter.svelte";
  import { watchNarrow } from "./layout";
  import type { Channel } from "./streams";
  import type { StreamLevel } from "./bridge";

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
    /// than hidden: a meter that is simply absent reads as a bug, while a
    /// silent one reads as a channel nobody is talking on — and neither is
    /// what happened.
    muted: Set<number>;
  }

  let { channels, levels, overloaded, visible, muted }: Props = $props();

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
</script>

<div class="meters" class:vertical={narrow}>
  {#each channels as channel, streamId (channel.key)}
    {#if visible === null || visible.has(streamId)}
    {@const level = levels.get(streamId) ?? empty(streamId)}
    <div class="stream" class:muted={muted.has(streamId)} style="--channel: {channel.color}">
      <span class="name">{channel.label}</span>
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
</div>

<style>
  .meters {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    gap: 0.35rem 1rem;
    padding: 0.5rem 1rem;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
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

  /* Dimmed rather than labelled: a muted stream keeps its meter, which
     simply stays at rest, and the greying says it receives no audio. */
  .stream.muted {
    opacity: 0.55;
  }

  .name {
    color: var(--channel);
    font-weight: 600;
    min-width: 5rem;
    max-width: 8rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Truncated rather than allowed to overrun the column: the whole sentence
     is in the tooltip, and a count that has run over the channel beside it
     costs more than the word it lost. */
  .error {
    color: #ff6b6b;
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
    padding: 0.4rem 0.75rem;
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
    width: 2.6rem;
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
</style>

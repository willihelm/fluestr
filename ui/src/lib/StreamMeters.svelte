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
  import GainKnob from "./GainKnob.svelte";
  import MuteButton from "./MuteButton.svelte";
  import { watchNarrow } from "./layout";
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
  }: Props = $props();

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

<style>
  .meters {
    display: grid;
    /* Wide enough for a name, an M, a knob and a bar that is still worth
       reading. Below this the bar is what gives way, and a meter narrower
       than its own label is not a meter. */
    grid-template-columns: repeat(auto-fit, minmax(18rem, 1fr));
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
</style>

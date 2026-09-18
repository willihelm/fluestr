<script lang="ts">
  // One meter per running stream, side by side.
  //
  // They arrive in a single `levels` event, so all the bars on screen are the
  // same age — which is the point of bundling them: eight meters read together
  // should have been measured together.

  import LevelMeter from "./LevelMeter.svelte";
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

  const empty = (streamId: number): StreamLevel => ({
    streamId,
    rms: 0,
    peak: 0,
    dropped: 0,
  });
</script>

<div class="meters">
  {#each channels as channel, streamId (channel.key)}
    {#if visible === null || visible.has(streamId)}
    {@const level = levels.get(streamId) ?? empty(streamId)}
    <div class="stream" class:muted={muted.has(streamId)} style="--channel: {channel.color}">
      <span class="name">{channel.label}</span>
      <LevelMeter rms={level.rms} peak={level.peak} dropped={level.dropped} />
      {#if (overloaded.get(streamId) ?? 0) > 0}
        <span class="error" title="The decode queue was full and the oldest segments were dropped">
          {overloaded.get(streamId)} not transcribed
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

  .error {
    color: #ff6b6b;
    white-space: nowrap;
  }
</style>

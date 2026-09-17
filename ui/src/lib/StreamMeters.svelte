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
    channels: Channel[];
    levels: Map<number, StreamLevel>;
    /// Segments the decode queue gave up on, per stream id.
    overloaded: Map<number, number>;
  }

  let { channels, levels, overloaded }: Props = $props();

  const empty = (streamId: number): StreamLevel => ({
    streamId,
    rms: 0,
    peak: 0,
    dropped: 0,
  });
</script>

<div class="meters">
  {#each channels as channel, streamId (channel.key)}
    {@const level = levels.get(streamId) ?? empty(streamId)}
    <div class="stream" style="--channel: {channel.color}">
      <span class="name">{channel.label}</span>
      <LevelMeter rms={level.rms} peak={level.peak} dropped={level.dropped} />
      {#if (overloaded.get(streamId) ?? 0) > 0}
        <span class="error" title="The decode queue was full and the oldest segments were dropped">
          {overloaded.get(streamId)} not transcribed
        </span>
      {/if}
    </div>
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

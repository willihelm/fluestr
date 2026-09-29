<script lang="ts">
  // One of the shoutbox's live readings as a meter (P26): a label, a bar over
  // a fixed range, and the number beside it.
  //
  // The bar fills from `origin` to the value, so the same meter reads a level
  // that grows from the bottom of its range and a gain reduction that grows
  // down from 0. A `mark` is a second value on the same scale — where the
  // gain is headed, the level it is held at — and a `guide` a setting that
  // stands still: the quiet threshold, the gate.

  interface Props {
    label: string;
    value: number | null | undefined;
    min: number;
    max: number;
    /// Where the fill starts. The bottom of the range unless said otherwise.
    origin?: number;
    mark?: number | null;
    guide?: number | null;
    readout: string;
    title?: string;
    warn?: boolean;
  }

  let {
    label,
    value,
    min,
    max,
    origin = min,
    mark = null,
    guide = null,
    readout,
    title,
    warn = false,
  }: Props = $props();

  const at = (db: number) => Math.max(0, Math.min(1, (db - min) / (max - min)));
  const known = (db: number | null | undefined): db is number =>
    db !== null && db !== undefined && Number.isFinite(db);

  let from = $derived(known(value) ? Math.min(at(origin), at(value)) : 0);
  let to = $derived(known(value) ? Math.max(at(origin), at(value)) : 0);
</script>

<div class="row" class:warn {title}>
  <span class="label">{label}</span>
  <div class="bar" style="--from: {from}; --to: {to}">
    <div class="fill"></div>
    {#if known(guide)}
      <div class="guide" style="--at: {at(guide)}"></div>
    {/if}
    {#if known(mark)}
      <div class="mark" style="--at: {at(mark)}"></div>
    {/if}
  </div>
  <span class="readout">{readout}</span>
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: 3.6rem minmax(4rem, 1fr) 5.6rem;
    align-items: center;
    gap: 0.5rem;
  }

  .label {
    font-size: 0.7rem;
    color: var(--text-dim);
  }

  .bar {
    position: relative;
    height: 0.55rem;
    border-radius: 0.25rem;
    background: var(--border);
  }

  /* Two numbers handed to CSS, as in `LevelMeter`: which edge the fill
     starts from is the origin's business, not a second calculation. The
     transition spans the 100 ms between status events, so it glides rather
     than steps. */
  .fill {
    position: absolute;
    top: 0;
    bottom: 0;
    left: calc(var(--from) * 100%);
    width: calc((var(--to) - var(--from)) * 100%);
    background: var(--accent);
    border-radius: inherit;
    transition:
      left 100ms linear,
      width 100ms linear;
  }

  .warn .fill {
    background: #f2b544;
  }

  .mark,
  .guide {
    position: absolute;
    top: 0;
    bottom: 0;
    left: calc(var(--at) * 100% - 1px);
    width: 2px;
  }

  .mark {
    background: var(--text);
    transition: left 100ms linear;
  }

  /* Past the bar's edges, so it reads over the fill as well as beside it. */
  .guide {
    top: -3px;
    bottom: -3px;
    width: 1px;
    left: calc(var(--at) * 100%);
    background: var(--text);
    opacity: 0.55;
  }

  .readout {
    font-size: 0.8rem;
    text-align: right;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .warn .readout {
    color: #f2b544;
  }
</style>

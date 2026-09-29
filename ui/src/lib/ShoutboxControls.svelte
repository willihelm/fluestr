<script lang="ts">
  // The shoutbox's hands and its curve (P26): mute, trim, what it reads, and
  // the curve it follows the room by.
  //
  // One component in two places: the Shoutbox section of the Settings page,
  // and the shoutbox window opened from the status bar — the part of the
  // setup that is touched during a show, in a window of its own beside the
  // DAW, like a plug-in's. The patch and the calibration stay on the Settings
  // page: they are set once, before anything plays.
  //
  // It edits the shoutbox it is given and no more. Where the edit goes from
  // there — the main window's save, or an event to it — is the caller's.

  import {
    setShoutboxLive,
    SHOUTBOX_GAIN_DB_MAX,
    SHOUTBOX_GAIN_DB_MIN,
    SHOUTBOX_TRIM_DB_MAX,
    SHOUTBOX_TRIM_DB_MIN,
    type ShoutboxStatus,
    type StoredShoutbox,
  } from "./bridge";
  import { curveTarget } from "./outputs";

  interface Props {
    shoutbox: StoredShoutbox;
    status: ShoutboxStatus | null;
  }

  let { shoutbox = $bindable(), status }: Props = $props();

  /// Trim and mute go to the running shoutbox at once, and into the profile
  /// for the save.
  const live = (trimDb: number, muted: boolean) => {
    shoutbox.trimDb = trimDb;
    shoutbox.muted = muted;
    void setShoutboxLive(trimDb, muted).catch(() => {});
  };

  let unit = $derived(shoutbox.calibrationDb != null ? "dB" : "dBFS");

  const fmt = (db: number | null | undefined, digits = 0) =>
    db === null || db === undefined || !Number.isFinite(db) ? "—" : db.toFixed(digits);

  // --- The curve, drawn ---------------------------------------------------

  // The plot sits inside margins that carry the axes: gain on the left, the
  // room along the bottom.
  const W = 340;
  const H = 190;
  const PAD = { left: 40, right: 10, top: 10, bottom: 36 };
  const plot = {
    left: PAD.left,
    right: W - PAD.right,
    top: PAD.top,
    bottom: H - PAD.bottom,
  };
  let span = $derived({
    from: shoutbox.quietDb - 20,
    to: shoutbox.quietDb + 50,
  });
  let gainSpan = $derived({
    from: Math.min(shoutbox.baseGainDb, shoutbox.maxGainDb) - 6,
    to: Math.max(shoutbox.baseGainDb, shoutbox.maxGainDb) + 6,
  });
  const x = (db: number) =>
    plot.left + ((db - span.from) / (span.to - span.from)) * (plot.right - plot.left);
  const y = (db: number) =>
    plot.bottom - ((db - gainSpan.from) / (gainSpan.to - gainSpan.from)) * (plot.bottom - plot.top);
  /// Whole multiples of `step` inside a span, for the tick labels.
  const ticks = (from: number, to: number, step: number) =>
    Array.from(
      { length: Math.floor(to / step) - Math.ceil(from / step) + 1 },
      (_, i) => (Math.ceil(from / step) + i) * step,
    );
  let roomTicks = $derived(ticks(span.from, span.to, 10));
  let gainTicks = $derived(
    ticks(gainSpan.from, gainSpan.to, gainSpan.to - gainSpan.from > 36 ? 12 : 6),
  );
  let path = $derived(
    Array.from({ length: 36 }, (_, i) => {
      const level = span.from + ((span.to - span.from) * i) / 35;
      return `${i === 0 ? "M" : "L"}${x(level).toFixed(1)},${y(curveTarget(shoutbox, level)).toFixed(1)}`;
    }).join(""),
  );
  let point = $derived(
    status?.state === "running" && status.levelDb != null
      ? {
          x: Math.min(plot.right, Math.max(plot.left, x(status.levelDb))),
          y: Math.min(plot.bottom, Math.max(plot.top, y(status.gainDb))),
        }
      : null,
  );
</script>

<!-- The hand on top of the automation. -->
<h3>Live</h3>
<div class="live">
  <button
    type="button"
    class="mute"
    class:on={shoutbox.muted}
    onclick={() => live(shoutbox.trimDb, !shoutbox.muted)}
    aria-pressed={shoutbox.muted}
  >
    {shoutbox.muted ? "Muted" : "Mute"}
  </button>
  <label class="trim">
    <span>Trim <output>{shoutbox.trimDb > 0 ? "+" : ""}{shoutbox.trimDb} dB</output></span>
    <input
      type="range"
      min={SHOUTBOX_TRIM_DB_MIN}
      max={SHOUTBOX_TRIM_DB_MAX}
      step="1"
      value={shoutbox.trimDb}
      oninput={(e) => live(Number(e.currentTarget.value), shoutbox.muted)}
    />
  </label>
</div>

{#if status?.state === "running"}
  <dl class="readings">
    <div>
      <dt>Room</dt>
      <dd>
        {fmt(status.liveDb)} {unit}
        {#if !status.referenceOk}
          <span class="warn">· mic lost, gain held</span>
        {:else if !status.measuring && status.levelDb != null}
          <span class="dim">· held at {fmt(status.levelDb)}</span>
        {/if}
      </dd>
    </div>
    <div>
      <dt>Gain</dt>
      <dd>{fmt(status.gainDb, 1)} dB <span class="dim">→ {fmt(status.targetDb, 1)}</span></dd>
    </div>
    <div>
      <dt>Program</dt>
      <dd>{fmt(status.programDbfs)} dBFS</dd>
    </div>
    <div>
      <dt>Limiter</dt>
      <dd class:warn={status.limitingDb < -0.5}>{fmt(status.limitingDb, 1)} dB</dd>
    </div>
    {#if status.underruns > 0 || status.skipped > 0}
      <div>
        <dt>Dropouts</dt>
        <dd class="warn">{status.underruns} / {status.skipped} skipped</dd>
      </div>
    {/if}
  </dl>
{/if}

<h3>Curve</h3>
<div class="curve">
  <figure>
    <svg
      viewBox="0 0 {W} {H}"
      width={W}
      height={H}
      role="img"
      aria-label="Shoutbox gain over room level"
    >
      {#each gainTicks as g (g)}
        <line x1={plot.left} x2={plot.right} y1={y(g)} y2={y(g)} class="grid" class:zero={g === 0} />
        <text x={plot.left - 5} y={y(g)} class="tick" text-anchor="end" dominant-baseline="middle">
          {g > 0 ? `+${g}` : g}
        </text>
      {/each}
      {#each roomTicks as r (r)}
        <text x={x(r)} y={plot.bottom + 12} class="tick" text-anchor="middle">{r}</text>
      {/each}
      <line x1={plot.left} x2={plot.right} y1={plot.bottom} y2={plot.bottom} class="axis" />
      <line x1={plot.left} x2={plot.left} y1={plot.top} y2={plot.bottom} class="axis" />
      <line
        x1={x(shoutbox.quietDb)}
        x2={x(shoutbox.quietDb)}
        y1={plot.top}
        y2={plot.bottom}
        class="guide"
      />
      <text x={x(shoutbox.quietDb) + 4} y={plot.top + 9} class="tick">quiet</text>
      <path d={path} class="line" />
      {#if point}
        <circle cx={point.x} cy={point.y} r="4" class="point" />
      {/if}
      <text x={(plot.left + plot.right) / 2} y={H - 4} class="axis-title" text-anchor="middle">
        Room level ({unit})
      </text>
      <text
        transform="translate(11 {(plot.top + plot.bottom) / 2}) rotate(-90)"
        class="axis-title"
        text-anchor="middle"
      >
        Shoutbox gain (dB)
      </text>
    </svg>
    <figcaption>
      <span class="key"><span class="swatch line-key"></span>The gain the shoutbox gets at each room level</span>
      <span class="key"><span class="swatch point-key"></span>Where it is now</span>
    </figcaption>
  </figure>
  <div class="fields">
    <label>
      <span>Quiet below ({unit})</span>
      <input type="number" bind:value={shoutbox.quietDb} />
    </label>
    <label>
      <span>Gain when quiet (dB)</span>
      <input
        type="number"
        min={SHOUTBOX_GAIN_DB_MIN}
        max={SHOUTBOX_GAIN_DB_MAX}
        bind:value={shoutbox.baseGainDb}
      />
    </label>
    <label>
      <span>dB of gain per dB of room (%)</span>
      <input type="number" min="0" max="200" step="5" bind:value={shoutbox.slopePercent} />
    </label>
    <label>
      <span>Most gain (dB)</span>
      <input
        type="number"
        min={SHOUTBOX_GAIN_DB_MIN}
        max={SHOUTBOX_GAIN_DB_MAX}
        bind:value={shoutbox.maxGainDb}
      />
    </label>
  </div>
</div>

<style>
  h3 {
    margin: 1rem 0 0.4rem;
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: 0.7rem;
    color: var(--text-dim);
    flex: 1 1 14rem;
    max-width: 24rem;
  }

  input[type="number"] {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.9rem;
  }

  input[type="range"] {
    accent-color: var(--accent);
  }

  button {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.7rem;
    font: inherit;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .live {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    margin: 0.6rem 0;
  }

  .mute {
    min-width: 5rem;
    font-weight: 600;
  }

  /* Lit when muted, in the colour a desk's mute is lit in. */
  .mute.on {
    background: #c0392b;
    border-color: #c0392b;
    color: #fff;
  }

  .trim {
    max-width: 20rem;
  }

  output {
    font-variant-numeric: tabular-nums;
  }

  .readings {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: minmax(0, 1fr);
    gap: 0.4rem 0.8rem;
    margin: 0.4rem 0 0;
  }

  .readings dt {
    font-size: 0.7rem;
    color: var(--text-dim);
  }

  .readings dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }

  .dim {
    color: var(--text-dim);
  }

  .warn {
    color: #f2b544;
  }

  .curve {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    align-items: flex-start;
  }

  .curve svg {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    max-width: 100%;
    height: auto;
  }

  .curve .fields {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    flex: 1 1 12rem;
  }

  /* The shared label's flex basis is a width in a row; in this column it
     would be a height, and every field 14rem tall. */
  .curve .fields label {
    flex: none;
  }

  .line {
    fill: none;
    stroke: var(--accent);
    stroke-width: 2;
  }

  .guide {
    stroke: var(--border);
    stroke-dasharray: 3 3;
  }

  .point {
    fill: #f2b544;
  }

  .curve figure {
    margin: 0;
    max-width: 100%;
  }

  .grid {
    stroke: var(--border);
    stroke-opacity: 0.5;
  }

  .grid.zero {
    stroke-opacity: 1;
  }

  .axis {
    stroke: var(--text-dim);
  }

  .tick {
    fill: var(--text-dim);
    font-size: 9px;
  }

  .axis-title {
    fill: var(--text-dim);
    font-size: 10px;
  }

  .curve figcaption {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem 1rem;
    margin-top: 0.3rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .key {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }

  .swatch {
    display: inline-block;
  }

  .line-key {
    width: 14px;
    height: 2px;
    background: var(--accent);
  }

  .point-key {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #f2b544;
  }

</style>

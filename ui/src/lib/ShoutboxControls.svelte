<script lang="ts">
  // The shoutbox's hands and its curve (P26): mute, trim, what it reads, the
  // curve it follows the room by, and the gate and limiter behind it.
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
    DEFAULT_OUTPUTS,
    setShoutboxLive,
    SHOUTBOX_CEILING_DBFS_MAX,
    SHOUTBOX_CEILING_DBFS_MIN,
    SHOUTBOX_GAIN_DB_MAX,
    SHOUTBOX_GAIN_DB_MIN,
    SHOUTBOX_GATE_DBFS_MAX,
    SHOUTBOX_GATE_DBFS_MIN,
    SHOUTBOX_HOLD_MS_MAX,
    SHOUTBOX_HOLD_MS_MIN,
    SHOUTBOX_TRIM_DB_MAX,
    SHOUTBOX_TRIM_DB_MIN,
    type ShoutboxStatus,
    type StoredShoutbox,
  } from "./bridge";
  import { curveTarget, offsetOf } from "./outputs";
  import Knob from "./Knob.svelte";
  import ShoutboxMeter from "./ShoutboxMeter.svelte";

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

  /// The room is always read in dB SPL; without a calibration the numbers
  /// are an estimate, and say so.
  const unit = "dB SPL";
  let estimated = $derived(shoutbox.calibrationDb == null);

  /// What a double-tap on a knob goes back to.
  const defaults = DEFAULT_OUTPUTS.shoutbox;

  /// The quiet threshold's knob, over what the mic can read: its weighted
  /// dBFS scale moved by the offset in force, measured or estimated. Never
  /// narrower than the value it holds, or the knob would show it wrongly.
  let quietRange = $derived.by(() => {
    const offset = offsetOf(shoutbox.calibrationDb);
    const [from, to] = [Math.round(offset - 90), Math.round(offset)];
    return { min: Math.min(from, shoutbox.quietDb), max: Math.max(to, shoutbox.quietDb) };
  });

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
  /// The room meter's whole story, which its readout has no room for.
  let roomTitle = $derived.by(() => {
    if (!status) return "";
    const now = `Room ${fmt(status.liveDb)} ${unit}${estimated ? " (estimated)" : ""}`;
    if (!status.referenceOk) return `${now}. The measurement mic reads as lost; the level is held.`;
    if (!status.measuring && status.levelDb != null)
      return `${now}, the shoutbox included. Held at ${fmt(status.levelDb)} (the mark) while it plays.`;
    return `${now}. The dim mark is the quiet threshold.`;
  });

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
  <Knob
    label="Trim"
    hint="Your hand on top of the automation: added to whatever gain the curve sets. Takes effect at once."
    unit="dB"
    value={shoutbox.trimDb}
    min={SHOUTBOX_TRIM_DB_MIN}
    max={SHOUTBOX_TRIM_DB_MAX}
    origin={0}
    reset={0}
    onchange={(db) => live(db, shoutbox.muted)}
  />
  {#if status?.state === "running"}
    <div class="readings">
      <ShoutboxMeter
        label="Room"
        value={status.liveDb}
        min={span.from}
        max={span.to}
        guide={shoutbox.quietDb}
        mark={status.measuring ? null : status.levelDb}
        readout={status.referenceOk ? `${estimated ? "≈ " : ""}${fmt(status.liveDb)} ${unit}` : "mic lost"}
        warn={!status.referenceOk}
        title={roomTitle}
      />
      <ShoutboxMeter
        label="Gain"
        value={status.gainDb}
        min={gainSpan.from}
        max={gainSpan.to}
        mark={status.targetDb}
        readout="{fmt(status.gainDb, 1)} dB"
        title="Gain {fmt(status.gainDb, 1)} dB, on its way to {fmt(status.targetDb, 1)} dB (the mark)"
      />
      <ShoutboxMeter
        label="Program"
        value={status.programDbfs}
        min={-60}
        max={0}
        guide={shoutbox.gateDbfs}
        readout="{fmt(status.programDbfs)} dBFS"
        title="Intercom peak before the gain; the mark is Room Hold Above: louder than it, the room level is held"
      />
      <ShoutboxMeter
        label="Limiter"
        value={status.limitingDb}
        min={-12}
        max={0}
        origin={0}
        readout="{fmt(status.limitingDb, 1)} dB"
        warn={status.limitingDb < -0.5}
        title="How far the limiter is holding the output down"
      />
      {#if status.underruns > 0 || status.skipped > 0}
        <p class="dropouts">Dropouts {status.underruns} / {status.skipped} skipped</p>
      {/if}
    </div>
  {/if}
</div>

<div class="curve">
  <div class="layout">
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
          Room level ({estimated ? "≈ " : ""}{unit})
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
        <span class="key"><span class="swatch line-key"></span>Gain target curve</span>
        <span class="key"><span class="swatch point-key"></span>Current value</span>
      </figcaption>
    </figure>
    <div class="fields">
      <Knob
        label="Quiet below"
        hint="Below this room level the shoutbox stays at Gain when quiet. Above it, the gain starts to follow the room."
        {unit}
        value={shoutbox.quietDb}
        min={quietRange.min}
        max={quietRange.max}
        reset={defaults.quietDb}
        onchange={(v) => (shoutbox.quietDb = v)}
      />
      <Knob
        label="Gain when quiet"
        hint="The shoutbox's gain in a quiet room: at and below Quiet below."
        unit="dB"
        value={shoutbox.baseGainDb}
        min={SHOUTBOX_GAIN_DB_MIN}
        max={SHOUTBOX_GAIN_DB_MAX}
        origin={0}
        reset={defaults.baseGainDb}
        onchange={(v) => (shoutbox.baseGainDb = v)}
      />
      <Knob
        label="Rise"
        hint="How long the gain takes to rise by 10 dB on its way to the target. Slower rises are less noticeable; faster ones keep up with a sudden loud song."
        unit="s per 10 dB"
        value={shoutbox.riseMs / 1000}
        min={0.5}
        max={30}
        step={0.5}
        reset={defaults.riseMs / 1000}
        onchange={(s) => (shoutbox.riseMs = Math.round(s * 1000))}
      />
      <Knob
        label="Gain per dB of room"
        hint="How closely the gain follows the room above Quiet below. 100 % keeps the shoutbox a fixed distance above the room; 50 % adds 1 dB of gain for every 2 dB of room."
        unit="%"
        value={shoutbox.slopePercent}
        min={0}
        max={200}
        step={5}
        reset={defaults.slopePercent}
        onchange={(v) => (shoutbox.slopePercent = v)}
      />
      <Knob
        label="Most gain"
        hint="The most gain the curve will ever ask for, however loud the room. The Limit Ceiling still applies after it."
        unit="dB"
        value={shoutbox.maxGainDb}
        min={SHOUTBOX_GAIN_DB_MIN}
        max={SHOUTBOX_GAIN_DB_MAX}
        origin={0}
        reset={defaults.maxGainDb}
        onchange={(v) => (shoutbox.maxGainDb = v)}
      />
      <Knob
        label="Fall"
        hint="How long the gain takes to fall by 10 dB on its way to the target. 0 is no limit, leaving only the room averaging to slow it."
        unit="s per 10 dB"
        value={shoutbox.fallMs / 1000}
        min={0}
        step={0.5}
        max={Math.max(30, shoutbox.fallMs / 1000)}
        reset={defaults.fallMs / 1000}
        onchange={(s) => (shoutbox.fallMs = Math.round(s * 1000))}
      />
    </div>
    <div class="safety">
      <Knob
        label="Room Hold Above"
        hint="When the intercom program is louder than this, the shoutbox counts as talking. The mic would hear the shoutbox itself, so the room level is held instead of measured. The gain keeps moving towards what the held level asks for. The mark on the Program meter."
        unit="dBFS"
        value={shoutbox.gateDbfs}
        min={SHOUTBOX_GATE_DBFS_MIN}
        max={SHOUTBOX_GATE_DBFS_MAX}
        reset={defaults.gateDbfs}
        onchange={(v) => (shoutbox.gateDbfs = v)}
      />
      <Knob
        label="Room Hold Time"
        hint="How long the room level stays held after the program drops below Room Hold Above: the tail of the last word and the room's reverb of it."
        unit="ms"
        value={shoutbox.gateHoldMs}
        min={SHOUTBOX_HOLD_MS_MIN}
        max={SHOUTBOX_HOLD_MS_MAX}
        step={100}
        reset={defaults.gateHoldMs}
        onchange={(v) => (shoutbox.gateHoldMs = v)}
      />
      <Knob
        label="Limit Ceiling"
        hint="A limiter behind everything: nothing leaves the shoutbox above this level, whatever the curve and the trim ask for. Takes effect at once."
        unit="dBFS"
        value={shoutbox.ceilingDbfs}
        min={SHOUTBOX_CEILING_DBFS_MIN}
        max={SHOUTBOX_CEILING_DBFS_MAX}
        reset={defaults.ceilingDbfs}
        onchange={(v) => (shoutbox.ceilingDbfs = v)}
      />
    </div>
  </div>
</div>


<style>
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

  /* Mute, trim and the readings in one row: the part watched during a show
     takes one knob's height, and the curve starts right below it. */
  .live {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.8rem;
    margin: 0.6rem 0;
  }

  /* A finger's size: this is pressed on a touchscreen as often as clicked. */
  .mute {
    min-width: 5.5rem;
    min-height: 3rem;
    font-weight: 600;
  }

  /* Lit when muted, in the colour a desk's mute is lit in. */
  .mute.on {
    background: var(--channel-mute);
    border-color: var(--channel-mute);
    color: color-mix(in oklch, var(--channel-mute), black 72%);
    /* Lit like an LED key, with a little of its own light around it. */
    box-shadow: 0 0 0.6rem color-mix(in oklch, var(--channel-mute) 45%, transparent);
  }

  /* Four meters beside the trim, in the knob's height. The ranges are the
     curve's own where there is one, so a meter and the plot below agree. */
  .readings {
    flex: 1 1 16rem;
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
  }

  .dropouts {
    margin: 0;
    font-size: 0.75rem;
    color: var(--warn);
  }

  /* No headings over the two blocks: the meters and the plot say what they
     are, and the window's height goes to them instead. */
  /* The plot with the curve's knobs beside it and the safety three under
     it; when there is no room beside, all three stacked — plot, curve
     knobs, safety — so the knobs that shape the curve stay next to it.
     Queried on this block's own width rather than the window's: the
     Settings page gives it less of the window than the shoutbox's own
     does. 43rem is the plot and the three knob columns side by side, and a
     little to spare. */
  .curve {
    margin-top: 1rem;
    container-type: inline-size;
  }

  .layout {
    display: grid;
    grid-template-areas: "plot" "fields" "safety";
    justify-content: start;
    align-items: start;
    gap: 1rem;
  }

  @container (min-width: 43rem) {
    .layout {
      grid-template-columns: max-content max-content;
      grid-template-areas:
        "plot fields"
        "safety fields";
    }
  }

  .layout figure {
    grid-area: plot;
  }

  .curve svg {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    max-width: 100%;
    height: auto;
  }

  /* The curve's four beside it, and the two speeds in a third column: all
     of it the room-following, in one block. */
  .curve .fields {
    grid-area: fields;
    display: grid;
    grid-template-columns: repeat(3, max-content);
    gap: 0.4rem 0.8rem;
    align-content: start;
    /* Knobs and readouts level across a row even when a label wraps. */
    align-items: end;
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
    fill: var(--warn);
  }

  /* The gate, its hold and the ceiling: what keeps the shoutbox from
     hearing itself and from leaving too loud, apart from the knobs that
     shape the curve. */
  .safety {
    grid-area: safety;
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 0.8rem;
    align-items: flex-end;
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
    background: var(--warn);
  }

</style>

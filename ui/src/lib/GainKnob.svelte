<script lang="ts">
  // The input trim of one channel, as a knob.
  //
  // A knob and not a number box, because of how it is used: an operator turns
  // it while watching the meter beside it and listening to whether the channel
  // starts producing text. That is a continuous act with a value nobody reads
  // — the number is only ever consulted afterwards, to write the setup down —
  // so the number is there, under the knob, and the knob is what is grabbed.
  //
  // Dragged vertically rather than rotationally. A rotary drag is what the
  // shape suggests, but on a 2.2 rem control it means a pointer travelling a
  // few pixels around a centre it has to keep track of, and it breaks entirely
  // the moment the pointer passes over the middle. Up is more, down is less,
  // which is also what the arrow keys do.

  import { GAIN_DB_MIN, GAIN_DB_MAX } from "./generated/constants";
  import { clampGain } from "./streams";

  interface Props {
    /// The trim in whole decibels. Read, never written back: the owner of the
    /// setup applies the change, because it is also the one that has to store
    /// it and push it into the running session.
    value: number;
    /// What the knob is the trim *of*, for the screen reader — the channel's
    /// label. The visible name is beside it and would be read twice otherwise.
    label: string;
    disabled?: boolean;
    /// Called with every value the drag passes through, not only the one it
    /// ends on. Turning a trim is done by ear against the meter, and a knob
    /// whose effect arrives on release cannot be set that way.
    onchange: (db: number) => void;
  }

  let { value, label, disabled = false, onchange }: Props = $props();

  /// How far the knob turns, in degrees either side of straight up. 135 is the
  /// familiar amount: the gap at the bottom is what makes the pointer's
  /// position readable at a glance rather than ambiguous at the ends.
  const SWEEP = 135;

  /// Pixels of drag per decibel. The whole range in about two finger-lengths,
  /// which is coarse enough to find 0 dB again without the pointer leaving the
  /// window.
  const PX_PER_DB = 4;

  const angleOf = (db: number): number => {
    const span = GAIN_DB_MAX - GAIN_DB_MIN;
    return ((db - GAIN_DB_MIN) / span) * (SWEEP * 2) - SWEEP;
  };

  /// Where the value arc starts. Unity rather than the minimum, because that
  /// is what the arc is about: a trim reads as how far it is from doing
  /// nothing, in which direction — not as a level.
  const unity = angleOf(0);

  /// A point on the knob's face, in the 0–100 box the SVG is drawn in.
  const point = (degrees: number, radius: number) => {
    const rad = ((degrees - 90) * Math.PI) / 180;
    return [50 + radius * Math.cos(rad), 50 + radius * Math.sin(rad)];
  };

  /// One arc of the ring, as an SVG path. `large` is the flag SVG needs to
  /// know which way round to go; our sweep never exceeds 270°, so it is only
  /// ever set by the arc running past a half turn.
  const arc = (from: number, to: number, radius: number): string => {
    const [x1, y1] = point(from, radius);
    const [x2, y2] = point(to, radius);
    const large = Math.abs(to - from) > 180 ? 1 : 0;
    const clockwise = to > from ? 1 : 0;
    return `M ${x1} ${y1} A ${radius} ${radius} 0 ${large} ${clockwise} ${x2} ${y2}`;
  };

  let current = $derived(angleOf(clampGain(value)));

  const set = (db: number) => {
    const next = clampGain(db);
    if (next !== value) onchange(next);
  };

  // Where the drag started, in pixels and in decibels. Kept rather than
  // accumulated per move event: adding up deltas loses the fraction of a
  // decibel each rounding throws away, and a slow drag across the whole range
  // would then arrive somewhere other than where the pointer is.
  let from: { y: number; db: number } | null = null;

  const down = (event: PointerEvent) => {
    if (disabled) return;
    from = { y: event.clientY, db: clampGain(value) };
    // So the drag survives the pointer leaving a 2.2 rem circle, which it
    // does immediately — the travel is vertical and the knob is not tall.
    event.currentTarget instanceof Element &&
      event.currentTarget.setPointerCapture(event.pointerId);
  };

  const move = (event: PointerEvent) => {
    if (from === null) return;
    // Up is more: the screen's y axis points the other way.
    set(from.db + Math.round((from.y - event.clientY) / PX_PER_DB));
  };

  const up = (event: PointerEvent) => {
    from = null;
    event.currentTarget instanceof Element &&
      event.currentTarget.releasePointerCapture(event.pointerId);
  };

  const key = (event: KeyboardEvent) => {
    if (disabled) return;
    const step =
      event.key === "ArrowUp" || event.key === "ArrowRight"
        ? 1
        : event.key === "ArrowDown" || event.key === "ArrowLeft"
          ? -1
          : event.key === "PageUp"
            ? 6
            : event.key === "PageDown"
              ? -6
              : 0;

    if (step !== 0) {
      event.preventDefault();
      set(value + step);
      return;
    }

    // Home is the way back to doing nothing — the same thing the double click
    // does, for the keyboard. Not "the minimum", which is a trim setting
    // nobody wants to land on by pressing one key.
    if (event.key === "Home") {
      event.preventDefault();
      set(0);
    }
  };
</script>

<!-- `role="slider"`: it is one, and announcing it as anything else would cost
     the arrow keys their meaning to anybody using the knob without seeing it.
     The unit is in `aria-valuetext`, because "minus six" is not what this is
     set to. -->
<div
  class="knob"
  class:off={value === 0}
  class:disabled
  role="slider"
  tabindex={disabled ? -1 : 0}
  aria-label="Input gain, {label}"
  aria-valuemin={GAIN_DB_MIN}
  aria-valuemax={GAIN_DB_MAX}
  aria-valuenow={value}
  aria-valuetext="{value > 0 ? '+' : ''}{value} dB"
  aria-disabled={disabled}
  title="Input gain for {label} — drag, or use the arrow keys. Double-click for 0 dB."
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
  ondblclick={() => !disabled && set(0)}
  onkeydown={key}
>
  <svg viewBox="0 0 100 100" aria-hidden="true">
    <path class="track" d={arc(-SWEEP, SWEEP, 40)} />
    <!-- Drawn only when there is something to draw: an arc of zero length
         renders as a dot at the top on some engines, which reads as a setting
         rather than as the absence of one. -->
    {#if value !== 0}
      <path class="value" d={arc(unity, current, 40)} />
    {/if}
    <!-- The pointer, stopping short of the ring: overlapping it would make
         the two hardest to tell apart exactly where the value is read. -->
    <line
      class="pointer"
      x1={point(current, 8)[0]}
      y1={point(current, 8)[1]}
      x2={point(current, 31)[0]}
      y2={point(current, 31)[1]}
    />
  </svg>
  <span class="db">{value > 0 ? "+" : ""}{value}</span>
</div>

<style>
  .knob {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.05rem;
    width: 2.2rem;
    /* Not `pointer`: the knob is dragged, not clicked, and the cursor should
       say which way. */
    cursor: ns-resize;
    touch-action: none;
    user-select: none;
    color: var(--text-dim);
  }

  .knob:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    border-radius: 0.4rem;
  }

  .knob.disabled {
    opacity: 0.45;
    cursor: default;
  }

  svg {
    width: 1.85rem;
    height: 1.85rem;
    overflow: visible;
  }

  /* Lighter than `--border`, which is meant for edges between surfaces: an
     unlit track at that contrast disappears, and a knob whose range cannot be
     seen cannot be read at a glance either. */
  .track {
    fill: none;
    stroke: color-mix(in srgb, var(--text) 18%, transparent);
    stroke-width: 7;
    stroke-linecap: round;
  }

  .value {
    fill: none;
    stroke: var(--accent);
    stroke-width: 7;
    stroke-linecap: round;
  }

  .pointer {
    stroke: var(--text);
    stroke-width: 6;
    stroke-linecap: round;
  }

  /* A knob doing nothing says so quietly. Eight channels at unity is the
     normal state of a setup, and eight bright numbers would make the one
     channel that *is* trimmed harder to find rather than easier. */
  .knob.off .db {
    opacity: 0.5;
  }

  .db {
    font-size: 0.6rem;
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }
</style>

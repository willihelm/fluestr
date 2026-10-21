<script lang="ts">
  // A knob for one of the shoutbox's numbers (P26), sized for a finger.
  //
  // `GainKnob`'s manner — dragged vertically, up is more, an arc from the
  // value that does nothing — with the range, the step and the unit given
  // rather than fixed, and big enough to be turned on a touchscreen beside
  // the desk. The number is under it, the name over it: on a touchscreen there
  // is no hover to find out what a knob is.
  //
  // Double-tap, double-click or Alt-click goes back to `reset`; the arrow
  // keys as on any slider. Not the wheel: the knobs sit on a page that is
  // scrolled, and a scroll that passes over one must not turn it.

  interface Props {
    value: number;
    min: number;
    max: number;
    step?: number;
    /// Where the arc starts: the value that does nothing, if there is one.
    origin?: number;
    /// What a double-tap and Home go back to.
    reset: number;
    /// The visible name, and what a screen reader calls it.
    label: string;
    /// What the knob does, for the tooltip: the label has room for a name
    /// and not for what the number means.
    hint?: string;
    unit: string;
    /// Called with every value the drag passes through, so the change is
    /// heard while the knob is turned.
    onchange: (value: number) => void;
  }

  let { value, min, max, step = 1, origin, reset, label, hint, unit, onchange }: Props = $props();

  const SWEEP = 135;
  /// The whole range in about this many pixels of drag, whatever its size:
  /// a finger's travel, without leaving the knob's column.
  const TRAVEL_PX = 220;
  /// Two taps closer than this, and not moved between, are a double-tap.
  const DOUBLE_TAP_MS = 320;

  const clamp = (v: number) =>
    Math.min(max, Math.max(min, Math.round((v - min) / step) * step + min));

  const angleOf = (v: number) => ((clamp(v) - min) / (max - min)) * (SWEEP * 2) - SWEEP;

  const point = (degrees: number, radius: number) => {
    const rad = ((degrees - 90) * Math.PI) / 180;
    return [50 + radius * Math.cos(rad), 50 + radius * Math.sin(rad)];
  };

  const arc = (from: number, to: number, radius: number): string => {
    const [x1, y1] = point(from, radius);
    const [x2, y2] = point(to, radius);
    const large = Math.abs(to - from) > 180 ? 1 : 0;
    const clockwise = to > from ? 1 : 0;
    return `M ${x1} ${y1} A ${radius} ${radius} 0 ${large} ${clockwise} ${x2} ${y2}`;
  };

  let current = $derived(angleOf(value));
  let start = $derived(angleOf(origin ?? min));
  let pxPerStep = $derived(TRAVEL_PX / ((max - min) / step));

  const set = (v: number) => {
    const next = clamp(v);
    if (next !== value) onchange(next);
  };

  // Where the drag started, kept rather than accumulated — see `GainKnob`.
  let from: { y: number; value: number } | null = null;
  let moved = false;
  let lastTap = 0;

  const down = (event: PointerEvent) => {
    // Alt-click back to the default, as in a DAW. No drag follows it.
    if (event.altKey) {
      event.preventDefault();
      set(reset);
      return;
    }
    from = { y: event.clientY, value };
    moved = false;
    // Or the press starts a text selection, and the drag stretches it over
    // everything the pointer crosses. It also stops the press focusing the
    // knob, so that is done by hand.
    event.preventDefault();
    if (event.currentTarget instanceof HTMLElement) {
      event.currentTarget.focus();
      event.currentTarget.setPointerCapture(event.pointerId);
    }
  };

  const move = (event: PointerEvent) => {
    if (from === null) return;
    const steps = Math.round((from.y - event.clientY) / pxPerStep);
    if (steps !== 0) moved = true;
    set(from.value + steps * step);
  };

  const up = (event: PointerEvent) => {
    from = null;
    event.currentTarget instanceof Element &&
      event.currentTarget.releasePointerCapture(event.pointerId);
    if (moved) {
      lastTap = 0;
      return;
    }
    // A double-tap by hand: a touchscreen sends no `dblclick` to an element
    // that has taken the gestures over with `touch-action: none`.
    if (event.timeStamp - lastTap < DOUBLE_TAP_MS) {
      lastTap = 0;
      set(reset);
    } else {
      lastTap = event.timeStamp;
    }
  };

  const key = (event: KeyboardEvent) => {
    const delta =
      event.key === "ArrowUp" || event.key === "ArrowRight"
        ? step
        : event.key === "ArrowDown" || event.key === "ArrowLeft"
          ? -step
          : event.key === "PageUp"
            ? step * 5
            : event.key === "PageDown"
              ? -step * 5
              : 0;
    if (delta !== 0) {
      event.preventDefault();
      set(value + delta);
    } else if (event.key === "Home") {
      event.preventDefault();
      set(reset);
    }
  };

  /// Signed only around a value that does nothing — a gain above unity is
  /// "+6". A speed or a percentage from the bottom of its range is not.
  let text = $derived(`${origin !== undefined && value > origin ? "+" : ""}${value}`);
</script>

<div
  class="knob"
  role="slider"
  tabindex="0"
  aria-label={label}
  aria-valuemin={min}
  aria-valuemax={max}
  aria-valuenow={value}
  aria-valuetext="{text} {unit}"
  title={`${label}${hint ? `: ${hint}` : ""}\n\nDrag up or down. Double-tap or Alt-click for ${reset} ${unit}.`}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
  onkeydown={key}
>
  <span class="label">{label}</span>
  <svg viewBox="0 0 100 100" aria-hidden="true">
    <circle class="face" cx="50" cy="50" r="30" />
    <path class="track" d={arc(-SWEEP, SWEEP, 42)} />
    {#if current !== start}
      <path class="value" d={arc(start, current, 42)} />
    {/if}
    <line
      class="pointer"
      x1={point(current, 10)[0]}
      y1={point(current, 10)[1]}
      x2={point(current, 26)[0]}
      y2={point(current, 26)[1]}
    />
  </svg>
  <span class="readout">{text} <span class="unit">{unit}</span></span>
</div>

<style>
  .knob {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.2rem;
    min-width: 5.5rem;
    padding: 0.3rem 0.2rem;
    border-radius: 0.5rem;
    cursor: ns-resize;
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
    color: var(--text-dim);
  }

  .knob:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .label {
    font-size: 0.7rem;
    text-align: center;
    line-height: 1.2;
    max-width: 7rem;
  }

  svg {
    width: 4rem;
    height: 4rem;
    overflow: visible;
  }

  .face {
    fill: var(--surface);
    stroke: var(--border);
    stroke-width: 1.5;
  }

  /* Lighter than `--border`, for `GainKnob`'s reason. */
  .track {
    fill: none;
    stroke: color-mix(in srgb, var(--text) 18%, transparent);
    stroke-width: 6;
    stroke-linecap: round;
  }

  .value {
    fill: none;
    stroke: var(--accent);
    stroke-width: 6;
    stroke-linecap: round;
  }

  .pointer {
    stroke: var(--text);
    stroke-width: 5;
    stroke-linecap: round;
  }

  .readout {
    font-size: 0.9rem;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }

  .unit {
    font-size: 0.7rem;
    color: var(--text-dim);
  }
</style>

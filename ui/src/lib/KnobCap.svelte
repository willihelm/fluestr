<svelte:options namespace="svg" />

<script lang="ts">
  // The cap both knobs are turned by: brushed aluminium in a dark bezel, with
  // an engraved line that turns with the value. audiocn's "mini" cap
  // (github.com/audiocn/ui), drawn for the 100 × 100 view box `Knob` and
  // `GainKnob` share, inside their tracks.
  //
  // A cap rather than a flat disc because it is what a hand reaches for: on a
  // screen full of bars and numbers, the things that turn should look like
  // things that turn. The light comes from above, as in the room — the bezel
  // brightens at its top edge, the rim at its bottom, and the groove of the
  // pointer catches it on its lower wall.
  //
  // No grain filter, which audiocn adds: eight of these sit in the meter
  // strip and one `feTurbulence` each is the cost of a texture nobody sees
  // at 1.85 rem.

  interface Props {
    /// Where the pointer is, in degrees clockwise from straight up.
    angle: number;
  }

  let { angle }: Props = $props();

  const uid = $props.id();
  const id = `knob-cap-${uid}`;

  /// Radii, in view box units: the shadow under the cap, the bezel it sits
  /// in, the bright rim, and the brushed face.
  const HALO = 36;
  const BEZEL = 32;
  const RIM = 29.2;
  const FACE = 28.2;
  /// The engraved pointer, from near the centre to near the edge of the face.
  const LINE = { inner: 8.5, outer: 23, width: 3.4, lip: 0.6 };

  const point = (degrees: number, radius: number) => {
    const rad = ((degrees - 90) * Math.PI) / 180;
    return { x: 50 + radius * Math.cos(rad), y: 50 + radius * Math.sin(rad) };
  };

  let inner = $derived(point(angle, LINE.inner));
  let outer = $derived(point(angle, LINE.outer));
</script>

<g class="cap">
  <defs>
    <radialGradient id="{id}-halo">
      <stop offset="0.8" stop-color="#000" stop-opacity="0.55" />
      <stop offset="1" stop-color="#000" stop-opacity="0" />
    </radialGradient>
    <linearGradient id="{id}-bezel" x1="0" x2="0" y1="0" y2="1">
      <stop offset="0" stop-color="#fff" stop-opacity="0.16" />
      <stop offset="1" stop-color="#fff" stop-opacity="0" />
    </linearGradient>
    <linearGradient id="{id}-rim" x1="0" x2="0" y1="0" y2="1">
      <stop offset="0" stop-color="#000" stop-opacity="0.62" />
      <stop offset="1" stop-color="#000" stop-opacity="0.08" />
    </linearGradient>
  </defs>
  <circle cx="50" cy="50" r={HALO} fill="url(#{id}-halo)" />
  <circle class="bezel" cx="50" cy="50" r={BEZEL} />
  <circle cx="50" cy="50" r={BEZEL} fill="url(#{id}-bezel)" />
  <circle class="rim" cx="50" cy="50" r={RIM} />
  <circle cx="50" cy="50" r={RIM} fill="url(#{id}-rim)" />
  <!-- SVG has no conic gradient, and brushed metal is one: the face is HTML
       with a CSS one, as in audiocn. -->
  <foreignObject x={50 - FACE} y={50 - FACE} width={FACE * 2} height={FACE * 2}>
    <div class="face" xmlns="http://www.w3.org/1999/xhtml"></div>
  </foreignObject>
  <line
    class="lip"
    x1={inner.x}
    y1={inner.y + LINE.lip}
    x2={outer.x}
    y2={outer.y + LINE.lip}
    stroke-width={LINE.width}
  />
  <line
    class="groove"
    x1={inner.x}
    y1={inner.y}
    x2={outer.x}
    y2={outer.y}
    stroke-width={LINE.width}
  />
</g>

<style>
  .cap {
    --metal: #fff;
    --shade: #000;
    pointer-events: none;
  }

  .bezel {
    fill: color-mix(in oklab, var(--shade) 85%, transparent);
    stroke: color-mix(in oklab, var(--metal) 15%, transparent);
    stroke-width: 0.4;
  }

  .rim {
    fill: var(--metal);
  }

  .face {
    --brush: color-mix(in oklab, var(--shade) 7%, transparent);
    --sheen: color-mix(in oklab, var(--shade) 34%, transparent);
    --sheen-deep: color-mix(in oklab, var(--shade) 50%, transparent);
    --pitch: 0.9px;
    width: 100%;
    height: 100%;
    border-radius: 50%;
    background-color: var(--metal);
    background-image:
      repeating-radial-gradient(
        circle,
        var(--brush) 0 var(--pitch),
        transparent var(--pitch) calc(var(--pitch) * 2)
      ),
      conic-gradient(
        from 15deg,
        var(--sheen),
        transparent 9%,
        var(--sheen) 21%,
        transparent 32%,
        var(--sheen-deep) 46%,
        transparent 58%,
        var(--sheen) 70%,
        transparent 83%,
        var(--sheen)
      );
  }

  .lip,
  .groove {
    stroke-linecap: round;
  }

  .lip {
    stroke: color-mix(in oklab, var(--metal) 70%, transparent);
  }

  .groove {
    stroke: color-mix(in oklab, var(--shade) 80%, transparent);
  }
</style>

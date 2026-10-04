<script lang="ts">
  // Provisional stand-in for P8. It answers exactly one question — "is there
  // signal on this channel at all?" — which is the first thing to check when
  // no text appears. Peak hold, clipping and per-stream health come later.

  interface Props {
    rms: number;
    peak: number;
    dropped: number;
    /// A column that fills upwards instead of a row that fills rightwards.
    ///
    /// What the narrow layout uses: eight horizontal meters are eight lines
    /// stacked above the chat, and on a phone that is the chat gone. Eight
    /// columns are one strip the height of a single row. See `StreamMeters`,
    /// which decides when — this component only draws what it is told to.
    vertical?: boolean;
  }

  let { rms, peak, dropped, vertical = false }: Props = $props();

  // Linear amplitude is useless as a bar: normal speech sits below 0.1 and
  // would barely move. dBFS mapped over a 60 dB range matches what an engineer
  // reads on a desk.
  const toBar = (amplitude: number) => {
    if (amplitude <= 0) return 0;
    const db = 20 * Math.log10(amplitude);
    return Math.max(0, Math.min(1, (db + 60) / 60));
  };

  let level = $derived(toBar(rms));
  let peakMark = $derived(toBar(peak));
  let clipping = $derived(peak >= 0.99);
</script>

<!-- The two readings are handed to CSS as numbers rather than as a `width`,
     because which edge they grow from is the layout's business and not this
     script's: the same pair drives a width in a row and a height in a column,
     and a component that wrote `width:` here would need a second copy of both
     calculations to be able to turn. -->
<div
  class="meter"
  class:clipping
  class:vertical
  style="--level: {level}; --peak: {peakMark}"
  title="RMS {rms.toFixed(3)} · peak {peak.toFixed(3)}"
>
  <div class="fill"></div>
  <div class="peak"></div>
</div>
{#if dropped > 0}
  <span
    class="dropped"
    class:vertical
    title="Samples lost because the processing thread fell behind: {dropped}"
  >
    <!-- The figure survives into the column; the word it is a count of does
         not. A 3 rem column has room for "412" and none for "412 dropped",
         and the whole sentence is in the tooltip either way. -->
    {dropped}<span class="word">&nbsp;dropped</span>
  </span>
{/if}

<style>
  /* audiocn's LED ladder. The whole track is lit in the zone colours a desk
     uses — green to -18 dB, yellow to -6, red above — and the level uncovers
     as much of it as it reaches, so the colour of the top of the bar is the
     colour of the zone it is in rather than one colour that changes all at
     once. The unlit segments are drawn faintly, so the ladder and where its
     zones begin are visible on a silent channel too.

     On the 60 dB scale above, -18 dB is 0.7 of the way up and -6 is 0.9. RMS
     rather than peak is what fills the bar, which is why the zones sit lower
     than on a peak meter: speech that fills it into the yellow is loud. */
  .meter {
    --zones: linear-gradient(
      var(--toward, to right),
      var(--ok) 0 70%,
      var(--warn) 70% 90%,
      var(--danger) 90% 100%
    );
    /* Segments 4 px long with a 1.5 px gap — fine enough that the bar still
       moves smoothly, coarse enough to read as LEDs. */
    --segments: repeating-linear-gradient(
      var(--toward, to right),
      #000 0 4px,
      transparent 4px 5.5px
    );
    position: relative;
    height: 0.5rem;
    border-radius: 999px;
    background: var(--muted);
    overflow: hidden;
    flex: 1;
    min-width: 6rem;
  }

  .meter::before,
  .fill {
    position: absolute;
    inset: 0;
    background: var(--zones);
    -webkit-mask: var(--segments);
    mask: var(--segments);
  }

  .meter::before {
    content: "";
    opacity: 0.18;
  }

  /* Uncovered rather than stretched, so the gradient stays put under it. */
  .fill {
    clip-path: inset(0 calc((1 - var(--level)) * 100%) 0 0);
    transition: clip-path 60ms linear;
  }

  .peak {
    position: absolute;
    top: 0;
    left: calc(var(--peak) * 100% - 1px);
    width: 2px;
    height: 100%;
    background: var(--text);
    border-radius: 1px;
  }

  /* Clipped: the hold mark turns red and glows, the light a desk's clip LED
     gives. The ladder is already red at the top on its own. */
  .clipping .peak {
    background: var(--danger);
    box-shadow: 0 0 6px var(--danger);
  }

  /* Truncated rather than allowed to overrun the row it is in — the sentence
     is in the tooltip. See `StreamMeters`, which does the same with its own
     count for the same reason. */
  .dropped {
    color: var(--danger);
    font-size: 0.8rem;
    white-space: nowrap;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* --- Turned on its side ------------------------------------------------

     A fader rather than a progress bar: the bar grows from the bottom, which
     is the direction every meter on every desk in the room grows in, and the
     peak mark rides across it. Both are the same two numbers as above. */
  .meter.vertical {
    --toward: to top;
    height: auto;
    width: 0.55rem;
    min-width: 0;
    /* Tall enough to read a level off at a glance, short enough that the
       strip stays a strip above the chat. */
    min-height: 2.6rem;
  }

  .vertical .fill {
    clip-path: inset(calc((1 - var(--level)) * 100%) 0 0 0);
  }

  .vertical .peak {
    left: 0;
    top: auto;
    bottom: calc(var(--peak) * 100% - 1px);
    width: 100%;
    height: 2px;
  }

  .dropped.vertical {
    font-size: 0.65rem;
    line-height: 1;
  }

  .dropped.vertical .word {
    display: none;
  }
</style>

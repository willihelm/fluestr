<script lang="ts">
  // Provisional stand-in for P8. It answers exactly one question — "is there
  // signal on this channel at all?" — which is the first thing to check when
  // no text appears. Peak hold, clipping and per-stream health come later.

  interface Props {
    rms: number;
    peak: number;
    dropped: number;
  }

  let { rms, peak, dropped }: Props = $props();

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

<div class="meter" class:clipping title="RMS {rms.toFixed(3)} · peak {peak.toFixed(3)}">
  <div class="fill" style="width: {level * 100}%"></div>
  <div class="peak" style="left: {peakMark * 100}%"></div>
</div>
{#if dropped > 0}
  <span class="dropped" title="Samples lost because the processing thread fell behind">
    {dropped} dropped
  </span>
{/if}

<style>
  .meter {
    position: relative;
    height: 0.5rem;
    border-radius: 0.25rem;
    background: var(--border);
    overflow: hidden;
    flex: 1;
    min-width: 6rem;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 60ms linear;
  }

  .peak {
    position: absolute;
    top: 0;
    width: 2px;
    height: 100%;
    background: var(--text);
  }

  .clipping .fill {
    background: #ff6b6b;
  }

  .dropped {
    color: #ff6b6b;
    font-size: 0.8rem;
    white-space: nowrap;
  }
</style>

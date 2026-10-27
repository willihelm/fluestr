<script lang="ts">
  // Whether the machine is going to keep up, in the status bar (P8).
  //
  // Next to the session line rather than on the Settings page: the question it
  // answers — "is this machine going to keep up?" — is asked while the show
  // runs, and a figure somebody has to change pages to find is a figure nobody
  // looks at. It is shown whether or not a session runs, because the load
  // before Start is what decides how many channels are sensible.
  //
  // The GPU rather than the CPU, which this used to show: decoding runs on
  // Metal and eight workers contend for one queue, so the GPU is what actually
  // runs out. The CPU figure moved with the audio plumbing and never reached
  // the thresholds that would have meant anything.

  import type { ResourceUsage } from "./bridge";
  import { detail, formatGpu, pressure } from "./resources";

  interface Props {
    /// `null` until the first sample arrives — a second after the window
    /// opens, which is the sampler's interval.
    usage: ResourceUsage | null;
  }

  let { usage }: Props = $props();

  // Nothing at all until the counter has a reading, rather than an empty bar:
  // a placeholder where a live figure belongs reads as zero.
  let gpu = $derived(
    usage && usage.gpu !== null ? Math.min(100, Math.max(0, usage.gpu)) : null,
  );
  let level = $derived(pressure(gpu ?? 0));
</script>

{#if usage && gpu !== null}
  <div class="usage" title={detail(usage)}>
    <span class="label">GPU</span>
    <div class="bar" class:busy={level === "busy"} class:critical={level === "critical"}>
      <div class="fill" style="width: {gpu}%"></div>
    </div>
    <!-- Tabular figures: without them the whole row shifts sideways every
         second as the digits change width, which is what makes a live number
         unreadable. -->
    <span class="value">{formatGpu(gpu)}</span>
  </div>
{/if}

<style>
  .usage {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.8rem;
    color: var(--text-dim);
    white-space: nowrap;
  }

  .label {
    letter-spacing: 0.04em;
  }

  .value {
    color: var(--text);
    /* Fixed width per digit, so the row does not jitter once a second. */
    font-variant-numeric: tabular-nums;
    min-width: 3.5rem;
  }

  .bar {
    position: relative;
    width: 3.5rem;
    height: 0.4rem;
    border-radius: 0.2rem;
    background: var(--border);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    /* Just under the sampling interval: the bar glides between samples
       instead of stepping, and still settles before the next one. */
    transition: width 900ms linear;
  }

  .busy .fill {
    background: #e8c36a;
  }

  .critical .fill {
    background: #ff6b6b;
  }

  /* --- A window the width of a hand --------------------------------------

     Same breakpoint as everywhere else; see `layout.ts`. The figure and no
     bar: the question this answers — "is this machine going to keep up?" —
     is answered by the number, and the bar is what makes it quick to read on
     a screen that has room for both. On a phone the same strip is competing
     with the session line for one row, and the bar is the half to drop.

     The colour the bar carried is not lost: it moves to the figure, so a
     machine in trouble still says so on the small screen. */
  @media (max-width: 45rem) {
    .bar {
      display: none;
    }

    .value {
      min-width: 0;
    }

    .usage {
      gap: 0.25rem;
    }

    /* The hidden bar is still in the document and still carries the class,
       so the figure after it is reached the same way it always was. */
    .bar.busy + .value {
      color: #e8c36a;
    }

    .bar.critical + .value {
      color: #ff6b6b;
    }
  }
</style>

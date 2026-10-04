<script lang="ts">
  // Whether the machine is going to keep up, in the status bar (P8).
  //
  // Next to the session line rather than on the Settings page: the question it
  // answers — "is this machine going to keep up?" — is asked while the show
  // runs, and a figure somebody has to change pages to find is a figure nobody
  // looks at. It is shown whether or not a session runs, because the load
  // before Start is what decides how many channels are sensible, and the fall
  // after Stop is how the memory a session held is seen to come back.
  //
  // The GPU rather than the CPU, which this used to show: decoding runs on
  // Metal and eight workers contend for one queue, so the GPU is what actually
  // runs out. The CPU figure moved with the audio plumbing and never reached
  // the thresholds that would have meant anything.

  import type { ResourceUsage } from "./bridge";
  import { detail, formatGpu, formatMemory, pressure } from "./resources";

  interface Props {
    /// `null` until the first sample arrives — a second after the window
    /// opens, which is the sampler's interval.
    usage: ResourceUsage | null;
  }

  let { usage }: Props = $props();

  // A machine that does not report a GPU shows the memory half alone rather
  // than an empty bar; see `summary` in `resources.ts` for why not a dash.
  let gpu = $derived(
    usage && usage.gpu !== null ? Math.min(100, Math.max(0, usage.gpu)) : null,
  );
  let level = $derived(pressure(gpu ?? 0));
  // Memory as a share of the machine, which is the only way a bar can mean
  // anything. A machine that will not say how much it has gets no bar rather
  // than a full one.
  let memoryShare = $derived(
    usage && usage.totalMemory > 0 ? (usage.memory / usage.totalMemory) * 100 : null,
  );
</script>

{#if usage}
  <div class="usage" title={detail(usage)}>
    {#if gpu !== null}
      <span class="label">GPU</span>
      <div class="bar" class:busy={level === "busy"} class:critical={level === "critical"}>
        <div class="fill" style="width: {gpu}%"></div>
      </div>
      <!-- Tabular figures: without them the whole row shifts sideways every
           second as the digits change width, which is what makes a live number
           unreadable. -->
      <span class="value">{formatGpu(gpu)}</span>
    {/if}

    <span class="label">RAM</span>
    {#if memoryShare !== null}
      <div class="bar">
        <div class="fill" style="width: {Math.min(100, memoryShare)}%"></div>
      </div>
    {/if}
    <span class="value">{formatMemory(usage.memory)}</span>
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
    border-radius: 999px;
    background: var(--muted);
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
    background: var(--warn);
  }

  .critical .fill {
    background: var(--danger);
  }

  /* --- A window the width of a hand --------------------------------------

     Same breakpoint as everywhere else; see `layout.ts`. Two figures and no
     bars: the question this answers — "is this machine going to keep up?" —
     is answered by the numbers, and the bars are what makes them quick to
     read on a screen that has room for both. On a phone the same strip is
     competing with the session line for one row, and a bar nobody can size
     against anything is the half to drop.

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

    /* The GPU figure, which is the one with a pressure to report. The hidden
       bar is still in the document and still carries the class, so the figure
       after it is reached the same way it always was. */
    .bar.busy + .value {
      color: var(--warn);
    }

    .bar.critical + .value {
      color: var(--danger);
    }
  }
</style>

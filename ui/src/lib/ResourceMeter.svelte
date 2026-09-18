<script lang="ts">
  // What the app is costing the machine, in the status bar (P8).
  //
  // Next to the session line rather than on the Settings page: the question it
  // answers — "is this machine going to keep up?" — is asked while the show
  // runs, and a figure somebody has to change pages to find is a figure nobody
  // looks at. It is shown whether or not a session runs, because the load
  // before Start is what decides how many channels are sensible, and the fall
  // after Stop is how the memory a session held is seen to come back.

  import type { ResourceUsage } from "./bridge";
  import { detail, formatCpu, formatMemory, pressure } from "./resources";

  interface Props {
    /// `null` until the first sample arrives — a second after the window
    /// opens, because a CPU percentage needs two points in time.
    usage: ResourceUsage | null;
  }

  let { usage }: Props = $props();

  let cpu = $derived(usage ? Math.min(100, Math.max(0, usage.cpu)) : 0);
  let level = $derived(pressure(cpu));
  // Memory as a share of the machine, which is the only way a bar can mean
  // anything. A machine that will not say how much it has gets no bar rather
  // than a full one.
  let memoryShare = $derived(
    usage && usage.totalMemory > 0 ? (usage.memory / usage.totalMemory) * 100 : null,
  );
</script>

{#if usage}
  <div class="usage" title={detail(usage)}>
    <span class="label">CPU</span>
    <div class="bar" class:busy={level === "busy"} class:critical={level === "critical"}>
      <div class="fill" style="width: {cpu}%"></div>
    </div>
    <!-- Tabular figures: without them the whole row shifts sideways every
         second as the digits change width, which is what makes a live number
         unreadable. -->
    <span class="value">{formatCpu(usage.cpu)}</span>

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
</style>

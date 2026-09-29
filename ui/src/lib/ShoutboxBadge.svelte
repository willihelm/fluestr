<script lang="ts">
  // The shoutbox in the status bar (P26): whether it is running, what gain it
  // is at, and its mute — in every view, without opening Settings.
  //
  // It is the one thing in this app that is in the signal path, and a
  // shoutbox that stopped is a position that went deaf without anybody
  // noticing, because nobody was talking at the time. So its state is on
  // screen for as long as it is switched on, and red when it is not running.

  import { onMount } from "svelte";
  import {
    onShoutboxStatus,
    setShoutboxLive,
    shoutboxStatus,
    type ShoutboxStatus,
    type StoredShoutbox,
  } from "./bridge";

  interface Props {
    /// The active profile's shoutbox, or `null` without a profile.
    shoutbox: StoredShoutbox | null;
  }

  let { shoutbox = $bindable() }: Props = $props();

  let status = $state<ShoutboxStatus | null>(null);

  onMount(() => {
    let unlisten: (() => void) | undefined;
    void onShoutboxStatus((next) => (status = next)).then((fn) => (unlisten = fn));
    shoutboxStatus()
      .then((s) => (status ??= s))
      .catch(() => {});
    return () => unlisten?.();
  });

  const toggleMute = () => {
    if (!shoutbox) return;
    shoutbox.muted = !shoutbox.muted;
    void setShoutboxLive(shoutbox.trimDb, shoutbox.muted).catch(() => {});
  };

  let running = $derived(status?.state === "running");
  let gain = $derived(
    running && status && shoutbox ? Math.round(status.gainDb + shoutbox.trimDb) : null,
  );
  let title = $derived(
    status?.message ??
      (running
        ? `Room ${status?.liveDb?.toFixed(0) ?? "—"}, ${status?.measuring ? "measuring" : `held at ${status?.levelDb?.toFixed(0) ?? "—"} while it plays`}`
        : "Shoutbox"),
  );
</script>

{#if shoutbox?.enabled}
  <span class="badge" {title}>
    <span class="dot" class:ok={running} class:bad={status?.state === "failed"}>●</span>
    <span>Shoutbox</span>
    {#if gain !== null}
      <span class="gain">{gain > 0 ? "+" : ""}{gain} dB</span>
    {:else if status?.state === "failed"}
      <span class="bad">not running</span>
    {/if}
    <button
      type="button"
      class:on={shoutbox.muted}
      onclick={toggleMute}
      aria-pressed={shoutbox.muted}
      title={shoutbox.muted ? "Unmute the shoutbox" : "Mute the shoutbox"}
    >
      M
    </button>
  </span>
{/if}

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.8rem;
    color: var(--text-dim);
    white-space: nowrap;
  }

  .dot {
    font-size: 0.6rem;
  }

  .ok {
    color: #5fd38d;
  }

  .bad {
    color: #ff6b6b;
  }

  .gain {
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }

  button {
    background: transparent;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 0.25rem;
    padding: 0 0.35rem;
    font: inherit;
    font-size: 0.75rem;
    font-weight: 600;
    cursor: pointer;
  }

  button.on {
    background: #c0392b;
    border-color: #c0392b;
    color: #fff;
  }
</style>

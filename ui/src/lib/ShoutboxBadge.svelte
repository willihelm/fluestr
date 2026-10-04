<script lang="ts">
  // The shoutbox in the status bar (P26): whether it is running, what gain it
  // is at, and its mute — in every view, without opening Settings.
  //
  // It is the one thing in this app that is in the signal path, and a
  // shoutbox that stopped is a position that went deaf without anybody
  // noticing, because nobody was talking at the time. So its state is on
  // screen for as long as it is switched on, and red when it is not running.
  //
  // Its name opens the shoutbox window, in the app: the controls beside the
  // DAW, like a plug-in's. In a browser it is only a name — the shoutbox is
  // tuned there on the Settings page.

  import { onMount } from "svelte";
  import {
    isRemote,
    onShoutboxStatus,
    openShoutboxWindow,
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
        ? `Room ${status?.liveDb?.toFixed(0) ?? "—"} dB SPL${status?.calibrated ? "" : " (estimated)"}, ${status?.measuring ? "measuring" : `held at ${status?.levelDb?.toFixed(0) ?? "—"} while it plays`}`
        : "Shoutbox"),
  );
</script>

{#if shoutbox?.enabled}
  <span class="badge" {title}>
    <span class="dot" class:ok={running} class:bad={status?.state === "failed"}>●</span>
    {#if isRemote}
      <span>Shoutbox</span>
    {:else}
      <button
        type="button"
        class="open"
        onclick={() => void openShoutboxWindow().catch((e) => console.warn("the shoutbox window did not open:", e))}
        title="Open the shoutbox in a window of its own"
      >
        Shoutbox
      </button>
    {/if}
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
    color: var(--ok);
  }

  .bad {
    color: var(--danger);
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

  button.open {
    border: none;
    padding: 0;
    font-size: inherit;
    font-weight: inherit;
    color: inherit;
  }

  button.open:hover {
    color: var(--text);
    text-decoration: underline;
  }

  button.on {
    background: var(--channel-mute);
    border-color: var(--channel-mute);
    color: color-mix(in oklch, var(--channel-mute), black 72%);
    /* Lit like an LED key, with a little of its own light around it. */
    box-shadow: 0 0 0.6rem color-mix(in oklch, var(--channel-mute) 45%, transparent);
  }
</style>

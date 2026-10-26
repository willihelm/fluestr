<script lang="ts">
  // The shoutbox in a window of its own (P26), opened from the status bar.
  //
  // What a DAW gives a plug-in: its controls, beside the work, without the
  // rest of the setup around them. Mute, trim, the readings and the curve —
  // the part that is touched during a show. The patch and the calibration
  // stay on the Settings page, where they are set once.
  //
  // It follows the active profile's shoutbox from the configuration file,
  // re-read on every `config-changed`, and hands its edits to the main window,
  // which stores them: the configuration has one writer. Trim and mute also go
  // to the running shoutbox at once, as they do from the Settings page.
  //
  // In a browser (P16) it is a pop-up of the main tab, and the main tab is
  // the writer it hands its edits to — so one closed behind it is said.

  import { onMount } from "svelte";
  import ShoutboxControls from "./ShoutboxControls.svelte";
  import { canonical } from "./profiles";
  import { canPin, setWindowOnTop, setWindowTitle } from "./windows";
  import {
    isRemote,
    loadConfig,
    onConfigChanged,
    onReconnected,
    onShoutboxStatus,
    requestShoutboxEdit,
    shoutboxStatus,
    type ShoutboxStatus,
    type StoredConfig,
    type StoredShoutbox,
  } from "./bridge";

  let shoutbox = $state<StoredShoutbox | null>(null);
  /// The active profile's position, which is how the main window is told
  /// where an edit belongs.
  let profile = $state<number | null>(null);
  let status = $state<ShoutboxStatus | null>(null);
  let failure = $state<string | null>(null);
  let onTop = $state(false);
  /// Whether the main tab this pop-up was opened from has gone, so that
  /// nothing stores what is changed here. Never in the app, whose main window
  /// is there for as long as the app is.
  let orphaned = $state(false);

  /// The shoutbox as this window last knew the app to have it, in a form that
  /// does not depend on field order. An edit equal to it is no edit.
  let known: string | null = null;
  /// What this window has sent and not yet heard back, oldest first.
  ///
  /// Every edit comes back as a `config-changed`, but not necessarily one by
  /// one: the main window saves through a debounce, and an echo of an older
  /// keystroke can arrive after a newer one was made. Adopting it would take
  /// the newer one back. So an echo of anything still in here is this
  /// window's own, and it and everything sent before it are done with; only
  /// what matches none of it is somebody else's change.
  const sent: string[] = [];

  const adopt = (config: StoredConfig) => {
    const active = config.profiles[config.active];
    if (!active) {
      shoutbox = null;
      profile = null;
      return;
    }
    const incoming = canonical(active.outputs.shoutbox);
    if (profile === config.active) {
      if (incoming === known) return;
      const echo = sent.indexOf(incoming);
      if (echo !== -1) {
        sent.splice(0, echo + 1);
        return;
      }
    }
    sent.length = 0;
    known = incoming;
    profile = config.active;
    shoutbox = active.outputs.shoutbox;
  };

  onMount(() => {
    // What the backend names the app's window; a pop-up names itself.
    if (isRemote) setWindowTitle("Shoutbox — Fluestr");
    const subscriptions = [
      onConfigChanged(adopt),
      onShoutboxStatus((next) => (status = next)),
    ];
    const first = () => {
      loadConfig()
        .then(adopt)
        .catch((e) => (failure = String(e)));
      shoutboxStatus()
        .then((s) => (status ??= s))
        .catch(() => {});
    };
    first();
    // A pop-up that lost the socket missed every change meanwhile; it reads
    // them again as it did when it opened. Never fires in the app.
    const reconnected = onReconnected(() => {
      status = null;
      first();
    });
    return () => {
      reconnected();
      for (const subscription of subscriptions) {
        subscription.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  });

  // Every change made here, to the main window. After an adoption too, which
  // is harmless: it equals `known` and goes nowhere.
  $effect(() => {
    if (shoutbox === null || profile === null) return;
    const snapshot = $state.snapshot(shoutbox);
    const edit = canonical(snapshot);
    if (edit === known) return;
    known = edit;
    sent.push(edit);
    if (isRemote) orphaned = !window.opener || window.opener.closed;
    void requestShoutboxEdit(profile, snapshot).catch(() => {});
  });

  const togglePin = () => {
    onTop = !onTop;
    setWindowOnTop(onTop);
  };

  const stateText: Record<string, string> = {
    off: "Off",
    starting: "Opening its devices …",
    running: "Running",
    failed: "Not running",
  };
</script>

<main>
  <header>
    <h2>Shoutbox</h2>
    <span class="state {status?.state ?? 'off'}">{stateText[status?.state ?? "off"]}</span>
    {#if canPin}
      <button
        type="button"
        class="pin"
        class:on={onTop}
        onclick={togglePin}
        aria-pressed={onTop}
        title={onTop ? "Stop keeping this window in front" : "Keep this window in front"}
      >
        In front
      </button>
    {/if}
  </header>

  {#if failure}
    <p class="failed">The configuration could not be read: {failure}</p>
  {:else if shoutbox}
    {#if !shoutbox.enabled}
      <p class="note">The shoutbox is switched off. It is switched on, and patched, in Settings.</p>
    {:else if status?.state === "failed" && status.message}
      <p class="failed">{status.message}</p>
    {/if}
    {#if orphaned}
      <p class="note">
        The Fluestr tab this was opened from is closed. Changes made here are only kept while
        Fluestr is open in another tab of this browser; trim and mute reach the shoutbox
        either way, until it next starts.
      </p>
    {/if}
    <ShoutboxControls bind:shoutbox {status} />
  {/if}
</main>

<style>
  /* The body does not scroll (see `app.css`); a short window has to. */
  main {
    height: 100vh;
    box-sizing: border-box;
    overflow-y: auto;
    padding: 0.6rem 1rem 1rem;
  }

  header {
    display: flex;
    align-items: center;
    gap: 0.8rem;
  }

  h2 {
    margin: 0;
    font-size: 1rem;
  }

  .state {
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .state.running {
    color: #5fd38d;
  }

  .state.failed,
  .failed {
    color: #ff6b6b;
  }

  .failed,
  .note {
    margin: 0.4rem 0 0;
    font-size: 0.8rem;
  }

  .note {
    color: var(--text-dim);
  }

  .pin {
    margin-left: auto;
    background: var(--bg);
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.2rem 0.6rem;
    font: inherit;
    font-size: 0.8rem;
    cursor: pointer;
  }

  .pin.on {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--bg);
  }
</style>

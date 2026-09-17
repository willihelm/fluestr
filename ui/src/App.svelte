<script lang="ts">
  import ChatList from "./lib/ChatList.svelte";
  import StreamMeters from "./lib/StreamMeters.svelte";
  import StreamSetup from "./lib/StreamSetup.svelte";
  import { newChannel, toConfig, validate, type Channel } from "./lib/streams";
  import {
    listDevices,
    onLevels,
    onOverload,
    onSessionState,
    onTranscript,
    startSession,
    stopSession,
    type DeviceOption,
    type SessionState,
    type StreamLevel,
    type TranscriptEvent,
  } from "./lib/bridge";

  // Several channels of one multichannel device, one chat. Still to come in
  // P7: the group sidebar, the virtualised list, and moving the channel setup
  // onto its own Settings page.

  let devices = $state<DeviceOption[]>([]);
  let device = $state("");
  let channels = $state<Channel[]>([]);

  let status = $state<SessionState | { state: "starting" } | { state: "idle" }>({
    state: "idle",
  });
  let messages = $state<TranscriptEvent[]>([]);
  let levels = $state(new Map<number, StreamLevel>());
  // Segments the decode queue had to give up on, per stream. Shown rather than
  // only logged: an operator who spoke and saw no text needs to be able to
  // tell "the app fell behind" from "the channel was not listening".
  let overloaded = $state(new Map<number, number>());

  let running = $derived(status.state === "running");
  let busy = $derived(status.state === "starting");
  let selected = $derived(devices.find((d) => d.name === device));
  let deviceChannels = $derived(selected?.channels ?? 0);
  // The stream id is the row's position, which is exactly how the backend
  // assigns it — see `SessionConfig::streams`.
  let colors = $derived(new Map(channels.map((c, streamId) => [streamId, c.color])));
  let problem = $derived(validate(channels, device, deviceChannels));

  $effect(() => {
    listDevices()
      .then((found) => {
        devices = found;
        device = (found.find((d) => d.isDefault) ?? found[0])?.name ?? "";
      })
      .catch((e) => (status = { state: "failed", message: String(e) }));
  });

  // One channel to begin with, so the app is usable without first having to
  // work out that a channel has to be added. Only when there are none at all:
  // switching devices must never throw away a setup the operator typed in.
  $effect(() => {
    if (deviceChannels > 0 && channels.length === 0) {
      channels = [newChannel([], deviceChannels)];
    }
  });

  $effect(() => {
    const subscriptions = [
      onTranscript((event) => {
        messages = [...messages, event];
      }),
      onLevels((event) => {
        levels = new Map(event.streams.map((s) => [s.streamId, s]));
      }),
      onOverload((event) => {
        overloaded = new Map(overloaded).set(event.streamId, event.dropped);
      }),
      onSessionState((event) => (status = event)),
    ];

    // Subscribing can be refused — `listen` needs the `core:event` capability,
    // and without it the promise rejects. Unhandled, that failure is completely
    // silent: the backend runs, the events go nowhere, and the UI looks like it
    // is waiting on a hung session. It has to be visible.
    Promise.all(subscriptions).catch((e) => {
      status = { state: "failed", message: `event bridge unavailable: ${e}` };
    });

    return () => {
      for (const subscription of subscriptions) {
        subscription.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  });

  // One global Start for every configured channel — there is deliberately no
  // per-channel Start button. And it is non-destructive: the chat keeps what
  // is already in it and new transcripts are appended. Clearing history, if it
  // is ever offered, is a separate explicit action.
  const start = async () => {
    if (problem) {
      status = { state: "failed", message: problem };
      return;
    }

    // The model load happens inside `start_session` and can take seconds on a
    // machine's first ever start — the UI has to say so rather than look hung.
    status = { state: "starting" };
    try {
      status = await startSession({ device, streams: toConfig(channels) });
    } catch (e) {
      status = { state: "failed", message: String(e) };
    }
  };

  const stop = async () => {
    await stopSession();
    status = { state: "idle" };
    levels = new Map();
    overloaded = new Map();
  };
</script>

<main>
  <header class="bar">
    <h1>Intercom2Chat</h1>

    <label>
      Device
      <select bind:value={device} disabled={running || busy}>
        {#each devices as option (option.name)}
          <option value={option.name}>
            {option.name} ({option.channels} ch)
          </option>
        {/each}
      </select>
    </label>

    {#if running}
      <button onclick={stop}>Stop</button>
    {:else}
      <button onclick={start} disabled={busy || device === "" || problem !== null}>
        {busy ? "Starting …" : "Start"}
      </button>
    {/if}
  </header>

  <StreamSetup bind:channels {deviceChannels} locked={running || busy} />

  <div class="bar status">
    {#if status.state === "running"}
      <span class="ok">●</span>
      <span>
        {status.device} · {status.streams.length}
        {status.streams.length === 1 ? "channel" : "channels"} · {status.sampleRate} Hz ·
        {status.model.split("/").pop()} · {status.gpu ? "GPU" : "CPU"}, loaded in
        {(status.loadMs / 1000).toFixed(1)} s
      </span>
    {:else if status.state === "starting"}
      <span class="dim">Loading the model … the first ever start compiles Metal shaders (~8 s).</span>
    {:else if status.state === "failed"}
      <span class="error">{status.message}</span>
    {:else if status.state === "stopped"}
      <span class="dim">Session ended.</span>
    {:else if problem}
      <span class="dim">{problem}</span>
    {:else}
      <span class="dim">Press Start to transcribe all configured channels.</span>
    {/if}
  </div>

  {#if running}
    <StreamMeters {channels} {levels} {overloaded} />
  {/if}

  <ChatList {messages} {colors} />
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    height: 100vh;
    position: relative;
  }

  .bar {
    display: flex;
    align-items: flex-end;
    gap: 0.75rem;
    padding: 0.6rem 1rem;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-wrap: wrap;
  }

  .status {
    align-items: center;
  }

  h1 {
    margin: 0 0.5rem 0 0;
    font-size: 1.1rem;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    font-size: 0.75rem;
    color: var(--text-dim);
  }

  select {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.9rem;
  }

  button {
    background: var(--accent);
    color: #12141a;
    border: none;
    border-radius: 0.3rem;
    padding: 0.4rem 1.1rem;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    margin-left: auto;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .status {
    font-size: 0.85rem;
    gap: 0.5rem;
  }

  .dim {
    color: var(--text-dim);
  }

  .ok {
    color: #5fd38d;
  }

  .error {
    color: #ff6b6b;
  }
</style>

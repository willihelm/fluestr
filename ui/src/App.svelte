<script lang="ts">
  import ChatList from "./lib/ChatList.svelte";
  import LevelMeter from "./lib/LevelMeter.svelte";
  import {
    listDevices,
    onLevel,
    onSessionState,
    onTranscript,
    startSession,
    stopSession,
    type DeviceOption,
    type LevelEvent,
    type SessionState,
    type TranscriptEvent,
  } from "./lib/bridge";

  // The narrow vertical slice: one device, one channel, one chat list. The
  // session setup screen with eight streams, labels and per-stream language
  // configuration is P7.

  let devices = $state<DeviceOption[]>([]);
  let device = $state("");
  // Counted from 1, the way channels are numbered on an interface and in a
  // patch list. The backend stays zero-based because that is what the audio
  // API uses; the conversion happens at this boundary and nowhere else.
  let channelNumber = $state(1);
  let label = $state("Regie");
  let language = $state("de");
  // The vocabulary prompt is the single strongest lever on code-switching
  // quality (P15). Here it is a plain text box so its effect is visible from
  // the first run — measured at "Q5" becoming "Cue".
  let prompt = $state("Intercom-Kommandos: Standby, Cue, Go, Fade out, Blackout, Talkback, Regie.");

  let status = $state<SessionState | { state: "starting" } | { state: "idle" }>({
    state: "idle",
  });
  let messages = $state<TranscriptEvent[]>([]);
  let level = $state<LevelEvent>({ streamId: 0, rms: 0, peak: 0, dropped: 0 });

  let running = $derived(status.state === "running");
  let busy = $derived(status.state === "starting");
  let selected = $derived(devices.find((d) => d.name === device));

  $effect(() => {
    listDevices()
      .then((found) => {
        devices = found;
        device = (found.find((d) => d.isDefault) ?? found[0])?.name ?? "";
      })
      .catch((e) => (status = { state: "failed", message: String(e) }));
  });

  $effect(() => {
    const subscriptions = [
      onTranscript((event) => {
        messages = [...messages, event];
      }),
      onLevel((event) => (level = event)),
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

  const start = async () => {
    messages = [];
    // Keep the number inside what the device offers. The backend rejects an
    // out-of-range channel with a precise message, but that message counts
    // from zero and would contradict the field the operator just typed into.
    const channels = selected?.channels ?? 1;
    channelNumber = Math.min(Math.max(1, Math.round(channelNumber)), channels);

    // The model load happens inside `start_session` and can take seconds on a
    // machine's first ever start — the UI has to say so rather than look hung.
    status = { state: "starting" };
    try {
      status = await startSession({
        device,
        channel: channelNumber - 1,
        label,
        language,
        initialPrompt: prompt.trim() === "" ? null : prompt,
      });
    } catch (e) {
      status = { state: "failed", message: String(e) };
    }
  };

  const stop = async () => {
    await stopSession();
    status = { state: "idle" };
    level = { streamId: 0, rms: 0, peak: 0, dropped: 0 };
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

    <label>
      Channel
      <input
        type="number"
        min="1"
        max={selected?.channels ?? 1}
        bind:value={channelNumber}
        disabled={running || busy}
      />
    </label>

    <label>
      Label
      <input type="text" bind:value={label} disabled={running || busy} />
    </label>

    <label>
      Language
      <input
        type="text"
        size="3"
        bind:value={language}
        disabled={running || busy}
        title="ISO 639-1 code. The full list comes from whisper.cpp in P7 — nothing is hard-coded to de/en."
      />
    </label>

    {#if running}
      <button onclick={stop}>Stop</button>
    {:else}
      <button onclick={start} disabled={busy || device === ""}>
        {busy ? "Starting …" : "Start"}
      </button>
    {/if}
  </header>

  <div class="bar vocabulary">
    <label class="grow">
      Vocabulary prompt
      <input type="text" bind:value={prompt} disabled={running || busy} />
    </label>
  </div>

  <div class="bar status">
    {#if status.state === "running"}
      <span class="ok">●</span>
      <span>
        {status.device} · channel {status.channel + 1} · {status.sampleRate} Hz ·
        {status.model.split("/").pop()} · {status.gpu ? "GPU" : "CPU"}, loaded in
        {(status.loadMs / 1000).toFixed(1)} s
      </span>
      <LevelMeter rms={level.rms} peak={level.peak} dropped={level.dropped} />
    {:else if status.state === "starting"}
      <span class="dim">Loading the model … the first ever start compiles Metal shaders (~8 s).</span>
    {:else if status.state === "failed"}
      <span class="error">{status.message}</span>
    {:else if status.state === "stopped"}
      <span class="dim">Session ended.</span>
    {:else}
      <span class="dim">Pick a device and press Start.</span>
    {/if}
  </div>

  <ChatList {messages} />
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

  .vocabulary,
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

  .grow {
    flex: 1;
  }

  select,
  input {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.9rem;
  }

  input[type="number"] {
    width: 4.5rem;
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

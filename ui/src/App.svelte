<script lang="ts">
  import ChatList from "./lib/ChatList.svelte";
  import StreamMeters from "./lib/StreamMeters.svelte";
  import SettingsPage from "./lib/SettingsPage.svelte";
  import GroupSidebar from "./lib/GroupSidebar.svelte";
  import ClearCommunication from "./lib/ClearCommunication.svelte";
  import { newChannel, toConfig, validate, mutedChannels, type Channel } from "./lib/streams";
  import { forgetChannels, visibleStreams, type Group } from "./lib/groups";
  import { DEFAULT_VOCABULARY, toConfig as vocabularyConfig } from "./lib/vocabulary";
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

  // Two pages: the live chat, and the Settings page that holds everything
  // about how the session is configured. Start/Stop belongs to neither — it is
  // session-wide and stays in the header, reachable from both.
  //
  // Still to come in P7: the virtualised list. An hour across eight streams is
  // tens of thousands of nodes.

  type View = "chat" | "settings";

  let view = $state<View>("chat");

  let devices = $state<DeviceOption[]>([]);
  let device = $state("");
  let channels = $state<Channel[]>([]);
  let groups = $state<Group[]>([]);
  let vocabulary = $state(DEFAULT_VOCABULARY);
  /// The group the chat is focused on; `null` is the all-channels view.
  let selectedGroup = $state<number | null>(null);

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
  // Which streams receive no audio because the device has no such channel.
  // While a session runs this comes from the backend's own answer rather than
  // from the UI's copy of the setup — that is the authoritative mapping, and
  // the two would differ the moment the device is swapped under a running
  // session.
  let muted = $derived(
    status.state === "running"
      ? new Set(status.streams.filter((s) => s.muted).map((s) => s.streamId))
      : new Set(mutedChannels(channels, deviceChannels)),
  );

  let group = $derived(groups.find((g) => g.key === selectedGroup) ?? null);
  // `null` means every stream, which is not the same as an empty set: a group
  // with no members shows nothing, and saying so is more useful than silently
  // falling back to showing everything.
  let visible = $derived(visibleStreams(group, channels));
  let shown = $derived(
    visible === null ? messages : messages.filter((m) => visible.has(m.streamId)),
  );

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
      channels = [newChannel([])];
    }
  });

  // A deleted channel has to leave the groups it was in, and a deleted group
  // has to release the chat. Both are cheap to do on every change and easy to
  // forget at the one call site that causes them.
  $effect(() => {
    const pruned = forgetChannels(groups, channels);
    if (pruned.some((g, i) => g.members.length !== groups[i].members.length)) {
      groups = pruned;
    }
    if (selectedGroup !== null && !groups.some((g) => g.key === selectedGroup)) {
      selectedGroup = null;
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
      onSessionState((event) => {
        status = event;
        // A failed session is over — but its threads and its loaded model are
        // only released when something takes it out of the backend's slot, and
        // the failure was discovered inside the pipeline, not by a command.
        // Stop is idempotent, so saying so here costs nothing and frees the
        // sidecar instead of keeping it alive until the next Start.
        if (event.state === "failed") {
          stopSession().catch(() => {});
        }
      }),
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
  // is already in it and new transcripts are appended. Emptying it is the
  // separate, confirmed Clear all communication action and nothing else.
  const start = async () => {
    if (problem) {
      status = { state: "failed", message: problem };
      view = "settings";
      return;
    }

    // The model load happens inside `start_session` and can take seconds on a
    // machine's first ever start — the UI has to say so rather than look hung.
    status = { state: "starting" };
    try {
      status = await startSession({
        device,
        streams: toConfig(channels),
        // One vocabulary for every stream — see `vocabulary.ts`.
        vocabulary: vocabularyConfig(vocabulary),
      });
      view = "chat";
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

  // Every transcript, from every session and every group — not only the ones
  // the current view happens to show. Clearing what is on screen while leaving
  // the rest would make the chat depend on which group was selected when the
  // button was pressed.
  //
  // P10 adds the other half: the same action has to delete the stored
  // transcripts and the FTS index, and nothing else in the database.
  const clearCommunication = () => {
    messages = [];
    overloaded = new Map();
  };
</script>

<main>
  <header class="bar">
    <h1>Intercom2Chat</h1>

    <nav class="tabs">
      <button class="tab" class:active={view === "chat"} onclick={() => (view = "chat")}>
        Chat
      </button>
      <button
        class="tab"
        class:active={view === "settings"}
        onclick={() => (view = "settings")}
      >
        Settings
      </button>
    </nav>

    {#if running}
      <button class="primary" onclick={stop}>Stop</button>
    {:else}
      <button
        class="primary"
        onclick={start}
        disabled={busy || device === "" || problem !== null}
      >
        {busy ? "Starting …" : "Start"}
      </button>
    {/if}
  </header>

  <div class="bar status">
    {#if status.state === "running"}
      <span class="ok">●</span>
      <span>
        {status.device} · {status.streams.length - muted.size}
        {status.streams.length - muted.size === 1 ? "channel" : "channels"}{muted.size > 0
          ? `, ${muted.size} muted`
          : ""} · {status.sampleRate} Hz ·
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

    {#if view === "chat"}
      <div class="right">
        <ClearCommunication
          {running}
          empty={messages.length === 0}
          clear={clearCommunication}
        />
      </div>
    {/if}
  </div>

  {#if view === "settings"}
    <SettingsPage
      {devices}
      bind:device
      bind:channels
      bind:groups
      bind:vocabulary
      {deviceChannels}
      locked={running || busy}
    />
  {:else}
    <div class="chat">
      <GroupSidebar {groups} {channels} bind:selected={selectedGroup} />

      <div class="stream">
        {#if running}
          <!-- The meters follow the same view — eight bars for a
               three-channel group are five things to read past. The filtering
               happens by stream id inside the component, because the stream id
               *is* the position in `channels`. -->
          <StreamMeters {channels} {levels} {overloaded} {visible} {muted} />
        {/if}

        {#if visible !== null && visible.size === 0}
          <p class="empty">
            This group has no channels yet. Add some on the Settings page.
          </p>
        {:else}
          <ChatList messages={shown} {colors} />
        {/if}
      </div>
    </div>
  {/if}
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
    align-items: center;
    gap: 0.75rem;
    padding: 0.6rem 1rem;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-wrap: wrap;
  }

  h1 {
    margin: 0 0.5rem 0 0;
    font-size: 1.1rem;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .tabs {
    display: flex;
    gap: 0.2rem;
  }

  .tab {
    background: transparent;
    color: var(--text-dim);
    border: 1px solid transparent;
    border-radius: 0.3rem;
    padding: 0.3rem 0.8rem;
    font: inherit;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .tab:hover {
    color: var(--text);
  }

  .tab.active {
    background: var(--bg);
    border-color: var(--border);
    color: var(--text);
    font-weight: 600;
  }

  .chat {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  .stream {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  .right {
    margin-left: auto;
  }

  .status {
    font-size: 0.85rem;
    gap: 0.5rem;
  }

  .empty {
    color: var(--text-dim);
    font-size: 0.85rem;
    padding: 1rem;
  }

  .primary {
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

  .primary:disabled {
    opacity: 0.5;
    cursor: default;
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

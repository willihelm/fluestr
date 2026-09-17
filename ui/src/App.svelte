<script lang="ts">
  import ChatList from "./lib/ChatList.svelte";
  import StreamMeters from "./lib/StreamMeters.svelte";
  import SettingsPage from "./lib/SettingsPage.svelte";
  import GroupSidebar from "./lib/GroupSidebar.svelte";
  import ClearCommunication from "./lib/ClearCommunication.svelte";
  import { toConfig, validate, mutedChannels } from "./lib/streams";
  import { forgetChannels, visibleStreams } from "./lib/groups";
  import {
    fromStoredConfig,
    newProfile,
    toStoredConfig,
    type ModelChoice,
    type Profile,
  } from "./lib/profiles";
  import { DEFAULT_VOCABULARY, toConfig as vocabularyConfig } from "./lib/vocabulary";
  import {
    listDevices,
    loadConfig,
    onLevels,
    onOverload,
    onSessionState,
    onTranscript,
    saveConfig,
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

  /// How long after the last keystroke the configuration is written.
  ///
  /// Long enough that typing a label is one save rather than twelve, short
  /// enough that nobody can build a setup and close the window in between. The
  /// write itself is atomic, so a save landing at an awkward moment costs
  /// nothing.
  const SAVE_DEBOUNCE_MS = 400;

  let view = $state<View>("chat");

  let devices = $state<DeviceOption[]>([]);
  // The stored setups (P12) and the one being used. Device, channel map and
  // groups all live inside the active profile; the vocabulary sits beside them
  // because it is one list for every profile — see `profiles.ts`.
  let profiles = $state<Profile[]>([]);
  let activeKey = $state<number | null>(null);
  let vocabulary = $state(DEFAULT_VOCABULARY);
  // Which model transcribes (P11). Empty is "never chose", which resolves to
  // the recommended one — so a fresh machine is not asked a question it has no
  // way to answer yet.
  let models = $state<ModelChoice>({});
  // Nothing is saved before the configuration has been read: an empty UI
  // saving itself over a file it has not seen yet is how a setup is lost.
  let loaded = $state(false);
  let saveError = $state<string | null>(null);
  /// The group the chat is focused on; `null` is the all-channels view.
  let selectedGroup = $state<number | null>(null);

  let active = $derived(profiles.find((p) => p.key === activeKey) ?? null);
  // Read-only views of the active profile, so everything below reads the way
  // it did when these were top-level state. Writing happens on the Settings
  // page, into the profile itself — which is what makes an edit something that
  // gets saved.
  let device = $derived(active?.device ?? "");
  let channels = $derived(active?.channels ?? []);
  let groups = $derived(active?.groups ?? []);

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
  // A restored profile may name an interface that is not plugged in — that is
  // the normal case for a setup carried between the office and the house, and
  // it is a different problem from a device with no input channels.
  let connected = $derived(selected !== undefined);
  // The stream id is the row's position, which is exactly how the backend
  // assigns it — see `SessionConfig::streams`.
  let colors = $derived(new Map(channels.map((c, streamId) => [streamId, c.color])));
  let problem = $derived(validate(channels, device, deviceChannels, connected));
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

  // Startup: the devices that are there, and the setup from last time.
  //
  // Both before anything else, and the configuration is not optional to the
  // result — a window that came up with an empty channel map and then filled
  // it in would spend its first moment looking like the setup was lost.
  $effect(() => {
    const first = async () => {
      const found = await listDevices();
      devices = found;
      const fallback = (found.find((d) => d.isDefault) ?? found[0])?.name ?? "";

      // A configuration that cannot be read is not a reason to refuse to
      // transcribe: the app comes up empty and says why, rather than not
      // coming up. Saying why matters because the same failure is the one that
      // will silently lose whatever is built next.
      let stored = null;
      try {
        stored = await loadConfig();
      } catch (e) {
        saveError = String(e);
      }

      const restored = stored
        ? fromStoredConfig(stored)
        : { profiles: [], activeKey: null, vocabulary: null, models: {} };

      if (restored.profiles.length === 0) {
        // First start: one profile on the default device, with one channel, so
        // the app is usable without first having to work out that a profile
        // and a channel have to be created.
        const created = newProfile([], fallback);
        profiles = [created];
        activeKey = created.key;
      } else {
        profiles = restored.profiles;
        activeKey = restored.activeKey;
      }

      // `null` is a file that has never held a vocabulary, which keeps the
      // default; an empty string is a box the operator cleared, which stays
      // cleared — refilling it would turn the prompt back on behind their
      // back.
      if (restored.vocabulary !== null) vocabulary = restored.vocabulary;
      models = restored.models;

      loaded = true;
    };

    first().catch((e) => (status = { state: "failed", message: String(e) }));
  });

  // Saving is a consequence of the state changing, not of any particular
  // button: there is no Save on the Settings page, because a setup an operator
  // believes is stored and is not is worse than no persistence at all.
  // Debounced, so typing a label is one write; the write itself is atomic on
  // the backend, so an unlucky moment costs nothing.
  $effect(() => {
    if (!loaded || activeKey === null) return;

    // Built here rather than in the timer: this is what reads every field, and
    // reading them is what makes this effect run again when one of them
    // changes.
    const config = toStoredConfig(profiles, activeKey, vocabulary, models);

    const timer = setTimeout(() => {
      saveConfig(config)
        .then(() => (saveError = null))
        .catch((e) => (saveError = String(e)));
    }, SAVE_DEBOUNCE_MS);

    return () => clearTimeout(timer);
  });

  // A deleted channel has to leave the groups it was in, and a deleted group
  // has to release the chat. Both are cheap to do on every change and easy to
  // forget at the one call site that causes them.
  //
  // Selecting a different profile goes through the same path: its groups are
  // other groups with other keys, so the chat falls back to the all-channels
  // view instead of staying pointed at a group that is no longer there.
  $effect(() => {
    if (active) {
      const pruned = forgetChannels(active.groups, active.channels);
      if (pruned.some((g, i) => g.members.length !== active.groups[i].members.length)) {
        active.groups = pruned;
      }
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
  // This used to be half the action, with P10 deleting the stored transcripts
  // and the FTS index. P10 was set aside on 2026-09-18 — the chat lives in the
  // window — so this is the whole of it.
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
      bind:profiles
      bind:activeKey
      bind:vocabulary
      bind:models
      {deviceChannels}
      locked={running || busy}
      {saveError}
      {problem}
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

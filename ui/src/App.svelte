<script lang="ts">
  import { untrack } from "svelte";
  import ChatList from "./lib/ChatList.svelte";
  import StreamMeters from "./lib/StreamMeters.svelte";
  import SettingsPage from "./lib/SettingsPage.svelte";
  import GroupSidebar from "./lib/GroupSidebar.svelte";
  import ClearCommunication from "./lib/ClearCommunication.svelte";
  import SetupGuide from "./lib/SetupGuide.svelte";
  import ResourceMeter from "./lib/ResourceMeter.svelte";
  import { toConfig, validate, absentChannels } from "./lib/streams";
  import { forgetChannels, visibleStreams } from "./lib/groups";
  import { forget as forgetInRules } from "./lib/rules";
  import {
    adoptDeviceIds,
    canonical,
    connectedDevice,
    fromStoredConfig,
    newProfile,
    reuseKeys,
    toStoredConfig,
    clampTuning,
    DEFAULT_TUNING,
    NO_REMOTE,
    type ModelChoice,
    type Profile,
  } from "./lib/profiles";
  import { DEFAULT_VOCABULARY, toConfig as vocabularyConfig } from "./lib/vocabulary";
  import { DEFAULT_DIRECTION, type SortDirection } from "./lib/sort";
  import { compile as compileRules, sameCompiled, type CompiledRule } from "./lib/rules";
  import { mergeHistory, toBubbles } from "./lib/bubbles";
  import { attentionFor } from "./lib/attention";
  import { ALL_CHANNELS_VIEW, type GroupView } from "./lib/windows";
  import { beforeLeaving, whenEditingEnds } from "./lib/editing";
  import { watchNarrow } from "./lib/layout";
  import {
    clearCommunication as clearOnBackend,
    closeGroupWindows,
    connectionState,
    isRemote,
    listDevices,
    listModels,
    loadConfig,
    onCleared,
    onConfigChanged,
    onConnection,
    onGroupOnTop,
    onReconnected,
    onLevels,
    onOverload,
    onResources,
    onSessionState,
    onWebhookStatus,
    onTranscript,
    requestAttention,
    saveConfig,
    sessionSnapshot,
    setRules,
    startSession,
    stopSession,
    updateStreamConfig,
    updateTuning,
    startMonitor,
    stopMonitor,
    setMonitorGain,
    webhookStatus,
    type DeviceOption,
    type ResourceUsage,
    type SessionState,
    type Connection,
    type StoredConfig,
    type StoredRemote,
    type StreamLevel,
    type TranscriptEvent,
    type Tuning,
    type WebhookStatus,
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

  /// How long after the last change to the channel map the input device is
  /// opened for metering (P8).
  ///
  /// Shorter than the save: this costs nothing but a device open, and the
  /// operator is waiting to see a bar move. Long enough that typing a
  /// two-digit channel number is one open and not two.
  const METER_DEBOUNCE_MS = 250;

  let view = $state<View>("chat");

  /// The window's keyboard shortcuts.
  ///
  /// Three of them, all on the modifier key, and all for things an operator
  /// does with one hand while the other is on a fader: switch between the two
  /// pages, and start or stop capture.
  ///
  /// - **⌘1** — Chat, and **⌘2** — Settings, the pair every tabbed Mac
  ///   application has, in the order the tabs are in.
  /// - **⌘,** — Settings as well, because that is where a Mac user's hand
  ///   goes for a setup page. It opens and never toggles: a second press that
  ///   threw the operator back into the chat would be a different action
  ///   wearing the same key.
  /// - **⌘T** — Start, or Stop while a session runs. One key for both,
  ///   because it is pressed in the dark at the top of a show and again at
  ///   the end of it, and hunting for the second of two keys is the part that
  ///   goes wrong. It is refused in exactly the cases the button is disabled
  ///   in — mid-start, no device, a setup that does not validate — rather
  ///   than starting something that cannot run.
  ///
  /// Ctrl as well as ⌘ throughout, for the browser peer (P16), which may not
  /// be on a Mac. ⌘T is the browser's own "new tab" there and will not reach
  /// the page; the Start button is, and stays, the way that always works.
  const shortcut = (event: KeyboardEvent) => {
    if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;

    switch (event.key) {
      case "1":
        view = "chat";
        break;
      case "2":
      case ",":
        view = "settings";
        break;
      case "t":
      case "T":
        if (running) stop();
        else if (!busy && device !== "" && problem === null) start();
        break;
      default:
        return;
    }
    event.preventDefault();
  };

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
  // Whether this machine serves the browser peer, and on which address (P16).
  // Beside the models rather than inside a profile: which network the machine
  // is on belongs to the machine and to the house it is standing in, not to the
  // production being carried to it. The listener follows this through
  // `save_config` — there is no separate command to switch it on, because the
  // configuration has one writer.
  let remote = $state<StoredRemote>({ ...NO_REMOTE });
  // Which end of the chat new lines arrive at (P7). The app's default: this
  // window's own chat may be turned over from the bar above it, and that is
  // the view's business and never written here.
  let chatSort = $state<SortDirection>(DEFAULT_DIRECTION);
  // How speech is found and where a segment is cut (P12). Beside the
  // vocabulary rather than inside a profile: how hard the intercom is to hear
  // belongs to the house and its beltpacks, not to the production being
  // carried through it. Pushed into a running session as well as stored —
  // these are set by ear, which means while somebody is talking.
  let tuning = $state<Tuning>({ ...DEFAULT_TUNING });
  // Whether the active profile's session starts the moment the app comes up
  // (P12), without the operator reaching for the Start button. Checked once,
  // in the startup effect below, and never again — an operator who stops the
  // session mid-evening has not asked the app to start it back up on its own.
  let autoStart = $state(false);
  // Whether the first-run guide has been walked, or left. Stored rather than
  // worked out from the state of the machine: "has a model" is asked of the
  // disk on every start and is a different question — an operator who removes
  // a model in Settings is standing in the section that fixes it, and does not
  // need a welcome screen thrown over it.
  let setupDone = $state(false);
  /// Whether the guide is on screen right now.
  ///
  /// Not simply `!setupDone`: it is also opened from the Settings page by
  /// somebody who wants it back, and closing it is a separate act from the
  /// flag being written.
  let guide = $state(false);
  // Whether this view can still reach the app. Always "connected" in the app's
  // own windows; on a remote it is the difference between a live chat and a
  // frozen picture of one, which has to be said rather than looked like
  // nothing being said.
  let connection = $state<Connection>(connectionState());
  // Nothing is saved before the configuration has been read: an empty UI
  // saving itself over a file it has not seen yet is how a setup is lost.
  let loaded = $state(false);
  let saveError = $state<string | null>(null);
  let liveError = $state<string | null>(null);
  /// The group the chat is focused on; `null` is the all-channels view.
  let selectedGroup = $state<number | null>(null);
  /// Whether the window is too narrow to show the sidebar beside the chat —
  /// the LAN remote on a phone (P16), or the app on half a laptop screen. The
  /// sidebar decides this for itself too; what it is needed for *here* is the
  /// burger, which is a control in the bar above the chat and not in the
  /// sidebar it opens.
  let narrow = $state(false);
  $effect(() => watchNarrow((next) => (narrow = next)));
  /// Whether that sidebar is currently drawn over the chat. Bound rather than
  /// read, because both ends close it: the burger, and picking a view in it.
  let sidebarOpen = $state(false);

  // The drawer belongs to the chat. Leaving for Settings with it open would
  // leave it standing over a page it is not about, waiting to be found again
  // on the way back.
  $effect(() => {
    if (view !== "chat") sidebarOpen = false;
  });

  let active = $derived(profiles.find((p) => p.key === activeKey) ?? null);
  // Read-only views of the active profile, so everything below reads the way
  // it did when these were top-level state. Writing happens on the Settings
  // page, into the profile itself — which is what makes an edit something that
  // gets saved.
  let device = $derived(active?.device ?? "");
  let deviceId = $derived(active?.deviceId ?? "");
  let channels = $derived(active?.channels ?? []);
  let groups = $derived(active?.groups ?? []);
  // What fires on this profile's channels, and what it does (P19). On the
  // profile because a trigger names channels and groups — see `rules.ts`.
  let rules = $derived(active?.rules ?? []);

  let status = $state<SessionState | { state: "starting" } | { state: "idle" }>({
    state: "idle",
  });
  let messages = $state<TranscriptEvent[]>([]);
  // When each session's own clock was zero, in epoch milliseconds, so a
  // bubble's `tStartMs` can be shown as a time of day. Keyed by session id and
  // kept for sessions that have already stopped: Start is non-destructive, so
  // the chat holds lines from several of them at once and each needs its own
  // zero point.
  let sessionStarts = $state(new Map<number, number>());
  let levels = $state(new Map<number, StreamLevel>());
  // Whether the machine is going to keep up (P8): the GPU's share, which is
  // what decoding runs out of, and what this app and its workers hold in
  // memory. `null` until the first sample, a second after the window opens —
  // showing 0 % in the meantime would be a lie told at exactly the moment the
  // model is being loaded.
  let usage = $state<ResourceUsage | null>(null);
  // Segments the decode queue had to give up on, per stream. Shown rather than
  // only logged: an operator who spoke and saw no text needs to be able to
  // tell "the app fell behind" from "the channel was not listening".
  let overloaded = $state(new Map<number, number>());
  // What the rules' outbound action has managed to deliver (P19). `null` until
  // the first answer. Cumulative across the sessions of an evening and not
  // cleared with the chat: a cue that failed an hour ago is still the reason
  // the desk did not get it, and the counter is the only trace of it.
  let webhooks = $state<WebhookStatus | null>(null);

  /// Remembers a session's zero point. Called from both places a running state
  /// arrives — the answer to `start_session` and the `session-state` event —
  /// because either can be the first to be seen.
  const noteSession = (state: SessionState | { state: "starting" }) => {
    if (state.state !== "running") return;
    sessionStarts = new Map(sessionStarts).set(state.sessionId, state.startedAtMs);
  };

  /// Carries a channel's new label, language, input trim or mute into the
  /// session that is already running (P6).
  ///
  /// The edit itself has already happened in `profiles`, which is what gets
  /// saved; this is the other half — the session, which the file cannot
  /// describe because it is not the file. Without it a rename during a show
  /// means Stop and Start, and Stop is the one thing an operator will not do
  /// while the room is talking.
  ///
  /// Nothing is done with the state it answers with: the backend announces it
  /// to every view, and this window adopts it through `onSessionState` like
  /// any other — including the group windows and a browser on the LAN, which
  /// get it the same way and at the same time.
  const liveStream = (
    streamId: number,
    update: { label?: string; language?: string; gainDb?: number; muted?: boolean },
  ) => {
    updateStreamConfig({ streamId, ...update })
      .then(() => (liveError = null))
      .catch((e) => (liveError = String(e)));
  };

  /// Sets a channel's input trim from the meter strip.
  ///
  /// Both halves, like the Settings page's own knob does inline: the profile,
  /// which is what gets saved, and — while a session runs — the pipeline. The
  /// strip is where a trim is actually found, because finding it means
  /// watching the bar it moves; the Settings page is where it is written down
  /// beforehand.
  ///
  /// Writing into the profile rather than into `channels`: that is a derived
  /// view of it, and the save follows the profile.
  const setGain = (streamId: number, gainDb: number) => {
    const channel = active?.channels[streamId];
    if (!channel || channel.gainDb === gainDb) return;
    channel.gainDb = gainDb;
    // Whichever of the two has the device open. Neither is the record of the
    // value — the profile above is, and it is what the next session starts
    // with; these are the audio it applies to right now.
    if (running) liveStream(streamId, { gainDb });
    else setMonitorGain(streamId, gainDb).catch(() => {});
  };

  /// Mutes or unmutes a channel from the meter strip.
  ///
  /// The same two halves as the trim — the profile, which is what gets saved,
  /// and the running session — with one difference: there is nothing to tell
  /// the monitor. Mute stops the transcription and not the metering, and the
  /// monitor only ever meters, so while no session runs the flag is simply
  /// recorded and applied by the next Start.
  const setMute = (streamId: number, muted: boolean) => {
    const channel = active?.channels[streamId];
    if (!channel || channel.muted === muted) return;
    channel.muted = muted;
    if (running) liveStream(streamId, { muted });
  };

  let running = $derived(status.state === "running");
  let busy = $derived(status.state === "starting");
  // By stable id where the profile has one — a renamed interface is still
  // this profile's interface, and a second unit of the same model is not.
  let selected = $derived(active === null ? undefined : connectedDevice(devices, active));
  let deviceChannels = $derived(selected?.channels ?? 0);
  // A restored profile may name an interface that is not plugged in — that is
  // the normal case for a setup carried between the office and the house, and
  // it is a different problem from a device with no input channels.
  let connected = $derived(selected !== undefined);
  // The stream id is the row's position, which is exactly how the backend
  // assigns it — see `SessionConfig::streams`.
  let colors = $derived(new Map(channels.map((c, streamId) => [streamId, c.color])));
  let problem = $derived(validate(channels, device, deviceChannels, connected));

  /// Why the input could not be metered with no session running (P8), or
  /// `null` while it can be.
  ///
  /// The meters are how a trim is set, and a trim is set *before* a show. So
  /// the window opens the device on its own: no model, no sidecar, no
  /// transcripts — a few ring buffers and a thread that adds up squares. See
  /// `crates/app/src/monitor.rs`.
  ///
  /// In every view and not only in Settings: "is anything coming in on this
  /// line?" is the first question of a get-in, and the answer should not be
  /// three clicks away behind a page about configuration.
  ///
  /// Only the failure is kept, and not a "metering now" flag: the strip is on
  /// screen either way, and what the operator needs said is the one case
  /// where the bars are never going to move.
  let meterError = $state<string | null>(null);

  /// What the meters would be opened on, or `null` when they should not be.
  ///
  /// A string of exactly the two things `start_monitor` cares about — the
  /// device and the channel map — so that editing a label, a colour or a
  /// language does not reopen the interface underneath a bar somebody is
  /// watching.
  let meterKey = $derived(
    !running && !busy && connected && channels.length > 0
      ? `${deviceId || device}\u0000${channels.map((c) => c.channel).join(",")}`
      : null,
  );

  $effect(() => {
    if (meterKey === null) {
      meterError = null;
      levels = new Map();
      stopMonitor().catch(() => {});
      return;
    }

    // Debounced, because the channel column is a number box: typing "12" goes
    // through channel 1 on the way, and opening the device for it would mean
    // one reopen per keystroke.
    const timer = setTimeout(() => {
      // Read here rather than above on purpose: a timer callback is outside
      // the effect's tracking, so the labels and colours this reads do not
      // make it run again. `meterKey` alone decides that, which is the point
      // of having it.
      startMonitor({
        deviceId: deviceId || device,
        streams: toConfig(channels),
        vocabulary: null,
        // Neither of the last two reaches anything here — metering does not
        // segment and does not decode — but the meters are keyed on the
        // stream ids in `streams`, and a second shape of what starts the
        // device would be a second thing that can disagree about which
        // channel is which.
        tuning: clampTuning(tuning),
      })
        .then(() => (meterError = null))
        .catch((e) => (meterError = String(e)));
    }, METER_DEBOUNCE_MS);

    return () => clearTimeout(timer);
  });
  // Which streams receive no audio because the device has no such channel —
  // not the ones the operator muted, which are being listened to and are
  // marked by their own lit M. While a session runs this comes from the
  // backend's own answer rather than from the UI's copy of the setup — that
  // is the authoritative mapping, and the two would differ the moment the
  // device is swapped under a running session.
  let absent = $derived(
    status.state === "running"
      ? new Set(status.streams.filter((s) => s.absent).map((s) => s.streamId))
      : new Set(absentChannels(channels, deviceChannels)),
  );
  // How many channels the operator has silenced, for the status bar. From the
  // session's own answer for `absent`'s reason, and excluding the channels
  // that are not on the device anyway — counting those would say "2 muted"
  // about an interface with too few inputs, which points the operator at the
  // wrong fix.
  let mutedCount = $derived(
    status.state === "running"
      ? status.streams.filter((s) => s.muted && !s.absent).length
      : 0,
  );

  /// The status bar's one line: device, channel count, rate, model.
  ///
  /// Four short facts and nothing else. What used to be here as well — the
  /// absent and muted counts, CPU or GPU, how long the model took to load —
  /// wrapped onto a second row on a phone and is anyway read better
  /// elsewhere: an absent or muted channel is marked on its own meter, and
  /// the model's origin is on the Settings page. It is kept as the bar's
  /// title, where a problem being reported can still be read off it.
  let statusLine = $derived.by(() => {
    if (status.state !== "running") return "";
    const count = status.streams.length - absent.size;
    const model = status.model.split("/").pop();
    return `${status.device} · ${count} ch. · ${status.sampleRate / 1000}kHz · ${model}`;
  });

  let statusDetail = $derived.by(() => {
    if (status.state !== "running") return "";
    const count = status.streams.length - absent.size;
    const channels = `${count} ${count === 1 ? "channel" : "channels"}`;
    const missing = absent.size > 0 ? `, ${absent.size} not on the device` : "";
    const muted = mutedCount > 0 ? `, ${mutedCount} muted` : "";
    const model = status.model.split("/").pop();
    const where = status.gpu ? "GPU" : "CPU";
    const load = (status.loadMs / 1000).toFixed(1);
    return `${status.device} · ${channels}${missing}${muted} · ${status.sampleRate} Hz · ${model} · ${where}, loaded in ${load} s`;
  });

  let group = $derived(groups.find((g) => g.key === selectedGroup) ?? null);
  // Which view this window currently *is*, as a rule addresses one: the
  // selected group's position, or the view of everything. The sidebar changes
  // it, which is right — a rule that flashes the stage manager's window
  // flashes this one while this one is showing that group.
  let viewing = $derived<GroupView>(
    group === null ? ALL_CHANNELS_VIEW : groups.findIndex((g) => g.key === group.key),
  );
  // `null` means every stream, which is not the same as an empty set: a group
  // with no members shows nothing, and saying so is more useful than silently
  // falling back to showing everything.
  let visible = $derived(visibleStreams(group, channels));
  let shown = $derived(
    visible === null ? messages : messages.filter((m) => visible.has(m.streamId)),
  );
  // The whole session grouped the way the main chat groups it, which is what
  // an attention request is counted per (P17): a sentence arrives as three
  // segments, and three flashes for one sentence is the noise this feature
  // must not add. Every channel, not the selected group's — the main window
  // is the view of the whole session.
  //
  // A second pass over the messages next to the chat's own. It is the same
  // order of work as the append the list already does, and it buys the one
  // thing a per-segment rule cannot get right; what would change that is
  // loading a session from disk, which P10 was set aside for.
  let bubbles = $derived(toBubbles(messages));

  // Startup: the devices that are there, and the setup from last time.
  //
  // Both before anything else, and the configuration is not optional to the
  // result — a window that came up with an empty channel map and then filled
  // it in would spend its first moment looking like the setup was lost.
  $effect(() => {
    const first = async () => {
      const found = await listDevices();
      devices = found;
      const fallback = found.find((d) => d.isDefault) ?? found[0];

      // A configuration that cannot be read is not a reason to refuse to
      // transcribe: the app comes up empty and says why, rather than not
      // coming up. Saying why matters because the same failure is the one that
      // will silently lose whatever is built next.
      let stored = null;
      try {
        stored = await loadConfig();
        // What the app has, before this view has changed anything. Without
        // this seed the first save writes the file back to itself, which every
        // other view then hears as a change worth adopting — a burst of work
        // on every window for a configuration nobody touched.
        agreed = canonical(stored);
      } catch (e) {
        saveError = String(e);
      }

      const restored = stored
        ? fromStoredConfig(stored)
        : {
            profiles: [],
            activeKey: null,
            vocabulary: null,
            models: {},
            remote: { ...NO_REMOTE },
            chatSort: DEFAULT_DIRECTION,
            tuning: { ...DEFAULT_TUNING },
            autoStart: false,
            setupDone: false,
          };

      if (restored.profiles.length === 0) {
        // First start: one profile on the default device, with one channel, so
        // the app is usable without first having to work out that a profile
        // and a channel have to be created.
        const created = newProfile([], {
          name: fallback?.name ?? "",
          id: fallback?.id ?? "",
        });
        profiles = [created];
        activeKey = created.key;
      } else {
        // A setup written before stable ids names its interface and does not
        // identify it. If that interface is plugged in now, this is where it
        // gains an id — and the save below carries it to the file, so the
        // upgrade costs the operator nothing and happens once.
        profiles = adoptDeviceIds(restored.profiles, found) ?? restored.profiles;
        activeKey = restored.activeKey;
      }

      // `null` is a file that has never held a vocabulary, which keeps the
      // default; an empty string is a box the operator cleared, which stays
      // cleared — refilling it would turn the prompt back on behind their
      // back.
      if (restored.vocabulary !== null) vocabulary = restored.vocabulary;
      models = restored.models;
      remote = restored.remote;
      tuning = restored.tuning;
      autoStart = restored.autoStart;
      setupDone = restored.setupDone;

      loaded = true;

      // Who gets the guide, decided once and here rather than by a component
      // that would have to ask again on every render.
      //
      // A file written before the guide existed says nothing, and most of
      // those machines are set up and running — greeting them with a welcome
      // screen would be the app forgetting a setup that is plainly there. So
      // the question asked of an installation that has never answered it is
      // the one the guide is actually for: **can this machine transcribe?**
      // If it can, the flag is settled silently and the operator never sees
      // this; if it cannot, they see it once.
      //
      // Never on the LAN remote (P16). The models are downloaded onto the host
      // machine and the interface is plugged into the host machine; a phone
      // that has just connected to a running production is not the thing being
      // set up, and a wizard over the chat is the last thing it needs.
      if (!setupDone && !isRemote) {
        try {
          const state = await listModels();
          // Speech detection counts as well as the model. It is 0.9 MB and the
          // app is much worse without it, so a machine missing it has not
          // finished being set up even though it would start.
          if (state.problem === null && state.vad) setupDone = true;
          else guide = true;
        } catch {
          // No model directory at all: nothing can be downloaded and the guide
          // would only be able to say so. The Settings page says it in the
          // place that can act on it.
          setupDone = true;
        }
      }

      await catchUp();

      // Reached only once, right after the setup from last time has been
      // restored: an operator who stopped the session and left it stopped has
      // not asked the app to start it back up the moment something else in
      // the file changes.
      if (autoStart && !running) await start();
    };

    first().catch((e) => (status = { state: "failed", message: String(e) }));
  });

  /// What the backend already holds.
  ///
  /// Normally nothing — this window is the one that has been here all along —
  /// but a reloaded WebView comes back to the chat it was showing instead of
  /// to an empty one, a remote page opens in the middle of a show, and a
  /// remote that lost its socket missed whatever was said while it was gone.
  /// All three are the same catch-up, from the same place, merged into
  /// whatever is already held.
  const catchUp = async () => {
    const snapshot = await sessionSnapshot();
    messages = mergeHistory(snapshot.transcripts, messages);
    sessionStarts = new Map([
      ...snapshot.sessions.map((s): [number, number] => [s.sessionId, s.startedAtMs]),
      ...sessionStarts,
    ]);
    if (status.state === "idle" && snapshot.state) status = snapshot.state;
    // The outbound counters are caught up the same way and for the same
    // reason: they are cumulative, and a window opened after a cue failed has
    // to see that it did. Its own call, because it is not part of the chat.
    webhooks = await webhookStatus();
  };

  // A group window is a view onto a group of the profile that is running, and
  // the stream ids its transcripts carry are positions in *that* profile's
  // channel list. Carried over to another production it would be filtering the
  // right numbers against the wrong channels, so switching profiles closes the
  // windows — a setup action, not something done during a show.
  let showing: number | null = null;
  $effect(() => {
    const key = activeKey;
    if (!loaded) return;
    untrack(() => {
      const previous = showing;
      showing = key;
      if (previous === null || previous === key) return;
      closeGroupWindows().catch((e) => console.warn("a group window stayed open:", e));
    });
  });

  // Saving is a consequence of the state changing, not of any particular
  // button: there is no Save on the Settings page, because a setup an operator
  // believes is stored and is not is worse than no persistence at all.
  // Debounced, so typing a label is one write; the write itself is atomic on
  // the backend, so an unlucky moment costs nothing.
  /// The configuration this view knows the app already has, in a form that
  /// does not depend on field order — see `canonical`.
  ///
  /// One value, doing the whole of the last-write-wins arrangement (P16), and
  /// the rule it enforces is: **never write what is already there**.
  ///
  /// It is seeded from the file at startup, set to what is written on every
  /// save, and set to what arrives on every adopted save. The save effect then
  /// skips a configuration that equals it. Without that, adopting is itself a
  /// state change, which schedules a save, which is announced to the peer that
  /// sent it, which adopts it in turn — two views writing the same
  /// configuration at each other several times a second, forever, with every
  /// field on the Settings page rebuilt under the cursor on each pass.
  ///
  /// A guard rather than a flag on the adoption, because it is true of any
  /// save and not only of that one: an edit that ends up where it started is
  /// not a change either.
  let agreed: string | null = null;

  /// Writes a configuration, unless it is already what the app has.
  ///
  /// The guard is what keeps two views from bouncing one configuration off
  /// each other: adopting another peer's save is a state change like any
  /// other and schedules a save of its own, and writing that back is the start
  /// of a conversation nothing ends. The backend refuses a no-op save as well,
  /// which is what makes that true of every peer rather than only of this one
  /// — this half is here so the round trip is not made at all.
  const write = (config: StoredConfig) => {
    const written = canonical(config);
    if (written === agreed) return;

    // Set before the write, not after it: the echo can arrive while the save
    // is still resolving, and an echo that is not recognised is this page
    // adopting its own keystroke. A save that then fails says so in the strip
    // at the top, and no echo follows it, because nothing was written.
    agreed = written;
    saveConfig(config)
      .then(() => (saveError = null))
      .catch((e) => {
        saveError = String(e);
        // And taken back again, or this view believes the app holds something
        // it refused: the save effect skips a configuration that equals
        // `agreed`, so the edit would sit here unwritten until some unrelated
        // field changed. Only if nothing has been written since — a later save
        // owns the value now, and this one is simply the older failure.
        if (agreed === written) agreed = null;
      });
  };

  /// The configuration as this view currently holds it, for the one save that
  /// cannot wait for anything — see `beforeLeaving`.
  let latest: StoredConfig | null = null;

  $effect(() => {
    if (!loaded || activeKey === null) return;

    // Built here rather than in the timer: this is what reads every field, and
    // reading them is what makes this effect run again when one of them
    // changes.
    const config = toStoredConfig(
      profiles,
      activeKey,
      vocabulary,
      models,
      remote,
      chatSort,
      tuning,
      autoStart,
      setupDone,
    );
    latest = config;

    let stopWaiting: (() => void) | null = null;
    const timer = setTimeout(() => {
      // Not while a field is being typed into. Writing the configuration is
      // what every view rebuilds its Settings page from, and rebuilding the
      // field under the cursor is how a label comes to be typed one character
      // at a time. Leaving the field is the operator saying they are done with
      // it, and it is something they were going to do anyway — see
      // `editing.ts`. A change that is not typed at all — a colour, a
      // language, a device — saves as soon as the debounce is up, because
      // there is nothing half-finished about it.
      stopWaiting = whenEditingEnds(() => write(config));
    }, SAVE_DEBOUNCE_MS);

    return () => {
      clearTimeout(timer);
      // Another keystroke: the wait is re-armed below with the newer
      // configuration rather than saving the one it was holding.
      stopWaiting?.();
    };
  });

  // The net under that rule. A window closed with the cursor still in a label
  // would otherwise take the label with it, and the one thing the Settings
  // page must never do is lose a setup the operator believes is stored.
  $effect(() => beforeLeaving(() => latest && write(latest)));

  /// Takes on a configuration somebody else wrote.
  ///
  /// The configuration is last-write-wins and every save is announced to every
  /// view, so this is the half that makes the side which *lost* see it happen
  /// rather than sit on a stale form until its next keystroke overwrites the
  /// other peer's work.
  ///
  /// Keys are carried over by position — see `reuseKeys`. Without that, a
  /// channel renamed on the remote would come back as an all-new profile with
  /// an all-new active key, which this window would read as somebody switching
  /// production: the selected group would fall back to the all-channels view,
  /// every detached group window would close, and every field on the Settings
  /// page would be torn down and rebuilt under the cursor.
  const adopt = (stored: StoredConfig) => {
    const incoming = canonical(stored);
    // Its own echo. Every save comes back this way, including this window's,
    // and it has been through the backend — so it is compared in a form that
    // does not depend on which order the two sides write their fields in.
    if (incoming === agreed) return;
    agreed = incoming;

    const restored = fromStoredConfig(stored);
    profiles = reuseKeys(restored.profiles, profiles);
    activeKey = profiles[stored.active]?.key ?? profiles[0]?.key ?? null;
    if (restored.vocabulary !== null) vocabulary = restored.vocabulary;
    models = restored.models;
    remote = restored.remote;
    chatSort = restored.chatSort;
    tuning = restored.tuning;
    autoStart = restored.autoStart;
    // Adopted like everything else: the guide is walked in one window, and a
    // second one still showing it would be asking for a setup that has just
    // been made. It closes the guide but never reopens it — a peer that has
    // not been set up is not this window's errand.
    setupDone = restored.setupDone;
    if (setupDone) guide = false;
  };

  // The compiled rules live on the backend, where every transcript is
  // evaluated against them before it reaches any view (P19). Pushed from here
  // whenever they change rather than read from the file by the backend: the
  // file is a debounce behind the rule that was just written, and a rule that
  // starts working a second late is a rule nobody can tell is working.
  //
  // Compiled against the *active profile's* channels and groups, so this also
  // runs when a group gains a member or a channel is deleted: a rule that
  // names a group follows it, and the stream ids it resolves to are what
  // changed. Switching profile is the same event — the other profile's rules
  // are other rules — which is why this reads `rules`, `channels` and `groups`
  // rather than only the first.
  //
  // Sent even when it is empty: that is how deleting the last rule takes
  // effect, and how a reloaded window matches a backend still holding what it
  // was told before.
  let compiled: CompiledRule[] | null = null;
  $effect(() => {
    if (!loaded) return;
    const next = compileRules(rules, channels, groups);
    // Writing a word is one keystroke at a time, and every intermediate state
    // is a rule: without this, "Feuer" is five compilations, four of which
    // fire on a word nobody is looking for.
    if (compiled !== null && sameCompiled(compiled, next)) return;

    const timer = setTimeout(() => {
      setRules(next)
        .then(() => (compiled = next))
        .catch((e) => console.warn("the rules were not compiled:", e));
    }, SAVE_DEBOUNCE_MS);

    return () => clearTimeout(timer);
  });

  // The detection thresholds, pushed into the running session the way the
  // rules are and for the same reason (P12): the file is a debounce behind
  // the slider, and a threshold that starts working a second late is a
  // threshold nobody can set by ear.
  //
  // Only when a session is running. Without one there is nothing to push to —
  // the next Start carries the numbers itself — and the backend would
  // cheerfully accept a call that reached nothing.
  let pushed: Tuning | null = null;
  $effect(() => {
    if (!loaded || !running) return;
    const next = clampTuning(tuning);
    // Dragging a slider fires per pointer event, and the pipeline does not
    // need the forty values on the way to the one that was let go of.
    if (pushed !== null && sameTuning(pushed, next)) return;

    const timer = setTimeout(() => {
      updateTuning(next)
        .then(() => (pushed = next))
        .catch((e) => console.warn("the thresholds did not reach the session:", e));
    }, SAVE_DEBOUNCE_MS);

    return () => clearTimeout(timer);
  });

  const sameTuning = (a: Tuning, b: Tuning) =>
    a.silenceTimeoutMs === b.silenceTimeoutMs &&
    a.maxSegmentMs === b.maxSegmentMs &&
    a.speechThresholdPercent === b.speechThresholdPercent;

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
      // And out of every rule that named it (P19). A rule left pointing at a
      // deleted channel is worse than a group left pointing at one: with the
      // last of its channels gone the trigger reads as "anywhere", so `forget`
      // switches that rule off rather than letting it grow louder than it was
      // written.
      const rewritten = forgetInRules(active.rules, active.channels, active.groups);
      if (
        rewritten.some(
          (r, i) =>
            r.channels.length !== active.rules[i].channels.length ||
            r.views.length !== active.rules[i].views.length,
        )
      ) {
        active.rules = rewritten;
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
        attend(event);
      }),
      onLevels((event) => {
        levels = new Map(event.streams.map((s) => [s.streamId, s]));
      }),
      onOverload((event) => {
        overloaded = new Map(overloaded).set(event.streamId, event.dropped);
      }),
      onResources((event) => {
        usage = event;
      }),
      // Every outcome, not a sample: the first drop is the interesting one.
      onWebhookStatus((status) => {
        webhooks = status;
      }),
      // Emptied here or in a group window — the backend holds the copy they
      // are all caught up from, so the deletion happens there and every window
      // hears about it the same way, including the one that asked.
      onCleared(clearLocally),
      // A group window pinning or unpinning itself. It has already placed
      // itself; what it is asking for is that the next window it opens as
      // comes up the same way. Stored here because the configuration has one
      // writer — see `requestGroupOnTop` — and it is saved by the same
      // debounce as everything else on this page.
      onGroupOnTop(({ group, onTop }) => {
        if (!active) return;
        const target = active.groups[group];
        if (!target || target.onTop === onTop) return;
        target.onTop = onTop;
      }),
      // A save made somewhere else: the LAN remote (P16), and in principle any
      // other view that writes the configuration. The group windows have
      // followed this all along; the main window has to as well now that it is
      // no longer the only writer.
      onConfigChanged(adopt),
      onSessionState((event) => {
        status = event;
        noteSession(event);
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

    // A remote page that lost the socket missed everything said while it was
    // gone. It opens again: the same snapshot, merged into what it still
    // holds. In the app this never fires — an `invoke` has no connection to
    // lose.
    const reconnected = onReconnected(() => {
      catchUp().catch((e) => console.warn("the chat was not caught up:", e));
    });
    const watching = onConnection((state) => (connection = state));

    return () => {
      reconnected();
      watching();
      for (const subscription of subscriptions) {
        subscription.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  });

  /// Makes this window's half of what the rules asked for (P19).
  ///
  /// *What* was decided on the backend, once, for every view. What is decided
  /// here is whether this window answers for it. An attention request is made
  /// for every stream: this is the view of the whole session, and a rule that
  /// fires on a channel the operator is not currently looking at must still
  /// reach them. The flood is narrower on both counts — only for what is on
  /// screen, and only for a rule addressed to the view this window currently
  /// is. A rule that names the stage manager's window does not flood the view
  /// of everything just because everything includes that channel.
  ///
  /// Counted per **bubble**, not per segment: one sentence spoken across three
  /// segments would otherwise be three flashes. See `attention.ts`.
  ///
  /// Whether the window is focused is the backend's question, because the
  /// window is: an operator already looking at the chat is not asked to look
  /// at it. And never a sound — this is a room that is already full of speech.
  const attend = (event: TranscriptEvent) => {
    const asking = attentionFor(event, bubbles[bubbles.length - 1], {
      attention: null,
      // `visible` is `null` for the all-channels view, which is exactly what
      // `shown` means by it.
      shown: visible,
      view: viewing,
    });
    if (!asking) return;
    requestAttention(asking.ask, asking.loud).catch((e) =>
      console.warn("no attention request:", e),
    );
  };

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
        // The id, not the name. A profile that predates stable ids still
        // holds only a name; the backend resolves that too, which is what
        // lets such a setup start while its interface is unplugged elsewhere.
        deviceId: deviceId || device,
        streams: toConfig(channels),
        // One vocabulary for every stream — see `vocabulary.ts`.
        vocabulary: vocabularyConfig(vocabulary),
        // And one set of thresholds, for the same reason: they describe the
        // intercom system, not one position on it (P12).
        tuning: clampTuning(tuning),
      });
      noteSession(status);
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
  //
  // It is a backend command rather than an assignment here, because the chat no
  // longer lives in one window: the backend holds the copy every view is caught
  // up from, and a window that only emptied itself would leave an open group
  // window showing what this one was just told is gone. The local half happens
  // on the event that comes back.
  const clearCommunication = () => {
    clearOnBackend().catch((e) => console.warn("the chat was not cleared:", e));
  };

  /// Points the active profile at an interface, from the setup guide.
  ///
  /// Both halves at once, the way the Settings page's own picker does it: the
  /// id is what the profile is keyed on and what starting a session sends, and
  /// the name is stored beside it so the device can still be named on a
  /// machine it is not plugged into.
  const pickDevice = (id: string) => {
    const option = devices.find((d) => d.id === id);
    if (!option || !active) return;
    active.device = option.name;
    active.deviceId = option.id;
  };

  /// Closes the guide, whether it was finished or left.
  ///
  /// Either way it is done: a guide that comes back on every start is a guide
  /// that gets clicked through without being read, and it is one button away
  /// on the Settings page for anybody who wants it again. The flag is saved by
  /// the same debounce as everything else on this page.
  const closeGuide = () => {
    guide = false;
    setupDone = true;
  };

  const clearLocally = () => {
    messages = [];
    // The zero points are only there to time the lines that were just thrown
    // away — except the running session's, which the lines arriving from now
    // on still need. No `session-state` event is due while a session runs, so
    // dropping it here would leave the rest of that session untimed.
    sessionStarts =
      status.state === "running"
        ? new Map([[status.sessionId, status.startedAtMs]])
        : new Map();
    overloaded = new Map();
  };
</script>

<svelte:window onkeydown={shortcut} />

<main>
  <header class="bar">
    <h1>Fluestr</h1>

    <nav class="tabs">
      <button
        class="tab"
        class:active={view === "chat"}
        onclick={() => (view = "chat")}
        title="Chat (⌘1)"
      >
        Chat
      </button>
      <button
        class="tab"
        class:active={view === "settings"}
        onclick={() => (view = "settings")}
        title="Settings (⌘2, ⌘,)"
      >
        Settings
      </button>
    </nav>

    <div class="right">
      <!-- Which setup Start would start, beside the button that starts it.
           The name lives in Settings, three clicks away, and an operator who
           has two houses in the list can otherwise only tell them apart by
           what the channel labels say once audio is already running — which
           is one show too late to find out the wrong profile was loaded. -->
      {#if active}
        <span class="profile" title="Active profile">{active.name}</span>
      {/if}

      <!-- Beside the button it decides the fate of, rather than a page away
           in Settings: what it sets is exactly "what happens the next time
           this button would otherwise have to be pressed", and that is a
           question answered by looking at the button, not by opening a
           different page to guess at it. -->
      <label class="auto-start" title="Start automatically the next time the app opens">
        <input type="checkbox" bind:checked={autoStart} />
        <span>Auto-start</span>
      </label>

      {#if running}
        <!-- Red, unlike Start: it is the one button in the header that
             ends what is running, and during a show it is pressed by
             somebody looking for it rather than reading it. -->
        <button class="primary danger" onclick={stop} title="Stop (⌘T)">Stop</button>
      {:else}
        <button
          class="primary"
          onclick={start}
          disabled={busy || device === "" || problem !== null}
          title="Start (⌘T)"
        >
          {busy ? "Starting …" : "Start"}
        </button>
      {/if}
    </div>
  </header>

  <!-- A remote page that has lost the socket is a picture of a chat, not a
       chat: nothing new arrives and nothing pressed here reaches the session.
       Said at the top of the window rather than left to be inferred from
       silence, which on an intercom is also what a quiet room looks like
       (P16). Never shown in the app, where there is no connection to lose. -->
  {#if isRemote && connection !== "connected"}
    <div class="bar offline">
      <span class="error">●</span>
      <span>
        Not connected to Fluestr. What is on screen is what arrived
        before the connection went; it is being retried.
      </span>
    </div>
  {/if}

  {#if view !== "settings"}
    <div class="bar status">
      {#if status.state === "running"}
        <span class="ok">●</span>
        <!-- Four short facts, in the width of one bar — see `statusLine`. -->
        <span title={statusDetail}>{statusLine}</span>
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

      <!-- Clear first, then the meters. The button is a control and the
           meters are a reading, and the two are not looked for in the same
           way: a control is reached for, a number is glanced at. Putting the
           reading at the very end of the bar also stops the button moving
           sideways as the numbers in it change width. -->
      <div class="right">
        {#if view === "chat"}
          <ClearCommunication
            empty={messages.length === 0}
            clear={clearCommunication}
          />
        {/if}
        <ResourceMeter {usage} />
      </div>
    </div>
  {/if}

  <!-- On a narrow window the sidebar is not a column beside the chat but a
       drawer over it, and this is the line it is reduced to: the burger that
       opens it, and the name of the view that is showing.

       Directly above the chat rather than up in the header, and the same
       shape as the Settings page's rail bar: both are a way into a drawer
       over the thing below them, and a burger that sits two bars away from
       what it opens is a burger nobody connects to the drawer that appears.
       The name is the half that matters — a burger alone would leave "which
       of my groups am I reading?" to be answered by looking at the channel
       colours in the bubbles. -->
  {#if narrow && view === "chat"}
    <div class="bar viewbar">
      <button
        class="burger"
        onclick={() => (sidebarOpen = !sidebarOpen)}
        aria-expanded={sidebarOpen}
        aria-label={sidebarOpen ? "Close the list of views" : "Views"}
        title="Views"
      >
        <!-- Drawn rather than an emoji or a "≡", for the reason the group
             window's pin is drawn: it has to take its weight and its colour
             from the row it is in at every font size. -->
        <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
          <path
            d="M2 4h12M2 8h12M2 12h12"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
          />
        </svg>
      </button>
      <span class="where">{group ? group.name : "All channels"}</span>
    </div>
  {/if}

  {#if view === "settings"}
    <SettingsPage
      {devices}
      bind:profiles
      bind:activeKey
      bind:vocabulary
      bind:models
      bind:remote
      bind:sort={chatSort}
      bind:tuning
      {deviceChannels}
      locked={running || busy}
      {running}
      {saveError}
      {liveError}
      live={running ? liveStream : null}
      {levels}
      {meterError}
      {problem}
      openGuide={() => (guide = true)}
    />
  {:else}
    <div class="chat">
      <GroupSidebar
        {groups}
        {channels}
        bind:selected={selectedGroup}
        bind:open={sidebarOpen}
      />

      <div class="stream">
        {#if channels.length > 0}
          <!-- The meters follow the same view — eight bars for a
               three-channel group are five things to read past. The filtering
               happens by stream id inside the component, because the stream id
               *is* the position in `channels`.

               Shown before Start as well, off the monitor (P8): the bars are
               how an operator sees that the interface is plugged in and that
               somebody is talking on the line, and that is checked before the
               session, not after it.

               Mounted whenever the profile has channels, and not only while
               something is feeding it, because the two sources hand over
               through a gap: Start takes the device off the monitor before
               its own capture is up, and Stop gives it back a debounce
               later. Tied to either source the strip would vanish and come
               back on every Start and every Stop — and the chat below it
               would jump by the height of it, twice a show. Bars with
               nothing arriving read as a quiet line, which is what it is. -->
          <StreamMeters
            {channels}
            {levels}
            {overloaded}
            {visible}
            {absent}
            {webhooks}
            gain={setGain}
            mute={setMute}
          />
        {/if}

        {#if visible !== null && visible.size === 0}
          <p class="empty">
            This group has no channels yet. Add some on the Settings page.
          </p>
        {:else}
          <!-- Every new bubble flashes, in every view: it is how the line
               that just arrived is told from the ones above it. What a view's
               own setting adds is the loud request — the whole window in dark
               orange — and that is the attention request's business, not the
               chat's. -->
          <ChatList
            messages={shown}
            {colors}
            starts={sessionStarts}
            direction={chatSort}
            {running}
            startHint={isRemote
              ? "Press Start above to begin transcribing."
              : "Press ⌘T, or Start above, to begin transcribing."}
          />
        {/if}
      </div>
    </div>
  {/if}

  <!-- Over both pages, because it is about neither: a machine with no model
       has nothing to show in the chat and no reason to be reading Settings
       yet. Mounted last so it is over the header as well — Start is the one
       button on this window that cannot work until the guide's first two
       steps are done. -->
  {#if guide}
    <SetupGuide
      {devices}
      {deviceChannels}
      {pickDevice}
      profile={active}
      bind:models
      {levels}
      ondone={closeGuide}
    />
  {/if}
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    height: 100vh;
    /* The viewport a phone actually gives the page — see `app.css`. */
    height: 100dvh;
    position: relative;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.6rem 1rem;
    /* Under the notch and out again: the bar's background reaches the edge of
       the screen, its contents stop where the phone's own furniture starts.
       `max` and not an addition, so the ordinary 1 rem gutter is what a
       device with no insets keeps. */
    padding-left: max(1rem, var(--safe-left));
    padding-right: max(1rem, var(--safe-right));
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-wrap: wrap;
  }

  /* The topmost bar is the one that clears the status bar and the camera. */
  header.bar {
    padding-top: max(0.6rem, var(--safe-top));
  }

  /* A strip in the shape of the bars above it: it is part of the window's
     furniture, not a control floating over the transcripts. A button over the
     chat would cover the newest line, which on this page is the one thing
     that must never be covered. */
  .viewbar {
    flex-wrap: nowrap;
    gap: 0.5rem;
  }

  /* Where the operator is. The brightest thing on the line, because it is the
     whole reason the line is there. */
  .viewbar .where {
    font-size: 0.85rem;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .burger {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: none;
    background: transparent;
    color: var(--text-dim);
    border: 1px solid transparent;
    border-radius: 0.3rem;
    padding: 0.3rem 0.4rem;
    font: inherit;
    line-height: 0;
    cursor: pointer;
  }

  .burger:hover {
    color: var(--text);
  }

  /* Lit while the drawer it opened is showing, the way every other two-state
     control here is: a burger that looks the same open and closed is one
     nobody can tell they already pressed. */
  .burger[aria-expanded="true"] {
    color: var(--accent);
    background: var(--bg);
    border-color: var(--border);
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
    /* What the sidebar's drawer is positioned against on a narrow window, so
       it covers the transcripts and nothing above them. */
    position: relative;
  }

  .stream {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  .right {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-left: auto;
  }

  .status {
    font-size: 0.85rem;
    gap: 0.5rem;
  }

  .offline {
    font-size: 0.85rem;
    gap: 0.5rem;
    background: #3a1f1f;
  }

  .empty {
    color: var(--text-dim);
    font-size: 0.85rem;
    padding: 1rem;
    padding-left: max(1rem, var(--safe-left));
    padding-right: max(1rem, var(--safe-right));
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
  }

  /* Dim and unemphasised: it is a label on the button beside it, not a control
     — and a name in the app's accent beside a Start button reads as something
     to press. Truncated rather than allowed to push Start off a narrow
     window; the whole name is in the tooltip and in Settings. */
  .profile {
    font-size: 0.85rem;
    color: var(--text-dim);
    max-width: 14rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .auto-start {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.8rem;
    color: var(--text-dim);
    white-space: nowrap;
    cursor: pointer;
  }

  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }

  /* Stop, and only Stop. The same red every other destructive control in the
     app uses, so the button that ends the session is found by colour during a
     show rather than read. */
  .primary.danger {
    background: #ff6b6b;
  }

  .dim {
    color: var(--text-dim);
  }

  /* --- A window the width of a hand --------------------------------------

     The same 45 rem as `layout.ts` matches on, written out because a media
     query cannot read a constant. The header is the bar that has to give: it
     carries a title, two tabs, a profile name and Start, and on a phone in
     portrait that is two lines of furniture above a chat.

     What goes is the title, and only the title. The app is named in the tab
     strip and in the window's own chrome, and every other thing in the bar is
     either a place to go or the control the whole session turns on. */
  @media (max-width: 45rem) {
    h1 {
      display: none;
    }

    .bar {
      gap: 0.5rem;
      padding: 0.45rem 0.7rem;
      padding-left: max(0.7rem, var(--safe-left));
      padding-right: max(0.7rem, var(--safe-right));
    }

    header.bar {
      padding-top: max(0.45rem, var(--safe-top));
    }

    /* The status line is a sentence — the device, the channel count, the
       sample rate, the model — and on a phone it is the one thing here that
       may take a second line rather than push the meters off the end of the
       first. */
    .status {
      row-gap: 0.25rem;
    }

    /* Beneath the sentence rather than at the end of it, and pushed to the
       right the way it is on the desk: `margin-left: auto` on a wrapped line
       keeps the resource meter and Clear communication together at the edge
       instead of stranding them mid-row. */
    .status .right {
      gap: 0.5rem;
    }

    .profile {
      max-width: 8rem;
    }

    /* Gone entirely, label and box. It is set once, between shows, and the
       header on a phone is already a profile name and Start on one line —
       the one control here the session turns on. Auto-start keeps whatever it
       was given on the desk; it is not changed by being unreachable. */
    .auto-start {
      display: none;
    }
  }

  .ok {
    color: #5fd38d;
  }

  .error {
    color: #ff6b6b;
  }
</style>

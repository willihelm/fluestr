// Typed view of the backend.
//
// Every call here goes through one command layer on the other side
// (`crates/app/src/commands.rs`), and reaches it over whichever transport this
// frontend is running on: an `invoke` in the app's own WebView, a WebSocket
// frame from a browser served over the LAN (P16). See `transport.ts` — nothing
// below this line knows which of the two it is.
//
// Hand-written for now. P6 still owes types generated from the Rust structs
// with `ts-rs` or `specta`, which is what stops the two sides drifting apart —
// until then the types here have to be kept in step with
// `crates/app/src/session.rs`, and the round-trip tests there pin down the
// field names.

import { invoke } from "@tauri-apps/api/core";
import { type UnlistenFn } from "@tauri-apps/api/event";
import { call, emit, isRemote, on } from "./transport";
import { flashPage } from "./flash";
import { closeGroupTabs, openGroupTab } from "./windows";

/// One language whisper can be given, as the sidecar reports it. `code` is
/// ISO 639-1 ("de"), `name` is whisper's own spelling ("german").
export interface Language {
  code: string;
  name: string;
}

export interface DeviceOption {
  /// Stable across restarts — the CoreAudio UID on macOS. This is what a
  /// profile stores and what starting a session sends back; `name` is for the
  /// operator to read. See `audio_core::DeviceId`.
  id: string;
  name: string;
  channels: number;
  defaultSampleRate: number;
  isDefault: boolean;
}

// One intercom channel. `channel` is zero-based, like the audio API — the UI
// counts from 1 everywhere it is shown and converts at that one boundary.
//
// No stream id: the backend assigns it from the position in `streams`, so two
// streams can never be given the same one.
export interface StreamConfig {
  channel: number;
  label: string;
  language: string;
}

export interface SessionConfig {
  deviceId: string;
  streams: StreamConfig[];
  /// The active vocabulary, applied to every stream. Session-level rather than
  /// per stream because the terms of a production are the same on every
  /// channel — see `vocabulary.ts` and P15.
  vocabulary: string | null;
}

/// Where an alert term occurs in a transcript (P17), in **UTF-16 code units**
/// — which is what `String.prototype.slice` counts, so these can be used as
/// they arrive.
///
/// Matched on the backend, once, so that every view highlights the same
/// characters: the main chat, a detached group window and the LAN viewer (P16)
/// are given the same numbers rather than each running a matcher of its own.
export interface AlertMatch {
  start: number;
  end: number;
}

export interface TranscriptEvent {
  // Sequence numbers are per stream and start again at zero with every
  // session, so `(streamId, seq)` repeats as soon as a second session runs —
  // and Start is non-destructive, so the chat holds lines from several of them
  // at once. `(sessionId, streamId, seq)` is what identifies a line.
  sessionId: number;
  streamId: number;
  seq: number;
  label: string;
  tStartMs: number;
  tEndMs: number;
  text: string;
  detectedLanguage: string | null;
  decodeMs: number;
  latencyMs: number;
  /// Non-overlapping and in order. Empty whenever the alert list is empty,
  /// which is the normal state.
  alerts: AlertMatch[];
}

export interface StreamLevel {
  streamId: number;
  rms: number;
  peak: number;
  dropped: number;
}

// Every stream's meter in one event, at 20 Hz. All of them together rather
// than one event each: the meters are read side by side, so they are sampled
// side by side, and the cost on the bridge does not grow with the channel
// count.
export interface LevelsEvent {
  streams: StreamLevel[];
}

// A segment that was thrown away before it could be decoded, because the
// decode queue was full. Distinct from `StreamLevel.dropped`, which counts
// audio lost on the way *into* the pipeline — this is audio that was captured
// cleanly and then could not be kept up with.
export interface OverloadEvent {
  sessionId: number;
  streamId: number;
  seq: number;
  dropped: number;
}

// What the app and its whisper workers are costing the machine, once a second
// (P8). Mirrors `crates/app/src/resources.rs`.
//
// Sent whether or not a session runs: a machine that is already loaded is
// something to see before pressing Start, and the fall after Stop is how the
// memory a session held is seen to come back.
export interface ResourceUsage {
  /// Share of the whole machine in percent, so 100 means every core is busy.
  /// Already divided by `cores` on the backend — the raw per-core figure that
  /// the operating system reports reads as 800 % on eight busy workers.
  cpu: number;
  /// Resident bytes, summed over the app and its sidecars.
  memory: number;
  /// The machine's physical memory, so a gigabyte figure has a scale.
  totalMemory: number;
  /// How many processes the figures cover: the app plus one per worker.
  processes: number;
  cores: number;
}

// How the backend actually mapped a stream. Authoritative: the stream ids the
// events carry are defined here, not by the UI's own copy of the setup.
export interface RunningStream {
  streamId: number;
  channel: number;
  label: string;
  // The device has no such channel, so the stream runs silently: it keeps its
  // id, its colour and its groups and receives no audio. The backend mutes
  // rather than refusing to start, so one setup serves both the desk and a
  // two-input laptop interface.
  muted: boolean;
}

export type SessionState =
  | {
      state: "running";
      sessionId: number;
      /// Wall clock at which this session's own clock is zero, in milliseconds
      /// since the Unix epoch. A segment's `tStartMs` is on that session
      /// clock; this is what turns it into a time of day.
      startedAtMs: number;
      device: string;
      streams: RunningStream[];
      sampleRate: number;
      model: string;
      loadMs: number;
      gpu: boolean;
    }
  | { state: "stopped" }
  | { state: "failed"; message: string };

// --- Configuration (P12) --------------------------------------------------
//
// Mirrors `crates/app/src/config.rs`. A profile is one production's setup; the
// vocabulary sits beside them rather than inside one, because the jargon of a
// house outlives any one patch — see `profiles.ts`.

/// Schema version of the configuration file. Sent on every save so a later
/// change has something to branch on; the backend refuses to reinterpret a
/// file that claims a higher one.
export const CONFIG_VERSION = 1;

export interface StoredChannel {
  /// Zero-based, like the audio API. The UI shows it one-based.
  channel: number;
  label: string;
  language: string;
  /// Part of the channel's configuration, not a rendering detail: a channel is
  /// recognised by its colour long before anyone reads its label.
  color: string;
}

export interface StoredGroup {
  name: string;
  /// Whether a view of this group asks for the window's attention on every
  /// message rather than only on an alert term (P17). Optional because a file
  /// written before it existed has none, which reads as off.
  flash?: boolean;
  /// Whether this group's detached window stays in front of other windows
  /// (P7). Optional for the same reason as `flash`: a file written before it
  /// existed has none, which reads as off.
  onTop?: boolean;
  /// Positions in `StoredProfile.channels`, not the channel keys membership
  /// uses while the setup is being edited. `profiles.ts` owns the conversion.
  members: number[];
}

export interface StoredProfile {
  name: string;
  /// The device's display name. Stored beside the id so an interface that is
  /// not plugged in can still be named — "Scarlett 18i20 is not connected" is
  /// something an operator can act on, a UID is not.
  device: string;
  /// The device's stable id, and what the profile is really keyed on.
  ///
  /// Optional only because a file written before it existed has none:
  /// `profiles.ts` then fills it in the first time that device is seen
  /// connected.
  deviceId?: string;
  /// The language a channel added to this profile starts on. Optional here
  /// only because a file written before it existed has none — `profiles.ts`
  /// fills it in, and the backend serialises it from then on.
  language?: string;
  channels: StoredChannel[];
  groups: StoredGroup[];
}

export interface StoredConfig {
  version: number;
  /// Position of the profile last in use, so a restart lands where the
  /// operator left off. Clamped by the backend.
  active: number;
  /// `null` is "never set", and the UI fills in its default; an empty string is
  /// a box the operator cleared, which turns the prompt off and stays off.
  vocabulary: string | null;
  /// Catalogue id of the speech model (P11), or `null` for "never chose",
  /// which resolves to the recommended one. Beside the profiles rather than
  /// inside one: which model a machine can run is a property of the machine,
  /// not of the production.
  ///
  /// Optional, not because the backend ever omits it — it serialises `null` —
  /// but because a *file* written before P11 has none of these three, and the
  /// translation in `profiles.ts` is what has to survive reading one.
  model?: string | null;
  vadModel?: string | null;
  /// The custom model's `.bin`, from outside the catalogue — the way in for a
  /// machine that will never reach the internet. Used when `model` is
  /// `CUSTOM_MODEL`, and remembered while it is not.
  modelPath?: string | null;
  /// The words that must not be missed (P17), one per entry. Beside the
  /// profiles like the vocabulary: "Feuer" means the same thing in every
  /// production. Optional because a file written before P17 has none.
  alerts?: string[];
  /// The LAN remote client (P16): whether it is served, and on which address.
  /// Optional because a file written before it existed has none, which reads
  /// as "off" — a version that never offered a listener must not be read as
  /// having asked for one.
  remote?: StoredRemote;
  profiles: StoredProfile[];
}

// --- The model manager (P11) ----------------------------------------------
//
// Mirrors `crates/app/src/models.rs`.

export type ModelKind = "speech" | "vad";

/// The id the custom model is selected by — `models::CUSTOM` on the Rust side.
/// It is not a catalogue entry: there is nothing to download, only a path to
/// point at, and it sits in `model` so the picker has a single answer.
export const CUSTOM_MODEL = "custom";

/// One catalogue entry and what the machine has of it.
export interface ModelStatus {
  id: string;
  kind: ModelKind;
  label: string;
  /// One line on when to choose this one, written for an audio engineer.
  note: string;
  bytes: number;
  installed: boolean;
  /// Bytes of an interrupted download waiting to be continued, so the button
  /// can honestly offer to resume rather than to start over.
  partial: number;
  recommended: boolean;
}

/// Where the model in use came from. `development` is the `.scratch/models`
/// search that predates this package and only ever fires in a checkout.
export type ModelSource = "managed" | "custom" | "environment" | "development";

/// What the page has selected, as `list_models` wants it.
export interface ModelSelection {
  model: string | null;
  vadModel: string | null;
  modelPath: string | null;
}

export interface ModelsView {
  /// Shown on the page: nobody can be expected to know where macOS puts this.
  dir: string;
  models: ModelStatus[];
  /// The speech model a session would start with, or `null` for the "no model"
  /// state — in which case `problem` says what to do about it.
  active: string | null;
  source: ModelSource | null;
  problem: string | null;
  /// Whether speech detection would be running. Without it the app transcribes
  /// noise and silence, which is worth saying rather than leaving to be found.
  vad: boolean;
}

export type DownloadStage =
  | "downloading"
  /// Hashing 600 MB is not instant, and a bar stuck at 100 % looks like a hang.
  | "verifying"
  | "done"
  | "cancelled"
  | "failed";

export interface ModelProgress {
  id: string;
  /// Resumed bytes included, so a continued download picks the bar up where it
  /// left it.
  received: number;
  total: number;
  stage: DownloadStage;
  /// Set when `stage` is `failed`.
  message: string | null;
}

export const appVersion = () => call<string>("app_version");
/// The stored configuration, or an empty one. Only fails when the app cannot
/// work out where its configuration directory is — which is also the case in
/// which nothing can be saved, so it has to be visible.
export const loadConfig = () => call<StoredConfig>("load_config");
/// Writes the whole configuration, atomically on the other side.
export const saveConfig = (config: StoredConfig) => call<void>("save_config", { config });
/// The catalogue, what is installed, and which model is actually in use — one
/// call, because the answers are only useful together.
/// Opens the system file dialog on a `.bin`, and answers with what was chosen.
///
/// `current` is what the field holds; the dialog opens there rather than
/// wherever the operating system last was. `null` back is a cancelled dialog
/// rather than a failure — the likeliest outcome of opening a picker, and the
/// caller leaves the field alone.
export const pickModelFile = (current: string | null) =>
  invoke<string | null>("pick_model_file", { current });

/// Whether there is a file dialog to open at all.
///
/// One of the three things that stay on the desktop (P16): the picker is the
/// host machine's, and a browser on another machine has no view of the disk
/// the model would have to be on. The remote's way in is the managed
/// downloads, and the path field is still editable — what it cannot offer is
/// the button.
export const canPickModelFile = !isRemote;

/// The catalogue and what a session would start with.
///
/// The selection travels with the request rather than being read from the
/// stored configuration: Settings saves through a debounce, so the file is a
/// second behind the radio button that was just clicked, and the "in use" line
/// would contradict the page around it.
export const listModels = (selection?: ModelSelection) =>
  call<ModelsView>("list_models", { selection });
/// Starts (or resumes) a download. Resolves as soon as it is *running*; the
/// `model-progress` events carry the rest, including its failure.
export const downloadModel = (id: string) => call<void>("download_model", { id });
/// Stops a download and keeps what arrived, so pressing download again
/// continues instead of starting over.
export const cancelDownload = (id: string) => call<void>("cancel_download", { id });
export const removeModel = (id: string) => call<void>("remove_model", { id });
/// Compiles the alert list on the backend, where every transcript is matched
/// against it. Sent when the list changes and once at startup — not read from
/// the configuration file, which is a debounce behind the term just typed.
export const setAlertTerms = (terms: string[]) =>
  call<void>("set_alert_terms", { terms });

/// Asks the platform to draw attention to this window, if it is not already
/// the focused one — that check is the backend's, because the window is.
///
/// `urgent` is an alert term: it keeps asking until the app is looked at. A
/// group's flash-on-message is a single bounce. Never a sound, in either case:
/// this is an intercom room.
///
/// In a browser there is no window and no platform to ask (P16), so the page
/// asks with what a tab owns: its own surface and its title in the tab strip.
/// Same two requests, same rules — see `flash.ts`.
export const requestAttention = (urgent: boolean): Promise<void> => {
  if (isRemote) {
    flashPage(urgent);
    return Promise.resolve();
  }
  return invoke<void>("request_attention", { urgent });
};

// --- The chat, and the windows onto it (P7) -------------------------------

/// When one session's clock was zero, in epoch milliseconds.
export interface SessionStart {
  sessionId: number;
  startedAtMs: number;
}

/// The chat as it stands, for a window that has just opened.
///
/// Mirrors `crates/app/src/history.rs`. Asked for once per window: a detached
/// group window is opened in the middle of a show and would otherwise start
/// blank, and the main window uses it to come back from a reload with its chat
/// intact.
export interface Snapshot {
  transcripts: TranscriptEvent[];
  sessions: SessionStart[];
  /// `null` when nothing has run yet in this process.
  state: SessionState | null;
}

export const sessionSnapshot = () => call<Snapshot>("session_snapshot");

/// Empties the chat, in every window at once.
///
/// The backend holds the copy every view is caught up from, so this is where
/// the deletion has to happen — a window clearing only itself would leave a
/// group window showing what the main window was just told is gone. Every
/// window then empties its own list on `communication-cleared`, including the
/// one that asked.
export const clearCommunication = () => call<void>("clear_communication");

/// Opens a group in a window of its own, or focuses the one already open.
///
/// `group` is the group's position in the active profile — the same position
/// the configuration file stores membership by, and what the window's label is
/// built from. `name` is only the window's title.
/// In a browser it is a tab instead (P16), named after the group so that asking
/// for the same group twice reuses the tab that is already open rather than
/// stacking a second one on it — which is exactly what the window label does
/// on the desktop. `onTop` has no meaning there: nothing can be put in front
/// of anything else inside a browser.
export const openGroupWindow = (group: number, name: string, onTop: boolean): Promise<void> => {
  if (isRemote) {
    openGroupTab(group, name);
    return Promise.resolve();
  }
  return invoke<void>("open_group_window", { group, name, onTop });
};

/// A group window asking for its "keep this window in front" setting to be
/// remembered.
///
/// The window applies it to itself the moment it is clicked — that is its own
/// window and its own business — and this only asks for it to be *stored*. It
/// goes to the main window rather than to the backend because the
/// configuration has exactly one writer: the main window holds it and saves it
/// whole through its debounce, and a second writer editing the file underneath
/// that would be overwritten by the next keystroke on the Settings page.
export interface GroupOnTop {
  /// The group's position in the active profile — how a group is addressed
  /// between windows. See `openGroupWindow`.
  group: number;
  onTop: boolean;
}

export const requestGroupOnTop = (group: number, onTop: boolean) =>
  emit("group-on-top", { group, onTop } satisfies GroupOnTop);

export const onGroupOnTop = (fn: (e: GroupOnTop) => void): Promise<UnlistenFn> =>
  on<GroupOnTop>("group-on-top", (payload) => fn(payload));

/// Closes every detached group window. Called when the active profile changes:
/// a group window filters by stream id, and those are positions in the active
/// profile's channel list.
export const closeGroupWindows = (): Promise<void> => {
  if (isRemote) {
    closeGroupTabs();
    return Promise.resolve();
  }
  return invoke<void>("close_group_windows");
};

export const listDevices = () => call<DeviceOption[]>("list_devices");
/// Every language the sidecar accepts — around a hundred, in whisper's own
/// order. Asked for rather than listed in the UI, so that adding a language is
/// a whisper.cpp version and never a change here. Needs no model, so it
/// answers on a machine that has downloaded nothing.
export const listLanguages = () => call<Language[]>("list_languages");
/// Resolves with the running state, so the UI does not have to wait for an
/// event to learn that the session it just asked for actually started.
export const startSession = (config: SessionConfig) =>
  call<SessionState>("start_session", { config });
export const stopSession = () => call<void>("stop_session");

export const onTranscript = (fn: (e: TranscriptEvent) => void): Promise<UnlistenFn> =>
  on<TranscriptEvent>("transcript", (payload) => fn(payload));

export const onLevels = (fn: (e: LevelsEvent) => void): Promise<UnlistenFn> =>
  on<LevelsEvent>("levels", (payload) => fn(payload));

export const onOverload = (fn: (e: OverloadEvent) => void): Promise<UnlistenFn> =>
  on<OverloadEvent>("overload", (payload) => fn(payload));

export const onSessionState = (fn: (e: SessionState) => void): Promise<UnlistenFn> =>
  on<SessionState>("session-state", (payload) => fn(payload));

/// The chat was emptied, by this window or by another one.
export const onCleared = (fn: () => void): Promise<UnlistenFn> =>
  on<null>("communication-cleared", () => fn());

/// The configuration was written. What a detached group window follows: a
/// group renamed, refilled or given its flash setting on the Settings page
/// reaches the window showing it this way and no other.
export const onConfigChanged = (fn: (config: StoredConfig) => void): Promise<UnlistenFn> =>
  on<StoredConfig>("config-changed", (payload) => fn(payload));

/// Once a second, for as long as the app is open.
export const onResources = (fn: (e: ResourceUsage) => void): Promise<UnlistenFn> =>
  on<ResourceUsage>("resources", (payload) => fn(payload));

export const onModelProgress = (fn: (e: ModelProgress) => void): Promise<UnlistenFn> =>
  on<ModelProgress>("model-progress", (payload) => fn(payload));

// --- The LAN remote client (P16) ------------------------------------------
//
// Mirrors `crates/app/src/remote.rs`. The settings themselves are part of the
// configuration file like everything else on the Settings page — there is no
// "enable the remote" command, because the configuration has one writer and a
// second mutation path is how the file and the listener come to disagree.
// What is asked for here is only what the configuration cannot know: which
// addresses exist, and whether the listener actually came up on one.

/// One address this machine can be reached on.
export interface Interface {
  /// The operating system's name for it — `en0`, `en7`. Shown because a
  /// machine on two networks has two plausible addresses, and this is what
  /// tells them apart.
  name: string;
  address: string;
  /// `127.0.0.1`: reaches no other machine, which makes it the one address
  /// that is safe everywhere and useless for the thing this is for.
  loopback: boolean;
}

/// Whether the remote is being served, and to how many browsers.
export interface RemoteStatus {
  running: boolean;
  /// What to type into the other machine's browser, or `null` while nothing is
  /// served.
  url: string | null;
  /// The one number that answers "did it work" from the machine that cannot
  /// see the other screen.
  clients: number;
  /// Why it is not running although the configuration asks for it — a port
  /// that is taken, an address that has gone away. `null` when there is
  /// nothing to say.
  error: string | null;
}

/// Whether the listener is enabled, and where it binds.
///
/// Part of `StoredConfig`, so it is saved, loaded and broadcast like the rest
/// of the Settings page. `address` is one address and never `0.0.0.0`: an app
/// that serves every interface at once is an app on the venue's guest wifi
/// without anyone having said so.
export interface StoredRemote {
  enabled: boolean;
  address: string;
  port: number;
}

/// What a configuration written before P16 is read as: off, on no address.
export const REMOTE_DEFAULT_PORT = 8787;

export const listInterfaces = () => call<Interface[]>("list_interfaces");
export const remoteStatus = () => call<RemoteStatus>("remote_status");

/// The listener came up, went down, or gained or lost a browser.
export const onRemoteStatus = (fn: (status: RemoteStatus) => void): Promise<UnlistenFn> =>
  on<RemoteStatus>("remote-status", (payload) => fn(payload));

export { isRemote } from "./transport";
export { onConnection, connectionState, onReconnected, type Connection } from "./transport";

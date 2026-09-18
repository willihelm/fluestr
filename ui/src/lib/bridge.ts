// Typed view of the Tauri bridge.
//
// Hand-written for now. P6 still owes types generated from the Rust structs
// with `ts-rs` or `specta`, which is what stops the two sides drifting apart —
// until then the types here have to be kept in step with
// `crates/app/src/session.rs`, and the round-trip tests there pin down the
// field names.

import { invoke } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";

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

export const appVersion = () => invoke<string>("app_version");
/// The stored configuration, or an empty one. Only fails when the app cannot
/// work out where its configuration directory is — which is also the case in
/// which nothing can be saved, so it has to be visible.
export const loadConfig = () => invoke<StoredConfig>("load_config");
/// Writes the whole configuration, atomically on the other side.
export const saveConfig = (config: StoredConfig) => invoke<void>("save_config", { config });
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

/// The catalogue and what a session would start with.
///
/// The selection travels with the request rather than being read from the
/// stored configuration: Settings saves through a debounce, so the file is a
/// second behind the radio button that was just clicked, and the "in use" line
/// would contradict the page around it.
export const listModels = (selection?: ModelSelection) =>
  invoke<ModelsView>("list_models", { selection });
/// Starts (or resumes) a download. Resolves as soon as it is *running*; the
/// `model-progress` events carry the rest, including its failure.
export const downloadModel = (id: string) => invoke<void>("download_model", { id });
/// Stops a download and keeps what arrived, so pressing download again
/// continues instead of starting over.
export const cancelDownload = (id: string) => invoke<void>("cancel_download", { id });
export const removeModel = (id: string) => invoke<void>("remove_model", { id });
/// Compiles the alert list on the backend, where every transcript is matched
/// against it. Sent when the list changes and once at startup — not read from
/// the configuration file, which is a debounce behind the term just typed.
export const setAlertTerms = (terms: string[]) =>
  invoke<void>("set_alert_terms", { terms });

/// Asks the platform to draw attention to this window, if it is not already
/// the focused one — that check is the backend's, because the window is.
///
/// `urgent` is an alert term: it keeps asking until the app is looked at. A
/// group's flash-on-message is a single bounce. Never a sound, in either case:
/// this is an intercom room.
export const requestAttention = (urgent: boolean) =>
  invoke<void>("request_attention", { urgent });

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

export const sessionSnapshot = () => invoke<Snapshot>("session_snapshot");

/// Empties the chat, in every window at once.
///
/// The backend holds the copy every view is caught up from, so this is where
/// the deletion has to happen — a window clearing only itself would leave a
/// group window showing what the main window was just told is gone. Every
/// window then empties its own list on `communication-cleared`, including the
/// one that asked.
export const clearCommunication = () => invoke<void>("clear_communication");

/// Opens a group in a window of its own, or focuses the one already open.
///
/// `group` is the group's position in the active profile — the same position
/// the configuration file stores membership by, and what the window's label is
/// built from. `name` is only the window's title.
export const openGroupWindow = (group: number, name: string, onTop: boolean) =>
  invoke<void>("open_group_window", { group, name, onTop });

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
  listen<GroupOnTop>("group-on-top", (e) => fn(e.payload));

/// Closes every detached group window. Called when the active profile changes:
/// a group window filters by stream id, and those are positions in the active
/// profile's channel list.
export const closeGroupWindows = () => invoke<void>("close_group_windows");

export const listDevices = () => invoke<DeviceOption[]>("list_devices");
/// Every language the sidecar accepts — around a hundred, in whisper's own
/// order. Asked for rather than listed in the UI, so that adding a language is
/// a whisper.cpp version and never a change here. Needs no model, so it
/// answers on a machine that has downloaded nothing.
export const listLanguages = () => invoke<Language[]>("list_languages");
/// Resolves with the running state, so the UI does not have to wait for an
/// event to learn that the session it just asked for actually started.
export const startSession = (config: SessionConfig) =>
  invoke<SessionState>("start_session", { config });
export const stopSession = () => invoke<void>("stop_session");

export const onTranscript = (fn: (e: TranscriptEvent) => void): Promise<UnlistenFn> =>
  listen<TranscriptEvent>("transcript", (e) => fn(e.payload));

export const onLevels = (fn: (e: LevelsEvent) => void): Promise<UnlistenFn> =>
  listen<LevelsEvent>("levels", (e) => fn(e.payload));

export const onOverload = (fn: (e: OverloadEvent) => void): Promise<UnlistenFn> =>
  listen<OverloadEvent>("overload", (e) => fn(e.payload));

export const onSessionState = (fn: (e: SessionState) => void): Promise<UnlistenFn> =>
  listen<SessionState>("session-state", (e) => fn(e.payload));

/// The chat was emptied, by this window or by another one.
export const onCleared = (fn: () => void): Promise<UnlistenFn> =>
  listen<null>("communication-cleared", () => fn());

/// The configuration was written. What a detached group window follows: a
/// group renamed, refilled or given its flash setting on the Settings page
/// reaches the window showing it this way and no other.
export const onConfigChanged = (fn: (config: StoredConfig) => void): Promise<UnlistenFn> =>
  listen<StoredConfig>("config-changed", (e) => fn(e.payload));

/// Once a second, for as long as the app is open.
export const onResources = (fn: (e: ResourceUsage) => void): Promise<UnlistenFn> =>
  listen<ResourceUsage>("resources", (e) => fn(e.payload));

export const onModelProgress = (fn: (e: ModelProgress) => void): Promise<UnlistenFn> =>
  listen<ModelProgress>("model-progress", (e) => fn(e.payload));

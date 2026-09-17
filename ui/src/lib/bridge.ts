// Typed view of the Tauri bridge.
//
// Hand-written for now. P6 still owes types generated from the Rust structs
// with `ts-rs` or `specta`, which is what stops the two sides drifting apart —
// until then the types here have to be kept in step with
// `crates/app/src/session.rs`, and the round-trip tests there pin down the
// field names.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface DeviceOption {
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
  device: string;
  streams: StreamConfig[];
  /// The active vocabulary, applied to every stream. Session-level rather than
  /// per stream because the terms of a production are the same on every
  /// channel — see `vocabulary.ts` and P15.
  vocabulary: string | null;
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
  /// Positions in `StoredProfile.channels`, not the channel keys membership
  /// uses while the setup is being edited. `profiles.ts` owns the conversion.
  members: number[];
}

export interface StoredProfile {
  name: string;
  device: string;
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
  /// A `.bin` outside the catalogue — the fallback for a machine that will
  /// never reach the internet. Outranks `model` when set.
  modelPath?: string | null;
  profiles: StoredProfile[];
}

// --- The model manager (P11) ----------------------------------------------
//
// Mirrors `crates/app/src/models.rs`.

export type ModelKind = "speech" | "vad";

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
export type ModelSource = "managed" | "manual" | "environment" | "development";

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
export const listModels = () => invoke<ModelsView>("list_models");
/// Starts (or resumes) a download. Resolves as soon as it is *running*; the
/// `model-progress` events carry the rest, including its failure.
export const downloadModel = (id: string) => invoke<void>("download_model", { id });
/// Stops a download and keeps what arrived, so pressing download again
/// continues instead of starting over.
export const cancelDownload = (id: string) => invoke<void>("cancel_download", { id });
export const removeModel = (id: string) => invoke<void>("remove_model", { id });
export const listDevices = () => invoke<DeviceOption[]>("list_devices");
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

export const onModelProgress = (fn: (e: ModelProgress) => void): Promise<UnlistenFn> =>
  listen<ModelProgress>("model-progress", (e) => fn(e.payload));

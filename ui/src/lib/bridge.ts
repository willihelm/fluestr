// Typed view of the Tauri bridge.
//
// Hand-written for now. P6 generates these from the Rust structs with `ts-rs`
// or `specta`, which is what stops the two sides drifting apart — until then
// the types here have to be kept in step with `crates/app/src/session.rs`, and
// the round-trip tests there pin down the field names.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface DeviceOption {
  name: string;
  channels: number;
  defaultSampleRate: number;
  isDefault: boolean;
}

export interface SessionConfig {
  device: string;
  channel: number;
  label: string;
  language: string;
  initialPrompt: string | null;
}

export interface TranscriptEvent {
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

export interface LevelEvent {
  streamId: number;
  rms: number;
  peak: number;
  dropped: number;
}

// A segment that was thrown away before it could be decoded, because the
// decode queue was full. Distinct from `LevelEvent.dropped`, which counts
// audio lost on the way *into* the pipeline — this is audio that was captured
// cleanly and then could not be kept up with.
export interface OverloadEvent {
  streamId: number;
  seq: number;
  dropped: number;
}

export type SessionState =
  | {
      state: "running";
      device: string;
      channel: number;
      sampleRate: number;
      model: string;
      loadMs: number;
      gpu: boolean;
    }
  | { state: "stopped" }
  | { state: "failed"; message: string };

export const appVersion = () => invoke<string>("app_version");
export const listDevices = () => invoke<DeviceOption[]>("list_devices");
/// Resolves with the running state, so the UI does not have to wait for an
/// event to learn that the session it just asked for actually started.
export const startSession = (config: SessionConfig) =>
  invoke<SessionState>("start_session", { config });
export const stopSession = () => invoke<void>("stop_session");

export const onTranscript = (fn: (e: TranscriptEvent) => void): Promise<UnlistenFn> =>
  listen<TranscriptEvent>("transcript", (e) => fn(e.payload));

export const onLevel = (fn: (e: LevelEvent) => void): Promise<UnlistenFn> =>
  listen<LevelEvent>("level", (e) => fn(e.payload));

export const onOverload = (fn: (e: OverloadEvent) => void): Promise<UnlistenFn> =>
  listen<OverloadEvent>("overload", (e) => fn(e.payload));

export const onSessionState = (fn: (e: SessionState) => void): Promise<UnlistenFn> =>
  listen<SessionState>("session-state", (e) => fn(e.payload));

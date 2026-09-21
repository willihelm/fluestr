// Typed view of the backend.
//
// Every call here goes through one command layer on the other side
// (`crates/app/src/commands.rs`), and reaches it over whichever transport this
// frontend is running on: an `invoke` in the app's own WebView, a WebSocket
// frame from a browser served over the LAN (P16). See `transport.ts` — nothing
// below this line knows which of the two it is.
//
// **The types are generated, not written here** (P6). Everything that crosses
// the bridge is declared once, in Rust, and `cargo test export_bindings` writes
// it into `./generated` — with the Rust doc comments carried along, so the
// reason a field exists is where the field is. This file re-exports them so
// that the rest of the frontend still has one import for "the backend", and
// adds only what has no Rust side: the calls themselves, and the two messages
// windows send each other.
//
// What that buys is the failure mode: a field renamed in Rust is now a
// TypeScript error in the component that reads it, rather than a value that
// quietly arrives as `undefined` in a chat that is running during a show.
// Regenerating is part of `just check` — see `just types`.

import { invoke } from "@tauri-apps/api/core";
import { type UnlistenFn } from "@tauri-apps/api/event";
import { call, emit, isRemote, on } from "./transport";
import { flashPage, floodWindow } from "./flash";
import type { Ask } from "./attention";
import { ALL_CHANNELS_VIEW, closeGroupTabs, openGroupTab, type GroupView } from "./windows";
import type {
  DeviceOption,
  RunningStream,
  SessionConfig,
  SessionState,
  SessionStart,
  Snapshot,
  StreamConfig,
  StreamLevel,
  StreamUpdate,
  TranscriptEvent,
  LevelsEvent,
  OverloadEvent,
  AlertMatch,
} from "./generated/session";
import type { Language } from "./generated/language";
import type {
  StoredChannel,
  StoredConfig,
  StoredGroup,
  StoredProfile,
  StoredRemote,
  StoredRule,
  StoredSortDirection,
  Tuning,
} from "./generated/config";
import type {
  DownloadStage,
  ModelKind,
  ModelProgress,
  ModelSelection,
  ModelSource,
  ModelStatus,
  ModelsView,
} from "./generated/models";
import type { Interface, RemoteStatus } from "./generated/remote";
import type { WebhookStatus } from "./generated/webhook";
import type { Action, CompiledRule } from "./generated/rules";
import type { ResourceUsage } from "./generated/resources";

// Re-exported so the rest of the frontend imports "the backend" from one place
// rather than reaching into `generated/` by hand — and so that moving a type
// between generated files is not a change in twenty components.
export type {
  Action,
  AlertMatch,
  CompiledRule,
  DeviceOption,
  DownloadStage,
  Interface,
  Language,
  LevelsEvent,
  ModelKind,
  ModelProgress,
  ModelSelection,
  ModelSource,
  ModelStatus,
  ModelsView,
  OverloadEvent,
  RemoteStatus,
  ResourceUsage,
  RunningStream,
  SessionConfig,
  SessionStart,
  SessionState,
  Snapshot,
  StoredChannel,
  StoredConfig,
  StoredGroup,
  StoredProfile,
  StoredRemote,
  StoredRule,
  StoredSortDirection,
  StreamConfig,
  StreamLevel,
  StreamUpdate,
  TranscriptEvent,
  Tuning,
};

// The values the two sides agree on that are not types — a schema version, a
// default port, a worker count. Generated from the Rust constants too, by
// `crates/app/src/bindings.rs`, because a number copied by hand is a number
// that gets changed on one side only.
export {
  CONFIG_VERSION,
  CUSTOM_MODEL,
  DEFAULT_WORKERS,
  DEFAULT_TUNING,
  GAIN_DB_MAX,
  GAIN_DB_MIN,
  MAX_SEGMENT_MS_MAX,
  MAX_SEGMENT_MS_MIN,
  MAX_WORKERS,
  REMOTE_DEFAULT_PORT,
  SILENCE_MS_MAX,
  SILENCE_MS_MIN,
} from "./generated/constants";

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
/// Compiles the rules on the backend, where every transcript is evaluated
/// against them (P19). Sent when they change and once at startup — not read
/// from the configuration file, which is a debounce behind the rule just
/// written.
///
/// They go over **compiled**, with every group already resolved into the
/// stream ids of its members: groups exist only in this frontend, and the
/// backend names a channel by its stream id. See `rules.ts`.
export const setRules = (rules: CompiledRule[]) => call<void>("set_rules", { rules });

/// Asks the platform to draw attention to this window, if it is not already
/// the focused one — that check is the backend's, because the window is.
///
/// `ask` is what a rule's attention actions came to (P19): `"urgent"` keeps
/// asking until the app is looked at, `"once"` is a single bounce, `null` is
/// no platform request at all. Never a sound, in any case: this is an intercom
/// room.
///
/// `loud` is the flash action, and it is drawn rather than asked for: the
/// whole window in dark orange, here and not on the platform, because no
/// platform offers "paint this window". It is therefore the same in the app
/// and in a browser — the page is what the operator is looking at either way —
/// and it is the one request that is made even to a focused window. Somebody
/// watching the meters in this window has not seen the chat in it.
///
/// In a browser there is no window and no platform to ask (P16), so the rest
/// of the request is made with what a tab owns: its own surface and its title
/// in the tab strip. Same requests, same rules — see `flash.ts`.
export const requestAttention = (ask: Ask, loud: boolean): Promise<void> => {
  if (loud) floodWindow();
  if (ask === null) return Promise.resolve();
  if (isRemote) {
    flashPage(ask === "urgent");
    return Promise.resolve();
  }
  return invoke<void>("request_attention", { urgent: ask === "urgent" });
};

// --- The chat, and the windows onto it (P7) -------------------------------

export const sessionSnapshot = () => call<Snapshot>("session_snapshot");

/// Empties the chat, in every window at once.
///
/// The backend holds the copy every view is caught up from, so this is where
/// the deletion has to happen — a window clearing only itself would leave a
/// group window showing what the main window was just told is gone. Every
/// window then empties its own list on `communication-cleared`, including the
/// one that asked.
export const clearCommunication = () => call<void>("clear_communication");

/// Opens a view in a window of its own, or focuses the one already open.
///
/// `group` is the group's position in the active profile — the same position
/// the configuration file stores membership by, and what the window's label is
/// built from — or `ALL_CHANNELS_VIEW` for the view of every channel, which is
/// not a group and so has no position. It crosses to Rust as `null`, where the
/// absent position is exactly what it means. `name` is only the window's title.
/// In a browser it is a tab instead (P16), named after the group so that asking
/// for the same group twice reuses the tab that is already open rather than
/// stacking a second one on it — which is exactly what the window label does
/// on the desktop. `onTop` has no meaning there: nothing can be put in front
/// of anything else inside a browser.
export const openGroupWindow = (
  group: GroupView,
  name: string,
  onTop: boolean,
): Promise<void> => {
  if (isRemote) {
    openGroupTab(group, name);
    return Promise.resolve();
  }
  return invoke<void>("open_group_window", {
    group: group === ALL_CHANNELS_VIEW ? null : group,
    name,
    onTop,
  });
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

/// Renames a running channel, or puts it onto another language, without
/// stopping capture (P6).
///
/// The two things that are wrong in the middle of a show — somebody else took
/// the position over, the guest director speaks English — and the two that can
/// change without the capture being touched. The device channel cannot: the
/// channel map is fixed when the device opens, so a repatch is a restart.
///
/// Resolves with the running state and announces it, so the Settings page that
/// asked, a detached group window and a browser on the LAN all take the new
/// mapping from the backend rather than each from its own copy of the setup.
/// Changes nothing stored — the profile is saved on the Settings page as
/// always, by its one writer.
export const updateStreamConfig = (update: StreamUpdate) =>
  call<SessionState>("update_stream_config", { update });

/// Moves the detection thresholds of the running session (P12).
///
/// Beside the save rather than part of it: the file is where the three
/// numbers live, and the page writes it on its own debounce like every other
/// setting — but a threshold that only takes effect at the next Start is no
/// use to somebody setting it by ear while the room talks.
///
/// Succeeds with nothing running, and answers with nothing: no session state
/// changes, so there is nothing for a view to adopt. The next Start reads the
/// numbers from the file.
export const updateTuning = (tuning: Tuning) => call<void>("update_tuning", { tuning });

/// Opens the input device just to meter it, while no session is running (P8).
///
/// Takes the same channel list Start would: the meters are keyed on the
/// stream id, which is the position in that list, and a second shape of it
/// would be a second thing that can disagree about which channel is which.
///
/// Idempotent — asked for what is already being metered, it does nothing — so
/// the Settings page can ask on every edit without the bars stuttering.
/// Refused while a session runs, because a session's own meters are these
/// meters.
export const startMonitor = (config: SessionConfig) =>
  call<void>("start_monitor", { config });
export const stopMonitor = () => call<void>("stop_monitor");
/// Trims a metered channel, as the knob is turned before a show. Does nothing
/// when nothing is being metered — the profile is where the value is kept.
export const setMonitorGain = (streamId: number, gainDb: number) =>
  call<void>("set_monitor_gain", { streamId, gainDb });

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

export const listInterfaces = () => call<Interface[]>("list_interfaces");
export const remoteStatus = () => call<RemoteStatus>("remote_status");

/// The listener came up, went down, or gained or lost a browser.
export const onRemoteStatus = (fn: (status: RemoteStatus) => void): Promise<UnlistenFn> =>
  on<RemoteStatus>("remote-status", (payload) => fn(payload));

// --- The outbound action's health (P8, P19) -------------------------------

/// How the outbound queue is doing: what has gone out, and what has been given
/// up on. Asked for once as a view opens, the way the chat snapshot is — the
/// counters are cumulative, and a window opened after a cue failed has to see
/// that it did.
export const webhookStatus = () => call<WebhookStatus>("webhook_status");

/// A request went out, or was given up on.
///
/// Sent on every outcome rather than sampled once a second, because the first
/// drop is the interesting one: a dead endpoint silently swallowing every cue
/// is the one failure this feature can have that nobody notices until the cue
/// does not happen.
export const onWebhookStatus = (fn: (status: WebhookStatus) => void): Promise<UnlistenFn> =>
  on<WebhookStatus>("webhook-status", (payload) => fn(payload));

export type { WebhookStatus };

export { isRemote } from "./transport";
export { onConnection, connectionState, onReconnected, type Connection } from "./transport";

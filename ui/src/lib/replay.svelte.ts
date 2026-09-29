// Hearing a bubble again (P22).
//
// What this window knows about replays: where the line through the chat is,
// before which bubbles have lost their audio, and which bubble — if any — it
// is playing right now. One per window, like the search: a replay pressed in a
// group window does not light up a button in the main one.
//
// In the app a replay plays on the machine's listen output — its headphones,
// or the shoutbox if the profile says so. In a browser on the LAN (P16) it
// plays in that browser: somebody sitting elsewhere wants it in their own
// ears, not in the host's headphones.

import {
  isRemote,
  onCleared,
  onReplayHorizon,
  playReplay,
  replayHorizon,
  replayWav,
  stopReplay,
  type ClipKey,
  type Horizon,
} from "./bridge";

export const replay = $state<{
  horizon: Horizon | null;
  /// The key of the bubble that is playing in this window.
  playing: string | null;
  /// Why the last press did not play, and on which bubble.
  error: { key: string; message: string } | null;
}>({ horizon: null, playing: null, error: null });

let watching = false;
let timer: ReturnType<typeof setTimeout> | undefined;
let element: HTMLAudioElement | null = null;

/// Starts following the horizon. Idempotent, so every chat list can call it.
export const watchReplay = () => {
  if (watching) return;
  watching = true;
  // Subscribed before asking, for the history's reason: the other order
  // drops a move made in between.
  void onReplayHorizon((horizon) => (replay.horizon = horizon));
  void onCleared(() => {
    replay.horizon = null;
    void stop();
  });
  replayHorizon()
    .then((horizon) => {
      if (horizon !== null) replay.horizon = horizon;
    })
    .catch(() => {
      // A backend that predates P22: no horizon, and no bubble has audio.
    });
};

/// Plays a bubble, or stops it if it is the one playing.
export const toggle = async (key: string, segments: ClipKey[]) => {
  if (replay.playing === key) {
    await stop();
    return;
  }
  replay.error = null;
  try {
    let durationMs: number;
    if (isRemote) {
      const answer = await replayWav(segments);
      element?.pause();
      element = new Audio(`data:audio/wav;base64,${answer.wav}`);
      element.onended = () => {
        if (replay.playing === key) replay.playing = null;
      };
      await element.play();
      durationMs = answer.durationMs;
    } else {
      durationMs = (await playReplay(segments)).durationMs;
    }
    replay.playing = key;
    clearTimeout(timer);
    // The backend says how long it plays rather than sending an event when
    // it ends: one answer, and nothing to miss.
    timer = setTimeout(() => {
      if (replay.playing === key) replay.playing = null;
    }, durationMs + 150);
  } catch (e) {
    replay.playing = null;
    replay.error = { key, message: e instanceof Error ? e.message : String(e) };
  }
};

export const stop = async () => {
  replay.playing = null;
  clearTimeout(timer);
  if (element) {
    element.pause();
    element = null;
  }
  if (!isRemote) await stopReplay().catch(() => {});
};

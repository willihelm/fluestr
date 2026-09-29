// Who is talking right now, before there is any text (P23).
//
// What this window knows about the channels that have something on its way
// to the chat: the gate is open, or what was said is being decoded. The
// backend reports each change once and ends every one of them — a cough, a
// segment that decoded to nothing and one shed under overload all end like a
// sentence does — so the timeout here is the safety net behind that, not the
// mechanism. One copy per window, like the replay state: every view follows
// the same events, and each chat list reads the channels it shows.

import {
  onSessionState,
  onSpeechActivity,
  onTranscript,
  type SpeechActivity,
} from "./bridge";

/// How long a placeholder may go without a sign of life before this window
/// stops believing it. Longer than the longest segment the Detection page
/// allows plus a slow decode, so that somebody talking without a pause — who
/// produces a line at least that often — never loses theirs.
export const STALE_MS = 30_000;

interface Entry {
  activity: SpeechActivity;
  /// When this window last heard of the channel, on its own clock.
  seenAt: number;
}

export const speech = $state<{ active: Record<number, Entry> }>({ active: {} });

let watching = false;
/// When each channel's last live event arrived, ended ones included — what a
/// snapshot answer is checked against. Plain rather than state: nothing is
/// drawn from it.
const heardAt = new Map<number, number>();

const set = (activity: SpeechActivity) => {
  heardAt.set(activity.streamId, Date.now());
  if (activity.speaking || activity.decoding) {
    speech.active[activity.streamId] = { activity, seenAt: Date.now() };
  } else {
    delete speech.active[activity.streamId];
  }
};

/// Starts following the events. Idempotent, so every chat list can call it.
export const watchSpeech = () => {
  if (watching) return;
  watching = true;
  void onSpeechActivity(set);
  // A line from a channel is a sign of life: somebody talking without a
  // pause stays active for longer than any timeout, and produces text at
  // least once per segment ceiling.
  void onTranscript((event) => {
    const entry = speech.active[event.streamId];
    if (entry) entry.seenAt = Date.now();
  });
  // A session that is not running has nobody talking in it, whatever this
  // window missed on the way.
  void onSessionState((state) => {
    if (state.state !== "running") {
      speech.active = {};
      return;
    }
    for (const [stream, entry] of Object.entries(speech.active)) {
      if (entry.activity.sessionId !== state.sessionId) delete speech.active[Number(stream)];
    }
  });
  setInterval(() => {
    const now = Date.now();
    for (const [stream, entry] of Object.entries(speech.active)) {
      if (now - entry.seenAt > STALE_MS) delete speech.active[Number(stream)];
    }
  }, 5_000);
};

/// Takes a snapshot's answer as the whole truth — which is how a window that
/// missed an end while it was disconnected loses the placeholder — except for
/// what arrived live after the snapshot was asked for, which is newer.
export const adoptSpeech = (activity: SpeechActivity[], askedAt: number) => {
  watchSpeech();
  const fresh = (stream: number) => (heardAt.get(stream) ?? -Infinity) >= askedAt;
  const next: Record<number, Entry> = {};
  for (const a of activity) {
    if (!fresh(a.streamId)) next[a.streamId] = { activity: a, seenAt: Date.now() };
  }
  for (const [stream, entry] of Object.entries(speech.active)) {
    if (fresh(Number(stream))) next[Number(stream)] = entry;
  }
  speech.active = next;
};

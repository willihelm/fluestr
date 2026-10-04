// Hearing an input on the headphones: the S beside the M.
//
// Which inputs are soloed is the backend's to say — it is one set for the
// whole app, played on the machine's listen output — and every window follows
// it through the `solo` event, so an S pressed in a group window is lit in the
// main one too. Latching: pressing one adds it, pressing it again takes it
// out, and the others stay in.
//
// Not in a browser on the LAN (P16): the listen output is the host's
// headphones, and the backend refuses a remote that asks.

import { isRemote, onReconnected, onSolo, setSolo, soloStatus } from "./bridge";

export const solo = $state<{
  /// The soloed stream ids.
  streams: Set<number>;
  /// Why the last press did not solo, and on which stream.
  error: { streamId: number; message: string } | null;
}>({ streams: new Set(), error: null });

/// Whether this window can solo at all.
export const canSolo = !isRemote;

let watching = false;

const adopt = (streams: number[]) => {
  solo.streams = new Set(streams);
};

const refresh = () =>
  soloStatus()
    .then((state) => adopt(state.streams))
    .catch(() => {
      // A backend that predates solo: nothing is soloed.
    });

/// Starts following the solo set. Idempotent, so every strip can call it.
export const watchSolo = () => {
  if (watching || !canSolo) return;
  watching = true;
  // Subscribed before asking, so a change made in between is not lost.
  void onSolo((state) => adopt(state.streams));
  void onReconnected(() => void refresh());
  void refresh();
};

/// Solos a stream, or releases it if it is soloed.
export const toggleSolo = async (streamId: number) => {
  const next = !solo.streams.has(streamId);
  solo.error = null;
  try {
    adopt((await setSolo(streamId, next)).streams);
  } catch (e) {
    solo.error = { streamId, message: e instanceof Error ? e.message : String(e) };
  }
};

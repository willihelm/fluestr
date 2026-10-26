// How this frontend reaches the backend (P16).
//
// One bundle, two ways of being run. In the app it is a WebView and the
// backend is one `invoke` away; on a second machine it is an ordinary page
// served by the app over the LAN, and the backend is at the other end of a
// WebSocket. Everything above this module is written once and does not know
// which it is.
//
// Both transports carry the *same* calls, because the backend has one command
// layer and both dispatch into it — see `crates/app/src/commands.rs`. A call
// is `{command, args}`, an answer is a value or an error string, and an event
// is a name and a payload. That is the whole protocol.
//
// What differs is only what a browser has no equivalent for: a native file
// dialog, the platform's attention request, and a second OS window. Those are
// not here — see `windows.ts` and `bridge.ts`.
//
// The optional key is also only a remote concern, and it is arranged so that
// the usual case costs nothing: it rides in the URL's fragment, which is what
// the Settings page puts in the QR code, and this module trades it for a
// cookie once and then wipes it out of the address bar. A phone is asked for
// the key exactly never after the first scan.

import { invoke } from "@tauri-apps/api/core";
import { emit as tauriEmit, listen, type UnlistenFn } from "@tauri-apps/api/event";

/// Whether this frontend is a browser served over the LAN rather than the
/// app's own WebView.
///
/// Decided once, from Tauri's own marker on `window`, because everything that
/// branches on it is a question about capability rather than about preference:
/// there either is a window to flash and a dialog to open, or there is not.
/// `false` where there is no page at all — under the unit tests, which import
/// these modules in Node. Nothing there has a socket to open or a window to
/// ask, and "not remote" is the answer that leaves both alone.
export const isRemote =
  typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window);

/// Whether the socket is connected, for the banner a remote page shows.
///
/// Always `true` in the app: an `invoke` does not have a connection to lose,
/// and a view that showed a connection state there would be showing something
/// that cannot change.
export type Connection = "connected" | "connecting" | "lost";

type Listener = (payload: unknown) => void;

const listeners = new Map<string, Set<Listener>>();
const connectionWatchers = new Set<(state: Connection) => void>();
/// Views that have to be caught up after an interruption. A page that was
/// disconnected missed events, and nothing on the wire can tell it what — so
/// it re-reads the snapshot, exactly as it did when it first opened.
const reconnectWatchers = new Set<() => void>();

let connection: Connection = isRemote ? "connecting" : "connected";

// --- The key (remote only) ------------------------------------------------

/// Whether this page may open the socket at all.
///
/// `"open"` in the app and on a listener that has no key, which is the state
/// everything above this module is written for. `"needed"` is the one case
/// with a screen of its own — see `RemoteKey.svelte`.
export type Lock = "checking" | "open" | "needed";

/// The key the link carried.
///
/// Read once, at the moment this module is first evaluated, and taken out of
/// the address bar in the same breath. That is the whole reason it is in the
/// fragment: a key left in the address is a key in the browser's history, in
/// the next screenshot of this page and in whatever gets sent to a colleague
/// who asks what it looks like on their phone. Kept here instead, because the
/// app may not be answering yet and the key is still needed when it is.
const linkKey = isRemote ? takeKeyFromLink() : null;

function takeKeyFromLink(): string | null {
  try {
    const hash = window.location.hash;
    if (!hash.startsWith("#")) return null;
    const found = new URLSearchParams(hash.slice(1)).get("k");
    if (!found) return null;
    window.history.replaceState(null, "", window.location.pathname + window.location.search);
    return found;
  } catch (e) {
    // A browser that refuses `replaceState` still has a usable key; it just
    // keeps showing it. Better than not connecting.
    console.warn("the key could not be taken out of the address:", e);
    return null;
  }
}

let lock: Lock = isRemote ? "checking" : "open";
const lockWatchers = new Set<(state: Lock) => void>();

export const lockState = () => lock;

/// Follows the lock, and is called back at once with the state as it stands —
/// like `onConnection`, and for its reason.
export const onLock = (fn: (state: Lock) => void): (() => void) => {
  lockWatchers.add(fn);
  fn(lock);
  return () => lockWatchers.delete(fn);
};

const setLock = (state: Lock) => {
  if (lock === state) return;
  lock = state;
  for (const watcher of lockWatchers) watcher(state);
};

/// Trades a key for the cookie that opens the socket.
///
/// `null` when it worked; otherwise what to tell the operator. Exported
/// because the key screen calls it — the automatic path below calls it too,
/// with whatever the link carried, and there is one implementation of it.
export const offerKey = async (key: string): Promise<string | null> => {
  let answer: Response;
  try {
    answer = await fetch("/auth", { method: "POST", body: key });
  } catch {
    return "Fluestr did not answer.";
  }

  if (answer.ok) {
    setLock("open");
    connect();
    return null;
  }
  // The rate limit in front of the key. Said plainly, because the alternative
  // is a key that is right being refused with "that is not the key".
  if (answer.status === 429) {
    return "Too many wrong keys from this machine. Wait a minute and try again.";
  }
  return "That is not the key.";
};

/// Whether the socket may be opened, asking the app if need be.
///
/// Asked *before* every connection and every reconnection rather than once at
/// startup. A refused upgrade is not something a browser will say anything
/// useful about — `onclose` gives 1006 and no status, so "wrong key" and
/// "network gone" arrive as the same event — and this is what tells them
/// apart. It is also what turns a key replaced mid-show into the key screen
/// instead of a page that retries forever.
const admitted = async (): Promise<boolean> => {
  let state: { required?: boolean; unlocked?: boolean };
  try {
    const answer = await fetch("/auth", { cache: "no-store" });
    state = await answer.json();
  } catch {
    // The app is not answering at all, which is not a locked page but a dark
    // one. Let the socket try and fail: its failure is what drives the
    // "not connected" banner, and this must not replace that with a key
    // screen that no key will get past.
    return true;
  }

  if (!state.required || state.unlocked) {
    setLock("open");
    return true;
  }

  if (linkKey && (await offerKey(linkKey)) === null) return true;

  setLock("needed");
  return false;
};

/// The current state, for a view that mounts after the socket has settled.
export const connectionState = () => connection;

/// Follows the socket. Called back immediately with the state as it stands, so
/// a component that mounts late does not sit on a stale default.
///
/// In the app this fires once with `"connected"` and never again: an `invoke`
/// has no connection to lose.
export const onConnection = (fn: (state: Connection) => void): (() => void) => {
  connectionWatchers.add(fn);
  fn(connection);
  return () => connectionWatchers.delete(fn);
};

const setConnection = (state: Connection) => {
  if (connection === state) return;
  connection = state;
  for (const watcher of connectionWatchers) watcher(state);
};

// --- The socket (remote only) ---------------------------------------------

/// How long to wait before trying again, and the ceiling it climbs to.
///
/// Fast at first because the common interruption is the app restarting, which
/// is over in a second; slowing down because the other common one is a laptop
/// that was carried out of range, and hammering a dead network for an hour
/// costs the battery of the machine an operator still needs.
const RECONNECT_MIN_MS = 500;
const RECONNECT_MAX_MS = 5_000;

/// How long a call may wait for an answer.
///
/// Generous, because the slowest call in the set is `start_session`, which
/// loads the model — eight seconds on a machine's first ever start. What this
/// is for is the case where the socket is up and the answer never comes: a
/// button that stays disabled forever is worse than one that reports a
/// timeout.
const CALL_TIMEOUT_MS = 30_000;

interface Waiting {
  resolve: (value: unknown) => void;
  reject: (reason: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

let socket: WebSocket | null = null;
/// Whether the socket has ever been open. What tells a first connection from a
/// reconnection, which are the same event on the wire and two different things
/// to a view: one is opening, the other is having missed something.
let everConnected = false;
let nextId = 1;
let backoff = RECONNECT_MIN_MS;
const waiting = new Map<number, Waiting>();
/// Calls made while the socket is down, released when it comes back up. A
/// remote page starts by asking for its configuration and its snapshot, and
/// those are made before the socket has finished opening every single time.
let queued: (() => void)[] = [];
/// Whether a queued call was given up on while the socket was down. The page
/// that made it has an error and a gap: it asked for something the app never
/// heard, so when the socket does come back it is caught up like any other
/// interruption, even if this was its first connection.
let missedWhileQueued = false;

/// Where the app is. The page came *from* it, so it is the host of this
/// document — which is also why the remote never needs an address typed in
/// twice.
const socketUrl = () => {
  const scheme = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${window.location.host}/ws`;
};

/// Whether a connection is already on its way up.
///
/// The check in front of the socket is a round trip, so `socket` is still
/// `null` while it runs and everything that calls `connect` — a queued call,
/// the reconnect timer — would start a second one.
let opening = false;

/// Opens the socket, once the app has said this page may have one.
const connect = () => {
  if (!isRemote || socket || opening) return;
  opening = true;
  admitted()
    .then((may) => {
      opening = false;
      if (may) open();
      // Otherwise nothing is retried: the key screen is up, and what starts
      // the next attempt is the operator offering a key.
    })
    .catch((e) => {
      opening = false;
      console.warn("the app could not be asked whether this page may connect:", e);
      setTimeout(connect, backoff);
      backoff = Math.min(backoff * 2, RECONNECT_MAX_MS);
    });
};

const open = () => {
  if (socket) return;

  // "lost" is kept until the socket is actually open again: the banner it
  // drives is what tells a remote operator that what they are looking at has
  // stopped being live, and flickering it back to "connecting" every half
  // second says the opposite.
  const next = new WebSocket(socketUrl());
  socket = next;

  next.onopen = () => {
    backoff = RECONNECT_MIN_MS;
    const again = everConnected;
    everConnected = true;
    setConnection("connected");
    // Everything that was asked for while the socket was down, in the order it
    // was asked for. A reconnecting page re-reads its snapshot through exactly
    // this path — see `bridge.ts` — so the catch-up is a normal call and not a
    // second mechanism.
    const released = queued;
    queued = [];
    for (const send of released) send();

    const catchUp = missedWhileQueued;
    missedWhileQueued = false;

    // After the queue, so that a view's catch-up is asked for on a socket that
    // is already carrying whatever it asked for on the way down. The snapshot
    // is bounded by the same history every other view is caught up from, so a
    // page that slept longer than the buffer resumes from what is left rather
    // than from nothing.
    if (again || catchUp) {
      for (const watcher of reconnectWatchers) watcher();
    }
  };

  next.onmessage = (message) => {
    let frame: unknown;
    try {
      frame = JSON.parse(String(message.data));
    } catch (e) {
      console.warn("the app sent something that is not a frame:", e);
      return;
    }
    receive(frame);
  };

  next.onclose = () => {
    socket = null;
    setConnection("lost");
    // Nothing is going to answer these. Rejecting rather than leaving them
    // hanging: a caller that is told the connection went is a caller that can
    // say so, and every one of them already renders its own failure.
    for (const [, call] of waiting) {
      clearTimeout(call.timer);
      call.reject(new Error("the connection to the app was lost"));
    }
    waiting.clear();

    setTimeout(connect, backoff);
    backoff = Math.min(backoff * 2, RECONNECT_MAX_MS);
  };

  // `onclose` follows every `onerror`, so the reconnect is arranged in one
  // place rather than in two that would both schedule one.
  next.onerror = () => next.close();
};

const receive = (frame: unknown) => {
  if (typeof frame !== "object" || frame === null) return;
  const message = frame as Record<string, unknown>;

  if (message.type === "event" && typeof message.event === "string") {
    for (const fn of listeners.get(message.event) ?? []) fn(message.payload);
    return;
  }

  if (message.type === "result" && typeof message.id === "number") {
    const call = waiting.get(message.id);
    if (!call) return;
    waiting.delete(message.id);
    clearTimeout(call.timer);
    if (message.ok) call.resolve(message.value ?? null);
    else call.reject(new Error(String(message.error ?? "the call failed")));
  }
};

const callOverSocket = <T>(command: string, args?: object): Promise<T> =>
  new Promise<T>((resolve, reject) => {
    const id = nextId++;
    /// Running while this call is waiting for a socket to be sent on, rather
    /// than for an answer. Two different waits, and both of them have to end:
    /// without this, a button pressed while the link is down stays disabled
    /// for as long as the link stays down, with nothing to tell the operator
    /// that what they asked for never left the page.
    let waitingForSocket: ReturnType<typeof setTimeout> | null = null;
    const send = () => {
      if (waitingForSocket !== null) {
        clearTimeout(waitingForSocket);
        waitingForSocket = null;
      }
      const open = socket;
      if (!open || open.readyState !== WebSocket.OPEN) {
        reject(new Error("the connection to the app was lost"));
        return;
      }
      const timer = setTimeout(() => {
        waiting.delete(id);
        reject(new Error(`${command} was not answered`));
      }, CALL_TIMEOUT_MS);
      waiting.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timer,
      });
      open.send(JSON.stringify(args === undefined ? { id, command } : { id, command, args }));
    };

    if (socket?.readyState === WebSocket.OPEN) send();
    else {
      queued.push(send);
      waitingForSocket = setTimeout(() => {
        queued = queued.filter((waiting) => waiting !== send);
        missedWhileQueued = true;
        reject(new Error(`${command} was not answered`));
      }, CALL_TIMEOUT_MS);
      connect();
    }
  });

/// Called after the socket comes back, never on the first connection.
///
/// What a view does with it is re-read its snapshot — the same call it makes
/// when it opens — so reconnecting is the opening path run twice rather than a
/// second mechanism that could disagree with it.
export const onReconnected = (fn: () => void): (() => void) => {
  reconnectWatchers.add(fn);
  return () => reconnectWatchers.delete(fn);
};

if (isRemote) connect();

// --- What the rest of the frontend uses -----------------------------------

/// Runs one backend command.
///
/// The same call on both transports, and the same command layer behind both:
/// `invoke("command", …)` in the app, one socket frame from a browser. A
/// caller cannot tell the difference and must not need to.
export const call = <T>(command: string, args?: object): Promise<T> =>
  isRemote
    ? callOverSocket<T>(command, args)
    : invoke<T>("command", {
        request: args === undefined ? { command } : { command, args },
      });

/// Subscribes to a backend event.
///
/// Resolves with the unsubscribe function, like Tauri's own `listen`, so
/// everything that already unsubscribes on teardown keeps working unchanged.
export const on = <T>(event: string, fn: (payload: T) => void): Promise<UnlistenFn> => {
  if (!isRemote) return listen<T>(event, (e) => fn(e.payload));

  const wrapped = (payload: unknown) => fn(payload as T);
  let set = listeners.get(event);
  if (!set) {
    set = new Set();
    listeners.set(event, set);
  }
  set.add(wrapped);
  return Promise.resolve(() => {
    set.delete(wrapped);
  });
};

// --- Between this browser's pages (remote only) ---------------------------

/// What a page's sibling pages hear from it: the tabs and pop-ups opened from
/// the same app in the same browser. The browser's equivalent of Tauri's
/// window-to-window events, and on the same origin by construction — every
/// one of them was served by the same app.
///
/// `null` where there is none: in the app, under the unit tests, and in a
/// browser too old to have one, where a pop-up still shows what it shows and
/// only its requests to the main tab go nowhere.
const siblings: BroadcastChannel | null =
  isRemote && typeof BroadcastChannel !== "undefined" ? new BroadcastChannel("fluestr") : null;

if (siblings) {
  siblings.onmessage = (message) => {
    const frame = message.data as { event?: unknown; payload?: unknown } | null;
    if (typeof frame?.event !== "string") return;
    for (const fn of listeners.get(frame.event) ?? []) fn(frame.payload);
  };
}

/// Sends an event from this view to the app's other views.
///
/// Window-to-window traffic, and only ever that — a pop-up asking the main
/// window to store an edit or to show a line. In a browser it goes to the
/// pages this browser has open on the app, and to no other machine: what one
/// operator's pop-up asks of their own main tab is nobody else's business,
/// and the backend has no reason to route it.
export const emit = (event: string, payload: unknown): Promise<void> => {
  if (!isRemote) return tauriEmit(event, payload);
  siblings?.postMessage({ event, payload });
  return Promise.resolve();
};

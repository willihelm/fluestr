// Which view this page is (P7, P16).
//
// One bundle, mounted everywhere the app is looked at: its main window, a
// detached window per view — a group, or all channels — and, over the LAN, a
// browser tab for any of them. The window's own label says which in the app,
// the query string says it in a browser; both are readable before the first
// paint, so a detached view never renders the main view for a frame and then
// replaces it. See `windows.ts`. A ticker (P24) is a detached view laid out
// for the least room, told apart by its own label prefix. The shoutbox window
// (P26) is no view at all but the shoutbox's controls, under a label of its own.

import { mount } from "svelte";
import App from "./App.svelte";
import GroupWindow from "./lib/GroupWindow.svelte";
import TickerWindow from "./lib/TickerWindow.svelte";
import ShoutboxWindow from "./lib/ShoutboxWindow.svelte";
import RemoteGate from "./lib/RemoteGate.svelte";
import { currentGroup, currentTicker, isShoutboxWindow } from "./lib/windows";
import { watchForAttention } from "./lib/flash";
import { isRemote } from "./lib/transport";
import "./app.css";

const target = document.getElementById("app")!;
const group = currentGroup();
const ticker = currentTicker();

// A browser tab has no platform to ask for attention, so it flashes itself
// (P16) — and stops the moment it is actually looked at. Installed once for the
// page rather than per view, because it is about the tab and not about what is
// rendered in it. The app's own windows ask the platform instead and need
// none of this.
if (isRemote) watchForAttention();

// Through the gate, always, whichever view this page is. It is transparent in
// the app and on a listener with no key — see `RemoteGate.svelte` — and it is
// what puts the key screen in front of a browser that has nothing to get in
// with.
//
// A ticker skips it, and so does the shoutbox window: both only exist in the
// app, where the gate lets everything through anyway.
export default isShoutboxWindow()
  ? mount(ShoutboxWindow, { target })
  : ticker !== null
  ? mount(TickerWindow, { target, props: { view: ticker } })
  : group === null
    ? mount(RemoteGate, { target, props: { view: App } })
    : mount(RemoteGate, { target, props: { view: GroupWindow, props: { group } } });

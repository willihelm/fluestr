// Which view this page is (P7, P16).
//
// One bundle, mounted everywhere the app is looked at: its main window, a
// detached window per group, and — over the LAN — a browser tab for either. The
// window's own label says which in the app, the query string says it in a
// browser; both are readable before the first paint, so a group view never
// renders the main view for a frame and then replaces it. See `windows.ts`.

import { mount } from "svelte";
import App from "./App.svelte";
import GroupWindow from "./lib/GroupWindow.svelte";
import { currentGroup } from "./lib/windows";
import { watchForAttention } from "./lib/flash";
import { isRemote } from "./lib/transport";
import "./app.css";

const target = document.getElementById("app")!;
const group = currentGroup();

// A browser tab has no platform to ask for attention, so it flashes itself
// (P16) — and stops the moment it is actually looked at. Installed once for the
// page rather than per view, because it is about the tab and not about what is
// rendered in it. The app's own windows ask the platform instead and need
// none of this.
if (isRemote) watchForAttention();

export default group === null
  ? mount(App, { target })
  : mount(GroupWindow, { target, props: { group } });

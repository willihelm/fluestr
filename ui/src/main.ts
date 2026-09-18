// Which view this WebView is (P7).
//
// One bundle, mounted in every window the app opens: the main window, and a
// detached window per group. The window's own label says which — it is readable
// before the first paint, so a group window never renders the main view for a
// frame and then replaces it.

import { mount } from "svelte";
import App from "./App.svelte";
import GroupWindow from "./lib/GroupWindow.svelte";
import { currentGroup } from "./lib/windows";
import "./app.css";

const target = document.getElementById("app")!;
const group = currentGroup();

export default group === null
  ? mount(App, { target })
  : mount(GroupWindow, { target, props: { group } });

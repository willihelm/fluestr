import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],

  // Fixed port: tauri.conf.json points at it. Fail rather than silently
  // starting on a different port that Tauri would then not find.
  server: {
    port: 1420,
    strictPort: true,
    // Reachable from the machine the LAN remote (P16) is being tested from.
    //
    // The remote is served by the *app*, on its own port, and in a checkout
    // the app passes those requests through to here. But the page it hands
    // over carries Vite's hot-reload client, and that client connects back to
    // this server by host and port — from another machine, that has to be an
    // address and a port that machine can reach. Without this it cannot, and
    // the remote keeps running whatever it loaded first while every change
    // lands in the app's own window: a stale browser that looks like a bug
    // that will not be fixed.
    //
    // Development only — `vite dev` is not part of a built app, and the port
    // is open for as long as `just dev` runs and no longer.
    host: true,
    hmr: { port: 1420 },
  },

  // Tauri errors should stay visible in the terminal, not only in the WebView.
  clearScreen: false,
  build: {
    target: "safari15",
    sourcemap: true,
  },
});

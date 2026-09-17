import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],

  // Fixed port: tauri.conf.json points at it. Fail rather than silently
  // starting on a different port that Tauri would then not find.
  server: {
    port: 1420,
    strictPort: true,
  },

  // Tauri errors should stay visible in the terminal, not only in the WebView.
  clearScreen: false,
  build: {
    target: "safari15",
    sourcemap: true,
  },
});

<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  // P0 scaffolding: proves only that the Tauri bridge is up. The chat view
  // arrives in P7, once P6 emits the events.
  let version = $state<string | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    invoke<string>("app_version")
      .then((v) => (version = v))
      .catch((e) => (error = String(e)));
  });
</script>

<main>
  <h1>Intercom2Chat</h1>
  <p class="dim">Offline transcription for intercom streams</p>

  {#if error}
    <p class="error">Tauri bridge unreachable: {error}</p>
  {:else if version}
    <p class="ok">Backend connected — version {version}</p>
  {:else}
    <p class="dim">Connecting …</p>
  {/if}
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100vh;
    gap: 0.25rem;
  }

  h1 {
    margin: 0;
    font-size: 1.75rem;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .dim {
    color: var(--text-dim);
    margin: 0;
  }

  .ok {
    color: var(--accent);
    margin-top: 1rem;
  }

  .error {
    color: #ff6b6b;
    margin-top: 1rem;
  }
</style>

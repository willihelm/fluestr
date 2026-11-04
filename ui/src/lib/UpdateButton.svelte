<script lang="ts">
  // A newer release is out. Shown beside the app's name only when there is
  // one — the main window asks GitHub once as it opens — and silent otherwise:
  // an app that is up to date has nothing to say about it.
  //
  // Whether the changelog is open is the main window's to say (see
  // `App.svelte`): it opens by itself once per release, and from the app
  // menu's "Check for Updates…". This button is the way back to it after
  // "Later".

  import type { AvailableUpdate } from "./bridge";
  import { markSeen } from "./changelog";
  import ChangelogDialog from "./ChangelogDialog.svelte";

  interface Props {
    update: AvailableUpdate;
    /// Whether a session is running.
    running: boolean;
    open: boolean;
  }

  let { update, running, open = $bindable() }: Props = $props();

  const close = () => {
    markSeen(update.version);
    open = false;
  };
</script>

<button
  type="button"
  class="chip"
  title="Fluestr {update.version} is available"
  onclick={() => (open = true)}
>
  <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
    <path
      d="M8 2v8M4.5 6.5 8 10l3.5-3.5M3 13.5h10"
      fill="none"
      stroke="currentColor"
      stroke-width="1.6"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
  </svg>
  <span>Update to {update.version}</span>
</button>

{#if open}
  <ChangelogDialog {update} {running} onclose={close} />
{/if}

<style>
  .chip {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: 0.35rem;
    padding: 0.15rem 0.55rem;
    font-size: 0.75rem;
    white-space: nowrap;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
    border-radius: 999px;
    cursor: pointer;
  }

  .chip:hover {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
</style>

<script lang="ts">
  // What happens when the machine is switched on.
  //
  // For the machine that sits in the rack and is powered up before a show:
  // two switches, and with both on it comes up transcribing without anybody
  // touching it — the app opens at login, and the session starts once the app
  // has.
  //
  // The two are stored in different places, on purpose.
  //
  // **Starting the session** is `autoStart` in `config.json`, like the rest of
  // the page: there is no Apply, and it saves through the same debounce.
  //
  // **Opening the app at login** is not in the file at all. Where the app is
  // registered is the operating system's — Login Items on macOS, the Startup
  // apps on Windows — and either can be switched off there without the app
  // hearing of it. So that switch reads the system when the section is mounted
  // and shows what the system answers after a click, rather than what was
  // clicked. It is the host machine's alone, so the remote does not show it.

  import { onMount } from "svelte";
  import { autostartEnabled, canAutostart, setAutostart } from "./bridge";

  interface Props {
    autoStart: boolean;
  }

  let { autoStart = $bindable() }: Props = $props();

  let enabled = $state(false);
  let ready = $state(false);
  let busy = $state(false);
  let failed = $state<string | null>(null);

  onMount(() => {
    if (!canAutostart) return;
    autostartEnabled()
      .then((on) => {
        enabled = on;
        ready = true;
      })
      .catch((err) => (failed = `The login item could not be read: ${err}`));
  });

  const toggle = async (box: HTMLInputElement) => {
    busy = true;
    failed = null;
    try {
      enabled = await setAutostart(box.checked);
    } catch (err) {
      failed = `The login item could not be changed: ${err}`;
    } finally {
      busy = false;
      // Put back by hand: a refused change leaves `enabled` where it was, so
      // nothing re-renders, and the box would go on showing the click.
      box.checked = enabled;
    }
  };
</script>

<section class="panel">
  <header>
    <h2>Startup</h2>
  </header>

  {#if canAutostart}
    <label class="switch">
      <input
        type="checkbox"
        checked={enabled}
        disabled={!ready || busy}
        onchange={(e) => toggle(e.currentTarget)}
      />
      <span>
        Open Fluestr when you log in
        <small>Registered with the system: Login Items on macOS, Startup apps on Windows.</small>
      </span>
    </label>

    {#if failed}
      <p class="failed">{failed}</p>
    {/if}
  {/if}

  <label class="switch">
    <input type="checkbox" bind:checked={autoStart} />
    <span>
      Start the session when Fluestr opens
      <small>With the profile that was selected last. Stopping it later does not start it again.</small>
    </span>
  </label>
</section>

<style>
  /* The card itself is `app.css`; this only lays out what is inside it. */
  .panel {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .switch {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .switch > span {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
  }

  small {
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .failed {
    margin: 0;
    font-size: 0.8rem;
    color: #ff6b6b;
  }
</style>

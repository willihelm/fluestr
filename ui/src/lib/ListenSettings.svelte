<script lang="ts">
  // Where a bubble is heard again (P22): headphones, typically, so a replay
  // reaches the operator and nobody else.
  //
  // Part of the profile and reaching the backend through the save, like the
  // channel map: the backend opens what the file says, at startup and on
  // every save, independently of any session.
  //
  // In a browser on the LAN (P16) it is shown and not editable: the
  // headphones are the host machine's, and a replay there plays in the
  // browser.

  import { onMount } from "svelte";
  import {
    isRemote,
    listDevices,
    listOutputDevices,
    type DeviceOption,
    type OutputDeviceOption,
    type StoredOutputs,
  } from "./bridge";
  import {
    decodeChannels,
    deviceFor,
    encodeChannels,
    outputChoices,
    pickOutput,
    routeKey,
  } from "./outputs";

  interface Props {
    outputs: StoredOutputs;
    /// The input devices, the app's own list: Refresh here lists them again
    /// for the whole app, as it does in the Channels section.
    inputDevices: DeviceOption[];
  }

  let { outputs = $bindable(), inputDevices = $bindable() }: Props = $props();

  // Asked for again whenever the section opens, and on Refresh: headphones
  // plugged in after startup have to be offered without a restart.
  let outputDevices = $state<OutputDeviceOption[]>([]);
  let listError = $state<string | null>(null);

  const refresh = async () => {
    try {
      [inputDevices, outputDevices] = await Promise.all([listDevices(), listOutputDevices()]);
      listError = null;
    } catch (e) {
      listError = String(e);
    }
  };

  onMount(() => void refresh());

  let listen = $derived(outputs.listen);
</script>

<section class="panel">
  <header>
    <h2>Listen</h2>
    <button type="button" class="link" onclick={refresh} title="Look for input and output devices again">
      Refresh devices
    </button>
  </header>
  <p class="hint">Where a message is played again when its ▶ is pressed.</p>

  <div class="row">
    <label>
      <span>Output</span>
      <select
        value={routeKey(listen.output)}
        disabled={isRemote}
        onchange={(e) => pickOutput(listen.output, outputDevices, e.currentTarget.value)}
      >
        <option value="">System output</option>
        {#each outputDevices as option (option.id)}
          <option value={option.id}>{option.name} ({option.channels} ch)</option>
        {/each}
        {#if listen.output.device !== "" && !deviceFor(outputDevices, listen.output)}
          <option value={routeKey(listen.output)}>{listen.output.device} — not connected</option>
        {/if}
      </select>
    </label>
    {#if listen.output.device !== ""}
      <label class="narrow">
        <span>Channels</span>
        <select
          value={encodeChannels(listen.output.channels)}
          disabled={isRemote}
          onchange={(e) => (listen.output.channels = decodeChannels(e.currentTarget.value))}
        >
          <option value="">Default</option>
          {#each outputChoices(deviceFor(outputDevices, listen.output)?.channels ?? 2) as choice (choice.label)}
            <option value={encodeChannels(choice.channels)}>{choice.label}</option>
          {/each}
        </select>
      </label>
    {/if}
    <label class="narrow">
      <span>Keep (minutes of speech)</span>
      <input
        type="number"
        min="0"
        max="120"
        disabled={isRemote}
        bind:value={listen.keepMinutes}
      />
    </label>
  </div>

  {#if listError}
    <p class="failed">The audio devices could not be listed: {listError}</p>
  {/if}
</section>

<style>
  .hint {
    margin: 0 0 0.8rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .failed {
    margin: 0.4rem 0 0;
    font-size: 0.8rem;
    color: #ff6b6b;
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.6rem;
    align-items: flex-end;
    margin-bottom: 0.4rem;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: 0.7rem;
    color: var(--text-dim);
    flex: 1 1 14rem;
    max-width: 24rem;
  }

  label.narrow {
    flex: 0 1 11rem;
  }

  select,
  input[type="number"] {
    background-color: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.9rem;
  }

  select {
    padding-right: 1.5rem;
  }

  .link {
    margin-left: auto;
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    font-size: 0.8rem;
    color: var(--text-dim);
    text-decoration: underline;
    cursor: pointer;
  }
</style>

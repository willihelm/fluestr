<script lang="ts">
  // The shoutbox (P26): one intercom channel passed through to a talkback
  // speaker, its level following a measurement mic in the room. The replay
  // output beside it is `ListenSettings`.
  //
  // Everything here is part of the profile and reaches the backend through the
  // save, like the channel map: the backend opens what the file says, at
  // startup and on every save, independently of any session. The two things
  // that cannot wait for the save's debounce — the trim and the mute — also go
  // straight to it.
  //
  // Editable in full in a browser on the LAN (P16) too, because whoever tunes
  // it is standing by the speaker rather than at the laptop.

  import { onMount } from "svelte";
  import ShoutboxControls from "./ShoutboxControls.svelte";
  import {
    listDevices,
    listOutputDevices,
    SHOUTBOX_ESTIMATED_CALIBRATION_DB,
    onShoutboxStatus,
    setShoutboxLive,
    shoutboxStatus,
    type DeviceOption,
    type OutputDeviceOption,
    type ShoutboxStatus,
    type StoredOutputs,
  } from "./bridge";
  import {
    calibrationFor,
    recalibrate,
    isShoutboxDefault,
    roundTenth,
    shoutboxDefaults,
    decodeChannels,
    deviceFor,
    encodeChannels,
    outputChoices,
    pickInput,
    pickOutput,
    routeKey,
  } from "./outputs";

  interface Props {
    outputs: StoredOutputs;
    /// The input devices, the app's own list: asking again here refreshes
    /// the Audio section's picker as well.
    inputDevices: DeviceOption[];
  }

  let { outputs = $bindable(), inputDevices = $bindable() }: Props = $props();

  // Both lists are asked for again whenever the section opens, and on
  // Refresh. The app lists its inputs once, at startup, and a USB mic
  // connected after that would otherwise not be offered — the one place it is
  // needed here being the measurement mic.
  let outputDevices = $state<OutputDeviceOption[]>([]);
  let listError = $state<string | null>(null);
  let status = $state<ShoutboxStatus | null>(null);

  const refresh = async () => {
    try {
      [inputDevices, outputDevices] = await Promise.all([listDevices(), listOutputDevices()]);
      listError = null;
    } catch (e) {
      listError = String(e);
    }
  };

  onMount(() => {
    void refresh();
    let unlisten: (() => void) | undefined;
    void onShoutboxStatus((next) => (status = next)).then((fn) => (unlisten = fn));
    shoutboxStatus()
      .then((s) => (status ??= s))
      .catch(() => {});
    return () => unlisten?.();
  });

  let shoutbox = $derived(outputs.shoutbox);

  /// The tuning back to the defaults; see `shoutboxDefaults` for what stays.
  /// The trim goes the live way too, so the speaker follows at once.
  const resetShoutbox = () => {
    outputs.shoutbox = shoutboxDefaults(shoutbox);
    live(outputs.shoutbox.trimDb, outputs.shoutbox.muted);
  };

  /// The output must be on the program's interface — one clock. Said here,
  /// before the backend refuses it, in the same words.
  let clockProblem = $derived.by(() => {
    if (shoutbox.program.device === "" || shoutbox.output.device === "") return null;
    const same =
      (shoutbox.program.deviceId !== "" && shoutbox.program.deviceId === shoutbox.output.deviceId) ||
      shoutbox.program.device === shoutbox.output.device;
    return same
      ? null
      : `The shoutbox output has to be on the program's interface (${shoutbox.program.device}): two devices are two clocks.`;
  });

  /// Trim and mute go to the running shoutbox at once, and into the profile
  /// for the save.
  const live = (trimDb: number, muted: boolean) => {
    shoutbox.trimDb = trimDb;
    shoutbox.muted = muted;
    void setShoutboxLive(trimDb, muted).catch(() => {});
  };

  // --- Calibration --------------------------------------------------------

  /// What a calibrator puts out, so the usual case is one press.
  const CALIBRATOR_DB = 94;
  let meterReading = $state<number | null>(CALIBRATOR_DB);
  let offset = $derived(
    meterReading === null
      ? null
      : calibrationFor(meterReading, status?.liveDb ?? null, shoutbox.calibrationDb ?? null),
  );

  const stateText: Record<string, string> = {
    off: "Off",
    starting: "Opening its devices …",
    running: "Running",
    failed: "Not running",
  };
</script>

<section class="panel">
  <header>
    <h2>Shoutbox</h2>
    <span class="state {status?.state ?? 'off'}">{stateText[status?.state ?? "off"]}</span>
    <button type="button" class="link" onclick={refresh} title="Look for input and output devices again">
      Refresh devices
    </button>
    {#if !isShoutboxDefault(shoutbox)}
      <button
        type="button"
        class="link reset"
        onclick={resetShoutbox}
        title="Curve, speeds, gate, ceiling, averaging and trim. The patch, the calibration and the mute stay."
      >
        Back to the defaults
      </button>
    {/if}
  </header>

  <p class="hint">Passes one intercom channel to a talkback speaker, its level following a mic in the room.</p>

  <label class="switch">
    <input type="checkbox" bind:checked={shoutbox.enabled} />
    Shoutbox on
  </label>

  {#if status?.message}
    <p class="failed">{status.message}</p>
  {/if}
  {#if listError}
    <p class="failed">The audio devices could not be listed: {listError}</p>
  {/if}

  <h3>Patch</h3>
  <div class="row">
    <label>
      <span>Program input</span>
      <select
        value={routeKey(shoutbox.program)}
        onchange={(e) => pickInput(shoutbox.program, inputDevices, e.currentTarget.value)}
      >
        <option value="">Not patched</option>
        {#each inputDevices as option (option.id)}
          <option value={option.id}>{option.name}</option>
        {/each}
        {#if shoutbox.program.device !== "" && !deviceFor(inputDevices, shoutbox.program)}
          <option value={routeKey(shoutbox.program)}>{shoutbox.program.device} — not connected</option>
        {/if}
      </select>
    </label>
    <label class="narrow">
      <span>Channel</span>
      <select bind:value={shoutbox.program.channel}>
        {#each Array.from({ length: deviceFor(inputDevices, shoutbox.program)?.channels ?? shoutbox.program.channel + 1 }, (_, c) => c) as c (c)}
          <option value={c}>{c + 1}</option>
        {/each}
      </select>
    </label>
  </div>

  <div class="row">
    <label>
      <span>Measurement mic</span>
      <select
        value={routeKey(shoutbox.reference)}
        onchange={(e) => pickInput(shoutbox.reference, inputDevices, e.currentTarget.value)}
      >
        <option value="">Not patched</option>
        {#each inputDevices as option (option.id)}
          <option value={option.id}>{option.name}</option>
        {/each}
        {#if shoutbox.reference.device !== "" && !deviceFor(inputDevices, shoutbox.reference)}
          <option value={routeKey(shoutbox.reference)}>{shoutbox.reference.device} — not connected</option>
        {/if}
      </select>
    </label>
    <label class="narrow">
      <span>Channel</span>
      <select bind:value={shoutbox.reference.channel}>
        {#each Array.from({ length: deviceFor(inputDevices, shoutbox.reference)?.channels ?? shoutbox.reference.channel + 1 }, (_, c) => c) as c (c)}
          <option value={c}>{c + 1}</option>
        {/each}
      </select>
    </label>
  </div>

  <div class="row">
    <label>
      <span>Shoutbox output</span>
      <select
        value={routeKey(shoutbox.output)}
        onchange={(e) => pickOutput(shoutbox.output, outputDevices, e.currentTarget.value)}
      >
        <option value="">Not patched</option>
        {#each outputDevices as option (option.id)}
          <option value={option.id}>{option.name}</option>
        {/each}
        {#if shoutbox.output.device !== "" && !deviceFor(outputDevices, shoutbox.output)}
          <option value={routeKey(shoutbox.output)}>{shoutbox.output.device} — not connected</option>
        {/if}
      </select>
    </label>
    <label class="narrow">
      <span>Channels</span>
      <select
        value={encodeChannels(shoutbox.output.channels)}
        onchange={(e) => (shoutbox.output.channels = decodeChannels(e.currentTarget.value))}
      >
        <option value="">Default</option>
        {#each outputChoices(deviceFor(outputDevices, shoutbox.output)?.channels ?? 2) as choice (choice.label)}
          <option value={encodeChannels(choice.channels)}>{choice.label}</option>
        {/each}
      </select>
    </label>
  </div>
  {#if clockProblem}
    <p class="failed">{clockProblem}</p>
  {/if}

  <h3>Measurement</h3>
  <div class="row">
    <label class="narrow">
      <span>Averaging (s)</span>
      <input
        type="number"
        min="0.5"
        max="30"
        step="0.5"
        value={shoutbox.windowMs / 1000}
        onchange={(e) => (shoutbox.windowMs = Math.round(Number(e.currentTarget.value) * 1000))}
      />
    </label>
    <label class="narrow">
      <span>Calibration offset (dB)</span>
      <input
        type="number"
        min="-200"
        max="200"
        step="0.1"
        placeholder="{SHOUTBOX_ESTIMATED_CALIBRATION_DB} (estimated)"
        value={shoutbox.calibrationDb ?? ""}
        onchange={(e) =>
          recalibrate(
            shoutbox,
            e.currentTarget.value === "" ? null : roundTenth(Number(e.currentTarget.value)),
          )}
      />
    </label>
  </div>
  <div class="row calibrate">
    <label class="narrow">
      <span>An SPL meter at the mic reads (dB)</span>
      <input type="number" min="20" max="140" step="0.1" bind:value={meterReading} />
    </label>
    <button
      type="button"
      disabled={offset === null}
      onclick={() => {
        recalibrate(shoutbox, offset);
        meterReading = CALIBRATOR_DB;
      }}
    >
      Calibrate{offset !== null ? ` (offset ${offset.toFixed(1)} dB)` : ""}
    </button>
  </div>
  <p class="note">Without a calibration the room level is an estimate. Read the meter while the shoutbox is silent.</p>

  <ShoutboxControls bind:shoutbox={outputs.shoutbox} {status} />

</section>

<style>
  .hint {
    margin: 0 0 0.8rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .note {
    margin: 0.3rem 0 0;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .failed {
    margin: 0.4rem 0 0;
    font-size: 0.8rem;
    color: #ff6b6b;
  }

  h3 {
    margin: 1rem 0 0.4rem;
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
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

  label.switch {
    flex-direction: row;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.9rem;
    color: var(--text);
    margin: 0.3rem 0;
    max-width: none;
    cursor: pointer;
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

  input[type="checkbox"] {
    accent-color: var(--accent);
  }

  button {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.7rem;
    font: inherit;
    font-size: 0.85rem;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .link {
    margin-left: auto;
    background: none;
    border: none;
    padding: 0;
    font-size: 0.8rem;
    color: var(--text-dim);
    text-decoration: underline;
  }

  .link.reset {
    margin-left: 0;
  }

  .state {
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .state.running {
    color: #5fd38d;
  }

  .state.failed {
    color: #ff6b6b;
  }

  .calibrate button {
    margin-bottom: 0.05rem;
  }
</style>

<script lang="ts">
  // Where audio leaves the app (P22, P26).
  //
  // Two outputs and deliberately two: the **listen** output, where a bubble is
  // heard again — headphones, so nobody else hears it — and the **shoutbox**,
  // one intercom channel passed through to a talkback speaker with its level
  // following a measurement mic in the room.
  //
  // Everything here is part of the profile and reaches the backend through the
  // save, like the channel map: the backend opens what the file says, at
  // startup and on every save, independently of any session. The two things
  // that cannot wait for the save's debounce — the shoutbox's trim and mute —
  // also go straight to it.
  //
  // In a browser on the LAN (P16) the listen output is shown and not editable:
  // the headphones are the host machine's, and a replay there plays in the
  // browser. The shoutbox is editable in full, because whoever tunes it is
  // standing by the speaker rather than at the laptop.

  import { onMount } from "svelte";
  import ShoutboxControls from "./ShoutboxControls.svelte";
  import {
    isRemote,
    listDevices,
    listOutputDevices,
    SHOUTBOX_ESTIMATED_CALIBRATION_DB,
    onShoutboxStatus,
    setShoutboxLive,
    shoutboxStatus,
    type DeviceOption,
    type OutputDeviceOption,
    type ShoutboxStatus,
    type StoredInputRoute,
    type StoredOutputRoute,
    type StoredOutputs,
  } from "./bridge";
  import {
    calibrationFor,
    recalibrate,
    isShoutboxDefault,
    roundTenth,
    shoutboxDefaults,
    decodeChannels,
    encodeChannels,
    outputChoices,
  } from "./outputs";

  interface Props {
    outputs: StoredOutputs;
    /// The input devices as the app listed them at startup — what the
    /// pickers show until this section has asked again.
    inputDevices: DeviceOption[];
  }

  let { outputs = $bindable(), inputDevices: initialInputs }: Props = $props();

  // Both lists are asked for again whenever the section opens, and on
  // Refresh. The app lists its inputs once, at startup, and a Bluetooth
  // headset or a USB mic connected after that would otherwise be offered as
  // an output and not as an input — the one place it is needed here being the
  // measurement mic.
  // svelte-ignore state_referenced_locally
  let inputDevices = $state<DeviceOption[]>(initialInputs);
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

  const routeKey = (route: { deviceId: string; device: string }) => route.deviceId || route.device;

  const outputDevice = (route: StoredOutputRoute) =>
    outputDevices.find((d) => d.id === route.deviceId) ??
    outputDevices.find((d) => d.name === route.device);
  const inputDevice = (route: StoredInputRoute) =>
    inputDevices.find((d) => d.id === route.deviceId) ??
    inputDevices.find((d) => d.name === route.device);

  /// Picking a device sets both halves and forgets channels it may not have.
  const pickOutput = (route: StoredOutputRoute, id: string) => {
    const option = outputDevices.find((d) => d.id === id);
    if (id === "") {
      route.device = "";
      route.deviceId = "";
      route.channels = [];
      return;
    }
    if (!option) return;
    route.device = option.name;
    route.deviceId = option.id;
    if (route.channels.some((c) => c >= option.channels)) route.channels = [];
  };

  const pickInput = (route: StoredInputRoute, id: string) => {
    const option = inputDevices.find((d) => d.id === id);
    if (id === "") {
      route.device = "";
      route.deviceId = "";
      route.channel = 0;
      return;
    }
    if (!option) return;
    route.device = option.name;
    route.deviceId = option.id;
    if (route.channel >= option.channels) route.channel = 0;
  };

  let shoutbox = $derived(outputs.shoutbox);

  /// The tuning back to the defaults; see `shoutboxDefaults` for what stays.
  /// The trim goes the live way too, so the speaker follows at once.
  const resetShoutbox = () => {
    outputs.shoutbox = shoutboxDefaults(shoutbox);
    live(outputs.shoutbox.trimDb, outputs.shoutbox.muted);
  };
  let listen = $derived(outputs.listen);

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
    <h2>Listen</h2>
  </header>
  <p class="hint">
    Where a message is heard again when its ▶ is pressed. Headphones, typically, so a
    replay reaches you and nobody else. What is kept is only speech, and only in memory:
    clearing the chat clears it too.
    {#if isRemote}
      In this browser a replay plays here, in your own ears; this output is the host
      machine's.
    {/if}
  </p>

  <div class="row">
    <label>
      <span>Output</span>
      <select
        value={routeKey(listen.output)}
        disabled={isRemote}
        onchange={(e) => pickOutput(listen.output, e.currentTarget.value)}
      >
        <option value="">System output</option>
        {#each outputDevices as option (option.id)}
          <option value={option.id}>{option.name} ({option.channels} ch)</option>
        {/each}
        {#if listen.output.device !== "" && !outputDevice(listen.output)}
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
          {#each outputChoices(outputDevice(listen.output)?.channels ?? 2) as choice (choice.label)}
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

  {#if listen.keepMinutes === 0}
    <p class="note">Nothing is kept, so no message has a ▶.</p>
  {:else}
    <p class="note">
      Only speech counts, not time: silence is never kept, so on a quiet evening an old message can
      still be replayed. Once {listen.keepMinutes} minutes of talking have piled up, the oldest message
      goes.
    </p>
  {/if}
  {#if listError}
    <p class="failed">The audio devices could not be listed: {listError}</p>
  {/if}
</section>

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

  <p class="hint">
    One intercom channel passed through to a talkback speaker, its level following a
    measurement mic in the room: quiet in the quiet moments, above the room in the loud
    ones. The room is measured only while the shoutbox is silent — otherwise it would
    measure itself — so the level changes between sentences, never during one. Runs
    whenever it is switched on, with or without a transcription session.
  </p>

  <label class="switch">
    <input type="checkbox" bind:checked={shoutbox.enabled} />
    Shoutbox on
  </label>

  {#if status?.message}
    <p class="failed">{status.message}</p>
  {/if}

  <h3>Patch</h3>
  <div class="row">
    <label>
      <span>Program input</span>
      <select
        value={routeKey(shoutbox.program)}
        onchange={(e) => pickInput(shoutbox.program, e.currentTarget.value)}
      >
        <option value="">Not patched</option>
        {#each inputDevices as option (option.id)}
          <option value={option.id}>{option.name}</option>
        {/each}
        {#if shoutbox.program.device !== "" && !inputDevice(shoutbox.program)}
          <option value={routeKey(shoutbox.program)}>{shoutbox.program.device} — not connected</option>
        {/if}
      </select>
    </label>
    <label class="narrow">
      <span>Channel</span>
      <select bind:value={shoutbox.program.channel}>
        {#each Array.from({ length: inputDevice(shoutbox.program)?.channels ?? shoutbox.program.channel + 1 }, (_, c) => c) as c (c)}
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
        onchange={(e) => pickInput(shoutbox.reference, e.currentTarget.value)}
      >
        <option value="">Not patched</option>
        {#each inputDevices as option (option.id)}
          <option value={option.id}>{option.name}</option>
        {/each}
        {#if shoutbox.reference.device !== "" && !inputDevice(shoutbox.reference)}
          <option value={routeKey(shoutbox.reference)}>{shoutbox.reference.device} — not connected</option>
        {/if}
      </select>
    </label>
    <label class="narrow">
      <span>Channel</span>
      <select bind:value={shoutbox.reference.channel}>
        {#each Array.from({ length: inputDevice(shoutbox.reference)?.channels ?? shoutbox.reference.channel + 1 }, (_, c) => c) as c (c)}
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
        onchange={(e) => pickOutput(shoutbox.output, e.currentTarget.value)}
      >
        <option value="">Not patched</option>
        {#each outputDevices as option (option.id)}
          <option value={option.id}>{option.name}</option>
        {/each}
        {#if shoutbox.output.device !== "" && !outputDevice(shoutbox.output)}
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
        {#each outputChoices(outputDevice(shoutbox.output)?.channels ?? 2) as choice (choice.label)}
          <option value={encodeChannels(choice.channels)}>{choice.label}</option>
        {/each}
      </select>
    </label>
  </div>
  {#if clockProblem}
    <p class="failed">{clockProblem}</p>
  {:else}
    <p class="note">
      The output has to be on the program's interface — one clock for what is passed
      through. A macOS aggregate device counts as one. The mic may be anywhere.
    </p>
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
  <p class="note">
    The room is read in dB SPL, A-weighted. Without a calibration that is an estimate
    ({SHOUTBOX_ESTIMATED_CALIBRATION_DB} dB over weighted dBFS) and can be ten off either
    way. Calibrating moves the quiet threshold with it, so the shoutbox behaves as it
    did and only the numbers become right. Read the meter while nothing plays on the
    shoutbox.
  </p>

  <ShoutboxControls bind:shoutbox={outputs.shoutbox} {status} />

  <h3>Safety</h3>
  <div class="row">
    <label class="narrow">
      <span>Counts as playing above (dBFS)</span>
      <input type="number" min="-80" max="-10" bind:value={shoutbox.gateDbfs} />
    </label>
    <label class="narrow">
      <span>… and for (ms) after</span>
      <input type="number" min="200" max="10000" step="100" bind:value={shoutbox.gateHoldMs} />
    </label>
    <label class="narrow">
      <span>Never above (dBFS)</span>
      <input type="number" min="-20" max="0" bind:value={shoutbox.ceilingDbfs} />
    </label>
  </div>
  <p class="note">
    While the program is above the first number — and for the hold after it — the room is
    not measured and the gain does not move. The last is a limiter behind everything:
    nothing leaves above it, whatever the curve asked for.
  </p>
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
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.9rem;
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

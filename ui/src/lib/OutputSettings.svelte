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
  // In a browser on the LAN (P16) the routes and the curve are shown and not
  // editable: the speakers and headphones are the host machine's. Trim and mute
  // are, because somebody at the position is who needs them.

  import { onMount } from "svelte";
  import {
    isRemote,
    listDevices,
    listOutputDevices,
    onShoutboxStatus,
    setShoutboxLive,
    shoutboxStatus,
    SHOUTBOX_GAIN_DB_MAX,
    SHOUTBOX_GAIN_DB_MIN,
    SHOUTBOX_TRIM_DB_MAX,
    SHOUTBOX_TRIM_DB_MIN,
    type DeviceOption,
    type OutputDeviceOption,
    type ShoutboxStatus,
    type StoredInputRoute,
    type StoredOutputRoute,
    type StoredOutputs,
  } from "./bridge";
  import {
    calibrationFor,
    isShoutboxDefault,
    roundTenth,
    shoutboxDefaults,
    curveTarget,
    decodeChannels,
    encodeChannels,
    outputChoices,
    speed,
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

  let unit = $derived(shoutbox.calibrationDb != null ? "dB" : "dBFS");

  const fmt = (db: number | null | undefined, digits = 0) =>
    db === null || db === undefined || !Number.isFinite(db) ? "—" : db.toFixed(digits);

  // --- The curve, drawn ---------------------------------------------------

  // The plot sits inside margins that carry the axes: gain on the left, the
  // room along the bottom.
  const W = 340;
  const H = 190;
  const PAD = { left: 40, right: 10, top: 10, bottom: 36 };
  const plot = {
    left: PAD.left,
    right: W - PAD.right,
    top: PAD.top,
    bottom: H - PAD.bottom,
  };
  let span = $derived({
    from: shoutbox.quietDb - 20,
    to: shoutbox.quietDb + 50,
  });
  let gainSpan = $derived({
    from: Math.min(shoutbox.baseGainDb, shoutbox.maxGainDb) - 6,
    to: Math.max(shoutbox.baseGainDb, shoutbox.maxGainDb) + 6,
  });
  const x = (db: number) =>
    plot.left + ((db - span.from) / (span.to - span.from)) * (plot.right - plot.left);
  const y = (db: number) =>
    plot.bottom - ((db - gainSpan.from) / (gainSpan.to - gainSpan.from)) * (plot.bottom - plot.top);
  /// Whole multiples of `step` inside a span, for the tick labels.
  const ticks = (from: number, to: number, step: number) =>
    Array.from(
      { length: Math.floor(to / step) - Math.ceil(from / step) + 1 },
      (_, i) => (Math.ceil(from / step) + i) * step,
    );
  let roomTicks = $derived(ticks(span.from, span.to, 10));
  let gainTicks = $derived(
    ticks(gainSpan.from, gainSpan.to, gainSpan.to - gainSpan.from > 36 ? 12 : 6),
  );
  let path = $derived(
    Array.from({ length: 36 }, (_, i) => {
      const level = span.from + ((span.to - span.from) * i) / 35;
      return `${i === 0 ? "M" : "L"}${x(level).toFixed(1)},${y(curveTarget(shoutbox, level)).toFixed(1)}`;
    }).join(""),
  );
  let point = $derived(
    status?.state === "running" && status.levelDb != null
      ? {
          x: Math.min(plot.right, Math.max(plot.left, x(status.levelDb))),
          y: Math.min(plot.bottom, Math.max(plot.top, y(status.gainDb))),
        }
      : null,
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
    {#if !isRemote && !isShoutboxDefault(shoutbox)}
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
    <input type="checkbox" disabled={isRemote} bind:checked={shoutbox.enabled} />
    Shoutbox on
  </label>

  {#if status?.message}
    <p class="failed">{status.message}</p>
  {/if}

  <!-- The hand on top of the automation, reachable from the remote too. -->
  <div class="live">
    <button
      type="button"
      class="mute"
      class:on={shoutbox.muted}
      onclick={() => live(shoutbox.trimDb, !shoutbox.muted)}
      aria-pressed={shoutbox.muted}
    >
      {shoutbox.muted ? "Muted" : "Mute"}
    </button>
    <label class="trim">
      <span>Trim <output>{shoutbox.trimDb > 0 ? "+" : ""}{shoutbox.trimDb} dB</output></span>
      <input
        type="range"
        min={SHOUTBOX_TRIM_DB_MIN}
        max={SHOUTBOX_TRIM_DB_MAX}
        step="1"
        value={shoutbox.trimDb}
        oninput={(e) => live(Number(e.currentTarget.value), shoutbox.muted)}
      />
    </label>
  </div>

  {#if status?.state === "running"}
    <dl class="readings">
      <div>
        <dt>Room</dt>
        <dd>
          {fmt(status.liveDb)} {unit}
          {#if !status.measuring && status.levelDb != null}
            <span class="dim">· held at {fmt(status.levelDb)}</span>
          {/if}
        </dd>
      </div>
      <div>
        <dt>Gain</dt>
        <dd>{fmt(status.gainDb, 1)} dB <span class="dim">→ {fmt(status.targetDb, 1)}</span></dd>
      </div>
      <div>
        <dt>Program</dt>
        <dd>{fmt(status.programDbfs)} dBFS</dd>
      </div>
      <div>
        <dt>Measuring</dt>
        <dd>
          {#if !status.referenceOk}
            <span class="warn">Mic lost — gain held</span>
          {:else if status.measuring}
            Yes
          {:else}
            <span class="dim">Held while it plays</span>
          {/if}
        </dd>
      </div>
      <div>
        <dt>Limiter</dt>
        <dd class:warn={status.limitingDb < -0.5}>{fmt(status.limitingDb, 1)} dB</dd>
      </div>
      {#if status.underruns > 0 || status.skipped > 0}
        <div>
          <dt>Dropouts</dt>
          <dd class="warn">{status.underruns} / {status.skipped} skipped</dd>
        </div>
      {/if}
    </dl>
  {/if}

  <h3>Patch</h3>
  <div class="row">
    <label>
      <span>Program input</span>
      <select
        value={routeKey(shoutbox.program)}
        disabled={isRemote}
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
      <select bind:value={shoutbox.program.channel} disabled={isRemote}>
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
        disabled={isRemote}
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
      <select bind:value={shoutbox.reference.channel} disabled={isRemote}>
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
        disabled={isRemote}
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
        disabled={isRemote}
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
        disabled={isRemote}
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
        placeholder="none"
        disabled={isRemote}
        value={shoutbox.calibrationDb ?? ""}
        onchange={(e) =>
          (shoutbox.calibrationDb =
            e.currentTarget.value === "" ? null : roundTenth(Number(e.currentTarget.value)))}
      />
    </label>
  </div>
  {#if !isRemote}
    <div class="row calibrate">
      <label class="narrow">
        <span>An SPL meter at the mic reads (dB)</span>
        <input type="number" min="20" max="140" step="0.1" bind:value={meterReading} />
      </label>
      <button
        type="button"
        disabled={offset === null}
        onclick={() => {
          shoutbox.calibrationDb = offset;
          meterReading = CALIBRATOR_DB;
        }}
      >
        Calibrate{offset !== null ? ` (offset ${offset.toFixed(1)} dB)` : ""}
      </button>
    </div>
    <p class="note">
      With a calibration the levels below are in dB SPL, without it in weighted dBFS.
      Calibrating moves what the curve's numbers mean, so set it before the curve. Read
      the meter while nothing plays on the shoutbox.
    </p>
  {/if}

  <h3>Curve</h3>
  <div class="curve">
    <figure>
      <svg
        viewBox="0 0 {W} {H}"
        width={W}
        height={H}
        role="img"
        aria-label="Shoutbox gain over room level"
      >
        {#each gainTicks as g (g)}
          <line x1={plot.left} x2={plot.right} y1={y(g)} y2={y(g)} class="grid" class:zero={g === 0} />
          <text x={plot.left - 5} y={y(g)} class="tick" text-anchor="end" dominant-baseline="middle">
            {g > 0 ? `+${g}` : g}
          </text>
        {/each}
        {#each roomTicks as r (r)}
          <text x={x(r)} y={plot.bottom + 12} class="tick" text-anchor="middle">{r}</text>
        {/each}
        <line x1={plot.left} x2={plot.right} y1={plot.bottom} y2={plot.bottom} class="axis" />
        <line x1={plot.left} x2={plot.left} y1={plot.top} y2={plot.bottom} class="axis" />
        <line
          x1={x(shoutbox.quietDb)}
          x2={x(shoutbox.quietDb)}
          y1={plot.top}
          y2={plot.bottom}
          class="guide"
        />
        <text x={x(shoutbox.quietDb) + 4} y={plot.top + 9} class="tick">quiet</text>
        <path d={path} class="line" />
        {#if point}
          <circle cx={point.x} cy={point.y} r="4" class="point" />
        {/if}
        <text x={(plot.left + plot.right) / 2} y={H - 4} class="axis-title" text-anchor="middle">
          Room level ({unit})
        </text>
        <text
          transform="translate(11 {(plot.top + plot.bottom) / 2}) rotate(-90)"
          class="axis-title"
          text-anchor="middle"
        >
          Shoutbox gain (dB)
        </text>
      </svg>
      <figcaption>
        <span class="key"><span class="swatch line-key"></span>The gain the shoutbox gets at each room level</span>
        <span class="key"><span class="swatch point-key"></span>Where it is now</span>
      </figcaption>
    </figure>
    <div class="fields">
      <label>
        <span>Quiet below ({unit})</span>
        <input type="number" disabled={isRemote} bind:value={shoutbox.quietDb} />
      </label>
      <label>
        <span>Gain when quiet (dB)</span>
        <input
          type="number"
          min={SHOUTBOX_GAIN_DB_MIN}
          max={SHOUTBOX_GAIN_DB_MAX}
          disabled={isRemote}
          bind:value={shoutbox.baseGainDb}
        />
      </label>
      <label>
        <span>dB of gain per dB of room (%)</span>
        <input type="number" min="0" max="200" step="5" disabled={isRemote} bind:value={shoutbox.slopePercent} />
      </label>
      <label>
        <span>Most gain (dB)</span>
        <input
          type="number"
          min={SHOUTBOX_GAIN_DB_MIN}
          max={SHOUTBOX_GAIN_DB_MAX}
          disabled={isRemote}
          bind:value={shoutbox.maxGainDb}
        />
      </label>
    </div>
  </div>

  <h3>Speed and safety</h3>
  <div class="row">
    <label class="narrow">
      <span>Rise — {speed(shoutbox.riseMs)}</span>
      <input
        type="range"
        min="500"
        max="30000"
        step="500"
        disabled={isRemote}
        bind:value={shoutbox.riseMs}
      />
    </label>
    <label class="narrow">
      <span>Fall — {speed(shoutbox.fallMs)}</span>
      <input
        type="range"
        min="2000"
        max="120000"
        step="1000"
        disabled={isRemote}
        bind:value={shoutbox.fallMs}
      />
    </label>
  </div>
  <p class="note">
    Rising faster than falling: a loud song is followed within seconds, and a short break
    in it does not drop the shoutbox under the next chorus.
  </p>
  <div class="row">
    <label class="narrow">
      <span>Counts as playing above (dBFS)</span>
      <input type="number" min="-80" max="-10" disabled={isRemote} bind:value={shoutbox.gateDbfs} />
    </label>
    <label class="narrow">
      <span>… and for (ms) after</span>
      <input type="number" min="200" max="10000" step="100" disabled={isRemote} bind:value={shoutbox.gateHoldMs} />
    </label>
    <label class="narrow">
      <span>Never above (dBFS)</span>
      <input type="number" min="-20" max="0" disabled={isRemote} bind:value={shoutbox.ceilingDbfs} />
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

  input[type="range"],
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

  .live {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    margin: 0.6rem 0;
  }

  .mute {
    min-width: 5rem;
    font-weight: 600;
  }

  /* Lit when muted, in the colour a desk's mute is lit in. */
  .mute.on {
    background: #c0392b;
    border-color: #c0392b;
    color: #fff;
  }

  .trim {
    max-width: 20rem;
  }

  output {
    font-variant-numeric: tabular-nums;
  }

  .readings {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(9rem, 1fr));
    gap: 0.4rem 0.8rem;
    margin: 0.4rem 0 0;
  }

  .readings dt {
    font-size: 0.7rem;
    color: var(--text-dim);
  }

  .readings dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }

  .dim {
    color: var(--text-dim);
  }

  .warn {
    color: #f2b544;
  }

  .curve {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    align-items: flex-start;
  }

  .curve svg {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    max-width: 100%;
    height: auto;
  }

  .curve .fields {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    flex: 1 1 12rem;
  }

  /* The shared label's flex basis is a width in a row; in this column it
     would be a height, and every field 14rem tall. */
  .curve .fields label {
    flex: none;
  }

  .line {
    fill: none;
    stroke: var(--accent);
    stroke-width: 2;
  }

  .guide {
    stroke: var(--border);
    stroke-dasharray: 3 3;
  }

  .point {
    fill: #f2b544;
  }

  .curve figure {
    margin: 0;
    max-width: 100%;
  }

  .grid {
    stroke: var(--border);
    stroke-opacity: 0.5;
  }

  .grid.zero {
    stroke-opacity: 1;
  }

  .axis {
    stroke: var(--text-dim);
  }

  .tick {
    fill: var(--text-dim);
    font-size: 9px;
  }

  .axis-title {
    fill: var(--text-dim);
    font-size: 10px;
  }

  .curve figcaption {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem 1rem;
    margin-top: 0.3rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .key {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }

  .swatch {
    display: inline-block;
  }

  .line-key {
    width: 14px;
    height: 2px;
    background: var(--accent);
  }

  .point-key {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #f2b544;
  }

  .calibrate button {
    margin-bottom: 0.05rem;
  }
</style>

<script module lang="ts">
  /// The steps, in the order they are walked.
  ///
  /// Six, and they are in the order the machine needs them rather than the
  /// order the Settings page lists them in: nothing can be transcribed without
  /// a model, no channel can be patched before an interface is chosen, and a
  /// group is a view onto channels that have to exist first. Where audio
  /// leaves the app (P22, P26) comes last of the tasks: it is optional, and
  /// the shoutbox's program is one of the channels just patched. The welcome
  /// says what is about to be downloaded, and the last step says which button
  /// to press.
  export type Step = "welcome" | "models" | "audio" | "groups" | "outputs" | "ready";

  const STEPS: Step[] = ["welcome", "models", "audio", "groups", "outputs", "ready"];
</script>

<script lang="ts">
  // The first-run setup guide.
  //
  // What this replaces: a fresh install that comes up on an empty chat, with a
  // Start button that fails on "no speech model installed — download one on
  // the Settings page". That sentence is correct and it is also the whole of
  // the onboarding — it asks somebody who has just opened the app for the
  // first time to find a section they have no reason to know exists, and to
  // pick between three models by name. The Settings page is where a setup is
  // *changed*; this is where one is *made*.
  //
  // Four things it is careful about:
  //
  // - **The speech detection model is not a question.** There is one entry in
  //   the catalogue, it is 0.9 MB, and the app is materially worse without it
  //   — whisper captions room noise and silence as sentences nobody said. So
  //   it is fetched the moment the model step opens, and reported rather than
  //   offered. A choice with one sensible answer is not a choice, it is a step
  //   somebody can get wrong.
  // - **The speech model is a question, with an answer already filled in.**
  //   Three sizes, the recommended one selected, one line each on when to pick
  //   the other two. Nobody has to know what a quantised tensor is to get past
  //   this page.
  // - **The channel, group and output steps are the Settings page's own sections**,
  //   not simplified copies of them. `StreamSetup` and `GroupMembership` are
  //   mounted here as they are, editing the same profile through the same
  //   bindings and saved by the same debounce. A second, gentler channel
  //   editor would be a second place for the channel map to be wrong, and it
  //   would teach the operator a page that does not exist once the guide is
  //   closed — the point of walking these is that the operator has now seen
  //   where they live.
  // - **It can be left.** A machine that will never see the internet has a way
  //   in — a `.bin` carried in on a stick, pointed at in Settings — and a
  //   guide that cannot be dismissed would be standing between that operator
  //   and the page they need. Leaving it marks it done; it is on the Settings
  //   page for anybody who wants it back.

  import { onMount, untrack } from "svelte";
  import StreamSetup from "./StreamSetup.svelte";
  import { isMac, shortcut } from "./shortcut";
  import GroupMembership from "./GroupMembership.svelte";
  import OutputSettings from "./OutputSettings.svelte";
  import {
    cancelDownload,
    downloadModel,
    listLanguages,
    listModels,
    onModelProgress,
    type DeviceOption,
    type Language,
    type ModelProgress,
    type ModelStatus,
    type ModelsView,
    type StreamLevel,
  } from "./bridge";
  import type { ModelChoice, Profile } from "./profiles";

  interface Props {
    /// The interfaces this machine has, for the audio step. The same list the
    /// Settings page shows, and picked the same way — by stable id, because
    /// two identical interfaces are two entries.
    devices: DeviceOption[];
    /// How many inputs the selected device has. What the channel rows are
    /// checked against, and the one thing about the interface the operator
    /// cannot read off its name.
    deviceChannels: number;
    pickDevice: (id: string) => void;
    /// The setup being built. Mutated in place, exactly as the Settings page
    /// mutates it: it is an element of the app's `profiles` array, so writing
    /// to its fields is what makes the edit part of what gets saved.
    profile: Profile | null;
    /// Which model the machine transcribes with (P11). Written here as well as
    /// on the Settings page: the guide is the first place the question is
    /// asked, and the answer has to be the same one.
    models: ModelChoice;
    /// The levels from the monitor (P8), so the channel rows have a moving bar
    /// beside their trim while the map is being built. This is the moment the
    /// bars matter most — it is how "channel 3" is confirmed to be the channel
    /// somebody is talking on.
    levels?: Map<number, StreamLevel> | null;
    /// Called when the guide is finished or left. Either way it is done — a
    /// guide that reappears on every start is a guide that gets clicked
    /// through without being read.
    ondone: () => void;
  }

  let {
    devices,
    deviceChannels,
    pickDevice,
    profile,
    models = $bindable(),
    levels = null,
    ondone,
  }: Props = $props();

  let step = $state<Step>("welcome");
  let index = $derived(STEPS.indexOf(step));

  let view = $state<ModelsView | null>(null);
  let error = $state<string | null>(null);
  let progress = $state(new Map<string, ModelProgress>());

  /// The sidecar's language list, for the channel rows. Fetched here rather
  /// than handed down: the Settings page fetches its own and the guide is not
  /// mounted inside it. `null` stays `null` if the call fails — the picker
  /// renders that state, because a missing list costs discoverability and not
  /// the ability to set a language.
  let languages = $state<Language[] | null>(null);

  onMount(async () => {
    try {
      languages = await listLanguages();
    } catch (e) {
      console.warn("no language list from the sidecar:", e);
    }
  });

  const refresh = async () => {
    try {
      // `untrack` for the reason the Settings page untracks the same call: the
      // selection is read to ask the question, not subscribed to, and a view
      // per keystroke is a page arguing with the person typing into it.
      view = await listModels(
        untrack(() => ({
          model: models.model ?? null,
          vadModel: models.vadModel ?? null,
          modelPath: models.modelPath ?? null,
        })),
      );
      error = null;
    } catch (e) {
      // The platform would not say where the app may keep data. Nothing can be
      // downloaded at all, so the guide has nothing left to offer and says so
      // rather than showing buttons that cannot work.
      error = String(e);
    }
  };

  $effect(() => {
    refresh();
  });

  $effect(() => {
    const subscription = onModelProgress((event) => {
      progress = new Map(progress).set(event.id, event);
      if (event.stage === "done" || event.stage === "failed") refresh();
    });
    return () => {
      subscription.then((unlisten) => unlisten()).catch(() => {});
    };
  });

  const speech = $derived(view?.models.filter((m) => m.kind === "speech") ?? []);
  const vad = $derived(view?.models.find((m) => m.kind === "vad") ?? null);
  const recommended = $derived(speech.find((m) => m.recommended) ?? speech[0] ?? null);
  /// Which speech model the radios are on. Falls back to the recommended one
  /// rather than showing nothing selected: "never chose" is a real state that
  /// resolves to exactly that model, and an empty radio group would be asking
  /// a question the app has already answered sensibly.
  let chosen = $derived(models.model ?? recommended?.id ?? null);
  let chosenModel = $derived(speech.find((m) => m.id === chosen) ?? null);

  /// Whether the app could transcribe right now. Not "the chosen model is
  /// installed": `FLUESTR_MODEL`, a model left in `.scratch/models` and the
  /// custom path all count, and the backend is the one that knows.
  let hasSpeech = $derived(view !== null && view.problem === null);
  let hasVad = $derived(view?.vad === true);

  const stateOf = (model: ModelStatus | null) => (model ? progress.get(model.id) : undefined);
  const busy = (model: ModelStatus | null) => {
    const state = stateOf(model);
    return state?.stage === "downloading" || state?.stage === "verifying";
  };

  const megabytes = (bytes: number) => `${Math.round(bytes / 1_000_000)} MB`;

  const download = async (id: string) => {
    // Optimistic, so the row reacts to the click and not to the first chunk —
    // which on a venue's network is seconds away.
    progress = new Map(progress).set(id, {
      id,
      received: 0,
      total: 0,
      stage: "downloading",
      message: null,
    });
    try {
      await downloadModel(id);
    } catch (e) {
      progress = new Map(progress).set(id, {
        id,
        received: 0,
        total: 0,
        stage: "failed",
        message: String(e),
      });
    }
  };

  /// The VAD downloads the guide has already started, so a failure is not
  /// retried forever.
  ///
  /// A failed download refreshes the view, the refreshed view re-runs the
  /// effect below, and without this the effect would start the same doomed
  /// request again — on a machine with no network, for as long as the guide is
  /// open. Deliberately not `$state`: it must not itself cause a re-run.
  const attempted = new Set<string>();

  // Speech detection is fetched, not offered. It is 0.9 MB against the speech
  // model's 190, there is one entry in the catalogue, and the app without it
  // captions the silence between transmissions as plausible sentences — which
  // reads, to somebody who has had the app for ten minutes, as the app making
  // things up. So the model step starts it and reports it; the only decision
  // left is what to do when it fails, which is the Retry button below.
  $effect(() => {
    if (step !== "models" || vad === null || vad.installed) return;
    const id = vad.id;
    untrack(() => {
      if (attempted.has(id)) return;
      attempted.add(id);
      download(id);
    });
  });

  const select = (model: ModelStatus) => {
    models.model = model.id;
    // What a session would start with has changed, so the "in use" answer has
    // to be asked again — with the choice just made, which is why `refresh`
    // sends it rather than letting the backend read a file that is still a
    // debounce behind.
    refresh();
  };

  const back = () => {
    if (index > 0) step = STEPS[index - 1];
  };

  const next = () => {
    if (index < STEPS.length - 1) step = STEPS[index + 1];
    else ondone();
  };

  /// Whether the step the operator is on lets them move on.
  ///
  /// Only the model step ever says no, and only while the speech model is
  /// still arriving: a half-downloaded model is not a model, and "Next" on top
  /// of a progress bar invites somebody to walk away from the one thing this
  /// guide is for. Not downloaded at all is allowed through — that is the
  /// offline way in, and it is a link, not a wall.
  let waiting = $derived(step === "models" && busy(chosenModel));

  /// The interface this profile is on, or `null` for one that is not plugged
  /// in. The select below has to offer that as an option of its own — dropping
  /// it would silently re-point a setup carried in from another house at
  /// whatever is connected here.
  let missing = $derived(
    profile !== null &&
      profile.device !== "" &&
      !devices.some((d) => d.id === profile.deviceId),
  );
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === "Escape") ondone();
  }}
/>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="scrim" onclick={ondone}></div>

<div class="guide" role="dialog" aria-modal="true" aria-labelledby="setup-title">
  <header>
    <div class="dots" aria-hidden="true">
      {#each STEPS as s, i (s)}
        <span class="dot" class:done={i < index} class:here={i === index}></span>
      {/each}
    </div>
    <span class="count">Step {index + 1} of {STEPS.length}</span>
    <!-- Always reachable, on every step. The guide is a help, and a help that
         cannot be closed is an obstacle. -->
    <button class="quiet" onclick={ondone}>Skip setup</button>
  </header>

  <div class="body">
    {#if step === "welcome"}
      <!-- A welcome and an invitation, and nothing else. What the next steps
           will ask for is written on the next steps; listing it here only
           gives somebody four things to decide about before they have been
           shown any of them. -->
      <div class="done">
        <!-- The app's own mark, drawn rather than loaded: the same three
             stacked bars as the icon in the Dock — chat lines and a level
             meter at once, and, with the bottom one cut to a stub, the F of
             Fluestr. The geometry is `docs/generate-icon.py`'s, in a 100-unit
             tile, so the two cannot drift apart by eye. Drawn because the
             icon is a set of PNGs the frontend does not ship, and because
             this way it takes its colours from the theme. -->
        <svg class="logo" viewBox="0 0 100 100" width="96" height="96" aria-hidden="true">
          <rect x="0" y="0" width="100" height="100" rx="23" fill="var(--surface)" />
          <rect x="22" y="28" width="56" height="10" rx="5" fill="var(--accent)" />
          <rect x="22" y="45" width="40" height="10" rx="5" fill="var(--text-dim)" />
          <rect x="22" y="62" width="12" height="10" rx="5" fill="var(--accent)" />
        </svg>

        <h2 id="setup-title">Welcome to Fluestr</h2>
        <p>
          Fluestr writes down what is said on your intercom channels. Everything stays on
          this machine.
        </p>
        <p class="note">A few minutes, and this guide walks you through it.</p>
      </div>
    {:else if step === "models"}
      <h2 id="setup-title">Get the models</h2>

      {#if error}
        <p class="error">{error}</p>
        <p class="note">
          Without a place to keep them nothing can be downloaded here. A model file
          carried in by hand still works — Settings › Models › Custom model.
        </p>
      {:else if view === null}
        <p class="note">Reading the model directory …</p>
      {:else}
        <!-- The speech model first: it is the decision, and it is the download
             that takes the time. Speech detection is reported underneath
             because by the time it is read it is usually already there. -->
        <h3>Speech model</h3>
        <ul>
          {#each speech as model (model.id)}
            {@const state = progress.get(model.id)}
            <li class:selected={chosen === model.id}>
              <div class="row">
                <label class="choice">
                  <input
                    type="radio"
                    name="setup-speech"
                    checked={chosen === model.id}
                    onchange={() => select(model)}
                  />
                  <span class="name">
                    {model.label}
                    {#if model.recommended}<span class="tag">recommended</span>{/if}
                  </span>
                </label>

                <span class="size">{megabytes(model.bytes)}</span>

                {#if model.installed}
                  <span class="ready">Installed</span>
                {:else if busy(model)}
                  <button onclick={() => cancelDownload(model.id)}>Cancel</button>
                {:else}
                  <button class="primary" onclick={() => download(model.id)}>
                    <!-- Honest about what pressing it does: what arrived last
                         time is still there and is continued. -->
                    {model.partial > 0 ? `Resume (${megabytes(model.partial)})` : "Download"}
                  </button>
                {/if}
              </div>

              <p class="note">{model.note}</p>

              {#if busy(model)}
                <div class="bar">
                  <div
                    class="fill"
                    style:width="{state && state.total > 0
                      ? Math.min(100, (state.received / state.total) * 100)
                      : 0}%"
                  ></div>
                </div>
                <p class="note">
                  {#if state?.stage === "verifying"}
                    Checking the download against its published checksum …
                  {:else}
                    {megabytes(state?.received ?? 0)} of {megabytes(state?.total || model.bytes)}
                  {/if}
                </p>
              {:else if state?.stage === "failed"}
                <p class="error">{state.message}</p>
              {:else if state?.stage === "cancelled"}
                <p class="note">Stopped. What arrived is kept — press Resume to continue.</p>
              {/if}
            </li>
          {/each}
        </ul>

        <h3>Speech detection</h3>
        <!-- No radio and no choice: one entry, 0.9 MB, and the app is much
             worse without it. What this block is for is saying what is
             happening to the network connection and what to do if it fails. -->
        {#if vad === null}
          <p class="note">This build has no speech-detection model to offer.</p>
        {:else if hasVad}
          <p class="ok-line"><span class="ok">●</span> Installed.</p>
        {:else}
          {@const state = stateOf(vad)}
          {#if busy(vad)}
            <p class="note">Downloading {vad.label} ({megabytes(vad.bytes)}) …</p>
            <div class="bar">
              <div
                class="fill"
                style:width="{state && state.total > 0
                  ? Math.min(100, (state.received / state.total) * 100)
                  : 0}%"
              ></div>
            </div>
          {:else}
            <p class="warn">
              {state?.message ?? "Not installed yet."} Without it, silence and room noise
              are transcribed too.
            </p>
            <button onclick={() => download(vad.id)}>Retry</button>
          {/if}
        {/if}

        {#if !hasSpeech}
          <p class="note offline">
            No connection? Skip this and point the app at a <code>.bin</code> of your own
            under Settings › Models.
          </p>
        {/if}
      {/if}
    {:else if step === "audio"}
      <h2 id="setup-title">Audio input</h2>

      {#if profile === null}
        <p class="warn">No setup to configure yet.</p>
      {:else}
        <p>One multichannel interface carries every stream.</p>

        {#if devices.length === 0}
          <p class="warn">
            {#if isMac()}
              No input device found. Plug the interface in and allow microphone access
              when macOS asks.
            {:else}
              No input device found. Plug the interface in, and check that Settings ›
              Privacy &amp; security › Microphone lets desktop apps use it.
            {/if}
          </p>
        {:else}
          <label class="field">
            <span>Device</span>
            <!-- Keyed and valued by the stable id, like the Settings page's
                 own picker: two identical interfaces are two entries here and
                 must not collapse into one. -->
            <select
              value={profile.deviceId}
              onchange={(e) => pickDevice(e.currentTarget.value)}
            >
              {#each devices as option (option.id)}
                <option value={option.id}>{option.name} ({option.channels} ch)</option>
              {/each}
              {#if missing}
                <option value={profile.deviceId}>{profile.device} — not connected</option>
              {/if}
            </select>
          </label>
        {/if}

        <p class="note">
          One row per channel below: the input it is patched to, who is on it, the language
          they speak. Talk on a line and watch its bar move.
        </p>

        <!-- The Settings page's own section, mounted as it is. A gentler copy
             of it would be a second place the channel map can be wrong, and it
             would teach a page that stops existing when the guide closes. -->
        <div class="embedded">
          <StreamSetup
            bind:channels={profile.channels}
            bind:language={profile.language}
            {languages}
            {deviceChannels}
            locked={false}
            live={null}
            {levels}
          />
        </div>
      {/if}
    {:else if step === "groups"}
      <h2 id="setup-title">Groups</h2>

      {#if profile === null}
        <p class="warn">No setup to configure yet.</p>
      {:else}
        <p>
          A group is a view onto some of your channels — "Stage", "Lighting" — switched in
          the chat's sidebar, or opened in a window of its own.
        </p>
        <p class="note">Optional. Without one the chat shows every channel.</p>

        <div class="embedded">
          <GroupMembership bind:groups={profile.groups} channels={profile.channels} />
        </div>
      {/if}
    {:else if step === "outputs"}
      <h2 id="setup-title">Replay and shoutbox</h2>

      {#if profile === null}
        <p class="warn">No setup to configure yet.</p>
      {:else}
        <p>
          <strong>Listen</strong> is where a message is played again when you press its
          replay button — headphones, so only you hear it.
          <strong>Shoutbox</strong> passes one intercom channel through to a talkback
          speaker, louder when the room gets loud, measured by a mic in the room.
        </p>
        <p class="note">
          Optional. Without a listen output replays use the system output; the shoutbox
          stays off until you switch it on. The speaker has to be on the same interface as
          the program channel.
        </p>

        <div class="embedded">
          <OutputSettings bind:outputs={profile.outputs} inputDevices={devices} />
        </div>
      {/if}
    {:else}
      <!-- The one step that is a result rather than a task, so it is laid out
           as one: the mark first, at a size that reads across a desk, and
           three lines under it for what to do next. -->
      <div class="done">
        {#if hasSpeech}
          <svg class="mark" viewBox="0 0 48 48" width="48" height="48" aria-hidden="true">
            <circle cx="24" cy="24" r="21" fill="none" stroke="currentColor" stroke-width="3" />
            <path
              d="M15 24.5 L21.5 31 L33 19"
              fill="none"
              stroke="currentColor"
              stroke-width="3.5"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
          <h2 id="setup-title">You are all set</h2>
          {#if !hasVad}
            <p class="warn">Speech detection is still missing — silence is transcribed too.</p>
          {/if}
        {:else}
          <h2 id="setup-title">Almost there</h2>
          <p class="warn">No speech model yet — Start will refuse. Settings › Models.</p>
        {/if}
      </div>

      <ul class="plain">
        <li><strong>Start</strong> — in the header, or {shortcut("T")}.</li>
        <li><strong>Meters</strong> — above the chat, live before you start.</li>
        <li><strong>Settings</strong> — {shortcut(",")} for vocabulary, thresholds and rules.</li>
      </ul>
    {/if}
  </div>

  <footer>
    <button onclick={back} disabled={index === 0}>Back</button>
    <button class="primary" onclick={next} disabled={waiting}>
      {#if step === "ready"}
        Finish
      {:else if waiting}
        Downloading …
      {:else if step === "welcome"}
        Get started
      {:else}
        Next
      {/if}
    </button>
  </footer>
</div>

<style>
  /* Over everything, including the header: until there is a model there is
     nothing behind this worth reaching, and a guide that can be clicked past
     by accident is a guide half the machines never see. */
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 40;
  }

  .guide {
    position: fixed;
    z-index: 41;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    display: flex;
    flex-direction: column;
    /* Wide enough for the channel rows, which are the Settings page's own and
       carry a meter, a trim knob and a language picker per line. */
    width: min(52rem, calc(100vw - 2rem));
    /* One size for every step, and deliberately `height` rather than
       `max-height`: a dialog that shrinks to its shortest step and grows to
       its tallest moves Back and Next out from under the cursor on every
       press, and re-centres itself on the screen while it does. Sized for the
       tallest step — the channel rows — so the short ones carry some empty
       space, which is the cheaper of the two. Capped by the viewport, because
       on a laptop screen with the dock up a fixed height would take the
       footer, and with it Next, off the bottom of the window. */
    height: min(46rem, calc(100dvh - 2rem));
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    box-shadow: 0 1.5rem 3rem rgba(0, 0, 0, 0.45);
  }

  header {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.7rem 1rem;
    border-bottom: 1px solid var(--border);
  }

  .dots {
    display: flex;
    gap: 0.3rem;
  }

  .dot {
    width: 0.45rem;
    height: 0.45rem;
    border-radius: 50%;
    background: var(--border);
  }

  .dot.done {
    background: var(--text-dim);
  }

  .dot.here {
    background: var(--accent);
  }

  .count {
    font-size: 0.75rem;
    color: var(--text-dim);
  }

  header .quiet {
    margin-left: auto;
    border-color: transparent;
    color: var(--text-dim);
  }

  /* Takes whatever the header and footer leave and scrolls inside it, so the
     two of them stay exactly where they are on every step. `min-height: 0` is
     what lets it: without it a flex item refuses to shrink below its content
     and the scrollbar moves to the dialog, which is the shift again. */
  .body {
    flex: 1;
    min-height: 0;
    padding: 1rem;
    overflow-y: auto;
  }

  h2 {
    margin: 0 0 0.6rem;
    font-size: 1.15rem;
  }

  h3 {
    margin: 1rem 0 0.3rem;
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }

  p {
    margin: 0 0 0.6rem;
    font-size: 0.9rem;
    line-height: 1.5;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  ul.plain li {
    font-size: 0.9rem;
    line-height: 1.5;
    padding-left: 0.9rem;
    position: relative;
  }

  ul.plain li::before {
    content: "·";
    position: absolute;
    left: 0.2rem;
    color: var(--text-dim);
  }

  ul:not(.plain) li {
    border: 1px solid var(--border);
    border-radius: 0.35rem;
    padding: 0.5rem 0.6rem;
  }

  li.selected {
    border-color: var(--accent);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }

  .choice {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    flex: 1;
    min-width: 0;
    cursor: pointer;
  }

  .name {
    font-size: 0.9rem;
  }

  .tag {
    margin-left: 0.4rem;
    font-size: 0.65rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 0.2rem;
    padding: 0.05rem 0.3rem;
  }

  .size {
    font-size: 0.75rem;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .ready {
    font-size: 0.75rem;
    color: #5fd38d;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    max-width: 28rem;
  }

  .field span {
    font-size: 0.7rem;
    color: var(--text-dim);
  }

  /* The Settings sections are cards on the page background; inside the dialog
     there is no page behind them, so they sit flush and keep only their own
     frame. */
  .embedded {
    margin-top: 0.8rem;
  }

  select {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.85rem;
  }

  button {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.8rem;
    font: inherit;
    font-size: 0.8rem;
    cursor: pointer;
  }

  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #0b0b0c;
    font-weight: 600;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .bar {
    height: 0.3rem;
    margin-top: 0.5rem;
    background: var(--bg);
    border-radius: 0.15rem;
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s linear;
  }

  .note {
    margin: 0.35rem 0 0;
    font-size: 0.78rem;
    color: var(--text-dim);
  }

  .note.offline {
    margin-top: 0.9rem;
  }

  .warn {
    margin: 0.5rem 0 0;
    font-size: 0.82rem;
    color: #e0b341;
  }

  .error {
    margin: 0.35rem 0 0;
    font-size: 0.8rem;
    color: #ff6b6b;
  }

  /* The two steps that are not a form — the welcome and the result. Centred,
     and the only places in the dialog that are. */
  .done {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 1.2rem;
    text-align: center;
  }

  .done h2 {
    margin: 0;
  }

  .done p {
    margin: 0;
    max-width: 26rem;
  }

  .mark {
    color: #5fd38d;
  }

  /* Room under it, because the heading is the next thing read and the two
     should not touch. */
  .logo {
    margin-bottom: 0.4rem;
  }

  .ok-line {
    font-size: 0.85rem;
  }

  .ok {
    color: #5fd38d;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    padding: 0.7rem 1rem;
    border-top: 1px solid var(--border);
  }
</style>

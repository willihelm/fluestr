<script lang="ts">
  // The model manager (P11).
  //
  // What this replaces: a terminal, a `just model` invocation and a URL. An
  // audio engineer who installs the app should reach a first transcription
  // without any of those, and the previous behaviour — "no model found", with
  // no way forward inside the window — is the one thing between this app and
  // somebody who is not holding a terminal.
  //
  // Three things this page is careful about, all of them learned from what the
  // old state did badly:
  //
  // - **The "no model" state is loud and it says what to do.** It was silent
  //   before: a session that started and transcribed nothing looked like a
  //   broken microphone.
  // - **A download can be interrupted.** 600 MB on a venue's network is not a
  //   reliable proposition, so what arrived is kept and the button says
  //   "Resume" rather than pretending the attempt never happened.
  // - **There is a way out without a network at all.** The custom model takes a
  //   `.bin` carried in on a stick. It sits in the list as one more choice
  //   rather than as an override, so what is selected is what runs.

  import {
    CUSTOM_MODEL,
    DEFAULT_WORKERS,
    MAX_WORKERS,
    cancelDownload,
    downloadModel,
    listModels,
    onModelProgress,
    canPickModelFile,
    pickModelFile,
    removeModel,
    type ModelProgress,
    type ModelsView,
    type ModelStatus,
    type SpeechEngine,
  } from "./bridge";
  import { untrack } from "svelte";
  import type { ModelChoice } from "./profiles";
  import type { Flag } from "./SettingsPage.svelte";

  interface Props {
    models: ModelChoice;
    /// While a session runs the sidecar holds the model file open, so
    /// switching or deleting one is locked for the same reason the channel map
    /// is: it would take effect at a moment nobody chose.
    locked: boolean;
    /// Told whenever the answer to "can this machine transcribe" changes, so
    /// the Settings rail can mark this section from anywhere on the page. It
    /// is the one problem here that nothing outside this component can see:
    /// it is a question about files on disk, not about the setup.
    onstatus?: (flag: Flag) => void;
    /// Told which recogniser the active model runs on, so the vocabulary
    /// page can say when what is typed there is ignored.
    onengine?: (engine: SpeechEngine | null) => void;
  }

  let { models = $bindable(), locked, onstatus, onengine }: Props = $props();

  let view = $state<ModelsView | null>(null);
  let error = $state<string | null>(null);
  /// Downloads in flight, by model id. Keyed rather than a single value
  /// because the speech model and the VAD are two downloads an operator may
  /// reasonably start together.
  let progress = $state(new Map<string, ModelProgress>());

  const refresh = async () => {
    try {
      // `untrack`, so that this reads the selection without subscribing to it:
      // the path field updates on every keystroke, and a view per keystroke
      // would spend half of a typed path telling the operator it is not a file.
      // The explicit calls below are what says when the answer can change.
      view = await listModels(
        untrack(() => ({
          model: models.model ?? null,
          vadModel: models.vadModel ?? null,
          modelPath: models.modelPath ?? null,
        })),
      );
      error = null;
    } catch (e) {
      // The one failure that is not about a particular model: the platform
      // would not say where the app may keep data, so nothing can be
      // downloaded at all and the custom model is the only way through.
      error = String(e);
    }
  };

  $effect(() => {
    refresh();
  });

  $effect(() => {
    const subscription = onModelProgress((event) => {
      progress = new Map(progress).set(event.id, event);
      // A finished download changes what is installed and therefore what a
      // session would start with — including, for the first model on a fresh
      // machine, the difference between "no model" and a usable app.
      if (event.stage === "done" || event.stage === "failed") refresh();
    });
    return () => {
      subscription.then((unlisten) => unlisten()).catch(() => {});
    };
  });

  // No model at all is an error — the app cannot transcribe a word. No speech
  // detection is a warning: it works, but whisper invents sentences for the
  // silence between transmissions, which reads as the app making things up.
  $effect(() => {
    if (view === null) return;
    onstatus?.(view.problem ? "error" : view.vad ? null : "warn");
    onengine?.(view.engine);
  });

  const speech = $derived(view?.models.filter((m) => m.kind === "speech") ?? []);
  const vad = $derived(view?.models.filter((m) => m.kind === "vad") ?? []);
  const custom = $derived((models.model ?? null) === CUSTOM_MODEL);

  const megabytes = (bytes: number) => `${Math.round(bytes / 1_000_000)} MB`;

  const download = async (id: string) => {
    // Optimistic, so the row reacts to the click rather than to the first
    // chunk — which on a slow connection is seconds away.
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

  const remove = async (id: string) => {
    try {
      await removeModel(id);
      // Deleting the model that was selected leaves the selection pointing at
      // nothing, which resolves as "not downloaded yet" — correct, but the
      // operator did not choose it. Falling back to "never chose" lands on
      // whatever is still installed instead.
      if (models.model === id) models.model = null;
      if (models.vadModel === id) models.vadModel = null;
    } catch (e) {
      error = String(e);
    }
    await refresh();
  };

  const select = (model: ModelStatus) => {
    if (model.kind === "speech") models.model = model.id;
    else models.vadModel = model.id;
    // What runs changed, so the "in use" line has to be asked again — with the
    // choice just made, which is why it is sent rather than read from the file.
    refresh();
  };

  const selectCustom = () => {
    models.model = CUSTOM_MODEL;
    refresh();
  };

  /// The system file dialog, opened on whatever the field already holds — a
  /// second try after the wrong `.bin` starts where the first one ended.
  /// Typing an absolute path from memory is the kind of thing that works on
  /// the machine it was written on, and choosing a file is what the operator
  /// is actually doing; the field stays editable for a path sent in an email.
  const browse = async () => {
    try {
      const chosen = await pickModelFile(models.modelPath ?? null);
      // A cancelled dialog leaves everything as it was, including a path that
      // is already there: picking nothing is not the same as clearing it.
      if (chosen === null) return;
      models.modelPath = chosen;
      // Choosing a file is choosing the custom model. Leaving the radio where
      // it was would put a path in the box that changes nothing.
      models.model = CUSTOM_MODEL;
      await refresh();
    } catch (e) {
      error = String(e);
    }
  };

  const active = (model: ModelStatus) =>
    model.kind === "speech"
      ? (models.model ?? null) === model.id
      : (models.vadModel ?? null) === model.id;
</script>

<section class="panel">
  <header>
    <h2>Transcription model</h2>
  </header>

  {#if error}
    <p class="error">{error}</p>
  {/if}

  {#if view}
    <!-- The state line first, because on a fresh machine it is the only thing
         on this page that matters: it says whether the app can transcribe at
         all, and the catalogue below is how that is fixed. -->
    {#if view.problem}
      <p class="problem">
        <strong>No model.</strong>
        {view.problem}
      </p>
    {:else}
      <p class="inuse">
        <span class="ok">●</span>
        In use: <code>{view.active?.split("/").pop()}</code>
        {#if view.source === "custom"}
          — the custom model
        {:else if view.source === "environment"}
          — from <code>FLUESTR_MODEL</code>
        {:else if view.source === "development"}
          — found in <code>.scratch/models</code>
        {/if}
      </p>
      {#if !view.vad}
        <p class="warn">No speech detection installed — install the Silero model below.</p>
      {/if}
      <!-- Said here as well as on the vocabulary page: this is where the
           choice is made, and the vocabulary is the price of it. -->
      {#if view.engine === "parakeet"}
        <p class="warn">
          Parakeet does not read the vocabulary, and picks each segment's language itself — the
          channel languages do not apply.
        </p>
      {/if}
    {/if}

    {#each [{ title: "Speech", list: speech, custom: true }, { title: "Speech detection", list: vad, custom: false }] as section (section.title)}
      <h3>{section.title}</h3>
      <ul>
        {#each section.list as model (model.id)}
          {@const state = progress.get(model.id)}
          {@const busy = state?.stage === "downloading" || state?.stage === "verifying"}
          <li class:selected={active(model)}>
            <div class="row">
              <label class="choice">
                <input
                  type="radio"
                  name={section.title}
                  checked={active(model)}
                  disabled={!model.installed || locked}
                  onchange={() => select(model)}
                />
                <span class="name">
                  {model.label}
                  {#if model.recommended}<span class="tag">recommended</span>{/if}
                </span>
              </label>

              <span class="size">{megabytes(model.bytes)}</span>

              {#if busy}
                <button onclick={() => cancelDownload(model.id)}>Cancel</button>
              {:else if model.installed}
                <button onclick={() => remove(model.id)} disabled={locked}>Remove</button>
              {:else}
                <button onclick={() => download(model.id)}>
                  <!-- Honest about what pressing it does: the bytes from the
                       last attempt are still there and will be continued. -->
                  {model.partial > 0 ? `Resume (${megabytes(model.partial)})` : "Download"}
                </button>
              {/if}
            </div>

            <p class="note">{model.note}</p>

            {#if busy}
              <div class="bar">
                <div
                  class="fill"
                  style:width="{state.total > 0
                    ? Math.min(100, (state.received / state.total) * 100)
                    : 0}%"
                ></div>
              </div>
              <p class="note">
                {#if state.stage === "verifying"}
                  Checking the download against its published checksum …
                {:else}
                  {megabytes(state.received)} of {megabytes(state.total || model.bytes)}
                {/if}
              </p>
            {:else if state?.stage === "failed"}
              <p class="error">{state.message}</p>
            {:else if state?.stage === "cancelled"}
              <p class="note">Stopped. What arrived is kept — press Resume to continue.</p>
            {/if}
          </li>
        {/each}

        <!-- One more choice in the same list rather than an override below it:
             a path that silently beat the picker meant the selected model and
             the running model could disagree, with nothing on the page saying
             which had won. -->
        {#if section.custom}
          <li class:selected={custom}>
            <div class="row">
              <label class="choice">
                <input
                  type="radio"
                  name={section.title}
                  checked={custom}
                  disabled={locked}
                  onchange={selectCustom}
                />
                <span class="name">Custom model</span>
              </label>
            </div>

            <p class="note">
              A whisper <code>.bin</code> from outside this list, or a Parakeet model folder.
            </p>

            <div class="path">
              <label for="custom-model-path">Path to a model file</label>
              <div class="row">
                <input
                  id="custom-model-path"
                  type="text"
                  placeholder="/Volumes/Stick/ggml-small-q5_1.bin"
                  bind:value={
                    () => models.modelPath ?? "",
                    (v) => (models.modelPath = v.trim() === "" ? null : v)
                  }
                  disabled={locked}
                  onchange={refresh}
                />
                <!-- Only where there is a dialog to open (P16). The path
                     itself stays editable from the remote — a model file on
                     the host machine can be named, it just cannot be browsed
                     to from another one. -->
                {#if canPickModelFile}
                  <button onclick={browse} disabled={locked}>Choose …</button>
                {/if}
              </div>
            </div>
          </li>
        {/if}
      </ul>
    {/each}

    <!-- The one setting on this page that is not about which file runs but
         about how many copies of it do. It lives here rather than beside the
         channel map because it is the same kind of question as the model: what
         this machine can run, not what this production needs. -->
    <h3>Decoding</h3>
    <div class="workers">
      <label for="worker-count">Parallel decodes</label>
      <input
        id="worker-count"
        type="number"
        min="1"
        max={MAX_WORKERS}
        step="1"
        placeholder={String(DEFAULT_WORKERS)}
        bind:value={
          () => models.workers ?? null,
          // Cleared is "never chose", which is the default — not zero, which
          // would be a pool with nothing in it. Not clamped on the keystroke:
          // somebody typing over a 2 passes through states this box has no
          // business correcting. `profiles.ts` clamps on the way to the file.
          (v) => (models.workers = v === null || Number.isNaN(v) ? null : v)
        }
        disabled={locked}
      />
      <p class="note">How many model processes transcribe at once. Default {DEFAULT_WORKERS}.</p>
    </div>

    {#if !error}
      <p class="hint">Downloads go to <code>{view.dir}</code>.</p>
    {/if}
  {:else if !error}
    <p class="hint">Reading the model directory …</p>
  {/if}
</section>

<style>
  h3 {
    margin: 0.9rem 0 0.3rem;
    font-size: 0.75rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  li {
    border: 1px solid var(--border);
    border-radius: 0.35rem;
    padding: 0.5rem 0.6rem;
  }

  /* The one that runs, marked on the row rather than only by the radio: on a
     list of five the dot alone is easy to lose. */
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

  button {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.25rem 0.7rem;
    font: inherit;
    font-size: 0.8rem;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .note,
  .hint {
    margin: 0.35rem 0 0;
    font-size: 0.78rem;
    color: var(--text-dim);
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

  .problem {
    margin: 0;
    padding: 0.5rem 0.6rem;
    border: 1px solid #ff6b6b;
    border-radius: 0.35rem;
    font-size: 0.85rem;
  }

  .warn {
    margin: 0.5rem 0 0;
    font-size: 0.82rem;
    color: #e0b341;
  }

  .inuse {
    margin: 0;
    font-size: 0.85rem;
  }

  .ok {
    color: #5fd38d;
  }

  .error {
    margin: 0.35rem 0 0;
    font-size: 0.8rem;
    color: #ff6b6b;
  }

  code {
    font-size: 0.85em;
  }

  .path {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    margin-top: 0.35rem;
    font-size: 0.7rem;
    color: var(--text-dim);
    max-width: 34rem;
  }

  /* The field takes the width the button leaves: a path is long and the button
     is two words, and a half-visible path is the one thing this row is for. */
  .path input[type="text"] {
    flex: 1;
    min-width: 0;
  }

  .workers {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    max-width: 34rem;
  }

  .workers label {
    font-size: 0.7rem;
    color: var(--text-dim);
  }

  /* Wide enough for two digits and the spinner, and no wider: a box the width
     of the panel says a long number is expected here. */
  .workers input[type="number"] {
    width: 5rem;
  }

  input[type="number"],
  input[type="text"] {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.85rem;
  }
</style>

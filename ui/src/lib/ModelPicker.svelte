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
  // - **There is a way out without a network at all.** The manual path takes a
  //   `.bin` carried in on a stick, and it outranks the picker — somebody who
  //   typed a path meant it.

  import {
    cancelDownload,
    downloadModel,
    listModels,
    onModelProgress,
    removeModel,
    type ModelProgress,
    type ModelsView,
    type ModelStatus,
  } from "./bridge";
  import type { ModelChoice } from "./profiles";

  interface Props {
    models: ModelChoice;
    /// While a session runs the sidecar holds the model file open, so
    /// switching or deleting one is locked for the same reason the channel map
    /// is: it would take effect at a moment nobody chose.
    locked: boolean;
  }

  let { models = $bindable(), locked }: Props = $props();

  let view = $state<ModelsView | null>(null);
  let error = $state<string | null>(null);
  /// Downloads in flight, by model id. Keyed rather than a single value
  /// because the speech model and the VAD are two downloads an operator may
  /// reasonably start together.
  let progress = $state(new Map<string, ModelProgress>());

  const refresh = async () => {
    try {
      view = await listModels();
      error = null;
    } catch (e) {
      // The one failure that is not about a particular model: the platform
      // would not say where the app may keep data, so nothing can be
      // downloaded at all and the manual path is the only way through.
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

  const speech = $derived(view?.models.filter((m) => m.kind === "speech") ?? []);
  const vad = $derived(view?.models.filter((m) => m.kind === "vad") ?? []);
  /// An operator who typed a path has taken the choice out of the picker's
  /// hands, and a picker that still looks live would be lying about what runs.
  const overridden = $derived((models.modelPath ?? "").trim() !== "");

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
    // The selection is stored by `App.svelte`'s debounced save; refreshing is
    // what makes the "in use" line agree with the radio button that was just
    // clicked.
    setTimeout(refresh, 0);
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
        {#if view.source === "manual"}
          — from the path set below
        {:else if view.source === "environment"}
          — from <code>INTERCOM2CHAT_MODEL</code>
        {:else if view.source === "development"}
          — found in <code>.scratch/models</code>
        {/if}
      </p>
      {#if !view.vad}
        <p class="warn">
          No speech detection installed. Noise and silence are transcribed too, and
          whisper invents sentences for both — install the Silero model below.
        </p>
      {/if}
    {/if}

    {#each [{ title: "Speech", list: speech }, { title: "Speech detection", list: vad }] as section (section.title)}
      <h3>{section.title}</h3>
      <ul>
        {#each section.list as model (model.id)}
          {@const state = progress.get(model.id)}
          {@const busy = state?.stage === "downloading" || state?.stage === "verifying"}
          <li class:selected={active(model) && !overridden}>
            <div class="row">
              <label class="choice">
                <input
                  type="radio"
                  name={section.title}
                  checked={active(model)}
                  disabled={!model.installed || locked || overridden}
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
      </ul>
    {/each}

    <h3>Offline fallback</h3>
    <label class="path">
      <span>Path to a model file</span>
      <input
        type="text"
        placeholder="/Volumes/Stick/ggml-small-q5_1.bin"
        bind:value={
          () => models.modelPath ?? "", (v) => (models.modelPath = v.trim() === "" ? null : v)
        }
        disabled={locked}
        onchange={() => setTimeout(refresh, 0)}
      />
    </label>
    <p class="hint">
      For a machine that will never reach the internet: carry the <code>.bin</code> in and
      say where it is. It overrides the choice above.
      {#if !error}
        Downloads go to <code>{view.dir}</code>.
      {/if}
    </p>
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
    font-size: 0.7rem;
    color: var(--text-dim);
    max-width: 34rem;
  }

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

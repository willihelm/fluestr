<script lang="ts">
  // The three numbers that decide how the pipeline cuts and what it listens
  // to (P12).
  //
  // They were constants in the code until the plan's P14 — the latency and
  // accuracy sweep that was to tell us what they should be — was set aside in
  // favour of testing in the house. That inverts the argument for keeping them
  // out of reach rather than removing it: a number found by ear has to be
  // reachable while listening, and an operator who hears a sentence being cut
  // in half has to be able to do something about it before the next cue.
  //
  // Beside the vocabulary in scope rather than inside a profile: how hard the
  // intercom is to hear is a property of the house and its beltpacks, and it
  // outlives any one production.
  //
  // Beside them, one switch that is not a threshold: whether a channel's
  // previous sentence is carried into the prompt of its next segment (P25).
  // Here because it is the same kind of decision — what the pipeline hands
  // whisper — and because it is the one an operator reaches for when they hear
  // a wrong word coming back again and again.
  //
  // And one more: whether the open segment is decoded while it is still being
  // spoken and shown in its placeholder (P28). Here because it is paid for in
  // decodes like the rest of the page, and because whether text that rewrites
  // itself helps or distracts is judged by ear during a show, like the others.
  // With it, what the preview costs: how often the draft is redone — each one
  // decodes the whole open segment again, so a long monologue at a short step
  // keeps the GPU busy for as long as it lasts. Which channels are drafted
  // is decided per channel, with the D beside its M in the channel list.
  //
  // **Every change reaches the session that is already running**, on the next
  // segment rather than at the next Start. That is the whole point of the
  // page: an operator turns one of these while somebody talks, and listens to
  // what it did.

  import {
    MAX_SEGMENT_MS_MAX,
    MAX_SEGMENT_MS_MIN,
    PREVIEW_STEP_MS_MAX,
    PREVIEW_STEP_MS_MIN,
    SILENCE_MS_MAX,
    SILENCE_MS_MIN,
    type Tuning,
  } from "./bridge";
  import { DEFAULT_TUNING } from "./profiles";

  interface Props {
    tuning: Tuning;
    /// Whether a session is running, for the one line that says so. Nothing
    /// here is locked by it — unlike the channel map, these three can be moved
    /// while capture runs, and that is when they are usually moved at all.
    running: boolean;
  }

  let { tuning = $bindable(), running }: Props = $props();

  const isDefault = $derived(
    tuning.silenceTimeoutMs === DEFAULT_TUNING.silenceTimeoutMs &&
      tuning.maxSegmentMs === DEFAULT_TUNING.maxSegmentMs &&
      tuning.speechThresholdPercent === DEFAULT_TUNING.speechThresholdPercent &&
      tuning.carryContext === DEFAULT_TUNING.carryContext &&
      tuning.livePreview === DEFAULT_TUNING.livePreview &&
      tuning.previewStepMs === DEFAULT_TUNING.previewStepMs,
  );

  const reset = () => (tuning = { ...DEFAULT_TUNING });

  /// Milliseconds as an operator reads them: whole seconds where the number is
  /// one, milliseconds where it is not.
  const ms = (value: number) => (value >= 1000 ? `${(value / 1000).toFixed(1)} s` : `${value} ms`);

  /// The worst case somebody waits for a line, in words rather than in two
  /// numbers to add up.
  const worstCase = $derived(ms(tuning.maxSegmentMs + tuning.silenceTimeoutMs + 330));
</script>

<section class="panel">
  <header>
    <h2>Detection</h2>
    {#if !isDefault}
      <button type="button" class="reset" onclick={reset}>Back to the defaults</button>
    {/if}
  </header>

  <p class="hint">{running ? "Changes apply from the next segment." : "Changes apply on the next Start."}</p>

  <div class="control">
    <label for="silence-timeout">
      Pause that ends a segment
      <output for="silence-timeout">{ms(tuning.silenceTimeoutMs)}</output>
    </label>
    <input
      id="silence-timeout"
      type="range"
      min={SILENCE_MS_MIN}
      max={SILENCE_MS_MAX}
      step="50"
      bind:value={tuning.silenceTimeoutMs}
    />
    <p class="note">Shorter is faster, but cuts more sentences in two.</p>
  </div>

  <div class="control">
    <label for="max-segment">
      Longest segment
      <output for="max-segment">{ms(tuning.maxSegmentMs)}</output>
    </label>
    <input
      id="max-segment"
      type="range"
      min={MAX_SEGMENT_MS_MIN}
      max={MAX_SEGMENT_MS_MAX}
      step="500"
      bind:value={tuning.maxSegmentMs}
    />
    <p class="note">Below about two seconds whisper starts inventing sentences.</p>
  </div>

  <div class="control">
    <label for="speech-threshold">
      How sure the detector has to be
      <output for="speech-threshold">{tuning.speechThresholdPercent} %</output>
    </label>
    <input
      id="speech-threshold"
      type="range"
      min="0"
      max="100"
      step="5"
      bind:value={tuning.speechThresholdPercent}
    />
    <p class="note">Lower hears quiet speech; higher keeps noise out.</p>
  </div>

  <div class="control">
    <label class="switch">
      <input type="checkbox" bind:checked={tuning.carryContext} />
      Carry the previous sentence into the next segment
    </label>
    <p class="note">Turn it off if a wrong word keeps coming back.</p>
  </div>

  <div class="control">
    <label class="switch">
      <input type="checkbox" bind:checked={tuning.livePreview} />
      Show what is being said before the line is finished
    </label>
    <p class="note">
      The placeholder fills with a rough draft while somebody talks; the line replaces it. Turn it
      off if text that rewrites itself is more distracting than text that arrives later.
    </p>
  </div>

  <div class="control">
    <label for="preview-step">
      Update the draft every
      <output for="preview-step">{ms(tuning.previewStepMs)}</output>
    </label>
    <input
      id="preview-step"
      type="range"
      min={PREVIEW_STEP_MS_MIN}
      max={PREVIEW_STEP_MS_MAX}
      step="100"
      bind:value={tuning.previewStepMs}
      disabled={!tuning.livePreview}
    />
    <p class="note">
      Every update decodes everything said so far again. Longer takes load off the GPU when
      somebody talks for a long time.
    </p>
  </div>

  <p class="note">
    A single channel is taken out of the drafts with the D beside its M in the channel list.
  </p>

  <p class="budget">Worst case until a line appears: <strong>{worstCase}</strong></p>
</section>

<style>
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .reset {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    font-size: 0.8rem;
    color: var(--text-dim);
    text-decoration: underline;
    cursor: pointer;
  }

  .reset:hover {
    color: var(--text);
  }

  .hint {
    margin: 0 0 0.9rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .control + .control {
    margin-top: 0.9rem;
  }

  .control label {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
    font-size: 0.9rem;
    font-weight: 600;
  }

  /* The number belongs to the slider rather than under it: the slider says
     roughly where it is and this says exactly, and an operator reading one is
     already looking at the other. Tabular figures so it does not jump about
     while being dragged. */
  output {
    font-weight: 400;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
  }

  /* The label reads as the switch's own sentence, not as a heading over a
     control: there is no number to show beside it. */
  .control label.switch {
    justify-content: flex-start;
    align-items: center;
    cursor: pointer;
  }

  .switch input {
    margin: 0;
    accent-color: var(--accent);
  }

  input:disabled {
    opacity: 0.45;
  }

  input[type="range"] {
    width: 100%;
    margin: 0.3rem 0 0;
    accent-color: var(--accent);
  }

  .note {
    margin: 0.2rem 0 0;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  /* What the three add up to, which is the question all of them are really
     about and none of them answers alone. */
  .budget {
    margin: 1rem 0 0;
    padding-top: 0.7rem;
    border-top: 1px solid var(--border);
    font-size: 0.8rem;
    color: var(--text-dim);
  }
</style>

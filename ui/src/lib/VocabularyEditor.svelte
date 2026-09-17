<script lang="ts">
  // The one active vocabulary, for every channel of the session.
  //
  // A single box rather than one per channel — see `vocabulary.ts` for why —
  // and locked while a session runs, because the prompt is handed to the
  // workers when the session starts.

  import { DEFAULT_VOCABULARY, estimatedTokens, lengthWarning, TOKEN_LIMIT } from "./vocabulary";

  interface Props {
    vocabulary: string;
    locked: boolean;
  }

  let { vocabulary = $bindable(), locked }: Props = $props();

  let tokens = $derived(estimatedTokens(vocabulary));
  let warning = $derived(lengthWarning(vocabulary));
</script>

<section>
  <header>
    <h2>Vocabulary</h2>
    <span class="count">
      <!-- "about", because this is a heuristic and not whisper's tokeniser —
           see `estimatedTokens`. -->
      about {tokens} of {TOKEN_LIMIT} tokens
    </span>
    <button
      class="reset"
      onclick={() => (vocabulary = DEFAULT_VOCABULARY)}
      disabled={locked || vocabulary === DEFAULT_VOCABULARY}
    >
      Reset
    </button>
  </header>

  <p class="hint">
    Terms whisper should expect, on every channel: cue names, departments, house jargon.
    The same list applies to all streams — one production has one vocabulary.
  </p>

  <textarea bind:value={vocabulary} rows="3" disabled={locked}></textarea>

  {#if warning}
    <p class="warn">{warning}</p>
  {/if}
</section>

<style>
  section {
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    padding: 0.5rem 1rem 0.75rem;
  }

  header {
    display: flex;
    align-items: baseline;
    gap: 0.75rem;
    margin-bottom: 0.4rem;
  }

  h2 {
    margin: 0;
    font-size: 0.85rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }

  .count {
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .reset {
    margin-left: auto;
  }

  .hint {
    margin: 0 0 0.4rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  textarea {
    width: 100%;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.4rem 0.5rem;
    font: inherit;
    font-size: 0.85rem;
    resize: vertical;
  }

  .warn {
    color: #f2b544;
    font-size: 0.8rem;
    margin: 0.5rem 0 0;
  }

  button {
    background: var(--accent);
    color: #12141a;
    border: none;
    border-radius: 0.3rem;
    padding: 0.3rem 0.8rem;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>

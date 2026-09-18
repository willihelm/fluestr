<script lang="ts">
  // The alert list: the words that must not be missed (P17).
  //
  // A row per term rather than one box of text. A term is a thing — it is
  // added, renamed, removed — and the list is short; a textarea would make
  // "one per line" a rule the operator has to be told and the parser has to
  // guess at. It is also what the file stores, so what is on screen and what
  // is saved have the same shape.
  //
  // Editable while a session runs, unlike the vocabulary. The vocabulary is
  // handed to the workers when the session starts; this is matched against
  // finished text, so a term added in the middle of a show takes effect on the
  // next segment — which is exactly when somebody thinks of adding one.

  interface Props {
    terms: string[];
  }

  let { terms = $bindable() }: Props = $props();

  const add = () => {
    terms = [...terms, ""];
  };

  const remove = (index: number) => {
    terms = terms.filter((_, i) => i !== index);
  };
</script>

<section class="panel">
  <header>
    <h2>Alert terms</h2>
    <span class="count">
      {terms.filter((t) => t.trim() !== "").length}
      {terms.filter((t) => t.trim() !== "").length === 1 ? "term" : "terms"}
    </span>
    <button class="action" onclick={add}>Add term</button>
  </header>

  <p class="hint">
    Words that must not be missed — highlighted wherever they are said, on every channel.
    An unfocused window asks for attention once. Whole words only and any capitalisation:
    a term does not match inside a longer word, so add the compound if you need it.
  </p>

  {#if terms.length === 0}
    <p class="hint empty">No alert terms. Nothing is highlighted and nothing flashes.</p>
  {:else}
    <ul>
      <!-- Keyed by position, not by the text: the text is what is being typed,
           and re-keying on every keystroke takes the focus out of the field. -->
      {#each terms as _, index (index)}
        <li>
          <input
            type="text"
            bind:value={terms[index]}
            placeholder="Feuer"
            aria-label="Alert term"
          />
          <button class="remove" onclick={() => remove(index)} title="Remove this term">
            ×
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .hint {
    margin: 0 0 0.6rem;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .empty {
    margin-bottom: 0;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    /* A column of its own width rather than the page's: an alert term is a
       word, and a field the width of the window invites a sentence. */
    max-width: 22rem;
  }

  li {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }

  input {
    flex: 1;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.85rem;
    min-width: 0;
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

  .remove {
    background: transparent;
    color: var(--text-dim);
    font-size: 1.1rem;
    line-height: 1;
    padding: 0.2rem 0.45rem;
  }

  .remove:hover {
    color: #ff6b6b;
  }
</style>

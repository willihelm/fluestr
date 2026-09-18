<script lang="ts">
  // The one control that names a language, used both above the channel list
  // and on every row in it.
  //
  // Its options come from the sidecar (`listLanguages`), never from a list in
  // this repository: a language whisper gains is one this picker gains, with
  // no code change. That is a requirement, not a convenience — the house this
  // was built for switches between DE and EN, and the next one will not.
  //
  // It degrades to a text field rather than to nothing. The list needs the
  // sidecar binary, which a development checkout may not have built and a
  // broken install may not have at all; a picker that vanishes would take the
  // ability to set a language with it, so the code stays typeable.

  import type { Language } from "./bridge";

  interface Props {
    value: string;
    /// The sidecar's list, or `null` while it is still being fetched or after
    /// it failed. Fetched once by the page and handed down, rather than by
    /// each of nine instances of this component.
    languages: Language[] | null;
    label: string;
    disabled?: boolean;
  }

  let { value = $bindable(), languages, label, disabled = false }: Props = $props();

  // Alphabetical by name. Whisper's own order is roughly by training volume —
  // english, chinese, german, spanish — which puts the common ones first but
  // makes the other ninety-odd unfindable. A menu is scanned by name, so it is
  // ordered by name.
  let sorted = $derived(
    languages === null
      ? []
      : [...languages].sort((a, b) => a.name.localeCompare(b.name)),
  );

  // A stored code the sidecar does not offer — a hand-edited file, or a
  // profile from a build with a different whisper. It is kept as an option of
  // its own rather than silently snapping to the first entry: the operator
  // chose it, and losing it on the way past a picker is worse than showing an
  // unfamiliar code.
  let unknown = $derived(
    value !== "" && sorted.length > 0 && !sorted.some((l) => l.code === value),
  );

  const title = (name: string) => name.charAt(0).toUpperCase() + name.slice(1);
</script>

{#if languages === null}
  <input
    type="text"
    aria-label={label}
    bind:value
    {disabled}
    title="ISO 639-1 code. The sidecar's language list is not available — build it with `cargo build -p whisper-worker`."
  />
{:else}
  <select aria-label={label} bind:value {disabled}>
    {#if unknown}
      <option value={value}>{value} — not in this build</option>
    {/if}
    {#each sorted as language (language.code)}
      <!-- The code beside the name, not instead of it: the name is what the
           menu is scanned by, the code is what the operator sees again in the
           chat's language badge and in a hand-edited configuration file. -->
      <option value={language.code}>{title(language.name)} ({language.code})</option>
    {/each}
  </select>
{/if}

<style>
  select,
  input {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.25rem 0.4rem;
    font: inherit;
    font-size: 0.85rem;
    min-width: 0;
    /* The row it sits in is a grid with a narrow language column, and a select
       sized by its widest option would blow that column out to the width of
       "Serbo-croatian (sh)". */
    width: 100%;
  }

  select:disabled,
  input:disabled {
    opacity: 0.5;
  }
</style>

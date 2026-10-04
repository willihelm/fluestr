<script lang="ts">
  // The key screen a remote page shows when it has nothing to get in with
  // (P16).
  //
  // Meant to be seen rarely. The ordinary way in is the QR code on the
  // Settings page, which carries the key in the link — the page trades it for
  // a cookie that lasts 400 days and this screen never appears. What is left
  // for it is the operator who typed the address by hand, the browser whose
  // site data was cleared, and the morning after the key was replaced.
  //
  // It is deliberately the whole window rather than a banner over the chat.
  // Behind it there is no chat: the socket is what carries everything, so a
  // page that has not been let in has an empty app to show, and showing it
  // under a dialog would be showing an intercom log that is empty for the
  // wrong reason.

  import { offerKey } from "./transport";

  let key = $state("");
  let busy = $state(false);
  let refused = $state<string | null>(null);

  const offer = async (event: SubmitEvent) => {
    event.preventDefault();
    const offered = key.trim();
    if (offered === "" || busy) return;

    busy = true;
    refused = null;
    // `offerKey` opens the socket itself when the key is right, and this
    // screen goes away with the lock it is rendered on — so there is nothing
    // to do on success but stop saying anything.
    refused = await offerKey(offered);
    busy = false;
    if (refused !== null) key = "";
  };
</script>

<main>
  <form onsubmit={offer}>
    <h1>Fluestr</h1>
    <p>
      This session is served with a key. Scanning the QR code in the app's
      Settings puts it in for you; this is the other way in.
    </p>

    <label>
      <span>Key</span>
      <!-- `autocapitalize` off because the alphabet is upper case and a phone
           that capitalises the first letter and nothing else produces a key
           that is wrong in a way nobody can see. -->
      <input
        bind:value={key}
        type="text"
        autocomplete="off"
        autocapitalize="characters"
        autocorrect="off"
        spellcheck="false"
        disabled={busy}
      />
    </label>

    {#if refused}
      <p class="error" aria-live="polite">{refused}</p>
    {/if}

    <button type="submit" disabled={busy || key.trim() === ""}>
      {busy ? "Checking …" : "Connect"}
    </button>
  </form>
</main>

<style>
  main {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100dvh;
    /* The page is opened on a phone more often than not, and a form pinned to
       the very top edge is a form under the notch. */
    padding: 1.5rem max(1.5rem, env(safe-area-inset-left)) 1.5rem
      max(1.5rem, env(safe-area-inset-right));
    background: var(--bg);
    color: var(--text);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 0.9rem;
    width: min(22rem, 100%);
    padding: 1.5rem;
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    background: var(--surface);
  }

  h1 {
    margin: 0;
    font-size: 1.1rem;
  }

  p {
    margin: 0;
    font-size: 0.85rem;
    color: var(--text-dim);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: 0.85rem;
    color: var(--text-dim);
  }

  input {
    font: inherit;
    /* Wide-spaced and upper case, because what is being copied is eight
       characters that mean nothing — the one string where a reader needs to
       see each letter on its own. */
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 1.1rem;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    padding: 0.5rem 0.6rem;
    border-radius: 0.3rem;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
  }

  button {
    font: inherit;
    font-size: 0.9rem;
    /* Tall enough to be a target on a phone held at arm's length at a rack. */
    padding: 0.55rem 1rem;
    border-radius: 0.3rem;
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--on-accent);
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .error {
    color: var(--danger);
  }
</style>

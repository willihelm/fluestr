
<script lang="ts">
  // The LAN remote client (P16).
  //
  // The session runs where the audio interface is; this is what lets the
  // operator work from somewhere else. Turning it on serves the same UI on one
  // chosen interface, and the URL below is what gets typed into the other
  // machine's browser.
  //
  // The setting is part of the configuration like everything else on this
  // page — there is no Apply button, and the listener follows the file. What
  // is *not* in the configuration is whether the listener actually came up:
  // a port that is taken and an address that has gone away both leave the
  // setting on and nothing served, and that is precisely the state an operator
  // has to be able to see. So the status is reported separately and shown
  // beside the switch.
  //
  // The key is optional and on by default, and the point of the whole design
  // is that having one costs nothing: it is generated, it goes into the QR
  // code below, and the browser that scans it is asked for nothing and stays
  // in for 400 days. An operator who works remotely every day never types it.
  // Turning it off is a real choice for a house network somebody controls —
  // it is what this did before — and it is one click.
  //
  // What the key is *not* is a guarantee. The listener serves plain http, so
  // the key crosses the network in the clear the first time a browser offers
  // it. It keeps out somebody on the venue's guest wifi who types in the
  // address; it does not keep out somebody reading the wire. That is why the
  // interface is still picked deliberately and why the warning below stayed.

  import { onMount } from "svelte";
  import { newKey } from "./key";
  import { clampPort } from "./profiles";
  import { encodeQr, qrPath } from "./qr";
  import {
    listInterfaces,
    onRemoteStatus,
    remoteStatus as fetchStatus,
    type Interface,
    type RemoteStatus,
    type StoredRemote,
  } from "./bridge";

  interface Props {
    remote: StoredRemote;
  }

  let { remote = $bindable() }: Props = $props();

  let addresses = $state<Interface[]>([]);
  let status = $state<RemoteStatus | null>(null);
  let listFailed = $state<string | null>(null);

  // A machine that is on no network at all, which is a real state: a laptop in
  // a rehearsal room with the wifi off. Saying so is more useful than an empty
  // picker that looks broken.
  let routable = $derived(addresses.filter((a) => !a.loopback));

  onMount(() => {
    const load = async () => {
      addresses = await listInterfaces();
      // The address the file names may belong to an interface that is no
      // longer there — a setup carried from one house to another. It stays
      // selected and is marked, for the same reason an unplugged audio device
      // stays selected: it is what the operator chose, and throwing it away
      // silently re-points the setting at whatever happens to be connected.
      status = await fetchStatus();
    };
    load().catch((e) => (listFailed = String(e)));

    const subscription = onRemoteStatus((next) => (status = next));
    return () => {
      subscription.then((unlisten) => unlisten()).catch(() => {});
      if (sayTimer !== null) clearTimeout(sayTimer);
    };
  });

  // The address is not among the ones this machine has. Kept selectable, and
  // marked, rather than dropped.
  let missing = $derived(
    remote.address !== "" && !addresses.some((a) => a.address === remote.address),
  );

  // Nothing to serve on. The switch stays reachable but cannot be turned on:
  // a listener with no address is the one configuration that means nothing.
  let ready = $derived(remote.address !== "");

  // Picking an address does not also turn the listener on. Somebody choosing
  // which interface to serve is answering "which one", not "start now", and an
  // app that begins serving a network on that click is an app that started
  // serving a network nobody asked it to.
  const pick = (address: string) => (remote.address = address);

  // --- The key ------------------------------------------------------------

  /// Whether the operator has said anything about the key on this page.
  ///
  /// What it is for is the one case the automatic key gets wrong: somebody
  /// deliberately emptied the box, and then switched the listener off and on
  /// again. Without this a key would come back, and a setting that undoes
  /// itself is worse than one that was never offered.
  let decided = $state(false);

  /// Serving, and — the first time — with a key.
  ///
  /// Generated rather than asked for. A box that has to be filled in before
  /// the feature works is a box that gets filled in with the house's wifi
  /// password, and the key is in the QR code anyway, so there is nothing for
  /// an operator to gain by choosing it.
  const serve = (on: boolean) => {
    remote.enabled = on;
    if (on && remote.key === "" && !decided) remote.key = newKey();
  };

  // The box *is* the setting: empty is a listener anyone on the address can
  // drive, filled is one that asks. There is no switch beside it, because a
  // switch and a box that can disagree is a screen where "on, and empty" has
  // to mean something — and the only honest thing it could mean is off.

  /// A new key, which puts every browser that had the old one out.
  ///
  /// It does: the listener is rebound when the key changes — see `apply` in
  /// `remote.rs` — which drops the sockets, and the cookie those browsers hold
  /// is derived from the key they were let in with. So this is also the button
  /// for the phone that was left in a green room.
  const replace = () => {
    decided = true;
    remote.key = newKey();
  };

  // Written on change rather than on every keystroke, unlike the port. A key
  // that changed per character would rebind the listener per character, and
  // each rebind drops the browsers that are connected — so an operator typing
  // a key of their own would knock the room off eight times.
  let typed = $state("");
  $effect(() => {
    typed = remote.key;
  });
  const commit = () => {
    decided = true;
    remote.key = typed.trim();
  };

  // Copying the address, and saying whether it worked.
  //
  // Both halves were the bug. A browser only hands out `navigator.clipboard`
  // in a secure context, and the remote page is plain http on a LAN address —
  // so on the very machine this button exists for, the object is not there at
  // all and the optional call was a no-op. And because nothing was said
  // either way, a button that did nothing looked exactly like a button that
  // had worked, which is how an operator ends up pasting whatever was in the
  // clipboard before.

  /// How long the button's answer stays up. Long enough to read, short enough
  /// that it is gone before the next press.
  const SAID_MS = 1_600;

  let said = $state<"copied" | "failed" | null>(null);
  let sayTimer: ReturnType<typeof setTimeout> | null = null;

  const say = (what: "copied" | "failed") => {
    said = what;
    if (sayTimer !== null) clearTimeout(sayTimer);
    sayTimer = setTimeout(() => {
      sayTimer = null;
      said = null;
    }, SAID_MS);
  };

  /// The clipboard the old way. Deprecated everywhere, and the only thing
  /// that works on an http page — which is what the remote client is, and
  /// will stay until there is a certificate to serve it under.
  const copyTheOldWay = (text: string): boolean => {
    const field = document.createElement("textarea");
    field.value = text;
    field.setAttribute("readonly", "");
    // Placed off-screen rather than hidden: what the command copies is a
    // selection, and nothing that is not rendered can be selected.
    field.style.position = "fixed";
    field.style.top = "-1000px";
    document.body.appendChild(field);
    try {
      field.select();
      return document.execCommand("copy");
    } catch {
      return false;
    } finally {
      field.remove();
    }
  };

  const copy = async () => {
    const url = status?.url;
    if (!url) return;
    try {
      if (navigator.clipboard) {
        await navigator.clipboard.writeText(url);
        say("copied");
        return;
      }
    } catch (e) {
      // Present but refusing — no permission, or no user gesture the webview
      // recognised. The old way still tends to go through.
      console.warn("the clipboard refused the address:", e);
    }
    say(copyTheOldWay(url) ? "copied" : "failed");
  };

  // The address as something a phone can be pointed at.
  //
  // The other machine is often a phone, and typing `http://192.168.1.23:8420`
  // on a glass keyboard while standing at a rack is the worst part of setting
  // the remote up. The camera does it instead, and the operator hands the
  // phone to somebody else and has them scan it too.
  //
  // Drawn dark on white with a margin around it, in a window that is
  // otherwise dark throughout. That is not an oversight: the quiet zone is
  // part of the code, and a reader that has to find the finder patterns
  // against a dark page is a reader that sometimes does not.
  let code = $derived.by(() => {
    const url = status?.running ? status.url : null;
    if (!url) return null;
    try {
      const encoded = encodeQr(url);
      return { size: encoded.size, path: qrPath(encoded) };
    } catch (e) {
      // Only an address longer than a version 9 code holds, which a LAN URL
      // is not. The panel is still useful without the picture.
      console.warn("the address was not turned into a QR code:", e);
      return null;
    }
  });

  /// The quiet zone, in modules. Four is what the standard asks for, and less
  /// than that is the usual reason a code that looks fine will not scan.
  const QUIET = 4;
</script>

<section class="panel">
  <header>
    <h2>Remote client</h2>
  </header>

  <p class="hint">
    Open this interface in a browser on the network, or connect to it from another Fluestr.
  </p>

  <label class="switch">
    <input
      type="checkbox"
      checked={remote.enabled}
      onchange={(e) => serve(e.currentTarget.checked)}
      disabled={!ready}
    />
    <span>Serve on the local network</span>
  </label>

  <label>
    <span>Interface</span>
    <select value={remote.address} onchange={(e) => pick(e.currentTarget.value)}>
      {#if remote.address === ""}
        <option value="">Choose an address …</option>
      {/if}
      {#each addresses as option (option.address)}
        <option value={option.address}>
          {option.address} ({option.name}{option.loopback ? ", this machine only" : ""})
        </option>
      {/each}
      {#if missing}
        <!-- Selectable, because it is what the configuration says and
             deselecting it would throw that away. -->
        <option value={remote.address}>{remote.address} — not on this machine</option>
      {/if}
    </select>
  </label>

  <label>
    <span>Port</span>
    <input
      type="number"
      min="1024"
      max="65535"
      bind:value={remote.port}
      onblur={() => (remote.port = clampPort(remote.port))}
    />
  </label>

  <label>
    <span>Key</span>
    <!-- Written on blur or Enter, not per keystroke: see `commit`. -->
    <input
      class="key"
      type="text"
      placeholder="no key"
      bind:value={typed}
      onchange={commit}
      onblur={commit}
      autocomplete="off"
      autocapitalize="characters"
      autocorrect="off"
      spellcheck="false"
    />
    <button onclick={replace} title="Replaces the key and disconnects every browser that has the old one">
      New key
    </button>
  </label>

  {#if remote.key === ""}
    <p class="hint">Empty: anyone on the network can drive the session.</p>
  {:else}
    <p class="hint">A new key disconnects every browser using the old one.</p>
  {/if}

  {#if listFailed}
    <p class="error">The network interfaces could not be listed: {listFailed}</p>
  {:else if routable.length === 0}
    <p class="warn">Only the loopback address is available, so no other machine can connect.</p>
  {/if}

  {#if status?.error}
    <p class="error">{status.error}</p>
  {/if}

  {#if status?.running && status.url}
    <div class="url">
      <p>
        <span class="ok">●</span>
        Open <code>{status.url}</code> on the other machine.
      </p>
      {#if code}
        <svg
          class="qr"
          viewBox="{-QUIET} {-QUIET} {code.size + QUIET * 2} {code.size + QUIET * 2}"
          role="img"
          aria-label="The address as a QR code"
        >
          <rect
            x={-QUIET}
            y={-QUIET}
            width={code.size + QUIET * 2}
            height={code.size + QUIET * 2}
            fill="#fff"
          />
          <path d={code.path} fill="#000" />
        </svg>
      {/if}
      <div class="row">
        <button onclick={copy}>Copy address</button>
        {#if said === "copied"}
          <span class="dim" aria-live="polite">Copied.</span>
        {:else if said === "failed"}
          <!-- The address is on screen right above this: saying so is the
               whole of the recovery. -->
          <span class="error" aria-live="polite">Not copied — select the address and copy it.</span>
        {/if}
        <span class="dim">
          {status.clients}
          {status.clients === 1 ? "browser connected" : "browsers connected"}
        </span>
      </div>
    </div>
  {:else if remote.enabled && ready}
    <p class="dim">Starting …</p>
  {/if}

  <p class="warn">Plain http: use it only on a network you control.</p>
</section>


<style>
  /* The card itself is `app.css`; this only lays out what is inside it. */
  .panel {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  label {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-size: 0.85rem;
  }

  label > span {
    min-width: 5.5rem;
    color: var(--text-dim);
  }

  .switch {
    gap: 0.5rem;
  }

  .switch > span {
    min-width: 0;
    color: var(--text);
  }

  select,
  input[type="number"] {
    font: inherit;
    font-size: 0.85rem;
    padding: 0.3rem 0.4rem;
    border-radius: 0.3rem;
    border: 1px solid var(--border);
    background-color: var(--bg);
    color: var(--text);
  }

  select {
    padding-right: 1.5rem;
  }

  input[type="number"] {
    width: 7rem;
  }

  /* Eight characters that mean nothing, which is the one string where every
     letter has to be legible on its own. */
  .key {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    width: 9rem;
  }

  .url {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    background: var(--bg);
  }

  .url p {
    margin: 0;
    font-size: 0.85rem;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  /* Big enough to scan from arm's length, and no bigger: the panel is a
     settings card, not a poster. `crispEdges` keeps the modules square —
     anti-aliased edges at this size are what a reader trips over. */
  .qr {
    width: 9rem;
    height: 9rem;
    align-self: flex-start;
    border-radius: 0.2rem;
    shape-rendering: crispEdges;
  }

  button {
    font: inherit;
    font-size: 0.8rem;
    padding: 0.25rem 0.7rem;
    border-radius: 0.3rem;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
  }

  code {
    font-size: 0.85rem;
  }

  .hint,
  .dim {
    margin: 0;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  .warn {
    margin: 0;
    font-size: 0.8rem;
    color: #e0b341;
  }

  .error {
    margin: 0;
    font-size: 0.8rem;
    color: #ff6b6b;
  }

  .ok {
    color: #5fd38d;
  }
</style>

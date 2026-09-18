
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
  // The warning is not decoration. There is no authentication yet: anyone who
  // reaches the chosen address can start and stop the session and change the
  // setup. That is survivable on a house network the operator controls and is
  // not survivable on a venue's guest wifi, and the only thing standing
  // between the two is the address picked here — which is why one is picked,
  // deliberately, instead of the app serving every interface at once.

  import { onMount } from "svelte";
  import { clampPort } from "./profiles";
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

  const copy = () => {
    if (!status?.url) return;
    navigator.clipboard?.writeText(status.url).catch((e) => {
      // Not worth a strip on the page: the URL is on screen and can be read.
      console.warn("the address was not copied:", e);
    });
  };
</script>

<section class="panel">
  <header>
    <h2>Remote client</h2>
  </header>

  <p class="hint">
    Serves this same interface to a browser on the local network, so the session
    can run where the audio interface is and be worked from somewhere else. The
    browser is a full peer: it sees the whole chat and can start, stop and
    change the setup.
  </p>

  <label class="switch">
    <input type="checkbox" bind:checked={remote.enabled} disabled={!ready} />
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

  {#if listFailed}
    <p class="error">The network interfaces could not be listed: {listFailed}</p>
  {:else if routable.length === 0}
    <p class="warn">
      This machine is not on a network another machine could reach it over. Only
      the loopback address is available, which serves this machine and no other.
    </p>
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
      <div class="row">
        <button onclick={copy}>Copy address</button>
        <span class="dim">
          {status.clients}
          {status.clients === 1 ? "browser connected" : "browsers connected"}
        </span>
      </div>
    </div>
  {:else if remote.enabled && ready}
    <p class="dim">Starting …</p>
  {/if}

  <p class="warn">
    There is no password on this yet. Anyone who can reach the chosen address
    can start and stop the session and change the setup — so pick an interface
    on a network you control, and leave this off on a venue's guest wifi.
  </p>
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
    background: var(--bg);
    color: var(--text);
  }

  input[type="number"] {
    width: 7rem;
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

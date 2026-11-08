<script lang="ts">
  // Reading along with other instances (P29).
  //
  // Two halves, because they are two decisions. Which instances this machine
  // reads is a property of the machine and the house — it is stored beside
  // the profiles, like the LAN remote — and which of their channels are shown
  // is the production's, so it is stored in the active profile, as rows.
  //
  // A channel ticked here becomes a row of the channel list: it gets a colour,
  // it can be put in groups and named by rules, and its lines arrive in the
  // chat like any other. Unticking it removes the row, and with it its place
  // in the groups — exactly what removing a channel does.
  //
  // Nothing here changes the other instance. Reading along is reading: its
  // session, its settings and its shoutbox are its own, and this machine's
  // are this machine's.

  import { onMount } from "svelte";
  import {
    appVersion,
    discoverServers,
    onServersFound,
    onSources,
    probeServer,
    sourcesStatus,
    type FeedChannel,
    type FoundServer,
    type SourceStatus,
    type StoredSource,
  } from "./bridge";
  import { remoteChannel, type Channel } from "./streams";
  import type { Profile } from "./profiles";
  import { REMOTE_DEFAULT_PORT } from "./generated/constants";

  interface Props {
    sources: StoredSource[];
    /// The active profile, whose channel list the ticked channels are rows of.
    profile: Profile | null;
    locked: boolean;
  }

  let { sources = $bindable(), profile, locked }: Props = $props();

  let found = $state<FoundServer[]>([]);
  /// This machine's version, to tell an instance on another one apart.
  let version = $state<string | null>(null);
  let statuses = $state<SourceStatus[]>([]);
  let discoveryError = $state<string | null>(null);

  onMount(() => {
    let stop: (() => void)[] = [];
    let gone = false;
    (async () => {
      const listeners = await Promise.all([
        onServersFound((next) => (found = next)),
        onSources((next) => (statuses = next)),
      ]);
      if (gone) listeners.forEach((u) => u());
      else stop = listeners;
      appVersion()
        .then((v) => (version = v))
        .catch(() => {});
      try {
        statuses = await sourcesStatus();
      } catch (e) {
        console.warn("the subscriptions could not be read:", e);
      }
      try {
        found = await discoverServers();
      } catch (e) {
        discoveryError = String(e);
      }
    })();
    return () => {
      gone = true;
      stop.forEach((u) => u());
    };
  });

  let statusOf = $derived(new Map(statuses.map((s) => [s.id, s])));
  /// Found on the network and not yet read.
  let offered = $derived(found.filter((f) => !sources.some((s) => s.id === f.id)));

  const add = (server: FoundServer) => {
    if (sources.some((s) => s.id === server.id)) return;
    sources = [
      ...sources,
      {
        id: server.id,
        name: server.name,
        address: server.address,
        port: server.port,
        key: "",
        enabled: true,
      },
    ];
  };

  const remove = (id: string) => {
    sources = sources.filter((s) => s.id !== id);
    // Its rows go with it: a row reading from an instance that is not read
    // any more is a channel that will never say anything.
    if (profile) profile.channels = profile.channels.filter((c) => c.source?.server !== id);
  };

  const update = (id: string, change: Partial<StoredSource>) => {
    sources = sources.map((s) => (s.id === id ? { ...s, ...change } : s));
  };

  // --- By hand -----------------------------------------------------------

  let address = $state("");
  let port = $state<number>(REMOTE_DEFAULT_PORT);
  let probing = $state(false);
  let probeError = $state<string | null>(null);

  const probe = async () => {
    probeError = null;
    probing = true;
    try {
      add(await probeServer(address.trim(), Number(port) || REMOTE_DEFAULT_PORT));
      address = "";
    } catch (e) {
      probeError = String(e);
    } finally {
      probing = false;
    }
  };

  // --- Keys --------------------------------------------------------------

  /// What is typed into each key box, committed on blur or Enter: a key that
  /// is tried once per keystroke is seven wrong keys and a lockout.
  let typed = $state<Record<string, string>>({});
  const commitKey = (id: string) => {
    const key = (typed[id] ?? "").trim();
    const source = sources.find((s) => s.id === id);
    if (source && source.key !== key) update(id, { key });
  };

  // --- Channels ----------------------------------------------------------

  const rowOf = (server: string, channel: string): Channel | undefined =>
    profile?.channels.find((c) => c.source?.server === server && c.source.channel === channel);

  const toggle = (server: string, channel: FeedChannel, shown: boolean) => {
    if (!profile) return;
    if (shown) {
      if (rowOf(server, channel.id)) return;
      profile.channels = [
        ...profile.channels,
        remoteChannel(
          profile.channels,
          { server, channel: channel.id },
          channel.label,
          channel.color,
          profile.language,
        ),
      ];
    } else {
      profile.channels = profile.channels.filter(
        (c) => !(c.source?.server === server && c.source.channel === channel.id),
      );
    }
  };

  /// Rows of this profile that point at a channel the instance no longer
  /// offers — renamed is fine, deleted over there is not.
  const orphans = (source: StoredSource, status: SourceStatus | undefined): Channel[] => {
    if (!profile || !status || status.state !== "connected") return [];
    const offeredIds = new Set(status.channels.map((c) => c.id));
    return profile.channels.filter(
      (c) => c.source?.server === source.id && !offeredIds.has(c.source.channel),
    );
  };

  const stateText: Record<SourceStatus["state"], string> = {
    off: "Off",
    connecting: "Connecting …",
    connected: "Connected",
    needsKey: "Needs its key",
    lost: "Not reachable",
  };
</script>

<section class="panel">
  <header>
    <h2>Read along</h2>
  </header>

  <p class="hint">
    Shows what other Fluestr instances on the network transcribe, in this chat, beside this
    machine's own channels. This machine's session, listen output and shoutbox stay its own.
    An instance is found here while its <em>Remote client</em> is serving.
  </p>
  <p class="hint">
    An instance set to connect automatically is connected whenever Fluestr runs here, and
    reconnected on its own after it restarts or moves to a new address. For machines that
    nobody attends, turn on <em>Open Fluestr when you log in</em> under Startup on both — and on
    the one that transcribes, <em>Start the session when Fluestr opens</em> and its Remote
    client as well.
  </p>

  {#each sources as source (source.id)}
    {@const status = statusOf.get(source.id)}
    {@const state = status?.state ?? (source.enabled ? "connecting" : "off")}
    <div class="source">
      <div class="row">
        <span class="dot {state}" aria-hidden="true">●</span>
        <strong>{status?.name ?? source.name}</strong>
        <span class="dim">{status?.address ?? source.address}:{status?.port ?? source.port}</span>
        <span class="dim">{stateText[state]}</span>
        <label
          class="switch"
          title="Connects whenever Fluestr runs and reconnects on its own whenever the instance is reachable — at startup, after a restart over there, and after a new address"
        >
          <input
            type="checkbox"
            checked={source.enabled}
            onchange={(e) => update(source.id, { enabled: e.currentTarget.checked })}
            disabled={locked}
          />
          <span>Connect automatically</span>
        </label>
        <button class="remove" onclick={() => remove(source.id)} disabled={locked}>Remove</button>
      </div>

      {#if state === "lost" && status && !status.connectedOnce}
        <!-- Not an error yet: both machines were likely switched on together
             and the other one is still coming up. -->
        <p class="dim">Waiting for it to come up — {status.error ?? "not reachable yet"}</p>
      {:else if status?.error && state !== "connected"}
        <p class={state === "needsKey" ? "warn" : "error"}>{status.error}</p>
      {/if}

      {#if state === "needsKey" || source.key !== ""}
        <label>
          <span>Key</span>
          <input
            class="key"
            type="text"
            placeholder="the key its Remote client shows"
            value={typed[source.id] ?? source.key}
            oninput={(e) => (typed[source.id] = e.currentTarget.value)}
            onchange={() => commitKey(source.id)}
            onblur={() => commitKey(source.id)}
            autocomplete="off"
            autocapitalize="characters"
            autocorrect="off"
            spellcheck="false"
          />
        </label>
      {/if}

      {#if status && status.version && version && status.version !== version}
        <p class="warn">
          It runs Fluestr {status.version}, this machine {version}. Reading along works
          best between the same versions.
        </p>
      {/if}

      {#if status && status.channels.length > 0}
        <div class="channels">
          {#each status.channels as channel (channel.id)}
            {@const row = rowOf(source.id, channel.id)}
            <label class="channel" style="--channel: {row?.color ?? channel.color}">
              <input
                type="checkbox"
                checked={row !== undefined}
                onchange={(e) => toggle(source.id, channel, e.currentTarget.checked)}
                disabled={locked || profile === null}
              />
              <span class="swatch" aria-hidden="true"></span>
              <span>{channel.label}</span>
              {#if row && row.label !== channel.label}
                <span class="dim">as “{row.label}”</span>
              {/if}
            </label>
          {/each}
        </div>
        <p class="hint">Ticked channels are rows of this profile's channel list.</p>
      {:else if state === "connected"}
        <p class="dim">It offers no channels: its active profile has none of its own.</p>
      {/if}

      {#each orphans(source, status) as orphan (orphan.key)}
        <p class="warn">
          “{orphan.label}” is not offered any more — it was removed or its profile was switched
          over there.
        </p>
      {/each}
    </div>
  {/each}

  {#if sources.length === 0}
    <p class="dim">Not reading along with any instance.</p>
  {/if}

  <h3>On the network</h3>
  {#if offered.length > 0}
    <ul class="found">
      {#each offered as server (server.id)}
        <li>
          <strong>{server.name}</strong>
          <span class="dim">{server.address}:{server.port}</span>
          {#if server.version}<span class="dim">v{server.version}</span>{/if}
          <button onclick={() => add(server)} disabled={locked}>Read along</button>
        </li>
      {/each}
    </ul>
  {:else if discoveryError}
    <p class="error">Looking for other instances failed: {discoveryError}</p>
  {:else}
    <p class="dim">
      Looking … An instance shows up here while its Remote client is serving on this network.
    </p>
  {/if}

  <!-- By hand, because multicast does not cross a VLAN and some house
       networks filter it outright. -->
  <div class="manual">
    <label>
      <span>Address</span>
      <input
        class="address"
        type="text"
        placeholder="192.168.1.20"
        bind:value={address}
        onkeydown={(e) => e.key === "Enter" && address.trim() && probe()}
        autocomplete="off"
        spellcheck="false"
      />
    </label>
    <label>
      <span>Port</span>
      <input type="number" min="1" max="65535" bind:value={port} />
    </label>
    <button onclick={probe} disabled={locked || probing || address.trim() === ""}>
      {probing ? "Asking …" : "Add"}
    </button>
  </div>
  {#if probeError}
    <p class="error">{probeError}</p>
  {/if}

  <p class="warn">Plain http: use it only on a network you control.</p>
</section>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  h3 {
    margin: 0.5rem 0 0;
    font-size: 0.85rem;
    font-weight: 600;
  }

  label {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-size: 0.85rem;
  }

  label > span:first-child {
    min-width: 4.5rem;
    color: var(--text-dim);
  }

  .switch,
  .channel {
    gap: 0.4rem;
  }

  .switch > span,
  .channel > span {
    min-width: 0 !important;
    color: var(--text) !important;
  }

  .source {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    background: var(--bg);
  }

  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.6rem;
    font-size: 0.85rem;
  }

  .row .switch {
    margin-left: auto;
  }

  .dot {
    font-size: 0.7rem;
    color: var(--text-dim);
  }

  .dot.connected {
    color: #5fd38d;
  }

  .dot.needsKey,
  .dot.connecting {
    color: #e0b341;
  }

  .dot.lost {
    color: #ff6b6b;
  }

  .channels {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 1rem;
  }

  .swatch {
    width: 0.7rem;
    height: 0.7rem;
    border-radius: 50%;
    background: var(--channel);
    flex: none;
  }

  .found {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  .found li {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.6rem;
    font-size: 0.85rem;
  }

  .found button {
    margin-left: auto;
  }

  .manual {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.75rem;
  }

  input[type="text"],
  input[type="number"] {
    font: inherit;
    font-size: 0.85rem;
    padding: 0.3rem 0.4rem;
    border-radius: 0.3rem;
    border: 1px solid var(--border);
    background-color: var(--bg);
    color: var(--text);
  }

  input[type="number"] {
    width: 6rem;
  }

  .address {
    width: 10rem;
  }

  .key {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    width: 14rem;
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

  button:disabled {
    opacity: 0.5;
    cursor: default;
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
</style>

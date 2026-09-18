<script lang="ts">
  // A group in a window of its own (P7).
  //
  // A second WebView on the same frontend, not a second app: it starts no
  // capture and owns no worker pool, so Start and Stop stay global and a group
  // window costs the machine a view rather than a second transcription of the
  // same audio. Everything it shows — the history it opens with, the
  // transcripts, the meters, the session state — comes from the shared backend.
  //
  // Which group it is comes from the window's own label, so it is known before
  // the first paint; who is in that group comes from the configuration file,
  // re-read on every `config-changed`, which is how a membership change made on
  // the Settings page reaches a window that is already open.
  //
  // Closing it closes this view and nothing else: never the group, never its
  // history, never the session.

  import ChatList from "./ChatList.svelte";
  import StreamMeters from "./StreamMeters.svelte";
  import { visibleStreams, type Group } from "./groups";
  import { fromStoredConfig } from "./profiles";
  import { mergeHistory, toBubbles } from "./bubbles";
  import { attentionFor } from "./attention";
  import { canPin, setWindowOnTop, setWindowTitle } from "./windows";
  import type { Channel } from "./streams";
  import {
    loadConfig,
    onCleared,
    onConfigChanged,
    onReconnected,
    onLevels,
    onOverload,
    onSessionState,
    onTranscript,
    requestAttention,
    requestGroupOnTop,
    sessionSnapshot,
    type SessionState,
    type StoredConfig,
    type StreamLevel,
    type TranscriptEvent,
  } from "./bridge";

  interface Props {
    /// The group's position in the active profile — the same position the
    /// configuration file stores membership by. Read from the window label;
    /// see `windows.ts`.
    group: number;
  }

  let { group }: Props = $props();

  // Every channel of the active profile, never only this group's: the stream
  // id is the position in this list, exactly as the backend assigns it, and a
  // pre-filtered list would point every id at the wrong channel.
  let channels = $state<Channel[]>([]);
  let target = $state<Group | null>(null);
  // Whether a configuration has been read at all. Without it the window cannot
  // tell "this group was deleted" from "the file has not arrived yet", and it
  // would spend its first moment claiming the group is gone.
  let known = $state(false);
  let failure = $state<string | null>(null);
  /// Whether this window stays in front of other windows.
  ///
  /// The window's own state, not a mirror of the configuration. It is born
  /// with the group's stored setting — the backend applies that when it opens
  /// the window, and `adopt` reads the same value once — and from then on the
  /// pin in this header is the only thing that changes it, so nothing is
  /// gained by following the file and a save landing mid-click would undo a
  /// press. The main window stores it, so the next window opens the same way.
  let onTop = $state(false);
  let adopted = false;

  let messages = $state<TranscriptEvent[]>([]);
  let sessionStarts = $state(new Map<number, number>());
  let levels = $state(new Map<number, StreamLevel>());
  let overloaded = $state(new Map<number, number>());
  let status = $state<SessionState | { state: "idle" }>({ state: "idle" });

  let running = $derived(status.state === "running");
  let colors = $derived(new Map(channels.map((c, streamId) => [streamId, c.color])));
  // The stream ids this window shows. An empty set for a group with no members
  // — which is a group to fill in on the Settings page, not a reason to fall
  // back to showing everything.
  let visible = $derived(target === null ? new Set<number>() : visibleStreams(target, channels)!);
  let shown = $derived(messages.filter((m) => visible.has(m.streamId)));
  // While a session runs this is the backend's own answer rather than the UI's
  // copy of the setup — the authoritative mapping, and the two would differ the
  // moment the device is swapped under a running session. Only meaningful
  // while the meters are on screen, which is only while it runs.
  let muted = $derived(
    status.state === "running"
      ? new Set(status.streams.filter((s) => s.muted).map((s) => s.streamId))
      : new Set<number>(),
  );

  // This window's own chat, grouped the way it renders it — which is what an
  // attention request is counted per. A group window's bubbles are not the main
  // window's: a channel outside the group does not interrupt a bubble here.
  let bubbles = $derived(toBubbles(shown));

  /// Reads the group out of a stored configuration.
  ///
  /// Always the *active* profile: the stream ids the transcripts carry are
  /// positions in that profile's channel list, so a window showing another
  /// profile's group would be filtering the right numbers against the wrong
  /// channels. The main window closes these windows when the active profile
  /// changes, and this is the other half of that agreement.
  const apply = (stored: StoredConfig) => {
    const { profiles, activeKey } = fromStoredConfig(stored);
    const profile = profiles.find((p) => p.key === activeKey) ?? null;
    channels = profile?.channels ?? [];
    target = profile?.groups[group] ?? null;
    known = true;
    // The backend titled this window when it opened it; a rename afterwards
    // arrives here and nowhere else.
    if (target) setWindowTitle(`${target.name} — Intercom2Chat`);

    // The stored setting is what this window is *born* with, read once. The
    // backend already applied it when it built the window; this is what keeps
    // the pin in the header showing the truth.
    if (!adopted && target) {
      adopted = true;
      onTop = target.onTop;
    }
  };

  /// Pins this window in front of other windows, or lets it back into the
  /// stack.
  ///
  /// Applied here and asked to be remembered there: the window is this
  /// window's to place, and the configuration has one writer. See
  /// `requestGroupOnTop`.
  const pin = () => {
    onTop = !onTop;
    setWindowOnTop(onTop);
    requestGroupOnTop(group, onTop).catch((e) =>
      console.warn("the window setting was not stored:", e),
    );
  };

  $effect(() => {
    // Subscribed before the history is asked for. The other order drops
    // whatever is said during the round trip — which is the moment a group
    // window is opened, because something is happening. `mergeHistory` is what
    // makes the overlap harmless.
    const subscriptions = [
      onTranscript((event) => {
        messages = [...messages, event];
        if (visible.has(event.streamId)) attend(event);
      }),
      onLevels((event) => {
        levels = new Map(event.streams.map((s) => [s.streamId, s]));
      }),
      onOverload((event) => {
        overloaded = new Map(overloaded).set(event.streamId, event.dropped);
      }),
      onSessionState((event) => {
        status = event;
        if (event.state === "running") {
          sessionStarts = new Map(sessionStarts).set(event.sessionId, event.startedAtMs);
        }
      }),
      onCleared(() => {
        // The running session's zero point survives on the backend and comes
        // back with the next snapshot; here it is simply not thrown away,
        // because the lines arriving from now on are still on its clock.
        messages = [];
        sessionStarts =
          status.state === "running"
            ? new Map([[status.sessionId, status.startedAtMs]])
            : new Map();
        overloaded = new Map();
      }),
      onConfigChanged(apply),
    ];

    Promise.all(subscriptions).catch((e) => {
      failure = `event bridge unavailable: ${e}`;
    });

    const first = async () => {
      apply(await loadConfig());

      const snapshot = await sessionSnapshot();
      messages = mergeHistory(snapshot.transcripts, messages);
      // Merged rather than replaced: a `session-state` event may already have
      // arrived, and it is the newer of the two.
      sessionStarts = new Map([
        ...snapshot.sessions.map((s): [number, number] => [s.sessionId, s.startedAtMs]),
        ...sessionStarts,
      ]);
      if (status.state === "idle" && snapshot.state) status = snapshot.state;
    };

    first().catch((e) => (failure = String(e)));

    // A remote view that lost the socket missed everything said while it was
    // gone, and nothing on the wire can tell it what. So it opens again: the
    // same configuration read and the same snapshot, merged into what it still
    // holds. In the app this never fires — an `invoke` has no connection to
    // lose.
    const reconnected = onReconnected(() => {
      first().catch((e) => (failure = String(e)));
    });

    return () => {
      reconnected();
      for (const subscription of subscriptions) {
        subscription.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  });

  /// Asks this window to be looked at (P17).
  ///
  /// Narrower than the main window's on both counts, and deliberately: this is
  /// a view of one group, not of the session. An alert term on a channel that
  /// is not a member is somebody else's business — the main window sees every
  /// one of them. The flash-on-message setting, on the other hand, always
  /// applies here: this window *is* that group's view, so there is no "while it
  /// is selected" to qualify it.
  const attend = (event: TranscriptEvent) => {
    const asking = attentionFor(event, bubbles[bubbles.length - 1], {
      alerts: visible,
      flash: target?.flash === true ? visible : null,
    });
    if (!asking) return;
    requestAttention(asking.urgent).catch((e) => console.warn("no attention request:", e));
  };
</script>

<main>
  <header class="bar">
    <h1>{target?.name ?? "Group"}</h1>
    <span class="dots">
      {#each channels as channel, streamId}
        {#if visible.has(streamId)}
          <span class="dot" style="background: {channel.color}" title={channel.label}></span>
        {/if}
      {/each}
    </span>

    <span class="right">
      <!-- The window's own setting, on the window it is about: during a show
           the question is "keep this one where I can see it", and that is
           asked of the window in front of the operator, not of a page in
           another one. It is remembered per group, so the next time this group
           is opened it comes up the same way.

           Only where there are windows to be in front of (P16). Hidden rather
           than disabled in a browser tab: a control that cannot do anything is
           worse than no control, and there is nothing here to fix. -->
      {#if canPin}
        <button
          class="pin"
          class:on={onTop}
          onclick={pin}
          aria-pressed={onTop}
          title={onTop
            ? "In front of other windows — click to let it back into the stack"
            : "Keep this window in front of other windows"}
        >
          <!-- Drawn rather than an emoji: an emoji is somebody else's colour
               and somebody else's weight at every font size, and this sits in a
               row of dim monochrome text. `currentColor` means it is lit and
               dimmed by the same rule as everything beside it. -->
          <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor" aria-hidden="true">
            <path
              d="M10.23 1.55a.75.75 0 0 0-1.06 0L8.11 2.61l5.28 5.28 1.06-1.06a.75.75 0 0 0 0-1.06zM7.05 3.67 2.68 5.16a.75.75 0 0 0-.29 1.24l1.95 1.95-3.09 4.55a.3.3 0 0 0 .41.41l4.55-3.09 1.95 1.95a.75.75 0 0 0 1.24-.29l1.49-4.37z"
            />
          </svg>
        </button>
      {/if}

      {#if failure}
        <span class="error">{failure}</span>
      {:else if status.state === "running"}
        <span class="ok">●</span>
        <span class="dim">
          {visible.size}
          {visible.size === 1 ? "channel" : "channels"}
        </span>
      {:else if status.state === "failed"}
        <span class="error">{status.message}</span>
      {:else}
        <!-- Start lives in the main window and only there: a session is
             session-wide, and a button here would be a second way to do one
             thing. -->
        <span class="dim">Not running</span>
      {/if}
    </span>
  </header>

  {#if known && target === null}
    <p class="empty">
      This group is no longer in the active profile. Close this window, or make the
      group again on the Settings page.
    </p>
  {:else if known && visible.size === 0}
    <p class="empty">This group has no channels yet. Add some on the Settings page.</p>
  {:else}
    {#if running}
      <StreamMeters {channels} {levels} {overloaded} {visible} {muted} />
    {/if}
    <ChatList
      messages={shown}
      {colors}
      starts={sessionStarts}
      flash={target?.flash === true}
    />
  {/if}
</main>

<style>
  main {
    display: flex;
    flex-direction: column;
    height: 100vh;
    position: relative;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.5rem 0.9rem;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    font-size: 0.85rem;
  }

  h1 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    letter-spacing: -0.01em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* The member channels' colours, the same way the sidebar shows them — this
     window is one of those rows, opened. */
  .dots {
    display: flex;
    gap: 0.15rem;
  }

  .dot {
    width: 0.45rem;
    height: 0.45rem;
    border-radius: 50%;
  }

  .right {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin-left: auto;
  }

  /* Dim until it is on, like every other "this is switched on" state here:
     a pin that looks the same pressed and unpressed is a pin nobody trusts. */
  .pin {
    display: flex;
    align-items: center;
    background: transparent;
    color: var(--text-dim);
    border: 1px solid transparent;
    border-radius: 0.3rem;
    padding: 0.2rem 0.3rem;
    font: inherit;
    line-height: 0;
    cursor: pointer;
  }

  .pin:hover {
    color: var(--text);
  }

  .pin.on {
    color: var(--accent);
    background: var(--bg);
    border-color: var(--border);
  }

  .empty {
    color: var(--text-dim);
    font-size: 0.85rem;
    padding: 1rem;
  }

  .dim {
    color: var(--text-dim);
  }

  .ok {
    color: #5fd38d;
  }

  .error {
    color: #ff6b6b;
  }
</style>

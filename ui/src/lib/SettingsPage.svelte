<script module lang="ts">
  export type Section =
    | "profiles"
    | "audio"
    | "listen"
    | "shoutbox"
    | "groups"
    | "rules"
    | "models"
    | "detection"
    | "vocabulary"
    | "remote"
    | "view";

  /// How urgently a section wants attention, shown as a dot on its rail item.
  ///
  /// The point of the dot is that a problem buried three sections down used to
  /// be invisible until it was scrolled to — an unplugged interface and a
  /// machine with no model both look, from the chat, like an app that will not
  /// start for no reason.
  export type Flag = "warn" | "error" | null;

  /// Which section was open last time Settings was left.
  ///
  /// Module-level rather than component state because `App.svelte` unmounts
  /// the dialog whenever it is closed: an operator who shuts it to check a
  /// level and opens it again should land where they were, not at the top of
  /// the first section again.
  let lastSection: Section = "audio";
</script>

<script lang="ts">
  // Settings: everything about how the session is *configured*, in a dialog
  // over the live view rather than in place of it — the chat keeps running
  // behind it, which is the point of doing this during a show at all.
  //
  // The page is split in two: **General** — the models, the vocabulary, the
  // view and the listener, which stay behind when a setup travels — and
  // **Profile**, the setup itself: which profile is selected, its device and
  // channel map, its groups and its rules. Two headings and the section names
  // under them, nothing else: a section that needs explaining explains itself,
  // inside itself, where the controls it is talking about are.
  //
  // The profiles are a section like the others rather than a strip that was on
  // screen whichever section was open. Picking a setup is the rarest thing
  // done here and it was sitting in front of the most frequent ones, with
  // Delete under the cursor all evening.
  //
  // What is deliberately not here is Start/Stop: it is session-wide and stays
  // in the header. The dialog is modal and covers that header, so the way to
  // stop capture from inside it is ⌘T, which the window's own handler answers
  // whatever is on screen — and failing that, Escape, which is one press and
  // puts the header back.

  import ProfileSection from "./ProfileSection.svelte";
  import StreamSetup from "./StreamSetup.svelte";
  import VocabularyEditor from "./VocabularyEditor.svelte";
  import RuleEditor from "./RuleEditor.svelte";
  import GroupMembership from "./GroupMembership.svelte";
  import ModelPicker from "./ModelPicker.svelte";
  import RemoteSettings from "./RemoteSettings.svelte";
  import ViewSettings from "./ViewSettings.svelte";
  import DetectionSettings from "./DetectionSettings.svelte";
  import ListenSettings from "./ListenSettings.svelte";
  import ShoutboxSettings from "./ShoutboxSettings.svelte";
  import { lengthWarning } from "./vocabulary";
  import { connectedDevice, type ModelChoice, type Profile } from "./profiles";
  import type { SortDirection } from "./sort";
  import {
    isRemote,
    listDevices,
    listLanguages,
    type DeviceOption,
    type Language,
    type StoredRemote,
    type SpeechEngine,
    type StreamLevel,
    type Tuning,
  } from "./bridge";
  import { onMount } from "svelte";
  import { watchNarrow } from "./layout";

  interface Props {
    /// The input devices. Bindable because Refresh devices, here and in the
    /// Output section, lists them again for the whole app — a device plugged
    /// in after startup is then one Start can open, not only one to pick.
    devices: DeviceOption[];
    profiles: Profile[];
    activeKey: number | null;
    vocabulary: string;
    /// Which model the machine transcribes with (P11). Beside the profiles
    /// rather than inside one: what a machine can run depends on the machine,
    /// not on the production being carried to it.
    models: ModelChoice;
    /// Whether this machine serves the browser peer, and on which address
    /// (P16). Beside the models rather than inside a profile, and for the
    /// models' reason: which network this machine is on belongs to the
    /// machine, not to the production carried to it.
    remote: StoredRemote;
    /// Which end of the chat new lines arrive at (P7). With the vocabulary in
    /// scope rather than with the models: how a house reads its chat outlives
    /// any one patch, and it is not a property of the machine either — every
    /// view the remote serves opens the way this says.
    sort: SortDirection;
    /// How speech is found and where a segment is cut (P12). In the General
    /// half with the vocabulary, and for its reason: how hard the intercom is
    /// to hear belongs to the house, not to the production passing through
    /// it.
    tuning: Tuning;
    deviceChannels: number;
    /// Whether a session is starting — or restarting to take on an edit. The
    /// only time the setup is not editable: what is being opened is read from
    /// it, and an edit made in that moment would be one it does not have.
    locked: boolean;
    /// Whether the running session is about to be restarted to take on a
    /// change only a restart can apply — the profile, the device, the channel
    /// map or the model. Said on the page, because the restart is a second or
    /// two of audio that is not transcribed, and nobody should hear about it
    /// from the gap in the chat.
    pending: boolean;
    /// Whether capture is running. Narrower than `locked`, which also covers
    /// a session that is starting: the Detection section says whether a
    /// change reaches the audio *now*, and during a start there is nothing
    /// for it to reach yet.
    running: boolean;
    saveError: string | null;
    /// Why a change to the running session was refused, or `null`. Its own
    /// strip and not `saveError`: "Settings not saved" would be wrong twice
    /// over — the file was written, and what failed was the live session.
    liveError: string | null;
    /// Pushes a channel's label, language, input trim or mute into the
    /// running session (P6), or `null` when none is running. Handed down to
    /// the channel list, which is where those four fields are. See
    /// `StreamSetup`.
    live:
      | ((
          streamId: number,
          update: {
            label?: string;
            language?: string;
            gainDb?: number;
            muted?: boolean;
          },
        ) => void)
      | null;
    /// The levels to show beside the channels. Empty while nothing is
    /// arriving — between a session and the monitor, or on a device that is
    /// not plugged in — rather than `null`: a bar that disappears and comes
    /// back moves the whole row, and a still bar already says what a silent
    /// line looks like. `null` is for a view that never meters at all.
    /// Handed down to the channel list for the same reason `live` is: that is
    /// where the trim is, and a trim is set against a meter.
    levels?: Map<number, StreamLevel> | null;
    /// Why the input could not be metered while no session runs (P8), or
    /// `null`. Its own line rather than `liveError`: nothing was refused and
    /// no session was touched — the bars are simply not going to move, and an
    /// operator waiting for them has to be told why.
    meterError?: string | null;
    /// Why the session would refuse to start, or `null` — the same string the
    /// status bar shows. Every one of them is about the device or the channel
    /// map, which is what marks the Audio section rather than the page.
    problem: string | null;
    /// Opens the first-run setup guide again, or `null` in a view that does
    /// not have one. It walks the models, the channel map and the groups in
    /// the order a machine needs them — which is worth reaching a second time,
    /// by somebody setting up a new house rather than a new machine.
    openGuide?: (() => void) | null;
    /// Closes the dialog. Called by the Close button, by Escape and by a
    /// click on the scrim — the three ways every dialog on this machine is
    /// left, and an operator should not have to find out which one this one
    /// answers to.
    onclose: () => void;
  }

  let {
    devices = $bindable(),
    profiles = $bindable(),
    activeKey = $bindable(),
    vocabulary = $bindable(),
    models = $bindable(),
    remote = $bindable(),
    sort = $bindable(),
    tuning = $bindable(),
    deviceChannels,
    locked,
    pending,
    running,
    saveError,
    liveError,
    live,
    levels = null,
    meterError = null,
    problem,
    openGuide = null,
    onclose,
  }: Props = $props();

  let section = $state<Section>(lastSection);
  const show = (next: Section) => {
    section = next;
    lastSection = next;
    // On a narrow window the rail is a drawer over the section it opens onto,
    // so choosing is also the end of the errand it was opened for. On the desk
    // this says nothing: the rail is a column and is never open or shut.
    railOpen = false;
  };

  /// Whether the window is too narrow for a rail beside the sections. The
  /// chat's sidebar asks the same question of the same breakpoint — see
  /// `layout.ts`, which is where the answer lives so that the two cannot
  /// disagree at some width in between.
  let narrow = $state(false);
  $effect(() =>
    watchNarrow((next) => {
      narrow = next;
      // Grown back to desk width: the rail is a column again, and a drawer
      // left open would be waiting the next time the window is made small.
      if (!next) railOpen = false;
    }),
  );

  /// Whether that rail is currently drawn over the section.
  let railOpen = $state(false);

  // Mutated in place rather than reassigned: it is an element of the
  // `profiles` array, so writing to its fields is what makes the edit part of
  // the profile — and part of what gets saved.
  let active = $derived(profiles.find((p) => p.key === activeKey) ?? null);

  // A stored device that is not plugged in right now. Kept and shown rather
  // than silently replaced with whatever is connected: the profile describes
  // the production, and an operator who opens the app in the office without
  // the desk must find their setup intact, not quietly re-pointed at the
  // built-in microphone.
  let missing = $derived(
    active !== null && active.device !== "" && connectedDevice(devices, active) === undefined,
  );

  let listError = $state<string | null>(null);
  const refreshDevices = async () => {
    try {
      devices = await listDevices();
      listError = null;
    } catch (e) {
      listError = String(e);
    }
  };

  // Picking a device sets both halves at once. The id is what the profile is
  // keyed on and what starting a session sends; the name is stored with it so
  // this device can still be named on a machine it is not plugged into.
  const pickDevice = (id: string) => {
    const option = devices.find((d) => d.id === id);
    // No option means the "not connected" entry, which is the profile's own
    // device: re-picking it is not a change, and must not clear the id.
    if (!option || active === null) return;
    active.device = option.name;
    active.deviceId = option.id;
  };

  // Reported upwards by the model manager, which is the only thing that knows
  // whether this machine can transcribe at all. Nothing else on the page can
  // work it out — it is a question about files on disk, not about the setup.
  let modelFlag = $state<Flag>(null);
  // Also from the model manager: which recogniser runs, because Parakeet
  // ignores the vocabulary and that page should say so.
  let modelEngine = $state<SpeechEngine | null>(null);

  // The sidecar's language list, fetched once for the whole page and handed to
  // every picker on it. `null` until it arrives, and it stays `null` if the
  // call fails — which is a state the picker has to render rather than an
  // error the page reports, because a missing list costs discoverability and
  // not the ability to set a language.
  //
  // Not fetched in `App`: nothing outside this page asks what languages exist.
  let languages = $state<Language[] | null>(null);

  onMount(async () => {
    try {
      languages = await listLanguages();
    } catch (e) {
      // Worth a line in the console, not a strip on the page: the fallback is
      // the field this replaced, and an operator who never opens the picker
      // never notices.
      console.warn("no language list from the sidecar:", e);
    }
  });

  let flags = $derived<Record<Section, Flag>>({
    // Never flagged: a failed save is about the whole file, not about the
    // profile list, and it is said once in the strip at the top of the page.
    profiles: null,
    audio: problem === null ? null : "warn",
    // Never flagged: a replay with no output of its own plays on the system
    // output, which is a choice rather than a problem.
    listen: null,
    // Never flagged: the shoutbox reports its own trouble inside the section,
    // and it is off on most machines.
    shoutbox: null,
    groups: null,
    // Never flagged: an empty rule list is the normal state, not a problem.
    rules: null,
    vocabulary: lengthWarning(vocabulary) === null ? null : "warn",
    // Never flagged: the listener reports its own trouble inside the section,
    // where the address that caused it is. A dot on the rail would be pointing
    // at a setting that is off on most machines and fine on the rest.
    remote: null,
    models: modelFlag,
    // Never flagged: there is no wrong setting here, only one that suits the
    // house better or worse, and the operator finds that out by listening
    // rather than by being told.
    detection: null,
    // Never flagged: a reading direction cannot be wrong, and the window it
    // applies to is where it is seen.
    view: null,
  });

  // The rail, in two groups: what belongs to this machine and to every setup
  // first, then what is carried with the profile that is selected. Names
  // only — a section that needs explaining explains itself, inside itself.
  let rail = $derived<{ scope: string; items: { id: Section; title: string }[] }[]>([
    {
      scope: "General",
      items: [
        { id: "models", title: "Models" },
        { id: "detection", title: "Detection" },
        { id: "vocabulary", title: "Vocabulary" },
        { id: "view", title: "View" },
        // Only in the app. The remote's own Settings page shows everything
        // else, but the listener it is connected through is the host machine's
        // — and a browser switching off the thing serving it is a page pulling
        // the floor out from under itself.
        ...(isRemote ? [] : [{ id: "remote" as Section, title: "Remote client" }]),
      ],
    },
    {
      scope: "Profile",
      items: [
        { id: "profiles", title: "Profiles" },
        // The input device and the channel map on it.
        { id: "audio", title: "Channels" },
        // Under the channels, because a group is a set of them.
        { id: "groups", title: "Groups" },
        // Two sections and not one: the replay headphones and the talkback
        // speaker are two different jobs, set up by different people, and the
        // shoutbox alone is a long page (P22, P26).
        { id: "listen", title: "Listen" },
        { id: "shoutbox", title: "Shoutbox" },
        // With the profile, because a trigger names channels and groups and
        // neither exists outside one.
        { id: "rules", title: "Rules" },
      ],
    },
  ]);

  /// What the burger is labelled with: where the operator is, so the one line
  /// the rail is reduced to still says which of eight sections is open. Below
  /// `rail` because it reads it.
  let here = $derived(
    rail.flatMap((group) => group.items).find((item) => item.id === section)?.title ?? "Settings",
  );
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key !== "Escape") return;
    // One thing at a time, outermost last: Escape with the drawer over the
    // section shuts the drawer, and only a second press leaves Settings. The
    // other way round it would take the page out from under somebody who was
    // aiming at the drawer.
    if (railOpen) railOpen = false;
    else onclose();
  }}
/>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="scrim" onclick={onclose}></div>

<div class="page" role="dialog" aria-modal="true" aria-labelledby="settings-title">
  <header class="title">
    <h1 id="settings-title">Settings</h1>
    <!-- Saving is the debounce's business and happens as things are typed, so
         there is no OK to pair this with: Close is the only way out and it
         does not decide anything. -->
    <button class="close" onclick={onclose} title="Close (Esc)">Close</button>
  </header>

  <!-- The one thing that is worth the top of the page whichever section is
       open. Never swallowed and never buried in the section that happens to
       have caused it: an operator who builds a setup believing it is stored
       and finds it gone before the next show has been failed twice. -->
  {#if saveError}
    <p class="failed">Settings not saved: {saveError}</p>
  {/if}

  {#if liveError}
    <!-- The setup is stored either way; what did not happen is the change
         reaching the session that is running, which is the half an operator
         mid-show is actually asking about. -->
    <p class="failed">The running session was not changed: {liveError}</p>
  {:else if pending}
    <p class="pending">
      Applying your change — the session restarts for a moment. The chat is kept.
    </p>
  {/if}

  <!-- On a narrow window the rail is not a column beside the sections but a
       drawer over them, and this is the line it is reduced to: the burger that
       opens it, and the name of the section that is open. The name is the half
       that matters — a burger alone would mean scrolling a form to work out
       which of eight it belongs to. -->
  {#if narrow}
    <div class="railbar">
      <button
        class="burger"
        onclick={() => (railOpen = !railOpen)}
        aria-expanded={railOpen}
        aria-label={railOpen ? "Close the list of settings sections" : "Settings sections"}
      >
        <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
          <path
            d="M2 4h12M2 8h12M2 12h12"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
          />
        </svg>
      </button>
      <span class="where">{here}</span>
    </div>
  {/if}

  <div class="body">
    {#if narrow && railOpen}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="backdrop" onclick={() => (railOpen = false)}></div>
    {/if}

    <nav
      class="rail"
      class:drawer={narrow}
      class:open={railOpen}
      inert={narrow && !railOpen}
      aria-label="Settings sections"
    >
      {#each rail as group (group.scope)}
        <div class="scope">
          <h2>{group.scope}</h2>
          {#each group.items as item (item.id)}
            <button
              class="item"
              class:active={section === item.id}
              aria-current={section === item.id ? "page" : undefined}
              onclick={() => show(item.id)}
            >
              <span>{item.title}</span>
              {#if flags[item.id]}
                <span class="flag {flags[item.id]}" aria-hidden="true">●</span>
              {/if}
            </button>
          {/each}
        </div>
      {/each}

      <!-- Under the sections rather than among them, because it is not one: it
           opens a dialog over the page and comes back here when it is done.
           At the foot of the rail for the same reason a preferences window
           keeps its odd one out there — it is reached on purpose, once, and it
           must not sit in the way of the eight things that are reached daily.
           Only where there is a guide to run: the remote does not have one. -->
      {#if openGuide}
        <div class="foot">
          <button
            class="item"
            onclick={() => {
              openGuide();
              // The drawer was opened to get here, and what it opened is now
              // standing over it.
              railOpen = false;
            }}
          >
            <span>Setup guide</span>
          </button>
        </div>
      {/if}
    </nav>

    <!-- Every section stays mounted and inactive ones are hidden, rather than
         being created when its item is clicked. A model download is minutes
         long and belongs to the section it was started from; a rail that
         unmounted it would throw away the progress bar of a transfer that is
         still running. -->
    <div class="pane">
      <div class="sheet" hidden={section !== "profiles"}>
        <ProfileSection bind:profiles bind:activeKey {locked} />
      </div>

      {#if active}
        <div class="sheet" hidden={section !== "audio"}>
          <section class="panel">
            <header>
              <h2>Input</h2>
              <button type="button" class="link" onclick={refreshDevices} title="Look for input devices again">
                Refresh devices
              </button>
            </header>
            <label>
              <span>Device</span>
<!-- Keyed and valued by the stable id, not the name: two identical
                   interfaces are two entries here, and they must not collapse
                   into one. `onchange` rather than `bind:` because a pick
                   writes both the id and the name. -->
              <select
                value={active.deviceId}
                disabled={locked}
                onchange={(e) => pickDevice(e.currentTarget.value)}
              >
                {#each devices as option (option.id)}
                  <option value={option.id}>{option.name} ({option.channels} ch)</option>
                {/each}
                {#if missing}
                  <!-- Selectable, because it is what the profile says and
                       deselecting it would throw that away. -->
                  <option value={active.deviceId}>{active.device} — not connected</option>
                {/if}
              </select>
            </label>
          </section>

          <StreamSetup
            bind:channels={active.channels}
            bind:language={active.language}
            {languages}
            {deviceChannels}
            {locked}
            {live}
            {levels}
          />

          {#if listError}
            <p class="failed">The audio devices could not be listed: {listError}</p>
          {/if}
          {#if meterError}
            <p class="failed">The input could not be metered: {meterError}</p>
          {/if}
        </div>

        <div class="sheet" hidden={section !== "listen"}>
          <ListenSettings bind:outputs={active.outputs} bind:inputDevices={devices} />
        </div>

        <div class="sheet" hidden={section !== "shoutbox"}>
          <ShoutboxSettings bind:outputs={active.outputs} bind:inputDevices={devices} />
        </div>

        <div class="sheet" hidden={section !== "groups"}>
          <GroupMembership bind:groups={active.groups} channels={active.channels} />
        </div>

        <div class="sheet" hidden={section !== "rules"}>
          <RuleEditor
            bind:rules={active.rules}
            channels={active.channels}
            groups={active.groups}
          />
        </div>
      {/if}

      <div class="sheet" hidden={section !== "vocabulary"}>
        <VocabularyEditor bind:vocabulary {locked} ignored={modelEngine === "parakeet"} />
      </div>

      <div class="sheet" hidden={section !== "models"}>
        <ModelPicker
          bind:models
          {locked}
          {running}
          onstatus={(flag) => (modelFlag = flag)}
          onengine={(engine) => (modelEngine = engine)}
        />
      </div>

      {#if !isRemote}
        <div class="sheet" hidden={section !== "remote"}>
          <RemoteSettings bind:remote />
        </div>
      {/if}

      <div class="sheet" hidden={section !== "detection"}>
        <DetectionSettings bind:tuning {running} />
      </div>

      <div class="sheet" hidden={section !== "view"}>
        <ViewSettings bind:sort />
      </div>
    </div>
  </div>
</div>

<style>
  /* Over the window, including the header, the way the setup guide's is —
     except that this one is dismissed by a click, because everything behind
     it is worth reaching and nothing here has to be finished. Stop is behind
     it for as long as it is open, which is what ⌘T and Escape are for. */
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 30;
  }

  /* One size whichever section is open, and `height` rather than
     `max-height`, for the guide's reason: a dialog that resized itself around
     Models and then around the channel list would move the rail out from
     under the cursor on every pick. Wide enough for the channel rows, which
     carry a meter, a trim knob and a language picker beside the 14 rem rail;
     capped by the viewport, so on a phone it is the screen with a hairline of
     scrim around it. */
  .page {
    position: fixed;
    z-index: 31;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(64rem, calc(100vw - 2rem));
    height: min(44rem, calc(100dvh - 2rem));
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    box-shadow: 0 1.5rem 3rem rgba(0, 0, 0, 0.45);
  }

  /* The dialog's own bar, in the shape of the window's: what is on screen is
     said on the left and the way out is on the right, which is where every
     other Close in this app is. */
  .title {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex: none;
    padding: 0.6rem 1rem;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  .title h1 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
  }

  .close {
    margin-left: auto;
    background: transparent;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.25rem 0.7rem;
    font: inherit;
    font-size: 0.85rem;
    cursor: pointer;
  }

  .close:hover {
    color: var(--text);
  }

  .link {
    margin-left: auto;
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    font-size: 0.8rem;
    color: var(--text-dim);
    text-decoration: underline;
    cursor: pointer;
  }

  .body {
    display: flex;
    flex: 1;
    min-height: 0;
    /* What the rail's drawer is positioned against on a narrow window: the
       sections, and not the bar above them that says which one is open. */
    position: relative;
  }

  /* A strip in the shape of the two bars above it rather than a card, so it
     reads as part of the window's status furniture and not as one more thing
     on the page to configure. */
  .failed,
  .pending {
    margin: 0;
    padding: 0.4rem 1rem;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    color: #ff6b6b;
    font-size: 0.8rem;
  }

  /* Not a failure: nothing went wrong, the session is about to be told. */
  .pending {
    color: var(--text-dim);
  }

  /* --- The rail ---------------------------------------------------------

     Vertical rather than a row of tabs across the top: only a vertical list
     has room for the two headings the sections sit under, and a row of eight
     names directly under the dialog's own title bar reads as a second title
     bar. */
  .rail {
    flex: none;
    width: 14rem;
    border-right: 1px solid var(--border);
    background: var(--surface);
    padding: 0.8rem 0.6rem;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1.1rem;
  }

  .scope h2 {
    margin: 0 0 0.35rem;
    font-size: 0.68rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }

  .item {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    width: 100%;
    text-align: left;
    background: transparent;
    color: var(--text-dim);
    border: 1px solid transparent;
    border-radius: 0.3rem;
    padding: 0.3rem 0.5rem;
    font: inherit;
    font-size: 0.9rem;
    cursor: pointer;
  }

  .item:hover {
    color: var(--text);
  }

  /* Tinted and edged rather than boxed: a box in the surface colour with a
     hairline is what every input on this page looks like, and a rail item that
     borrowed it read as a field to type in. */
  .item.active {
    background: rgba(110, 168, 254, 0.12);
    box-shadow: inset 2px 0 0 var(--accent);
    color: var(--text);
    font-weight: 600;
  }

  .flag {
    margin-left: auto;
    font-size: 0.6rem;
    line-height: 1;
  }

  .flag.warn {
    color: #f2b544;
  }

  .flag.error {
    color: #ff6b6b;
  }

  /* --- The pane ---------------------------------------------------------

     One section at a time, so the gap between cards only ever separates
     things that belong to the same question. The width is capped because a
     settings form on a 27" display that runs the whole way across is a form
     nobody can follow a row of. */
  .pane {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 0.9rem 1rem 1.5rem;
  }

  .sheet {
    display: flex;
    flex-direction: column;
    gap: 0.9rem;
    max-width: 52rem;
  }

  .sheet[hidden] {
    display: none;
  }

  /* Pushed to the bottom of the rail and ruled off from the sections above
     it, so it reads as the thing that is not one of them. */
  .foot {
    margin-top: auto;
    padding-top: 0.6rem;
    border-top: 1px solid var(--border);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: 0.7rem;
    color: var(--text-dim);
    max-width: 28rem;
  }

  select {
    background-color: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 1.5rem 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.9rem;
  }

  /* --- The rail as a drawer (narrow) -------------------------------------

     A narrow window — a phone on the LAN remote (P16), or the app beside a
     cue sheet on half a screen — has no room for a 14 rem column beside a
     form that is already as narrow as it can usefully be.

     This used to wrap the rail into a row across the top. That works at a
     laptop's half-width and not at a phone's: eight section names and two
     headings wrap to three or four lines there, which is more of the screen
     than the column ever took, and it is above the form permanently rather
     than for the moment it is being used. A drawer costs one line — the
     burger and the name of the section that is open — and gives the rest
     back to the section.

     The same shape as the chat's sidebar, deliberately: two sidebars that
     behave differently on the same phone are two things to learn. */
  .railbar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex: none;
    padding: 0.4rem 0.7rem;
    padding-left: max(0.7rem, var(--safe-left));
    padding-right: max(0.7rem, var(--safe-right));
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  .burger {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: none;
    background: transparent;
    color: var(--text-dim);
    border: 1px solid transparent;
    border-radius: 0.3rem;
    padding: 0.3rem 0.4rem;
    font: inherit;
    line-height: 0;
    cursor: pointer;
  }

  .burger[aria-expanded="true"] {
    color: var(--accent);
    background: var(--bg);
    border-color: var(--border);
  }

  /* Where the operator is. The brightest thing on the line, because it is the
     whole reason the line is there. */
  .where {
    font-size: 0.85rem;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .rail.drawer {
    position: absolute;
    z-index: 20;
    top: 0;
    bottom: 0;
    left: 0;
    width: min(15rem, 76vw);
    padding-bottom: max(0.8rem, var(--safe-bottom));
    padding-left: max(0.6rem, var(--safe-left));
    box-shadow: 0 0 2rem rgba(0, 0, 0, 0.5);
    transform: translateX(-100%);
    transition: transform 160ms ease-out;
  }

  .rail.drawer.open {
    transform: translateX(0);
  }

  /* Closed: inert, so nothing in it is tabbed into or read out from off
     screen, and hidden once the slide has finished so a horizontal scroll
     cannot find it beside the window. */
  .rail.drawer[inert] {
    visibility: hidden;
    transition:
      transform 160ms ease-out,
      visibility 0s linear 160ms;
  }

  .backdrop {
    position: absolute;
    inset: 0;
    z-index: 15;
    background: rgba(0, 0, 0, 0.45);
  }

  @media (prefers-reduced-motion: reduce) {
    .rail.drawer,
    .rail.drawer[inert] {
      transition: none;
    }
  }

  @media (max-width: 45rem) {
    /* The whole screen, with no corners and no scrim showing round the edge:
       at this width the dialog is not something standing on the page, it is
       where the operator is until they close it. */
    .page {
      width: 100vw;
      height: 100dvh;
      border: none;
      border-radius: 0;
    }

    .title {
      padding: 0.45rem 0.7rem;
      padding-top: max(0.45rem, var(--safe-top));
      padding-left: max(0.7rem, var(--safe-left));
      padding-right: max(0.7rem, var(--safe-right));
    }

    /* Rows a thumb can hit, and a form that uses the width it was given back
       by the rail leaving the row. */
    .item {
      padding: 0.5rem 0.5rem;
      font-size: 0.95rem;
    }

    .pane {
      padding: 0.7rem 0.7rem 1.5rem;
      padding-left: max(0.7rem, var(--safe-left));
      padding-right: max(0.7rem, var(--safe-right));
      padding-bottom: max(1.5rem, var(--safe-bottom));
    }

    .failed,
    .pending {
      padding: 0.4rem 0.7rem;
      padding-left: max(0.7rem, var(--safe-left));
      padding-right: max(0.7rem, var(--safe-right));
    }

    /* A field is as wide as the phone: the 28 rem cap keeps a label from
       running the width of a desk display, and there is nothing here to
       cap. */
    label {
      max-width: none;
    }
  }
</style>

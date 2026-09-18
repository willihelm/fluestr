<script module lang="ts">
  export type Section =
    | "profiles"
    | "audio"
    | "groups"
    | "vocabulary"
    | "alerts"
    | "models";

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
  /// this page whenever the chat is shown: an operator who flips to the chat
  /// to check a level and comes back should land where they were, not at the
  /// top of the first section again.
  let lastSection: Section = "audio";
</script>

<script lang="ts">
  // Settings: everything about how the session is *configured*, away from the
  // live view.
  //
  // The page is split by **scope**, not by topic, because scope is the thing
  // an operator gets wrong. Three questions here have three different answers
  // to "what does changing this affect":
  //
  // - the device, the channel map and the groups belong to the *profile* — one
  //   production's setup, carried between houses;
  // - the vocabulary is shared by every profile, because the jargon of a house
  //   outlives any one patch;
  // - the installed models belong to the *machine*, because what can be run
  //   depends on the laptop, not on the production being carried to it.
  //
  // As one scrolling column all three looked alike, and the only hint of the
  // difference was a paragraph of prose in the profile card. The rail on the
  // left says it instead: each section sits under the heading of the thing it
  // belongs to, once, so no section has to explain its own scope.
  //
  // The profiles are a section of their own, above those headings, rather than
  // a strip that was on screen whichever section was open. Picking a setup is
  // the rarest thing done here and it was sitting in front of the most
  // frequent ones, with Delete under the cursor all evening. What has to stay
  // visible is not the switcher but the *name*, and the rail already carries
  // it: the sections that belong to a profile are listed under it.
  //
  // What is deliberately not here is Start/Stop: it is session-wide and stays
  // in the header, reachable from either page, because an operator who has to
  // find the right page before they can stop capture has already lost.

  import ProfileSection from "./ProfileSection.svelte";
  import StreamSetup from "./StreamSetup.svelte";
  import VocabularyEditor from "./VocabularyEditor.svelte";
  import AlertEditor from "./AlertEditor.svelte";
  import GroupMembership from "./GroupMembership.svelte";
  import ModelPicker from "./ModelPicker.svelte";
  import { lengthWarning } from "./vocabulary";
  import { connectedDevice, type ModelChoice, type Profile } from "./profiles";
  import { listLanguages, type DeviceOption, type Language } from "./bridge";
  import { onMount } from "svelte";

  interface Props {
    devices: DeviceOption[];
    profiles: Profile[];
    activeKey: number | null;
    vocabulary: string;
    /// The words that must not be missed (P17). Beside the profiles like the
    /// vocabulary: an emergency means the same thing in every production.
    alerts: string[];
    /// Which model the machine transcribes with (P11). Beside the profiles
    /// rather than inside one: what a machine can run depends on the machine,
    /// not on the production being carried to it.
    models: ModelChoice;
    deviceChannels: number;
    locked: boolean;
    saveError: string | null;
    /// Why the session would refuse to start, or `null` — the same string the
    /// status bar shows. Every one of them is about the device or the channel
    /// map, which is what marks the Audio section rather than the page.
    problem: string | null;
  }

  let {
    devices,
    profiles = $bindable(),
    activeKey = $bindable(),
    vocabulary = $bindable(),
    alerts = $bindable(),
    models = $bindable(),
    deviceChannels,
    locked,
    saveError,
    problem,
  }: Props = $props();

  let section = $state<Section>(lastSection);
  const show = (next: Section) => {
    section = next;
    lastSection = next;
  };

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
    groups: null,
    vocabulary: lengthWarning(vocabulary) === null ? null : "warn",
    // Never flagged: an empty alert list is the normal state, not a problem.
    alerts: null,
    models: modelFlag,
  });

  // The rail, grouped by what a section belongs to. The headings are the whole
  // explanation of scope on this page, so they name the thing rather than the
  // category: "Profile · Oper grosses Haus" is what an operator recognises,
  // "profile-scoped settings" is not.
  let rail = $derived<{ scope: string; note: string; items: { id: Section; title: string }[] }[]>([
    {
      // No heading of its own: it is what the headings below it are about, and
      // a scope line over the profile list would have to name the very thing
      // the list is for choosing.
      scope: "",
      note: "",
      items: [{ id: "profiles", title: "Profiles" }],
    },
    {
      scope: active ? `Profile · ${active.name}` : "Profile",
      note: "Carried between houses with the profile.",
      items: [
        { id: "audio", title: "Audio" },
        { id: "groups", title: "Groups" },
      ],
    },
    {
      scope: "All profiles",
      note: "Shared by every setup.",
      items: [
        { id: "vocabulary", title: "Vocabulary" },
        { id: "alerts", title: "Alert terms" },
      ],
    },
    {
      scope: "This machine",
      note: "Stays behind when the profile travels.",
      items: [{ id: "models", title: "Models" }],
    },
  ]);
</script>

<div class="page">
  <!-- The one thing that is worth the top of the page whichever section is
       open. Never swallowed and never buried in the section that happens to
       have caused it: an operator who builds a setup believing it is stored
       and finds it gone before the next show has been failed twice. -->
  {#if saveError}
    <p class="failed">Settings not saved: {saveError}</p>
  {/if}

  <div class="body">
    <nav class="rail" aria-label="Settings sections">
      {#each rail as group (group.scope)}
        <div class="scope">
          {#if group.scope}
            <h2>{group.scope}</h2>
            <p>{group.note}</p>
          {/if}
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
            <p class="hint">
              One multichannel interface for every stream. Two devices would mean two
              clocks, and no common timebase to order one chat by.
            </p>
          </section>

          <StreamSetup
            bind:channels={active.channels}
            bind:language={active.language}
            {languages}
            {deviceChannels}
            {locked}
          />
        </div>

        <div class="sheet" hidden={section !== "groups"}>
          <GroupMembership bind:groups={active.groups} channels={active.channels} />
        </div>
      {/if}

      <div class="sheet" hidden={section !== "vocabulary"}>
        <VocabularyEditor bind:vocabulary {locked} />
      </div>

      <div class="sheet" hidden={section !== "alerts"}>
        <AlertEditor bind:terms={alerts} />
      </div>

      <div class="sheet" hidden={section !== "models"}>
        <ModelPicker bind:models {locked} onstatus={(flag) => (modelFlag = flag)} />
      </div>
    </div>
  </div>
</div>

<style>
  .page {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  /* A strip in the shape of the two bars above it rather than a card, so it
     reads as part of the window's status furniture and not as one more thing
     on the page to configure. */
  .failed {
    margin: 0;
    padding: 0.4rem 1rem;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    color: #ff6b6b;
    font-size: 0.8rem;
  }

  /* --- The rail ---------------------------------------------------------

     Vertical rather than a row of tabs across the top, for two reasons: the
     header already has a row of tabs (Chat / Settings) and a second row under
     it reads as a continuation of the first, and only a vertical list has room
     for the scope headings that are the whole point of the arrangement. */
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
    margin: 0;
    font-size: 0.68rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    /* Long profile names are the rule, not the exception, and a rail that
       grows to fit one would move the sections every time a profile is
       renamed. */
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .scope p {
    margin: 0.15rem 0 0.4rem;
    font-size: 0.7rem;
    line-height: 1.35;
    color: var(--text-dim);
    opacity: 0.8;
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

  label {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    font-size: 0.7rem;
    color: var(--text-dim);
    max-width: 28rem;
  }

  select {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.3rem 0.45rem;
    font: inherit;
    font-size: 0.9rem;
  }

  .hint {
    margin: 0.4rem 0 0;
    font-size: 0.8rem;
    color: var(--text-dim);
  }

  /* A narrow window — the app beside a cue sheet on one screen — puts the rail
     across the top instead, where it costs a line rather than a third of the
     width. The scope notes go: at that size they are the first thing to spend,
     and the headings alone still carry the grouping. */
  @media (max-width: 46rem) {
    .body {
      flex-direction: column;
    }

    .rail {
      width: auto;
      flex-direction: row;
      flex-wrap: wrap;
      gap: 0.3rem 1.2rem;
      border-right: none;
      border-bottom: 1px solid var(--border);
      padding: 0.5rem 0.8rem;
      overflow-y: visible;
    }

    .scope {
      display: flex;
      align-items: center;
      gap: 0.3rem;
    }

    .scope h2 {
      margin-right: 0.2rem;
      max-width: 8rem;
    }

    .scope p {
      display: none;
    }

    .item {
      width: auto;
    }
  }
</style>

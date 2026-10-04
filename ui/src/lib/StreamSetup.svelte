<script lang="ts">
  // The channel setup: one row per intercom channel, with the device channel
  // it is patched to, its label, its language, its input trim, its level while
  // a session runs, and its colour. The meter is here and not only on the
  // strip above the chat because the trim is here: a knob is set against a
  // bar, and one on the page that is not on screen is no help at all.
  // Vocabulary is
  // not here — it is one global list for every channel, edited beside this on
  // the Settings page (P15). What *is* here is each channel's description for
  // the model (P25): the line under the row, read after the vocabulary before
  // every segment of that channel, for the terms only that position uses.
  //
  // Lives on the Settings page rather than above the chat, so the live view is
  // only the live view. It belongs to the profile selected above it and is
  // stored with it (P12) — an engineer does not remap eight channels before
  // every show.
  //
  // Most of it is locked while a session runs, because the channel map is
  // fixed when capture opens. Four fields are not: the label, the language,
  // the input trim and the mute go into the running session through
  // `update_stream_config` (P6). They are the four that turn out to be wrong
  // in the middle of a show — somebody else took the position over, the guest
  // director speaks English, nobody had spoken on that channel during the
  // sound check, somebody left their beltpack open in the workshop — and the
  // four that need no restart. The device channel and
  // the colour stay locked: the first is the channel map, the second would
  // repaint a chat whose old lines were read under the colour they were said
  // in.

  import {
    newChannel,
    duplicateChannels,
    absentChannels,
    applyLanguage,
    usedLanguages,
    type Channel,
  } from "./streams";
  import LanguagePicker from "./LanguagePicker.svelte";
  import GainKnob from "./GainKnob.svelte";
  import MuteButton from "./MuteButton.svelte";
  import SoloButton from "./SoloButton.svelte";
  import { canSolo, watchSolo } from "./solo.svelte";
  import LevelMeter from "./LevelMeter.svelte";
  import type { Language, StreamLevel } from "./bridge";
  import { channelPromptWarning } from "./vocabulary";

  interface Props {
    channels: Channel[];
    deviceChannels: number;
    /// The profile's default language: what a channel added here starts on.
    /// Bound, because the picker that sets it lives in this header — it is
    /// part of the channel setup and belongs where the channels are.
    language: string;
    /// The sidecar's language list, or `null` while it is being fetched or
    /// after it failed. Fetched once by the page above and handed down.
    languages: Language[] | null;
    locked: boolean;
    /// Every metered stream's level, keyed by stream id — from the session
    /// while one runs and from the monitor before it starts (P8). Empty means
    /// nothing is arriving, which the bars show by standing still; `null` is
    /// for a view that never meters and leaves the column out entirely.
    ///
    /// Here as well as on the meter strip, because this is where the trim is
    /// set before a show: a knob with no bar beside it is set by guesswork,
    /// and flipping to the chat to read the level of the channel being
    /// adjusted means the knob is on the page that is not on screen.
    levels?: Map<number, StreamLevel> | null;
    /// The global vocabulary and whether the previous sentence is carried —
    /// only to tell a channel whose description makes its whole prompt too
    /// long (P25). The three share one window, so no one box can say it.
    vocabulary?: string;
    carry?: boolean;
    /// Pushes a label, a language, a trim, a mute or a description into the
    /// session that is already running, or `null` when none is. Its presence
    /// is also what unlocks those fields: there is nothing live to change
    /// without it.
    ///
    /// The stream id is the row's position, which is how the backend assigns
    /// it — see `SessionConfig::streams`. Safe here precisely because the rest
    /// of the list is locked: no row can be added, removed or reordered under
    /// a running session, so a position means the same thing on both sides.
    live:
      | ((
          streamId: number,
          update: {
            label?: string;
            language?: string;
            gainDb?: number;
            muted?: boolean;
            description?: string;
          },
        ) => void)
      | null;
  }

  let {
    channels = $bindable(),
    deviceChannels,
    language = $bindable(),
    languages,
    locked,
    live,
    levels = null,
    vocabulary = "",
    carry = true,
  }: Props = $props();

  let duplicates = $derived(duplicateChannels(channels));
  // Not an error either: a row the current device has no channel for is
  // carried along silently, so one setup serves the desk and the laptop
  // interface alike. Distinct from a channel the operator muted, which is on
  // the device and is being listened to.
  let absent = $derived(new Set(absentChannels(channels, deviceChannels)));

  $effect(() => watchSolo());

  // What the picker above the list still describes. One language and it speaks
  // for the whole setup; several and it only speaks for the next channel —
  // which is worth saying, because a picker reading "German" over three
  // English rows is a lie the operator would otherwise find during a show.
  let divergent = $derived(usedLanguages(channels).filter((l) => l !== language));

  const add = () => {
    channels = [...channels, newChannel(channels, language)];
  };

  // Deliberately a button and not a side effect of the picker. Changing the
  // preset must not rewrite a map the operator built channel by channel; and
  // putting eight channels onto one language is common enough that doing it
  // eight times is eight chances to miss one. Two separate acts, each of them
  // what it says.
  const applyToAll = () => {
    channels = applyLanguage(channels, language);
  };

  const remove = (key: number) => {
    channels = channels.filter((c) => c.key !== key);
  };
</script>

<section class="panel">
  <header>
    <h2>Channels</h2>
    <span class="count">{channels.length} of {deviceChannels} on the device</span>

    <!-- In this header rather than beside the device: it is the channels this
         governs, and it governs them the moment one is added. -->
    <label class="preset">
      <span>New channels</span>
      <LanguagePicker
        bind:value={language}
        {languages}
        label="Default language for new channels"
        disabled={locked}
      />
    </label>
    <!-- Not capped at the device's channel count: the setup describes the
         production, not whatever interface happens to be plugged in now. -->
    <button class="action" onclick={add} disabled={locked}>Add channel</button>
  </header>

  <div class="rows">
    <!-- The field names once, over the columns, rather than over every row.
         Eight channels meant eight copies of "Ch  Label  Lang  M  Gain  Colour",
         which is four times as much text as the setup itself and hid the one
         thing
         the section is scanned for: which label sits on which channel. The
         inputs keep the names as `aria-label`, so nothing is lost to a screen
         reader. -->
    {#if channels.length > 0}
      <div class="head" aria-hidden="true">
        <span>Colour</span>
        <span>Ch</span>
        <span>Label</span>
        <span>Lang</span>
        <span>{canSolo ? "M S" : "M"}</span>
        <span>Gain</span>
        <span>Level</span>
        <span></span>
      </div>
    {/if}

    {#each channels as channel, index (channel.key)}
      <div
        class="row"
        class:absent={absent.has(index)}
        style="--channel: {channel.color}"
      >
        <!-- First in the row, and the only mark of the channel's colour: a
             dot beside a swatch said the same thing twice, and the swatch is
             the one that can be clicked. -->
        <input
          type="color"
          aria-label="Colour"
          title="Stream {index}"
          bind:value={channel.color}
          disabled={locked}
        />

        <!-- Counted from 1, the way channels are numbered on an interface and
             in a patch list. The backend stays zero-based; the conversion
             happens here and nowhere else. -->
        <input
          type="number"
          min="1"
          aria-label="Device channel"
          value={channel.channel + 1}
          oninput={(e) => (channel.channel = Number(e.currentTarget.value) - 1)}
          disabled={locked}
        />

        <!-- `onchange`, not `oninput`: a rename reaches the running session
             when the operator has finished typing it, not once per keystroke
             under a name that is half-entered. The binding keeps the stored
             setup in step either way. -->
        <input
          type="text"
          aria-label="Label"
          bind:value={channel.label}
          disabled={locked && live === null}
          onchange={() => live?.(index, { label: channel.label })}
        />

        <LanguagePicker
          bind:value={channel.language}
          {languages}
          label="Language of {channel.label}"
          disabled={locked && live === null}
          onpick={(code) => live?.(index, { language: code })}
        />

        <!-- Live like the label and the language. Never locked by the
             session, not even for a channel this device does not have: a
             setup carried back to the desk should still remember which
             position was silenced. -->
        <!-- One cell for the M and the S, the pair they are on a desk. The S
             is never locked either: it writes nothing, and checking a channel
             by ear is what this page is open for before a show. -->
        <div class="keys">
          <MuteButton
            muted={channel.muted}
            label={channel.label}
            disabled={locked && live === null}
            onchange={(next) => {
              channel.muted = next;
              live?.(index, { muted: next });
            }}
          />
          {#if canSolo}
            <SoloButton
              streamId={index}
              label={channel.label}
              disabled={absent.has(index)}
            />
          {/if}
        </div>

        <!-- Live like the label and the language, and for a stronger reason
             than either: a channel nobody spoke on during the sound check is
             found by somebody speaking on it during the show, and the fix has
             to be possible then. -->
        <GainKnob
          value={channel.gainDb}
          label={channel.label}
          disabled={locked && live === null}
          onchange={(db) => {
            channel.gainDb = db;
            live?.(index, { gainDb: db });
          }}
        />

        <!-- Wrapped, because `LevelMeter` draws a bar and, when samples have
             been lost, a count beside it — two elements, which in a grid row
             would land in two columns. -->
        <div class="level">
          {#if levels}
            {@const level = levels.get(index)}
            <LevelMeter
              rms={level?.rms ?? 0}
              peak={level?.peak ?? 0}
              dropped={level?.dropped ?? 0}
            />
          {/if}
        </div>

        <div class="end">
          <button class="remove" onclick={() => remove(channel.key)} disabled={locked}
            title="Remove this channel">×</button>
        </div>

        <!-- Live like the label, and on `onchange` for the label's reason: a
             description reaches the next segment when it has been typed, not
             once per keystroke. The placeholder is an example rather than an
             explanation because the one mistake worth preventing is writing
             an instruction here — whisper does not follow one, it continues
             it. -->
        <input
          class="description"
          type="text"
          aria-label="Words {channel.label} uses, for the model"
          placeholder="Words this channel uses, as they should be written — Stellwerk, Moving Heads, Preset 12"
          bind:value={channel.description}
          disabled={locked && live === null}
          onchange={() => live?.(index, { description: channel.description.trim() })}
        />
        {#if channelPromptWarning(vocabulary, channel.description, carry)}
          <p class="prompt-warn">
            {channelPromptWarning(vocabulary, channel.description, carry)}
          </p>
        {/if}
      </div>
    {/each}

    {#if channels.length > 0}
      <p class="hint">
        The line under each channel is read by the model before every segment of that
        channel, after the vocabulary. Write what the channel sounds like — its terms and
        names — not what the model should do: it does not follow instructions, it
        continues them.
      </p>
    {/if}

    {#if channels.length === 0}
      <p class="empty">No channels yet. Add one to start transcribing.</p>
    {/if}
  </div>

  {#if divergent.length > 0}
    <!-- Not a warning: channels on different languages is a legitimate setup —
         a touring production with an English follow-spot operator is exactly
         that. It is an offer, and it names what it would overwrite. -->
    <p class="diverges">
      {divergent.length === 1 ? "One language" : `${divergent.length} languages`} below
      {divergent.length === 1 ? "differs" : "differ"} from this: {divergent.join(", ")}.
      <button class="link" onclick={applyToAll} disabled={locked}>
        Put all {channels.length} channels on {language}
      </button>
    </p>
  {/if}

  {#if absent.size > 0}
    <!-- Said once, in full, rather than only implied by the greyed rows: an
         operator who sees a channel stay silent for a whole show has to be
         able to find out here why, and the reason is the device, not the
         setup. -->
    <p class="warn">
      {absent.size} of {channels.length} channels are above the {deviceChannels} the selected
      device has; they start silent and receive no audio.
    </p>
  {/if}

  {#if duplicates.length > 0}
    <!-- Allowed, not forbidden: two views of one channel is a legitimate if
         unusual setup. But during setup it is nearly always a slip, and the
         symptom — every line appearing twice — does not point at its cause. -->
    <p class="warn">
      Channel {duplicates.map((c) => c + 1).join(", ")} is used by more than one stream;
      it will be transcribed once per stream.
    </p>
  {/if}
</section>

<style>
  .rows {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  /* One template for the heading and every row, so the columns line up and
     stay lined up — the whole point of naming the fields once. */
  .head,
  .row {
    display: grid;
    /* The language column is wider than it was as a bare code field: it now
       holds a name, and "Portuguese (pt)" truncated to "Portug…" is no better
       than the code it replaced. */
    grid-template-columns:
      2.6rem 4rem minmax(6rem, 1fr) minmax(7rem, 0.8fr) 3.3rem 2.4rem
      minmax(3rem, 0.7fr) 2rem;
    align-items: center;
    gap: 0.5rem;
  }

  .head {
    font-size: 0.7rem;
    color: var(--text-dim);
  }

  /* Under the row, from the channel number to the end: it belongs to the
     channel, and it is the one field that needs a line's width to be read. */
  .description,
  .prompt-warn {
    grid-column: 2 / -1;
  }

  .description {
    font-size: 0.8rem;
  }

  .description::placeholder {
    color: var(--text-dim);
    opacity: 0.7;
  }

  .prompt-warn {
    color: var(--warn);
    font-size: 0.75rem;
    margin: 0;
  }

  .hint {
    color: var(--text-dim);
    font-size: 0.8rem;
    margin: 0.5rem 0 0;
  }

  /* Narrow: the three live controls — mute, gain, level — drop to a second
     line of their own, so the identity of the row (colour, channel, label,
     language) keeps the full width instead of every column being squeezed
     to nothing. The column headings go with them: they name a single line of
     fields, and over a row that wraps they name nothing. */
  @media (max-width: 45rem) {
    .head {
      display: none;
    }

    .row {
      grid-template-columns: 2.4rem 3.2rem minmax(0, 1fr) minmax(0, 1fr) 2rem;
      grid-template-areas:
        "colour ch label lang end"
        "mute gain level level level"
        "desc desc desc desc desc"
        "warn warn warn warn warn";
      row-gap: 0.35rem;
    }

    /* Two lines per channel means the eye needs telling where one channel
       ends: without it the mute and the bar of one row read as belonging to
       the label underneath them. A rule rather than a card, because eight
       boxed channels is a heavier page than the setup deserves. */
    .rows {
      gap: 0;
    }

    .row + .row {
      margin-top: 0.55rem;
      padding-top: 0.55rem;
      border-top: 1px solid var(--border);
    }

    /* The header's four parts do not fit on one line on a phone, and squeezed
       onto one they turn the count into three lines and the button into two.
       Title, count and action keep the first line; the language preset, which
       is a field and not a title, takes the second. */
    section.panel > header {
      flex-wrap: wrap;
      align-items: center;
    }

    .preset {
      flex: 1 1 100%;
      order: 1;
    }

    .preset :global(select) {
      flex: 1;
      min-width: 0;
    }

    /* Placed by position rather than by class: three of the eight cells are
       components, whose root element this file cannot name. */
    .row > *:nth-child(1) {
      grid-area: colour;
    }
    .row > *:nth-child(2) {
      grid-area: ch;
    }
    .row > *:nth-child(3) {
      grid-area: label;
    }
    .row > :global(*:nth-child(4)) {
      grid-area: lang;
    }
    .row > :global(*:nth-child(5)) {
      grid-area: mute;
    }
    .row > :global(*:nth-child(6)) {
      grid-area: gain;
    }
    .row > *:nth-child(7) {
      grid-area: level;
    }
    .row > *:nth-child(8) {
      grid-area: end;
    }
    .row > .description {
      grid-area: desc;
    }
    .row > .prompt-warn {
      grid-area: warn;
    }
  }

  .keys {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }

  /* The bar takes the channel's own colour, as it does on the meter strip:
     the same reading in two places should not be two different colours. */
  .level {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    min-width: 0;
    font-size: 0.75rem;
    --accent: var(--channel);
  }

  .end {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    justify-content: flex-end;
    min-width: 0;
  }

  /* Dimmed, not hidden or badged: a row the device has no channel for is
     still part of the setup, and it has to be editable — pointing it at a
     channel the device has is the fix. A row the operator muted is *not*
     dimmed: that one is working, and its M says so in red. */
  .row.absent {
    opacity: 0.55;
  }

  /* Sits with the count rather than with the Add button: it describes the
     list, it does not act on it. */
  .preset {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.75rem;
    color: var(--text-dim);
    min-width: 0;
  }

  .preset :global(select),
  .preset :global(input) {
    width: auto;
    min-width: 8rem;
  }

  .diverges {
    color: var(--text-dim);
    font-size: 0.8rem;
    margin: 0.5rem 0 0;
  }

  /* A link and not a second filled button: it is an offer next to a sentence,
     and beside "Add channel" a solid button would read as of equal standing
     with it. */
  .link {
    background: none;
    color: var(--accent);
    padding: 0;
    font-weight: 500;
    text-decoration: underline;
  }

  .link:disabled {
    text-decoration: none;
  }

  .empty {
    color: var(--text-dim);
    font-size: 0.85rem;
    margin: 0.25rem 0;
  }

  .warn {
    color: var(--warn);
    font-size: 0.8rem;
    margin: 0.5rem 0 0;
  }

  input {
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 0.3rem;
    padding: 0.25rem 0.4rem;
    font: inherit;
    font-size: 0.85rem;
  }

  input {
    min-width: 0;
  }

  input[type="color"] {
    width: 100%;
    height: 1.8rem;
    padding: 0.1rem;
  }

  button {
    background: var(--accent);
    color: var(--on-accent);
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

  .remove {
    background: transparent;
    color: var(--text-dim);
    font-size: 1.1rem;
    line-height: 1;
    padding: 0.25rem 0.5rem;
  }

  .remove:hover:not(:disabled) {
    color: var(--danger);
  }
</style>

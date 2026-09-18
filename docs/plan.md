# Plan and progress

The work packages of the MVP, and where each one actually stands.

Plan and status live in the same file on purpose. Kept apart they drift, and a
plan that no longer describes the code is worse than no plan — it reads as
authority while being fiction.

Evidence for the architecture decisions is in
[measurements.md](measurements.md); this file references it rather than
repeating numbers, so there is one place to correct when a measurement changes.

**Status of this document:** 2026-09-18, after Unicode normalisation closed
P17. P10 (SQLite persistence) was set aside the same day; see the package for
what that costs. P3b (language stabiliser) was dropped with the auto mode it
existed for — see *Deliberately out of scope*. This revision keeps what still
governs decisions and drops the account of how finished work came about — that
lives in the code and in git history.

---

## Context

Audio engineers monitor several intercom channels during a production and
inevitably lose information — nobody can follow eight conversations in
parallel, and what was said is gone. This app transcribes every intercom
stream of one multichannel interface locally and near-live and shows them as a
chat log, so what was missed stays readable and searchable.

Offline is a hard requirement: production environments without reliable
networking, and no cloud handling of crew communication.

### Constraints set by the user

These came out of questions rather than from the code, and several
architectural decisions look arbitrary without them.

| Aspect | Decision |
|---|---|
| Audio input | **One multichannel interface**; the app picks device and channels and demultiplexes |
| Languages | **DE/EN code switching is the normal case**; the language list is freely extensible, never hard-coded |
| Priority | **Latency first**, target ≈1–2 s until text appears |
| Platform | **macOS / Apple Silicon first**, Windows later — audio layer abstracted from the start |
| Frontend | **Svelte + TypeScript** |
| Whisper | **Sidecar process** (crash-isolated), pool of workers |
| Models | **Downloaded on first start**, selectable in settings |
| UI scope, MVP | Chat view with a group sidebar, live search and filter, levels and stream health, vocabulary editor, global alert terms with visual attention, a separate Settings page for channel setup, and an optional read-only LAN viewer in a browser |
| Not in the MVP | Export, markers, diarisation, cloud, interactive multi-user control |

---

## Status at a glance

**Done** — meets its acceptance criteria. **Partial** — usable, but something
named below is missing. **Open** — not started. **Deferred** — decided against
for now, with the reason recorded in the package.

| | Package | Status | What is missing |
|---|---|---|---|
| P0 | Scaffolding & toolchain | **Done** | — |
| P1 | Audio capture (macOS, multichannel) | **Partial** | Channel mapping unverified on real hardware |
| P2 | Ring buffer & resampling | **Done** | — |
| P3 | VAD & segment scheduler | **Done** | — |
| P4 | Whisper sidecar | **Partial** | Not registered as a Tauri `externalBin` |
| P5 | Worker pool & backpressure | **Partial** | No reference load test on live audio |
| P6 | Tauri API | **Partial** | No generated shared types; no `update_stream_config`, no `search_session` |
| P7 | Chat UI | **Partial** | The list is a window, not a jump to an arbitrary position |
| P8 | Levels & stream health | **Partial** | Level, overrun count and overload count only |
| P9 | Live search & filter | **Open** | Was specified on P10's FTS index, which is not coming — needs a decision on what it searches instead |
| P10 | SQLite persistence | **Deferred** | Set aside by the user: this is about live communication, not persistence |
| P11 | Model manager | **Done** | — |
| P12 | Settings & presets | **Partial** | Profiles, default language and stable device id are stored; every threshold is still a constant in the code |
| P13 | Packaging & signing | **Open** | `Info.plist` with the microphone usage description exists |
| P14 | Performance & latency budget | **Partial** | Latency measured on one stream; no reference load test, no WER baseline |
| P15 | Vocabulary & prompt management | **Partial** | One global box, stored, but not named lists and not measured against a baseline |
| P16 | LAN remote client | **Partial** | Verified against a mock app only, not from a second machine on a real network. No authentication — by decision |
| P17 | Alert terms & visual attention | **Done** | — |

**What runs today:** every channel of one multichannel device end to end —
capture, one ring buffer and segmenter per channel, Silero speech detection,
the shared worker pool, one colour-coded chat — at a measured ~330 ms on one
stream from the end of a spoken segment to the text appearing. The channel
count is not capped: a channel the interface does not have is muted rather
than refused, and what decides how many channels keep up is how many decodes
run at once, which is an operator's to set on the Models page.
Around it: a Settings page holding the device, the channel map, the global
vocabulary, the global alert terms and the groups; a chat page with a group
sidebar that narrows the view and detaches any group — or everything — into a
window of its own; profiles that come back at the next start, keyed on an
interface the app recognises after a rename; a model manager that brings a
fresh machine to its first transcription without a terminal; and, optionally,
the same UI served to a browser on the local network as a full peer.

**What a multi-channel patch still lacks is a measurement of its own:** every
latency and CPU number was taken with one stream running. Eight channels is
the load P14 reasons about and `tests/pool.rs` exercises synthetically; it is
a reference point, not a ceiling.

---

## Architecture decisions, with their reasons

**Sidecar with a persistent model, not `whisper-cli` per segment.** Spawning a
process per segment reloads the model every time — seconds, against a 1–2 s
target. Instead `whisper-worker` is a small binary in the same workspace that
links `whisper-rs`, loads the model once and serves a length-prefixed protocol
over stdin/stdout. That gives crash isolation *and* a warm model.

**Worker pool ≠ eight workers.** Eight parallel whisper instances contend for
the same Metal queue and make each other slower. The pool is therefore
decoupled from the stream count: two to three by default, fed from *one*
prioritised queue across all streams.

**Segmentation by VAD, not fixed windows.** Fixed windows cut words in half,
and whisper has no way to know it received a fragment. Force-flush after a
maximum duration keeps one continuous talker from breaking the latency target.

**Speech detection belongs where speech is modelled.** Energy tells loudness
apart from quiet, not speech from a bell. Silero therefore decides what the
segmenter cuts, in the app process rather than only in the sidecar: a segment
that was never speech costs 0.4 ms of detection instead of a ~320 ms decode
plus an IPC round trip. See P3 and measurements.md §10.

**The decode is the budget.** Measured end to end at ~330 ms, of which
300–340 ms is the decode; everything else in the pipeline costs 1–2 ms. That
settles where tuning is worth doing — model size and segment length — and where
it is not.

---

## Language strategy (code switching)

Mostly German is spoken on the channels, with English technical terms mixed in
("wir gehen auf Cue fünf, standby", "fade out"). Both have to land in the text.

Whisper assigns exactly *one* language token per segment. There is no "several
languages at once" mode, so the question is not *whether* a language is chosen
but how well the chosen mode carries foreign-language inserts.

1. **Fix the primary language per stream (default).** In German mode the
   multilingual model transcribes English inserts verbatim in Latin script,
   which is the desired result. Auto-detect actively makes this worse: a
   segment with a high English share flips to English mode, and the German rest
   is then translated or mangled.
2. **Force `translate = false`.** Silent translation is the most dangerous
   failure mode, because the output looks plausible.
3. **One global vocabulary prompt.** The strongest quality lever for code
   switching — measured, see measurements.md §4. The same active vocabulary is
   applied to every channel, so terms used across a production are maintained
   once rather than drifting between channel-specific prompts. Package P15.
4. **No auto-detect at all.** Every stream is on a fixed language and the
   remedy for a stream whose language genuinely changes is the operator
   changing it — see P7's one-click override on a running stream. A restricted
   auto mode with a stabiliser was specified as P3b and dropped; see
   *Deliberately out of scope* for why. The data model still carries
   `LanguageMode::Auto` and nothing constructs it — it can go when something
   else touches that enum.

**Extensibility is a hard requirement:** languages are hard-coded nowhere.
Modelled as `LanguageCode(String)` against the full whisper language list
queried from the sidecar. A third language costs configuration, not code.

---

## Packages

### P0 — Scaffolding & toolchain — **Done**

Cargo workspace with four crates: `app` (Tauri backend), `whisper-worker`
(sidecar binary), `whisper-protocol` (shared frame format), `audio-core`
(capture/VAD/segmentation, UI-free and therefore unit-testable; whisper.cpp
enters it only through the optional `silero` feature, so the Windows port and
the crate's own tests never have to build it). Frontend in `ui/` with Vite +
Svelte + TS, Tauri v2, pnpm, pinned Rust toolchain, `justfile`, CI on
`macos-14`. The frontend's non-component logic — bubble grouping, group
membership, the vocabulary prompt — is tested with vitest, run by `just check`
and by CI alongside the Rust tests.

### P1 — Audio capture (macOS, multichannel) — **Partial**

`AudioBackend` trait as the platform seam, macOS implementation on `cpal`.
Device enumeration, one input stream on the chosen multichannel device,
de-interleaving into N mono channels inside the callback, configurable
stream→channel mapping, disconnect handling.

The callback is a realtime context: **no allocation, no lock, no logging**. The
`Demuxer` therefore owns its sinks and its scratch buffer.

**Accepted when:** an integration test writes 10 s of two channels of an
aggregate device into separate WAVs and the channels are provably not swapped.

**Outstanding:** the de-interleaving is unit-tested, including a device buffer
spanning several scratch passes. But no signal was present on the 32-channel
interface during testing, so the end-to-end mapping is **unconfirmed on real
hardware**: patching a known source to one channel and checking that only the
matching WAV carries it remains a manual step.

### P2 — Ring buffer & resampling — **Done**

Lock-free SPSC ring buffer per stream between callback and processing thread.
Resampling to 16 kHz mono f32 with `rubato` (sinc) in the processing thread,
never in the callback. Per-stream overrun counters. f32 normalised to [-1, 1].
A 12 kHz tone that would alias onto 4 kHz is suppressed by 92 dB; see
measurements.md §7.

### P3 — VAD & segment scheduler — **Done**

`Vad` trait, with `SileroVad` as the implementation the app uses and
`EnergyVad` kept as the fallback for a machine without the VAD model.
Segmentation rules per stream: start on speech onset with ~300 ms pre-roll, end
after ~500 ms of silence, force-flush at `max_segment_len` (6 s) **cut at the
quietest frame of the recent past**, discard anything under ~300 ms.

Three things about the detector are worth knowing before changing it:

- **The LSTM state is cleared on every call.** `whisper_vad_detect_speech`
  resets it, so each call replays the preceding four windows and reads only the
  last probability. It costs 0.41 ms per 32 ms frame — about 1.3 % of a core
  per stream.
- **The frame size comes from the model.** whisper.cpp reads the window length
  out of the model file and exposes it nowhere, and the whole scheme rests on
  one probability meaning exactly one frame. `SileroVad` probes it once at
  startup instead of compiling in 512.
- **It fails open.** A frame the model refuses to judge counts as speech. The
  sidecar's gate is still behind it, so failing open costs a wasted decode
  while failing closed would cost an operator who spoke and was never
  transcribed.

**Outstanding:** nothing for the MVP. The threshold and the pre-roll/timeout
values are still constants in the code — they belong in P12, and
`silence_timeout` in particular is a *latency* parameter.

### P4 — Whisper sidecar — **Partial**

Own binary linking `whisper-rs` with `metal`. Loads the model once, reports
`ready`, then serves length-prefixed frames: JSON header plus raw f32 PCM,
binary rather than base64 to save copying on the latency path. `list_languages`
exposes the full whisper language list so none is hard-coded. Greedy decoding,
`no_context`, `single_segment`, no temperature fallback, `translate = false`.

Logging hooks are installed as the very first action — otherwise whisper.cpp
writes on stdout and corrupts the frame protocol.

**Speech gate.** A Silero VAD context is loaded alongside the model and run
*before* decoding. No speech means empty text and no decoder call. Since P3 the
same model also runs in front of the segmenter, so this is the backstop: it is
the only layer that sees a segment as a whole, and it is what still protects a
session that fell back to the energy detector. Three earlier attempts failed
and are documented in measurements.md §8, including one that appeared to work
and did nothing at all.

**Outstanding:** registration as a Tauri `externalBin` so the binary is bundled
and co-signed. Belongs with P13.

### P5 — Worker pool & backpressure — **Partial**

Pool of M sidecar processes (default 2, set on the Models page) on **one**
shared queue, oldest segment first so no stream starves. The bound is a
latency budget rather than a segment count: sixteen segments per worker that
actually came up — 32 behind two, 128 behind eight — which is ~5 s of work
whatever the pool size, because the whole queue drains in parallel. On
overflow the *oldest* is dropped and an `overload` event fired per stream —
**silent loss is explicitly forbidden**. Supervision: crash or timeout kills and respawns the
process, requeues the affected segment once, then discards it. A failed
respawn is retried three times before the pool gives up on that worker. A
result aggregator orders results by `seq` per stream.

**The invariant everything else rests on:** every segment the pool accepts
produces **exactly one** result, including the ones it gives up on. Without it
the reorder buffer would stall on the missing sequence number and lose that
stream for the rest of the session — a far worse failure than the dropped
segment itself. It is asserted over a whole run rather than per segment,
because that is the only way it can be asserted at all.

Three rules worth knowing before changing any of them:

- **Dropping the oldest is the opposite end from the one work is taken from.**
  Workers pop the oldest so a busy stream cannot starve a quiet one; overflow
  drops the oldest because by then its transcript is the one worth least. Both
  rules are right and they point in opposite directions.
- **A wedged worker is killed, not signalled.** The decode is blocked reading a
  pipe that will never deliver again, so there is nothing to interrupt. Killing
  the process closes the pipe, and the read then fails exactly like a crash —
  so timeout and crash are healed by one code path rather than two.
- **Stop drains the queue.** Stop is pressed *after* the last thing worth
  hearing was said, so discarding the backlog reliably loses the end of every
  session. The queue drains within a 5 s grace period before it closes.

**Accepted when:** a load test with eight synthetic continuous-speech streams
runs 10 minutes without memory growth; a `kill -9` on a worker is healed within
~2 s without a UI error. ✔ for the second — `tests/pool.rs` kills the pool's
own sidecars mid-session. Eight streams have been through the queue in
`eight_streams_share_the_queue_without_starving_each_other`, which asserts the
fairness claim but says nothing about memory or latency.

**Outstanding:** the reference load test, which belongs with P14 and now needs
live audio on eight channels rather than synthetic segments. Until then the
pool size, the backlog per worker and the timeouts are reasoned defaults, not measured
ones.

### P6 — Tauri API — **Partial**

Commands `list_devices`, `start_session`, `stop_session`, `load_config`,
`save_config`, `clear_communication`, `session_snapshot`, `open_group_window`,
`close_group_windows`; events `transcript`, `levels`, `overload`,
`session-state`, `config-changed`, `remote-status`.

`start_session` takes a **list** of streams on one device, and the session
builds one pipeline per channel — ring, resampler, speech detector, segmenter —
all feeding the one shared queue. The stream id is the position in that list,
assigned by the backend rather than by the UI, so two streams can never be
given the same one; `SessionState::Running` echoes the mapping back as the
authoritative answer to which channel ended up under which label.

Rules that still bind:

- **Tauri v2 gates the core plugins behind capabilities.** Commands defined by
  the app need no permission, but `listen` belongs to the core event plugin.
  Without `capabilities/default.json` every subscription is rejected — and an
  unhandled rejection is silent, which makes a working backend look like a hung
  one.
- **`start_session` returns the running state** rather than only emitting it,
  so the transition the operator waits for arrives as the answer to the click.
  It is also *announced*, because other windows have only the event.
- **`list_languages` needs no model and no session.** It runs the sidecar with
  `--languages` rather than sending `ListLanguages` down a worker's pipe — a
  worker answers only once a model is loaded, and the picker this feeds is used
  while the operator is still mapping channels. Cached for the life of the
  process.
- **Start is non-destructive.** It never clears the transcripts already
  displayed. Clearing is the separate, explicit `clear_communication`.
- **Every event carries a session id, because `seq` does not identify a
  segment.** Sequence numbers are per stream and restart at zero, so
  `(stream, seq)` repeats as soon as a second session runs, and the chat holds
  lines from several sessions at once. `(session, stream, seq)` is the
  identity. The id is counted per process, which with P10 deferred is the whole
  lifetime of a transcript.
- **A failed session has to be cleared, or Start never works again.** The
  failure is discovered on the processing thread, which cannot reach the
  session the Tauri state holds. `start_session` asks `Session::is_finished`
  and clears such a session before refusing; the UI calls the idempotent
  `stop_session` when it sees a failure, so the model is freed then.
- **Levels are one event for all streams**, at 20 Hz — hence `levels` rather
  than `level`. The cost no longer grows with the channel count, and meters
  read side by side were sampled side by side.
- **One processing thread serves every stream.** Resampling at ~2200×
  realtime and Silero at ~1.3 % of a core per stream make eight streams ~12 %
  of one core. What must not share a thread is decoding, and it does not.
- **Vocabulary is session-level, not per stream.** `SessionConfig::vocabulary`
  is handed to every stream's segmenter and `StreamConfig` carries no prompt,
  so "one vocabulary per production" is a property of the system rather than a
  convention a later caller could half-apply. The field is `#[serde(default)]`.
- **Configuration is two commands, not a CRUD surface.** `load_config` and
  `save_config` move the whole file, because it is a few kilobytes a human
  edits at human speed. So groups, the channel map and the profiles need no
  commands of their own. See P12.
- **`app::history` is the one copy every view is caught up from** — the last
  10 000 transcripts and the sessions' zero points, in memory, reaching no
  disk. It is what stops two windows disagreeing about what was said and about
  what "Clear all communication" cleared.

**Outstanding:** shared types generated from the Rust structs (`ts-rs` or
`specta`) instead of hand-written TypeScript; `update_stream_config` and
`search_session`. With P10 set aside, `search_session` has only the in-memory
chat to search, so whether it needs to be a backend command at all is an open
question — `clear_communication` was the same question and came out the other
way, because deletion has to happen once for every window rather than once per
window.

**Accepted when:** the frontend receives typed events; eight streams under full
load keep the WebView main thread under ~30 %. The second half is unmeasured —
it belongs with the P14 load test.

### P7 — Chat UI (Svelte) — **Partial**

Two pages. **Settings** holds the input device, the editable channel list
(device channel, label, language, colour), the global vocabulary, the alert
terms, the groups with their membership, the profiles and the model manager.
**Chat** holds a group sidebar, a meter per running stream in the selected
view, and the lines themselves, each with its channel's colour down the left
edge and its label, timestamp, language badges and measured latency, with a
"jump to latest" button. Start/Stop is in the header and belongs to neither
page: an operator who has to find the right page before they can stop capture
has already lost.

**A bubble is an utterance, not a segment.** Consecutive segments of the same
channel are merged up to a silence of 1.5 s, with ceilings on length and
duration, and a merged bubble carries every language its parts were detected
as. Merging only ever appends to the *last* bubble in the list — a channel
interrupted by another one starts a new bubble rather than growing upwards,
because the list is read as a chronology.

**The list renders a window, not the session.** The last 200 bubbles are in the
DOM; scrolling to the top adds a page at a time, with the scroll position
corrected by the height the new content added, and returning to the bottom
trims the window again. A window rather than a virtual scroller with estimated
heights: a bubble's height depends on how its text wraps, so estimates mean a
scrollbar that lies. What it costs is that a point an hour back takes several
steps to reach — which is what P9's search and its jump into the chat position
are for.

**The Settings page is split by scope, not by topic.** Its scoped sections sit
in a rail on the left under a heading naming what each belongs to: Audio and
Groups under the *profile* (listed under the active profile's name), Vocabulary
and Alerts under *all profiles*, Models under *this machine*. Scope is the
thing an operator gets wrong. Profiles sit first in the rail, above the scope
headings, as a list with one row per setup — name, channel and group count,
device, Duplicate and Delete — rather than a permanent strip across the top,
because choosing a setup is the rarest thing done on this page and New,
Duplicate and Delete under the cursor all evening sit next to the fields they
would discard. A failed save is the one thing that spans the page, in a strip
at the top. Each rail item carries a dot when its section wants attention —
amber for a setup that will not start or a vocabulary over the token budget,
red for a machine with no model at all. Sections stay mounted and inactive ones
are hidden, because a model download is minutes long.

Channels are numbered from 1, the way they are numbered on an interface; the
backend stays zero-based and the conversion happens at that one boundary. The
channel list names its fields once, above the columns.

**A channel the device does not have is muted, not a refusal to start.** A
setup built for the desk and carried to a laptop with a two-input interface may
hold more channels than the device offers: those keep their stream id, colour
and group membership, are marked `muted` in the running state, and receive no
audio. The session refuses to start only when *every* channel is above what the
device has, because then it would open a device and transcribe silence. Muted
rather than dropped is what keeps the ids stable — the stream id is the row's
position.

Each channel gets a palette colour when it is created and the operator can
change it. The colour is configuration, not decoration: with eight channels
interleaved in one list it is what makes a line's origin readable at a glance,
and it is the same colour on the meter strip.

Editing the channel map and switching profiles are locked while a session runs,
because they are fixed when capture opens. Group membership is *not* locked: a
group is a view, not capture configuration, and an operator watching eight
channels live is exactly who needs to narrow them to three.

Two streams reading the *same* device channel is allowed — mirroring one
channel into two views is unusual but not wrong — so it is a warning rather
than an error. During setup it is nearly always a slip, and the symptom, every
line appearing twice, does not point at its cause.

**The language is picked, and picked once.** Every language field is a menu
built from whisper's own table via `list_languages`, never written down here.
The picker above the list sets the *profile's* default: what a channel added
afterwards is born with. It presets and does not rewrite — when the rows below
disagree, the section says which languages differ and offers to put all of them
onto the picker's in one press. Two separate acts: eight channels in one
language is the normal case, but changing them silently would lose a deliberate
mixed setup. A code the running whisper build does not offer is kept as an
option of its own rather than snapping to the first entry.

**Groups and their windows.** The sidebar *selects*; groups are created,
renamed, deleted and filled on Settings, because during a show the question is
"show me just the stage" and a rename box under the cursor is a liability.
Groups are non-exclusive: a channel may belong to any number, and removing it
from a group never removes the channel. The sidebar also offers an all-channels
view, which is what the chat shows when no group is selected rather than a
group that happens to contain everything; a group with no members shows nothing
and says so.

The sidebar **width is dragged** at its edge, by pointer or arrow key, down to
a narrow column or out to 60 % of the window, with a double-click back to the
default. It is remembered per screen in the browser's own storage rather than
in `config.json`: the configuration is the production's setup and reaches every
view the LAN remote serves, so a width stored there would let a phone in the
auditorium decide how much of the desk's screen the sidebar takes. It is
re-clamped to whatever window it is read back into.

Every group can be opened in its own window from the sidebar, **and so can the
all-channels view** — a live view of its channels that receives history,
transcripts, meters and session state from the shared backend and reflects
later membership changes. It does not own a capture session, so Start/Stop
stays global; closing it closes only that view. All channels is not a group and
so has nothing in the profile to remember settings on: no flash-on-message, and
a pin that lasts as long as the window does.

**How it is one app in several windows.** The same bundle is mounted in every
WebView, and the window's *label* says which view it is — `group-3` is the
fourth group of the active profile, `group-all` the view of everything. The
label rather than a query string, because it is readable before the first
paint, so a group window never renders the main view for a frame and then
replaces itself; and because Tauri identifies a window by it, so asking for a
group that is already open focuses that window instead of stacking a second one
on it. A group is addressed by its **position** in the active profile, which is
the same handle the configuration file stores membership by; the keys the UI
edits by are issued per window and mean nothing outside it. The window then
follows `config-changed`.

A window subscribes to the live events *before* it asks for `session_snapshot`,
because the other order drops whatever is said during the round trip — which is
exactly the moment a group window is opened. The overlap is harmless: a segment
is identified by session, stream and sequence, and one in both answers is kept
once. Two bubbles with one key would not be cosmetic — it throws and stops the
list rendering.

**Switching profiles closes the group windows.** The stream ids a transcript
carries are positions in the active profile's channel list, so a window kept
open across a profile change would filter the right numbers against the wrong
channels. Switching profiles is a setup action between productions, not
something done during a show.

**A group's detached window can be pinned in front of other windows**, and that
is asked of the window itself — a pin in its own header, not a checkbox on
Settings. It is remembered per group, so the next window that group opens is
born in front. Per group rather than for every group window, because a window
always in front is a window covering something: the stage manager's view is
worth that, the general talkback is not. Off by default. The window applies it
to itself immediately and asks the main window to store it, because **the
configuration has exactly one writer**; the request travels as a `group-on-top`
event and the ordinary save carries it to disk.

**Clear all communication** clears every transcript, across every group and
session, in every open window at once. It does not change channel, group or
Settings configuration. Because it is irreversible it requires confirmation and
is available only after capture has been stopped.

**Accepted when:** eight streams run visibly live; speech that runs on is one
bubble rather than one per segment ✔; scrolling stays smooth at 10,000 segments
— the DOM is bounded by construction, but "smooth" is a claim about a machine
under load and belongs to the P14 test; a third language can be chosen without
a code change ✔; a channel has a preselected but editable colour ✔; a channel
can appear in two groups and is visible in both ✔; all configured channels
start from one action ✔; starting again after a Stop retains the preceding chat
history ✔; Clear all communication requires confirmation ✔, removes every
displayed transcript in every open window ✔ and leaves configuration intact ✔;
opening a group in its own window shows the same existing and incoming messages
limited to that group's channels, without a second capture session ✔.

**Outstanding:** a one-click language override on a running stream (beyond the
MVP line). The list can only be walked backwards a page at a time, so jumping
to an arbitrary position needs P9's search to drive it.

### P8 — Levels & stream health — **Partial**

Today: a level meter per running stream (RMS and peak, computed in Rust, all
arriving in one event so the bars on screen are the same age), the per-stream
overrun count from P2, and the per-stream count of segments the decode queue
gave up on (P5). The two counts are deliberately separate: one is audio lost on
the way *into* the pipeline, the other audio captured cleanly and then not kept
up with, and they call for different fixes.

Beside them is a **CPU and memory indicator** for the app itself, in the status
bar. It answers the question the overload counter only answers too late, and it
is the figure that decides how many channels a given laptop can be given.

- **It counts the workers too** — this process and everything descended from
  it, which is what an operator would see in Activity Monitor if they thought
  to add it up. Because that sum is not what any one row there shows, the
  tooltip says so.
- **The percentage is a share of the machine.** macOS counts one busy core as
  100 %, so eight busy workers report 800 % and read as a failure. The figure
  is divided by the core count, and the bar turns at 75 % rather than 90: a
  warning that arrives once every core is saturated arrives after the segments
  have started dropping.
- **It runs whether or not a session does**, sampled once a second rather than
  at the meters' 20 Hz — walking the process table is real work.

**Outstanding:** peak hold, silence indicator, clipping warning, worker queue
depth, and the rest of a global status bar — active workers, queue length and
throughput. `WorkerPool` already exposes `workers()` and `queue_depth()`.

**Accepted when:** a deliberately clipped channel and a silent one are both
immediately recognisable in the UI.

### P9 — Live search & filter — **Open**

**Needs respecifying.** It was full-text search over the stored session (SQLite
FTS5), which P10's deferral removes. What is left to search is the messages the
window is holding — a smaller job, and one that runs into the chat's DOM
window, since a hit may lie outside the rendered range. Filters for streams and
time range are unaffected.

**Accepted when:** incremental search while typing, with highlighting and a
jump into the chat position, over a session of realistic length.

### P10 — SQLite persistence — **Deferred**

**Set aside by the user on 2026-09-18**, with the reason: *this is about live
communication, not about persistence.* A scope decision rather than a
postponement.

What it costs: a session lives in the window and dies with it. Kill the app and
the chat is gone. The configuration is unaffected — profiles, groups, colours
and the model choice are in `config.json` and were never P10's business.

What it changes elsewhere: **P9** becomes a search over the in-memory messages;
**Clear all communication** is only the in-memory half, which is now the whole
action; **P6's `search_session`** may not need to exist; the whole-system
verification loses its "restart and the session loads" step; and export (out of
scope) would have to read the window's own messages.

**If it is ever revived,** the shape it was specified in: tables `sessions`,
`streams`, `segments`, `segments_fts`, `vocabularies`, `devices`; a segment
keyed by `(session, stream, seq)`, with the per-process session id replaced by
one that survives a restart; language fields written from day one, including
for fixed-language streams, because it costs nothing and makes misdetections
analysable; WAL mode with batched writes; versioned migrations; crash recovery
marking an incomplete session as ended and keeping the data.

### P11 — Model manager — **Done**

Curated model list with URL, SHA256, size and expected latency
(`crates/app/src/models.rs`, four entries); download with progress, resume and
checksum verification; `ModelPicker.svelte` as the Settings section; a clear
"no model" state; a custom model — a path to a `.bin` — as the offline way in.
Default `ggml-small-q5_1`, with `base` faster and `large-v3-turbo` more
accurate.

Downloads go to `models/` inside the app's *data* directory, not the
configuration one: hundreds of megabytes that can always be fetched again,
which on macOS is the difference between something the user's backup carries
forever and something it can skip. The choice is stored beside the profiles
rather than inside one, for the same reason the vocabulary is — which model a
machine can run depends on its memory, its GPU and the latency it can afford,
not on the production.

Four decisions worth keeping in view:

- **The checksum decides, and a failure discards.** A damaged model does not
  announce itself: whisper.cpp either refuses it or transcribes noise. Nothing
  is renamed into place before it hashes correctly, and a file that fails is
  deleted rather than kept — keeping it would leave a corrupt model one resume
  away from being installed.
- **Downloads resume.** The partial file is kept as `.bin.part` and continued
  with a range request; cancelling leaves it alone and the button then says
  "Resume" with the amount already there. A server that ignores the range
  header is detected and the write starts from the front, because appending
  would splice two copies together and only the checksum would notice.
- **Resolution has one order and it is written down** (`models::resolve`): the
  environment variable, then the custom model if it is the one selected, then
  the selected catalogue model, then the default one if installed, then the old
  `.scratch/models` search. The custom model is picked in the same list under
  the reserved id `custom` rather than being a path that silently overrode the
  picker. Its path is chosen through `pick_model_file` and the field stays
  typable. A custom path that is not there is an error rather than a quiet
  fall-through.
- **"Not downloaded yet" is a different sentence from "no model at all".**

The four network tests in `crates/app/tests/models.rs` (`just models`) run
against the real host rather than a mock: the two things most likely to be
wrong are the URL and the checksum, which are exactly the two a mock replaces.

**Outstanding, and deliberately so:** switching the model while a session runs
does not restart the workers — the picker is locked instead, like the channel
map. `just model` and `just vad-model` are kept for the integration tests and
`just slice`, which have no app directory to resolve against.

The remaining acceptance half — a machine that has never run this, guided to
its first transcription — needs P13.

### P12 — Settings & configuration persistence — **Partial**

**Accepted when:** a preset, including channel colours and non-exclusive group
memberships, and the global vocabulary and alert terms survive a restart and a
reconnect of the interface; each group's flash-on-message preference does too.
✔ for the identity half — the reconnect is verified against real hardware in
`capture_live.rs`, `#[ignore]`d because CI has no interface.

**Done:** the presets, as **profiles**. `config.json` in the app's
configuration directory holds a list of profiles — device, default language,
channel map with labels, languages and colours, groups with their memberships
and flash-on-message preference — plus the active vocabulary, alert terms, the
remote block and which profile was last in use. It is read at startup and
written on every edit after a short debounce; there is no Save button, because
a setup an operator believes is stored and is not is worse than no persistence
at all.

A profile's **default language** is what a channel added to it is born with —
in the profile rather than beside the vocabulary, because a house may run a
German main stage and an English touring production, and the channel map it
presets is profile-scoped too. A file written before it existed loads on the
default rather than on an empty string.

Three decisions worth keeping in view:

- **JSON, not the P10 database.** Configuration is read once and written when a
  human edits something, which is nothing like the write load transcripts have.
  It is also the one file an operator may have to repair by hand or send to
  someone else when it is wrong.
- **Saved membership is positions, live membership is keys.** The key is stable
  while a row is edited, which stops a deleted channel from re-pointing every
  group below it; the position is what still means something after a restart.
  `ui/src/lib/profiles.ts` owns that translation and is the only place that
  does; `profiles.test.ts` pins down the case it exists for.
- **Nothing here may stop the app from starting.** A missing file is a first
  run; an unreadable one, or one from a newer version, is moved to
  `config.bad.json` and the app comes up empty and says so. Writes go through a
  temporary file and a rename, so an interrupted save cannot truncate the real
  one.

**Device recognition by a stable id** is done. A profile stores the CoreAudio
**UID** beside the display name and is keyed on it, so an interface renamed in
Audio MIDI Setup is still recognised and two units of the same model are two
entries in the picker. cpal knows a device only by name and keeps its
`AudioDeviceID` private, so the id is read from CoreAudio beside cpal —
`audio-core/src/backend/stable_id.rs` — and matched to what cpal enumerated.

- **The name stays, and is not a leftover.** A device that is not plugged in
  still has to be named: "Scarlett 18i20 is not connected" is a sentence an
  operator can act on, a UID is not.
- **The migration costs the operator nothing.** A profile written before this
  has no id; the first time its interface is seen connected, the id is filled
  in and saved (`adoptDeviceIds`). A profile whose device is *absent* is left
  alone rather than bound to whatever is plugged in.
- **Once a profile has an id, the name stops being a fallback.** An id that is
  not there means the device is not there — the second unit of the same model
  must not stand in for it.
- **It degrades rather than fails.** A device CoreAudio will not identify, and
  every device on Windows until the WASAPI port brings its endpoint id, falls
  back to the name as its id — which is exactly what the app did before.

**Outstanding:** the settings proper — worker count, VAD thresholds, maximum
segment length, theme — are all still constants in the code and none is in the
file. Two are already identified as needing to be settings: the segmenter's
`silence_timeout` (a *latency* parameter, not only a quality one) and the
Silero threshold, which trades decode time against the risk of losing quietly
spoken commands.

### P13 — Packaging & signing (macOS) — **Open**

arm64 build, sidecar correctly bundled, code signing with hardened runtime,
notarisation, DMG.

**Done already:** `Info.plist` with `NSMicrophoneUsageDescription`. Without it
macOS does not even ask — the callback silently delivers zeros, and a silent
channel is indistinguishable from a denied permission.

**Outstanding:** the `com.apple.security.device.audio-input` entitlement, which
hardened runtime gates separately, and the sidecar's own signature.

**Accepted when:** the DMG installs on a foreign Mac and runs without a
Gatekeeper warning.

**Risk:** hardened runtime plus a sidecar process is the classic stumbling
block — the sidecar needs its own signature.

### P14 — Performance & latency budget — **Partial**

**Measured** (measurements.md §7 and §8): ~330 ms end to end on one stream, of
which 300–340 ms is the decode; resampling at ~2200× realtime; the speech gate
rejects non-speech in 3–8 ms instead of a ~320 ms decode.

**Outstanding:** instrumented measuring points across all stages, a written
latency budget held against the measurements, a repeatable "8 streams,
30 minutes" reference load test, and the **code-switching fixture set** with
reference transcripts for measuring word error rate separately over German and
English passages. The load test is unblocked and is the next thing to do: eight
streams run and `tests/pool.rs` puts eight through the queue with synthetic
segments — what is missing is live audio, duration, and anything being
*measured* rather than asserted.

**Accepted when:** a reproducible benchmark shows p95 latency within the target
on defined hardware; the WER baseline for mixed-language material is
documented.

**Caveat on what exists:** the current fixture is macOS text-to-speech in a
quiet room. Real intercom traffic with noise, compression and crosstalk will do
worse. The numbers are an upper bound, not a promise.

### P15 — Vocabulary & prompt management — **Partial**

Today: **one** free-text vocabulary on the Settings page, applied to every
stream through `SessionConfig::vocabulary` and passed to the sidecar as
`initial_prompt`. Its effect is measured — `Q5` becomes `Cue`, `Fadeout`
becomes `Fade out` (measurements.md §4). The box shows an estimated token count
against whisper's 224-token window and warns twice over: that an over-long
prompt loses the terms typed *first*, and that length itself makes
hallucination on quiet channels more likely, because whisper treats the prompt
as context it may continue.

The estimate is four characters to the token, a heuristic and not whisper's
tokeniser — German compounds and technical terms split worse than average, so
it reads low exactly where this list is densest. Hence the warning at 80 %
rather than at the limit, and hence `estimated` in the name of everything that
carries it.

The list is stored beside the profiles rather than inside one, because the
jargon of a house outlives any one patch and a copy per profile is a copy that
drifts. An emptied box stays empty across a restart: the file tells "never set",
which keeps the default, apart from "deliberately cleared", which turns the
prompt off and is how its effect gets measured at all.

**Outstanding:** *named* lists with exactly one active — today there is one list
and no way to keep a second — a shipped starter list, shortening that is
deliberate rather than whisper's silent truncation, and the measurement, which
needs the P14 fixture set.

**Accepted when:** on the P14 fixture set the enabled global vocabulary list
measurably improves the word error rate of the technical terms against the
baseline on every applicable channel.

### P17 — Alert terms & visual attention — **Done**

One global list of terms in its own Settings section, stored in `config.json`
and pushed to the backend when it changes rather than read from that file — the
file is a debounce behind the term just typed. `app::alerts` compiles the list
once and matches every finished transcript in `TauriEvents::transcript`, which
is the one place every view's copy of an event comes from, so the main chat,
detached group windows and the LAN viewer highlight exactly the same text. The
ranges travel with the event in UTF-16 code units, because the consumer slices
JavaScript strings. Alert metadata lives only as long as the live chat.

Matching is case-insensitive, Unicode-normalised and on whole words: a one-word
term matches a word and a multi-word entry matches that phrase, so `cue` does
not fire on `rescue` and `Feuer` does not fire on `Feuerwehrzufahrt`. Terms and
transcripts are normalised to NFC before they meet, so "Bühne" typed on a Mac
keyboard matches the same word with the umlaut as a combining character. That
fold runs per cluster rather than per character, and every byte it produces maps
back to the cluster it came from, which is what keeps the UTF-16 ranges pointing
at the text the operator sees — a decomposed word is two code units longer on
screen than the composed spelling the term was written in. The editor drops a
second spelling of a term for the same reason it drops a second casing.

The chat marks the exact range piece by piece, never as markup, because a
transcript is text from outside, and rings the bubble holding it.

**Attention, and who asks for it.** The desktop uses the platform's native
attention request for a new match in an unfocused window — deliberately not an
audible alarm, which must not add noise to an intercom environment. One rule in
`ui/src/lib/attention.ts` serves both views so they cannot drift: the main
window is urgent about every alert term whichever group is selected — it is the
view of the whole session — and flashes only for the group it is showing; a
group window is the other way round, urgent only for its own members and always
flashing when that group asks for it. Each group carries its own
**Flash on every new message** preference, off by default, stored with the group
and editable while a session runs; it fires for every new message from that
group's channels whether or not it contains an alert term. The all-channels view
has no implicit group setting. The request is counted per bubble and not per
segment — one sentence arrives as three segments, and three flashes for one
sentence is exactly the noise this must not add. The browser viewer highlights
the same characters but flashes its own surface and tab title, having no window
to ask the platform about.

### P16 — LAN remote client — **Partial**

The operator can optionally enable a local-network client from Settings. The
app then serves a browser build of the same UI on the selected LAN interface;
there is no cloud service and no data leaves the local network. The listener is
off by default and binds only to the explicitly selected interface; Settings
show a shareable URL.

The point of it is a second machine: the session runs where the audio interface
is, and the operator works from somewhere else. The browser is therefore a full
peer rather than an observer — all history and new transcripts including alert
highlights, every channel and group, levels, stream health, session state and
the configuration; and it can start and stop, clear communication, and edit
channels, groups, vocabulary, alert terms and profiles.

**Security as it stands.** There is no authentication yet. Anyone who reaches
the chosen interface can control the session, which is why the listener is off
by default and the interface is picked deliberately. The transport is arranged
so that a token check is one middleware in front of the socket when it is
wanted, not a redesign. What that does *not* cover, and is therefore checked
already: a WebSocket upgrade is not subject to the same-origin policy, so the
socket refuses an upgrade whose stated origin is not the address this app is
serving on — otherwise any page anybody on the network opened could drive the
session out of the operator's own browser.

**Every mutation exists exactly once**, in `crates/app/src/commands.rs` as one
arm of a `Command` enum. The Tauri bridge is a single `command` that hands its
request to `dispatch`, and the socket hands it the same request, so adding a
command is one arm and one typed wrapper in `bridge.ts` and the remote cannot
quietly fall behind the desktop. Three things stayed `#[tauri::command]`s
because a browser has no equivalent and an enum arm answering "not on this
transport" would suggest the remote was missing something: the native file
picker, the platform's attention request, and the detached group windows. The
remote therefore covers managed model downloads but not an arbitrary file on
the host, flashes in the page and tab title, and uses in-page panels or its own
browser tabs for group views. A group opened in a local window and the same
group on the remote are independent views of one backend state.

**The listener** is `axum` on the one address the operator picked, serving the
frontend out of Tauri's own asset resolver — the same bundle the app runs in
its WebView, embedded in the binary, so there is no second copy on disk to fall
a version behind. In a checkout it serves the Vite dev server instead, and it
checks for that *first*: the asset resolver answers in development too, out of
whatever was last left in `ui/dist`, so preferring it would run the operator's
window on the dev server and the remote on a build from some earlier afternoon.
That does not look like a stale build — it looks like a bug that will not be
fixed, because every fix lands in the window and none reaches the browser. A
released app has no dev URL and is unaffected.

Whether the listener runs is a consequence of the configuration file and of
nothing else: `save_config` applies the remote block, at every save and once at
startup. There is deliberately no "enable the remote" command, because the
configuration has one writer and a second mutation path is how the file and the
listener come to disagree. What the configuration cannot know — whether the
listener came up, on a port that may be taken, and how many browsers are
connected — is reported as a `remote-status` event.

**Configuration is last-write-wins**, broadcast to every client at once so the
side that lost sees it happen instead of holding a stale form. Two rules make
that survivable:

- **A save waits for the field to be left.** The write is what every view
  rebuilds its page from, so a save landing while a label is being typed
  rebuilds the field under the cursor. Only fields that are *typed into* wait —
  a colour, a language or a device is one discrete act. A window closed with the
  cursor still in the box writes on the way out, as a best effort.
- **A save that changes nothing is not a save.** Adopting another peer's
  configuration is a state change like any other, so it would schedule that
  view's own save — and a view that writes back what it was just given turns one
  edit into a conversation nothing ends, with the field being typed in torn down
  on each pass. `save_config` compares what it is sent with what the file holds,
  both tidied, and a configuration that changes nothing is neither written nor
  announced, whoever sent it.

Initial state arrives as a snapshot over the socket, followed by ordered
incremental events; reconnecting repeats the snapshot, which is the same call
the client makes when it opens rather than a second mechanism that could
disagree with it. The snapshot is bounded by the same in-memory history the
local app gets, so a client that slept longer than the buffer resumes from what
is left. Levels are downsampled for remote clients, so a slow link falls behind
on meters and never on transcripts. `transport.ts` is the seam; nothing above it
knows which transport it is on, and while a browser is disconnected it says so,
because a silent remote and a quiet intercom look identical. The main window now
follows `config-changed` like the group windows always have — it is no longer
the only writer — and carries the keys it handed out across an adopted
configuration by position, so a channel renamed on the remote does not read as a
profile switch and close the operator's group windows.

**Outstanding:** authentication, which the transport is shaped for and which
nothing yet asks for.

**Accepted when:** a second device on the chosen LAN opens the URL, sees the
same existing and incoming communication, levels and group views as the
operator, and can start and stop the session and change configuration with the
change appearing on the operator's screen; a remote config save reaches the
local app as an ordinary config update; disabling the listener immediately
drops the connections it was serving. ✔ for the command round trip, the event
stream and the drop on disable, against a mock app; the second machine waits on
the whole-system verification below.

---

## Dependencies

```
P0 Scaffolding
 ├─> P1 Capture ──> P2 Ring/Resample ──> P3 VAD/Segmenter ──┐
 ├─> P4 Sidecar ───────────────> P5 Worker pool <───────────┘
 │        │                            │
 │        └──> P15 Vocabulary ─────────┤
 │                                     │
 └─> P11 Model manager ────────────────┤
                                       │
                                       v
                            P6 Tauri API (commands/events)
                              ├─> P17 Alert terms ──> P7 Chat UI ──> P8 Health/levels ──> P9 Search/filter
                              │
                              └─> P16 LAN remote client   (P10 SQLite: deferred)
                                       │
                                       └── P12 Settings
                                       v
                                 P13 Packaging
```

**Milestone A (technical backbone):** P0 → P1 → P2 → P4 → P5. *Reached; what P5
still owes is a load test on live audio.*

**Milestone B (usable MVP):** + P6, P7, P11, P12, P16, P17. *All in place. What
is left is the settings proper — every threshold is still a constant in the
code.*

**Milestone C (language quality & production readiness):** + P14 fixtures, P15,
P8, P9, P13. *P10 left it on 2026-09-18; P3b was dropped.*

---

## Suggested next steps

In this order, with the reason each one comes where it does.

1. **P14 reference load test** — every number the remaining decisions rest on
   is a single-stream number. The 10 % CPU figure for eight streams' speech
   detection is arithmetic, and the queue has only been put under contention by
   synthetic segments. The chat's window bounds the DOM, which removes the
   frontend from the list of things that could explain a bad result. It is also
   the one step here that cannot be written: it needs the interface and
   somebody to talk into it.
2. **The rest of P12 — the settings proper.** The Silero threshold and the
   segmenter's `silence_timeout` and `max_segment` are still constants. They
   come *after* step 1 on purpose: exposing a number as a setting fixes the
   default somebody will keep, and the current defaults are single-stream
   guesses.
3. **P15** — develop vocabulary against the measurable baseline from step 1,
   not by feel. The control exists and is global; what is missing is the
   evidence that any particular list is better than another.
4. **P9, respecified.** What it becomes is a search over what the window holds,
   which runs into the chat's DOM window. Worth deciding before it is built.
5. **The remote, on a real network.** P16 has not had a second machine, a house
   network and somebody working from it. That comes before authentication —
   the obvious next thing to want, and the wrong thing to build against an
   arrangement nobody has used yet.

## Whole-system verification

1. `just dev` on Apple Silicon with a multichannel interface connected.
2. Start a session with eight channels on fixed languages.
3. Speak on all channels at once — text appears in the correct stream, latency
   within target.
   1. Speak mixed-language ("standby für Cue fünf, dann fade out") — the
      English terms stand verbatim in the German text, nothing was translated.
   2. Disable the global vocabulary list and repeat the same passage — the
      difference must be visible on every channel, otherwise P15 is not
      working.
   3. Select a third language in the setup and put a stream on it — from the
      picker, without a code change.
4. Overdrive one channel and leave another silent → P8 shows both.
5. Enter a search term → hits across all streams, jumping works (P9).
6. `kill -9` a sidecar → the app keeps running, the worker comes back.
7. On a machine that has never run this: no model, and the Settings page says
   so and offers the download. Fetch it, interrupt it, resume it, then
   transcribe — the first transcription without ever opening a terminal (P11).
8. Stop capture, press **Clear all communication**, confirm → all chat lines
   and search hits are gone, while channels, groups, the model choice and
   Settings remain.
9. Open a group in its own window, then speak on one member and one non-member
   channel → only the member transcript appears there; global Start/Stop is
   reflected without creating another capture session. Open **All channels** in
   its own window too → both lines appear, and asking for it a second time
   focuses the window already open. Drag the sidebar's edge narrow, restart the
   app → it comes back at the width it was left at.
10. Enable the remote client in Settings, pick the house interface, and open its
    URL from a second device → it sees the same history, incoming lines, groups,
    meters and session state. Start and stop from the browser and watch the
    operator's screen follow; rename a channel there and watch the Settings page
    take it on without the operator's own edits being thrown away; open a group
    and All channels from the browser's sidebar as tabs. Pull the network for
    ten seconds → the page says it is not connected, and on its return the chat
    is complete rather than missing the gap. Switch the listener off → the
    browser drops immediately.
11. Rename the interface in Audio MIDI Setup and restart → the profile still
    finds it and starts, under its new name. With two identical interfaces
    connected, the picker offers both and each profile keeps the one it was set
    to (P12).
12. Run the P14 reference load test, record p95 latency.
13. Add `cue` to Alerts, unfocus the main window and speak "Cue fünf" → the
    exact word is highlighted and the window requests attention; "rescue" does
    neither. Repeat from a channel in an open group to verify its separate
    window. Then enable that group's **Flash on every new message** and speak a
    non-alert message → its window requests attention; disable it and repeat →
    it does not. Verify that the browser highlights the same characters and
    flashes its own surface and tab title (P17, P16).

There is deliberately no "restart and the session loads" step: with P10 set
aside the chat lives in the window, and killing the app loses it by design.

---

## Deliberately out of scope

- Export (TXT/SRT/JSON). With P10 set aside it would have to read the window's
  own messages.
- Markers and bookmarks.
- Windows port: only P1 needs reimplementing, provided the `AudioBackend` trait
  is respected; plus the GPU backend choice in P4.
- Speaker diarisation.
- **Automatic language detection per segment, and the P3b stabiliser that went
  with it.** Whisper assigns one language per segment, so auto mode mangles
  exactly the DE/EN code switching that is the normal case here, and a
  stabiliser only contains damage the mode itself introduces. The case it would
  have served — a channel handed to someone speaking another language — is an
  announced, human-scale event, so the one-click override on a running stream
  (P7) is the remedy. Restricting detection to a candidate list also cost an
  extra encoder pass per segment through `WhisperState::lang_detect`, in the
  300 ms class against a ~330 ms budget, because `full_lang_id_from_state` is
  whisper's global argmax and arrives too late to restrict.
- Second decode of low-confidence segments in the alternative language, better
  result wins. Doubles the latency for those segments.

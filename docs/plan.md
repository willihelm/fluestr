# Plan and progress

The work packages of the MVP, and where each one actually stands.

Plan and status live in the same file on purpose. Kept apart they drift, and a
plan that no longer describes the code is worse than no plan — it reads as
authority while being fiction.

Evidence for the architecture decisions is in
[measurements.md](measurements.md); this file references it rather than
repeating numbers, so there is one place to correct when a measurement changes.

**Status of this document:** 2026-09-19, after the interface was made to fit a
phone — drawers for both sidebars, a strip of vertical meters and a page that
respects a device's safe areas, so the LAN remote (P16) is usable on the
screen it is most often opened on. The same day, P19's two outbound actions
became one: the MIDI cue was folded into the webhook, because the desk is
reached through Bitfocus Companion and Companion speaks HTTP — so the action
is a free-text request rather than a preset, written host-only, which is
the answer to the question P19 had parked — and it was built the same day,
which closes P19. Before that, P19's rules landed —
triggers and actions, which subsume P17's alert terms and flash flags and
migrate an existing `config.json` into them. Before that, Unicode normalisation
closed P17 and P18 was written up — the Windows port, which moved out of
*Deliberately out of scope* and into a package of its own. P10 (SQLite
persistence) was set aside the same day; see the package for what that costs. P3b (language stabiliser) was dropped with the auto mode it
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
| UI scope, MVP | Chat view with a group sidebar, live search and filter, levels and stream health, vocabulary editor, rules with visual attention, a separate Settings page for channel setup, and an optional read-only LAN viewer in a browser |
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
| P6 | Tauri API | **Done** | `search_session` decided against — search is per view, see below |
| P7 | Chat UI | **Done** | — |
| P8 | Levels & stream health | **Partial** | Level, overrun count and overload count only |
| P9 | Live search & filter | **Done** | Per view, over what the window holds. No time-range filter |
| P10 | SQLite persistence | **Deferred** | Set aside by the user: this is about live communication, not persistence |
| P11 | Model manager | **Done** | — |
| P12 | Settings & presets | **Partial** | Profiles, default language and stable device id are stored; every threshold is still a constant in the code |
| P13 | Packaging & signing | **Open** | `Info.plist` with the microphone usage description exists |
| P14 | Performance & latency budget | **Partial** | Latency measured on one stream; no reference load test, no WER baseline |
| P15 | Vocabulary & prompt management | **Partial** | One global box, stored, but not named lists and not measured against a baseline |
| P16 | LAN remote client | **Partial** | Verified against a mock app only, not from a second machine on a real network. No authentication — by decision |
| P17 | Alert terms & visual attention | **Done** | Subsumed by P19: the terms and the flash flags are rules now, and an existing file is migrated into them |
| P18 | Windows port | **Open** | Everything — capture on WASAPI/ASIO, the GPU backend, packaging |
| P19 | Rules: triggers and actions | **Done** | One outbound action: a free-text HTTP request, written host-only. The MIDI cue was folded into it — Companion reaches the desk |

**What runs today:** every channel of one multichannel device end to end —
capture, one ring buffer and segmenter per channel, Silero speech detection,
the shared worker pool, one colour-coded chat — at a measured ~330 ms on one
stream from the end of a spoken segment to the text appearing. The channel
count is not capped: a channel the interface does not have is carried rather
than refused, and what decides how many channels keep up is how many decodes
run at once, which is an operator's to set on the Models page.
Around it: a Settings page holding the device, the channel map — whose labels
and languages can be changed while the session runs, without stopping capture —
the global vocabulary, the groups and the rules that fire on them; a chat page with a
group sidebar that narrows the view and detaches any group — or everything —
into a window of its own, and a search that narrows it further to one channel or
one word without any of that reaching the other screens; profiles that come back at the next start, keyed on an
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
quietest frame of the recent past that the detector also called silence**,
discard anything under ~300 ms. A forced cut **overlaps**: `forced_overlap`
(500 ms) of audio before the boundary is handed to the next segment as well, so
a word the cut runs through is whole on the later side. The overlap comes out
of the ceiling rather than adding to it, and the duplicated words it produces
are accepted on purpose — a word twice is readable, a word bisected is not.
Both sides of a forced cut get a 5 ms fade, so the step to zero is not a click.

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

### P6 — Tauri API — **Done**

Commands `list_devices`, `start_session`, `stop_session`,
`update_stream_config`, `load_config`, `save_config`, `clear_communication`,
`session_snapshot`, `open_group_window`, `close_group_windows`; events
`transcript`, `levels`, `overload`, `session-state`, `config-changed`,
`remote-status`.

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

**The types are generated, not written twice.** Every struct that crosses the
bridge derives `ts_rs::TS` where it is declared, and `cargo test` writes
`ui/src/lib/generated/` — with the Rust doc comments carried along, so the
reason a field exists stays next to the field. `bridge.ts` re-exports them and
holds only what has no Rust side: the calls, and the two messages windows send
each other. The constants that are not types — the schema version, the default
port, the worker count — are generated too, by `crates/app/src/bindings.rs`,
because a number copied by hand is a number that gets changed on one side only.
The committed bindings are checked against the structs in CI; a field renamed
in Rust is now a TypeScript error in the component that reads it rather than a
value that arrives as `undefined` during a show.

One thing the generation *changed*, rather than only restated: the bindings
describe what the backend **sends**, and the backend fills every
`#[serde(default)]` before it sends anything. So the fields the frontend used
to treat as optional — a rule's `enabled`, a profile's `deviceId` — are not
optional on the wire, and the tolerance for a file written by an older version
lives in one place, in serde, where it is tested. The frontend's own fallbacks
are now belt and braces, and the tests that build a short file say so in as
many words.

**`update_stream_config` changes a channel of a session that is already
running**: its label, its language, or both. Those are the two things that turn
out to be wrong in the middle of a show — somebody else took the position over,
the guest director speaks English — and the two that need no restart. The label
is read by the aggregator per transcript, so a rename lands on the next line;
the language is sent to the processing thread, which owns the segmenters, so it
lands on the next segment. Neither rewrites what is already in the chat: a line
was said under the label it was said under. The device channel is *not* here —
the channel map is fixed when capture opens, so a repatch is honestly a
restart. The command answers with the running state and announces it, so the
Settings page that asked, a detached group window and a browser on the LAN all
take the new mapping from the one authority on it. It changes no configuration
file: the session is what is being edited, and P12's file keeps its one writer.

**`search_session` was decided against** (2026-09-18). Searching is something
one operator does in one view — "did anyone mention the fly bar", "show me only
what Licht said" — and it must not reach the other windows or the browsers on
the LAN, which are other people looking at the same show. That makes it the
opposite of `clear_communication`, which is a backend command precisely because
a deletion has to happen once for every window rather than once per window. And
with P10 set aside there is nothing to search that the view does not already
hold: every window is caught up from `session_snapshot` and then fed by
`transcript`, so the whole chat is in the page. A backend command would be a
round trip to fetch a filtered copy of what is already in memory, and it would
put one viewer's private filter on a shared surface. The search itself is P9,
in the frontend.

**Accepted when:** the frontend receives typed events; eight streams under full
load keep the WebView main thread under ~30 %. The second half is unmeasured —
it belongs with the P14 load test.

### P7 — Chat UI (Svelte) — **Done**

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

**Sort direction has a global default and local overrides.** The View section
of Settings stores one application-wide default — **Newest at bottom** by
default, or **Newest at top** — in `config.json`, beside the vocabulary and
for its reason: how a house reads its chat outlives any one patch. A main
window, a group window or a LAN browser opens with that direction, and a change
to it reaches every view that has not been turned over, while they are open.

The override is a one-press toggle at the head of each chat's own bar, beside
the search box: turning the chat over is done during a show, on the window in
front of the operator, and there are two directions rather than a menu of them.
It is labelled with the direction the view is *in*. There is deliberately no
third "follow the app" state beside them: a window is read the way it looks,
and a control that can be pressed without the chat changing reads as broken —
a view that has been turned is turned back by the same press, in the same
place. The override lives in that view's local storage, keyed per view, like
the sidebar width and for its reason — it belongs to the screen reading the
chat, and a phone in the auditorium must not be able to turn the desk's chat
upside down. So an operator can deliberately read one group in the opposite
direction without changing every other screen, and find it that way again
tomorrow.

The underlying bubble sequence remains chronological in both modes — merging a
continuing utterance therefore still has one unambiguous last bubble — while
the renderer chooses which end appears first. In newest-at-bottom mode, older
pages are added above and new bubbles arrive below; in newest-at-top mode those
directions are mirrored. The follow edge moves with the newest edge, and so
does what “Jump to latest” scrolls to — though not where that button sits,
because it is absolutely placed over the header bars at the other end of the
window. An operator reading older communication keeps their scroll anchor when
a new bubble arrives, and keeps it across the turn itself: the bubble across
the middle of the viewport is found before the flip and brought back to the
middle after it.

All of it is written once, in terms of "the newest edge" and "the history
edge", rather than as two behaviours — the follow, the jump, the end the next
page is added to, and the scroll correction that holds the reading position
while it is, which is needed only when the page lands *above* the viewport. The
DOM window itself does not turn over: it is always the newest 200 bubbles,
because which lines are rendered is not a question of which way round they are
shown.

**The Settings page is split by scope, not by topic.** Its scoped sections sit
in a rail on the left under a heading naming what each belongs to: Audio and
Groups under the *profile* (listed under the active profile's name), Vocabulary
and View under *all profiles*, Models and the remote client under *this
machine*. View sits with the vocabulary rather than with the models because it
is not a property of the machine: it is stored in the file and every view the
remote serves opens the way it says. It is named after the view rather than
after the chat because the chat is only the first thing a window is — how a
view is read is the same kind of answer as which channels it shows. Scope is the
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

**A channel the device does not have is carried along silently, not a refusal
to start.** A setup built for the desk and carried to a laptop with a two-input
interface may hold more channels than the device offers: those keep their
stream id, colour and group membership, are marked `absent` in the running
state, and receive no audio. Deliberately not called "muted" — that is the M an
operator presses on a channel that *is* there (P8). The session refuses to start only when *every* channel is above what the
device has, because then it would open a device and transcribe silence. Carried
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

On a **narrow window** there is no edge to drag, because the sidebar is not a
column: below 45 rem it becomes a drawer over the chat, opened from a burger in
the header and closed by the view that was picked in it. That is the width at
which a 12 rem column and a chat cannot both be read — a phone on the LAN
remote (P16), and the app beside a cue sheet on half a laptop screen. The
Settings rail does the same, and is reduced to one line carrying the burger and
the name of the section that is open; it used to wrap into a row across the
top, which at a phone's width was three lines of section names above a form.
The meters turn with it: eight rows of label-and-bar become a sideways-scrolled
strip of columns filling upwards, in the height of one row. One breakpoint for
all three, in `ui/src/lib/layout.ts`, because a window passing through a width
where the burger is showing and the drawer is still a column is a layout nobody
drew. The page also lays itself out under a phone's notch and home indicator
and pads its bars back out of them, sizes itself to the *dynamic* viewport so
the browser's address bar does not cover the newest line for good, and gives
every control a fingertip's height where the pointer is a finger.

Every group can be opened in its own window from the sidebar, **and so can the
all-channels view** — a live view of its channels that receives history,
transcripts, meters and session state from the shared backend and reflects
later membership changes. It does not own a capture session, so Start/Stop
stays global; closing it closes only that view. All channels is not a group, so
what it remembers sits on the profile rather than in `groups`: a pin that lasts
as long as the window does. What it *reacts* to is no longer a preference of
its own — a rule with no channel restriction is how "flash on everything" is
written now (P19).

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
**It is accepted when additionally:** switching direction leaves the same
bubbles visible in the inverse visual order ✔, preserves the reading position
✔, and makes new bubbles and “Jump to latest” use the selected edge in the main
window, a detached group window and the LAN client independently ✔ — the
default is one setting in `config.json` and each of the three may override it
for itself ✔.

Both of what this owed is now in: a running stream's language (and its label)
can be changed from the channel list without stopping capture — P6's
`update_stream_config` — and the list is no longer only walkable backwards a
page at a time, because P9's search grows the window to whatever it jumps to.

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

**An input trim per channel**, as a knob beside each meter on the strip above
the chat and again in the channel setup — which is why that setup grew a meter
of its own: a knob is set against a bar, and a bar on the page that is not on
screen is no help at all. A desk does not hand out eight channels at one level — the stage
manager's beltpack is hot, the follow-spot's matrix output is twenty dB below
it — and whisper gives up on the quiet one long before a human would.

- **Whole decibels, −20 to +20.** What a trim knob is marked in, what an
  operator would write their setup down in, and a range past which the only
  thing left to amplify is the interface's own hiss. The bounds are exported to
  the frontend as constants (`GAIN_DB_MIN`, `GAIN_DB_MAX`) rather than restated
  there; the backend clamps again on the way in, because the knob is not the
  only caller.
- **Applied on the processing thread, not in the capture callback.** The
  callback de-interleaves and nothing else. It is applied *before* the speech
  detector and before the level window, which is what makes the knob usable at
  all: it is turned while watching the bar it feeds, and a meter showing the
  untrimmed signal would be a knob with no visible effect.
- **Live, like the label and the language (P6).** It is wrong the first time
  anybody speaks on a channel nobody spoke on during the sound check, and that
  is during the show. Carried in `StreamUpdate` and held as an atomic rather
  than sent down the language channel — a knob being dragged produces a change
  per pointer event, and the pipeline only needs the one that is current when
  it next pumps.
- **Stored with the channel** (P12), because it compensates for how *this*
  intercom position is patched, which belongs to the production's patch list
  and therefore to the profile. `#[serde(default)]`, so a file written before
  it existed reads as unity.
- **Clamped to full scale on the way out.** Whisper would take a sample of 4.0
  without complaining, but the peak meter beside the knob would then have no
  top to read against — and an operator turning a trim up has to be told where
  "too far" is, by the meter, at the moment they pass it.

**A mute per channel**, as an M beside each meter on the strip and again in
the channel setup. What an operator reaches for is not a trim: it is a beltpack
left open in a noisy workshop, or a position that is patched but unstaffed
tonight, filling everybody's chat with a room.

- **Mute means not transcribed, not not heard.** The channel keeps its ring,
  its trim and its meter; only the segmenter stops being fed. The bar beside a
  lit M goes on moving, which is what makes the button honest: the channel is
  visibly alive and visibly not arriving. A dead bar would read as a dead
  channel, and the operator would go looking for a cable.
- **The open segment is closed and decoded** at the moment the M goes down.
  Those words were said before it was pressed and belong in the chat; left in
  the segmenter they would arrive whenever the channel came back, under a
  timestamp from before the interval.
- **Live, like the label, the language and the trim (P6)**, carried in
  `StreamUpdate` and held as an atomic the pipeline reads once per pump — the
  transition is what it compares, which is how the flush above happens on the
  thread that owns the segmenter.
- **Stored with the channel** (P12), `#[serde(default)]` for the trim's
  reason. A position muted for this production is muted for a reason that
  outlives the evening, and finding it again after every restart is the kind
  of chore that ends in somebody transcribing a workshop for an act.
- **"Muted" is now one word for one thing.** A channel the device has no input
  for used to be called muted too; it is `absent` everywhere now — in
  `RunningStream`, in the strip, in the status bar. An operator looking at a
  silent channel has to be able to tell "I pressed M" from "this interface has
  four inputs", and those want different fixes. An absent channel is dimmed, a
  muted one is not: it is working, and its red M says so.
- **It does not reach the monitor** (below), which only ever meters and never
  transcribes. Muting before a session is recorded in the profile and applied
  by the next Start.

**Meters without a session**, so the trim can be set before a show rather than
discovered during one. `monitor.rs` opens the device, de-interleaves it and
measures it — no model, no sidecar, no segmenter, no transcripts — and emits
the same `levels` event a session does, on the same name: the meters do not
care where a reading came from, and a second kind of level event would mean
every view learning to read both.

- **One device, one holder.** `start_session` stops the monitor before opening
  its own capture, and it refuses to start while a session runs, whose meters
  are these meters. The Settings page starts it again when the session ends.
- **Idempotent, and asked for on every edit.** `start_monitor` takes the same
  channel list Start does, and does nothing when it is already metering that
  device and channel map — so typing a label does not reopen the interface
  under a bar somebody is watching. The frontend debounces the channel map on
  top of that, because that column is a number box and "12" passes through 1.
- **The trim reaches it** through `set_monitor_gain`, which is deliberately not
  `update_stream_config`: that one changes a session and answers with the
  session state, of which there is none here.
- **A disconnected device ends the metering quietly.** It is not a failed
  session — there is no session — and saying one just failed would be a lie
  about something nobody started. The device list on the same page is what
  says the interface is gone.

**Outstanding:** peak hold, silence indicator, clipping warning, worker queue
depth, and the rest of a global status bar — active workers, queue length and
throughput. `WorkerPool` already exposes `workers()` and `queue_depth()`.

**Accepted when:** a deliberately clipped channel and a silent one are both
immediately recognisable in the UI.

### P9 — Live search & filter — **Done**

**Respecified after P10 was set aside.** It was full-text search over the
stored session (SQLite FTS5); what it is now is a search over the messages the
window is holding, which is the whole chat — every view is caught up from
`session_snapshot` and then fed by `transcript`, so nothing has to be fetched
to search it.

**It belongs to the view and to nothing else.** The chat is shared — the desk,
a detached group window and every browser served over the LAN are looking at
the same lines — but a search is one person's question of it, and pushing it
onto the other screens would put one operator's filter in front of everyone
else's show. So it is frontend state in `ChatList`, emitted nowhere and stored
nowhere, and `search_session` was decided against in P6 for the same reason.
That it lives in the component is also why every view has it: a window that
renders the chat can search it, and there is no second place to add it to.

**One box, two readings**, because an operator switches between them in the
middle of a question. Typing marks the hits and leaves the chat around them —
"did anyone mention the fly bar" wants the sentences either side of the answer.
The *only matches* toggle throws the rest away — "let me watch for this word"
wants nothing else on screen. Beside them, a channel picker narrows the view to
one of its channels, which is the other half of "search in any channel": the
group sidebar already narrows to a group, this narrows inside one.

**Matching is the alert list's rule minus the whole-word requirement.**
Case-insensitive, NFC-normalised, so "Bühne", "BÜHNE" and the decomposed
spelling of either are one word — the same fold the backend's matcher uses
(`crates/app/src/alerts.rs`), because two matchers that disagreed about an
umlaut would highlight a word for the house's alert list and not for the
operator looking for it. Whole words are *not* required, and that difference is
deliberate: an alert fires by itself and a false one teaches the operator to
ignore the next, while a search is read by the person who just typed it, so
"Bühne" finding "Bühnenmeister" is the answer they wanted.

The offsets survive the folding because the fold is done one cluster at a time
and each unit of the folded form remembers where its cluster began. Without
that map a decomposed umlaut earlier in the line moves every later highlight
onto the wrong characters — which is exactly the line this house's transcripts
are full of.

**A hit an hour back is not in the DOM**, which is the one place this runs into
P7's window. ↑ and ↓ therefore grow the window to whole pages until the line is
in it, and then scroll to it — so the window's cost, that a point an hour back
takes several steps of scrolling to reach, is paid by a search that takes none.
Landing there stops the view chasing the bottom, exactly as scrolling up does,
and "Jump to latest" is how it is given back. Enter and Shift+Enter step, and
Escape clears: the other hand is on a fader.

**Both kinds of highlight share one string.** An alert term is the house's
standing rule and a hit is this operator's current question; a word can be
both, and the line is cut at every boundary of either so that neither has to be
dropped. A piece that is both keeps the alert amber — it is the louder of the
two, and what somebody glancing across the room should still see.

**No time-range filter.** The plan named one and it was not built: the channel
picker and the query are what the question is actually asked in, and a
from/to pair on a chat that is read as a chronology is a fourth control to
maintain for something ↑ and ↓ already do. Worth adding if an operator asks for
it; not worth guessing at.

**Accepted when:** incremental search while typing, with highlighting and a
jump into the chat position, over a session of realistic length. *Met — checked
against 600 bubbles: the hit count follows every keystroke, stepping to the
oldest of 86 hits grew the window from 200 to 600 bubbles and put the line in
the viewport, and the channel picker and the only-matches toggle narrow the
list as specified.*

### P10 — SQLite persistence — **Deferred**

**Set aside by the user on 2026-09-18**, with the reason: *this is about live
communication, not about persistence.* A scope decision rather than a
postponement.

What it costs: a session lives in the window and dies with it. Kill the app and
the chat is gone. The configuration is unaffected — profiles, groups, colours
and the model choice are in `config.json` and were never P10's business.

What it changes elsewhere: **P9** becomes a search over the in-memory messages;
**Clear all communication** is only the in-memory half, which is now the whole
action; **P6's `search_session`** was decided against, and search became a
per-view filter over what the window already holds; the whole-system
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
memberships, and the global vocabulary survive a restart and a reconnect of the
interface; the rules that fire on those channels and groups do too (P19).
✔ for the identity half — the reconnect is verified against real hardware in
`capture_live.rs`, `#[ignore]`d because CI has no interface.

**Done:** the presets, as **profiles**. `config.json` in the app's
configuration directory holds a list of profiles — device, default language,
channel map with labels, languages and colours, groups with their memberships,
and the rules that fire on them (P19) — plus the active vocabulary, the
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

**The cheapest latency experiment, and the one to run first.** The ~330 ms is
measured from the *end* of a segment, which is not what anybody waits for. What
an operator waits for is `silence_timeout` plus however long the speaker keeps
going — bounded by `max_segment`, so the worst case is six seconds of talking
before a word of it is readable. Both numbers are constants today (P12), and
both are cheap to sweep: 500 → 300 → 200 ms of silence, 6 → 4 → 2 s of ceiling,
over the fixture set, recording the time from the last spoken word to the text
appearing *and* the word error rate for each pair. Shorter forced segments cost
accuracy — whisper is trained on 30 s windows and degrades badly below about
two seconds of audio, which is where the `[BREAK]` and the invented sentences
come from — so the sweep is not looking for a better default. It is measuring
the exchange rate between latency and accuracy, which is the thing nobody here
knows and every later decision about responsiveness rests on.

Two couplings will corrupt the result if they are not accounted for first:

- **`flush_window_frames` is derived from `silence_frames`** (segmenter.rs).
  Lowering the timeout also shrinks the window in which a forced cut looks for
  the quietest frame, so a single number moves two behaviours. Decouple them
  before the sweep, or the accuracy column measures both at once.
- **`SileroVad` fails open**, so the counter only advances on a frame the model
  confidently calls silence (P3). A lower timeout buys the shorter wait only
  where the detector is decisive, and the fixture set should therefore include
  material where it is not.

It comes before any work on streaming partial hypotheses — re-decoding a
growing buffer and committing only what two successive decodes agree on, the
LocalAgreement policy. That approach can deliver text while somebody is still
speaking, but because whisper's encoder always runs on a padded 30 s window, a
one-second partial costs about as much as a six-second segment: it multiplies
decoder load per stream and would be the first thing to break the P5 budget.
Paying that is only justified if the exchange rate measured here turns out to
be bad. If it is good, the same responsiveness is a changed constant.

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
view of the whole session — and floods only for the view it is showing; a
group window is the other way round, urgent only for its own members and always
flooding when its view asks for it. **Every new bubble is marked in the chat**,
in every view and with nothing to switch it on: that is how the line that just
arrived is told from the ones above it, which is a question an operator watching
eight interleaved channels has whatever their settings say. What a view's own
**Flash on every new message** preference adds is the loud request — the whole
window in dark orange for one pulse per message, distinct from the red of an
alert term. It is off by default, editable while a session runs, stored with the
group and on the profile for the all-channels view, which carries the same
preference. Being painted by the page rather than asked of the platform, it is
the same in the app and in a browser, and it is the one request made **even to a
focused window**: the view it is set on is the one that has to be noticed while
the operator is looking at the meters or another panel of the same window. The
request is counted per bubble and not per segment — one sentence arrives as three
segments, and three flashes for one sentence is exactly the noise this must not
add. The browser viewer highlights the same characters and, having no window to
ask the platform about, makes the rest of the request with its own surface and
tab title.

**Superseded by P19.** Both of these reactions were hard-wired — a term always
highlighted and asked for attention, a view's setting always flooded — and P19
took them apart into triggers and actions. The group flag's two halves survive
as the two halves a rule now has: its channels are when the rule fires, its
window is who is told. What is described above is still
what the app *does*; what has changed is how it is asked for. The alert list
and the two flash checkboxes are gone from Settings, replaced by one rules
editor, and an existing `config.json` is migrated into the equivalent rules on
the way in. The matcher in `crates/app/src/alerts.rs`, the UTF-16 ranges and
the per-bubble counting are unchanged and are what P19 is built on.

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

### P18 — Windows port (WASAPI shared, ASIO opt-in) — **Open**

The seam for this was built in P1 and has held: everything above
`AudioBackend` — ring buffer, resampler, segmenter, protocol, worker pool,
model manager, remote, frontend — is free of the platform. The port is four
things: capture, the GPU backend under whisper.cpp, the sidecar's file name,
and packaging. Only the first two are work.

**The interface has to stay shareable.** On a production machine the
multichannel interface is likely already owned by a DAW, and a transcript is
worth considerably less than the session it would interrupt. An app that takes
the device away from whatever is recording the show is not one an engineer can
have running. This governs the capture design rather than being a detail of it.

- **WASAPI shared mode is the default and the reason the constraint is
  satisfiable.** Shared mode is multi-client by construction — Windows mixes
  the clients, the app never takes exclusive ownership, and nothing it does can
  take audio away from another program. What it does *not* control is how many
  channels are reachable: a driver decides what it exposes as endpoints, and
  most interfaces present their inputs as stereo pairs rather than as one
  32-channel endpoint.
- **Several endpoints replace one stream, and that costs this app nothing.**
  The channels are never compared to one another. Each has its own ring, its
  own segmenter and its own timestamp, taken when the segment ends; there is no
  phase relationship anywhere in the pipeline for a second capture to lose. So
  N stereo captures are worth exactly as much as one N-channel capture here.
  That is what makes the shared-mode route viable at all, and it is worth
  stating because a capture layer written around one interleaved multichannel
  callback reads as though alignment mattered.
- **The fan-out happens below the handle, not in the seam.** `AudioBackend`
  hands back one `CaptureSession`, and the session layer holds exactly one
  (`app/src/session.rs`). The Windows backend therefore owns its several WASAPI
  streams behind a single `CaptureSession` and presents them as one. The
  alternative — a set of sessions above the seam — would rewrite the contract
  the whole app rests on, that `sinks` is ordered exactly like `channel_map`,
  in order to express something no caller needs to see. A stored device is then
  the ordered set of endpoint ids, and the channel map indexes across them in
  that fixed order.
- **ASIO is opt-in, and never reconfigures the driver.** Some interfaces reach
  their full channel count only through ASIO, so the route has to exist. But
  the ASIO specification is single-client; multi-client access is a vendor
  extension (RME and MOTU have it, most do not), and on everything else opening
  the driver locks out whoever holds it — which is the DAW. ASIO is therefore a
  deliberate choice on the Settings page, never a default and never something
  the app falls back into on its own. Where it is chosen, three rules:
  - **Never set the sample rate.** cpal calls `ASIOSetSampleRate` whenever the
    requested rate differs from the driver's current one — `check_config` in
    `cpal/src/host/asio/stream.rs` — and that changes the clock underneath a
    running DAW mid-session. The Windows path asks for the rate the driver is
    already on and lets P2 resample. The app has no preference to defend here:
    everything becomes 16 kHz mono downstream regardless. A rate stored in a
    profile is ignored on this path rather than honoured.
  - **Input only, and the driver's own buffer size.** No output buffers are
    created and the buffer size is taken as found.
  - **Held only while capture runs.** The driver is released on stop, not kept
    for the lifetime of the app.
- **Enumeration is itself intrusive.** Listing ASIO devices means loading each
  driver, and a driver another application holds may refuse or block. The
  device list has to survive that: a driver that will not load is reported as
  present and busy, not dropped and not allowed to fail the enumeration.
  "The interface is in use by another application" is a sentence an operator
  can act on; an empty device list is not.
- **The operator is told which route is running**, because the two are not
  equivalent and the difference is theirs to judge: shared mode may not reach
  every channel, ASIO may not coexist with the DAW.

**Device identity** is the `catalogue()` stub in
`audio-core/src/backend/stable_id.rs`, which already names its replacement:
`IMMDevice::GetId`. The WASAPI endpoint id is stable across reboots and tells
two identical interfaces apart, so it is the equal of the CoreAudio UID and P12
needs no change to use it. ASIO offers only the driver name, which is the weak
identity the module already degrades to — one entry per driver, two identical
units indistinguishable. That is a reason to prefer the shared-mode route, not
a defect to fix.

**The GPU backend is the second piece of real work.** `whisper-rs` is pinned to
`features = ["metal"]` in the workspace manifest, unconditionally, and does not
build on Windows at all. It has to become per-target — Metal on macOS, Vulkan
on Windows, CUDA where it is worth the build complexity, CPU as the floor —
which in the manifest means keeping *one* optional dependency behind
`audio-core`'s `silero` feature with per-target features, not two dependencies.
The mechanical part is small. What is not small is that every number in
measurements.md is a Metal number: the ~330 ms end-to-end budget, the ~8 s
first-start shader compilation the UI announces, the pool's sizing, the
0.41 ms per VAD frame. None of them carries over. The port therefore includes
re-running P14 on the target hardware, and the default worker count is likely
to come out differently.

**The rest is certain and small.** `worker::find_binary` joins
`"whisper-worker"` with no extension and needs `std::env::consts::EXE_SUFFIX`,
or the sidecar is never found. The bundle gains an `nsis` target, the installer
gains the WebView2 runtime as a prerequisite, and signing is an EV certificate
or Azure Trusted Signing with SmartScreen reputation in place of notarisation
and Gatekeeper — P13 done a second time rather than extended. Windows has no
`Info.plist` equivalent, but it has the same failure: the *Allow desktop apps
to access your microphone* privacy setting applies here too, and a denied
microphone has to be distinguishable from a silent channel. Two tests are
Unix-only — the `/bin/sh` child in `resources.rs` and the `pgrep`/`kill` pair
in `tests/pool.rs` — and `justfile` is bash with `curl`, `say` and `afconvert`
while `scripts/run.sh` hardcodes the `.app` path, so one of the two needs a
PowerShell sibling. `rust-toolchain.toml` lists only `aarch64-apple-darwin`,
and CI needs a second job with MSVC and cmake for whisper.cpp.

**The ASIO SDK cannot be redistributed**, which is why ASIO must be a cargo
feature CI builds *without*: the SDK plus LLVM/bindgen are the builder's to
have, and a build without them has to produce a working app with the
shared-mode route intact.

**Accepted when:** on a Windows machine with a multichannel interface that a
DAW is holding at the same time, eight channels transcribe and the DAW is
undisturbed — no dropout, no sample-rate change, no lost device. Separately:
the ASIO route runs on a multi-client driver, and on a single-client one it
fails with the interface named as busy rather than by taking it.

**Risk:** which channels a given driver exposes through WASAPI is a per-vendor
fact that no amount of reading settles, and the shared-mode plan rests on it
entirely. This is the same risk P1 still carries on macOS — the channel mapping
that only real hardware can confirm — and it should be answered on the target
interface early, because the answer decides whether the opt-in ASIO route is a
corner case or the normal one.

### P19 — Rules: triggers and actions — **Done**

**What it is.** One list of **rules**, each a *trigger* and one or more
*actions*, in place of the two hard-wired reactions the app had. A rule is
written on the Settings page, stored in `config.json`, and evaluated once per
transcript wherever the reaction it asks for belongs.

Before it there were exactly two reactions and they were two different
features. An alert term highlighted its word and asked the platform for
attention (P17); a view's **Flash on every new message** flooded the window
(P17). They were asked at different scopes — terms global, flashing per view —
matched in different places, and could not be combined or extended. The three
things an operator asks for next were all outside that shape: *"Feuer, but only
on the stage manager's channel"*, *"anything at all on channel 3"*, and *"when
the inspector says Go, fire the cue"*. None of the three is a variation on the
old two; all three are the same shape once triggers and actions are separate.

**Done: the model, the evaluation, the editor, the migration and the
webhook.**

**Triggers** are a conjunction of two questions: *these words* (or any text)
said on *these channels* (or any channel). The words go through P17's matcher
unchanged — case-insensitive, NFC-normalised, whole-word, so `cue` still does
not fire on `rescue`. An empty channel list is "every channel", which is what
the all-channels flash flag became; an empty word list is "any text", which is
what "anything at all on channel 3" needs. A trigger fires per **bubble**, not
per segment — P17's rule, and the reason one sentence spoken across three
segments is one event.

**A channel is an input; a group is a window**, and a rule says both
separately. They were one list of "places" in the first cut of P19, and that
was wrong in a way only the app showed: naming a group narrowed *when* a rule
fired and said nothing about *who was told*, so a rule restricted to the stage
manager's view still flooded every window that happened to show one of its
channels — the main window included. Narrowing the trigger cannot narrow the
audience. So the flash carries its own list: the **views** it is for, named by
the group positions a group window is addressed by (`group-3`). A window
floods when the channel is on screen *and* the flash was addressed to the view
it currently is; the all-channels view is addressed only by a flash that names
no view at all, which is the old "every window showing the channel" and still
the default. The main window answers with whatever its sidebar has selected,
because that is the view it is at that moment. The editor keeps the two in
separate rows for the same reason the model does, and a group that is deleted
switches off a rule addressed only to it rather than letting the flash widen
to every window.

**Actions, in the order they are worth building.** The first three are built.

1. *Flash the window* — the flood that existed, now one action among several.
2. *Highlight the match, ring the bubble* — the alert term's chat behaviour.
   Only a rule with words can ask for it, and the editor disables the box on
   one without: there would be nothing to mark.
3. *Ask the platform for attention*, urgently or once — the old two requests,
   now two actions that can be asked for separately.
4. *Call a webhook.* The last one. One HTTP request, written out on the rule:
   a URL, a method, headers and a body, all free text and all templated. It is
   one arm of `rules::Action`, and the request beside it on the rule —
   `CompiledRule::webhook`, the way `views` sits beside the flash — because an
   arm carrying a URL would put that URL into the action list that travels
   back out with every transcript, to every window and every browser. The
   firing is one branch in `Rules::evaluate`, because the evaluation was put
   where an action can fire exactly once — see below.

**Why one free-text request and not a list of integrations.** This action
replaces the MIDI cue that stood here until 2026-09-19, and it replaces it by
going around it. The thing a theatre actually points this at is Bitfocus
Companion, which is itself a fan-out point: a Companion button reaches every
device Companion has a module for — MIDI among them — so the cue still
arrives at the desk, by a route this app does not have to own. Speaking MIDI
directly would have reached what speaks MIDI; speaking HTTP to Companion
reaches that and everything else, for less code.

But Companion is one destination and not the shape of the feature. Binding the
action to it would mean the next thing somebody wants to call — a media
server, node-red, a light desk's own gateway, something bought after this was
written — needs a release. So there is no preset and no endpoint catalogue:
the editor has a URL box, a method, a header list and a body box, and what is
sent is what was typed. Companion appears as a *hint* under the URL field,
two examples to copy —

```
POST http://<companion-host>:8000/api/location/1/0/2/press
POST http://<companion-host>:8000/api/custom-variable/last_call/value
     body: {{text}}
```

— the first being the cue trigger, the second putting the transcript itself on
a button face, which is the thing a MIDI note could never have carried.

**The body is a template**, over the same values the highlight already has:
`{{text}}`, `{{channel}}`, `{{rule}}`, `{{matched}}`, `{{timestamp}}`. That is
what lets one mechanism serve both examples above without knowing which is
which. The URL and every header value are templated too, and each of the three
is escaped for the place it is: percent-encoding in the URL, so a transcript
cannot rewrite the request it was put into; control characters stripped from a
header value, because a newline there is header injection and a transcript is
text this app did not write; and JSON escaping in a body whose `Content-Type`
says JSON, because `{"said": "{{text}}"}` is what an operator writes for
node-red and a quotation mark in what was said would otherwise arrive
unparseable. A placeholder nobody defined is left as it was typed: a body
arriving with `{{txt}}` still in it says what the mistake was, where an empty
string would say only that nothing was said.

**No new dependency.** `reqwest` has been in `crates/app` since P11's model
downloads, with rustls and `system-proxy` already on — so an HTTPS
destination, including one behind a house proxy, works with what is in the
tree.

**Where a rule runs.** The two kinds of action have opposite homes, and the
split was designed before the first rule was stored:

- A *flash* is about a **view**. Which window flashes depends on what that
  window is showing *and* on which views the rule named. Several windows may
  each answer for one transcript, and that is correct — but only the ones the
  rule is for.
- A *webhook* is about the **session**. It must fire exactly once per matching
  bubble — not once per open window, not twice because the LAN viewer is also
  connected, and not zero times because every window was closed.

So the rules are evaluated in `crates/app/src/rules.rs`, from
`TauriEvents::transcript`, which is the one place every view's copy of an event
comes from: it produces the highlight ranges and the list of actions the
segment asked for, and both travel with the event. Each view then applies the
*visible* ones by its own scope — `ui/src/lib/attention.ts` — and counts them
per bubble, where the bubbles are: a group window's bubbles are not the main
window's. That is also what makes the outbound actions a matter of adding an
arm rather than a second mechanism. Evaluating only in the frontend would have
made a fired cue depend on which windows happen to be open, which is not a
property a cue may have.

**Compiled on the way in.** A rule crosses the bridge with its channels
already resolved to stream ids and its views reduced to group positions
(`set_rules`), because groups exist only in the frontend — the backend has
never had a notion of one — and a rule is evaluated against a stream id. The
positions it never interprets: they ride back out with the event, and each
window compares them with the view it is. The same argument as `set_alert_terms` before it applies
to *when*: the rules are pushed when they change rather than read from the
file, because the file is a debounce behind the rule just written, and a rule
that starts working a second late is a rule nobody can tell is working. A rule
narrowed to channels that have all been deleted is left out of what is sent
rather than sent with an empty stream list, because "listens to nothing" and
"listens to everything" must not look alike on the wire.

**Rules live on the profile**, unlike the vocabulary and unlike the alert list
they replace. A trigger names channels and groups, and neither exists outside a
profile: a shared list could hold "anything on channel 3", and channel 3 is a
different channel in the next production. The cost is that a word rule wanted
everywhere is written in each profile, which is why the migration copies the
old global terms into *every* profile rather than into the first.

**Rate limiting is per rule**, stored and editable, zero by default. A channel
trigger fires on every bubble, which on a busy talkback is several a minute:
harmless for a highlight, and exactly the flood the outbound actions must not
become. A rule inside its interval does nothing at all, including highlighting
— "this rule is quiet for a second" is one state an operator can hold, and a
rule that half-fires is two. The limit is spent by *firing*, not by being
asked, so a channel nobody stops talking on cannot keep a word rule quiet for a
word that was never said.

**A deleted channel leaves no rule pointing at nothing.** Positions are pruned
the way group membership is, on load and live, with one addition: a rule whose
*every* named place is gone is switched off rather than left to fire. With the
last of its channels deleted the trigger would read as "anywhere", and a rule
that grew louder because something was deleted is the one surprise a show
cannot absorb. It stays in the list, so it can be seen and re-pointed.

**Migration.** `config.json` went to version 2, and a version 1 file is
migrated on the way in: each alert term becomes a word rule with the highlight
and urgent-attention actions; each group with flashing on becomes a group
trigger with the flash action; the all-channels view's flag becomes a rule with
no trigger at all. The three legacy fields are read and never written, so the
migration is a one-way trip rather than a flag that comes back beside the rule
it became. Nothing is lost and nothing has to be re-entered — which is also why
P17's two Settings sections are replaced by one editor rather than kept beside
it.

**The editor** is a card per rule with the whole rule on it: the name and the
switch, the words as one comma-separated field, the channels and groups as
chips, the actions as checkboxes, and the interval. Triggers, conjunctions and
actions are a small language, and the way a small language goes unused is a
page that hides half of it behind a disclosure — so the line under the name
says the rule back in words ("Feuer, on Bühne"), which is what makes a list of
six of them scannable. Editable while a session runs, like the alert list it
replaces: a rule written mid-show takes effect on the next segment.

**Built on 2026-09-19**, in place of the MIDI cue that stood here until that
day rather than beside it. `crates/app/src/webhook.rs` is the whole of the
outbound side: the request, the template, the queue and the counters.

**The constraint, and the decision taken on 2026-09-19.** *Offline is a hard
requirement* and the remote deliberately never leaves the local network (P16).
A webhook is an outbound HTTP call from the machine holding crew
communication — and, by the user's decision, one that may name any host on the
LAN *or* the internet. That is a deliberate widening of "no cloud handling of
crew communication", not an oversight, and it is what makes the rest of this
paragraph necessary rather than cautious.

The question P19 parked was *who may write such a destination*. The rule editor
is reachable from the LAN remote, and the remote has no authentication by
decision. An arbitrary URL writable from there means anyone on the house
network can point crew speech at an off-site endpoint from a browser form,
which is a different proposition from the read-mostly remote that decision was
made for. So: **a webhook action is written host-only.** The remote may see a
rule that has one, switch it off and edit its trigger; it may not create or
change the request. The URL box does not exist on the remote. This costs
nothing an operator wanted — the machine is where a destination is set up
anyway — and it leaves P16's authentication where it was, an obvious next
thing to want rather than a prerequisite.

**The body defaults to the minimum.** `{{text}}` is the exposure; a rule name
and a channel are usually all a trigger needs to say. So a new action starts
with the transcript *out* of the body, and putting it in is a thing the
operator does on purpose. The default should not be the one that quietly ships
what was said in a production off the premises.

**It must not block the transcript.** Rules are evaluated in
`TauriEvents::transcript`, so a synchronous request to an unreachable host
would stall transcription for the length of its timeout — and an unreachable
host is the *normal* case in a venue with no route out. The send therefore
leaves on a task of its own, over a bounded queue, with a short timeout, **at
most one retry, and then a drop that increments a counter.** A cue that arrives
forty seconds late is worse than one that never arrives; this is a show, not a
message bus. The drop count belongs beside P8's stream health, so that a dead
endpoint is visible rather than silent. P19's per-rule rate limit already keeps
a channel trigger on a busy talkback from becoming a flood of requests.

**Accepted when:** a rule saying "Go on the inspector's channel" flashes one
window and sends one HTTP request, once, with two group windows and a browser
viewer open; a rule with a channel trigger and no word fires on every bubble
from that channel; deleting a channel leaves no rule pointing at nothing; an
existing `config.json` from P17 comes back as the equivalent rules; a webhook
whose host does not answer costs the transcript nothing and shows up as a drop;
and the URL box is absent from the rule editor served to the remote. ✔

**What the webhook is not.** The host-only rule is a rule about the *editor*,
which is what the threat it was written against is — an operator, or anyone
else on the house network, typing a destination into the form the app serves.
It is not enforced on the wire: a browser that sends a crafted `save_config`
frame can still put a URL on a rule, because the remote has no authentication
and the configuration round-trips through it whole. That is the same gap P16
already has and the same one its authentication closes; nothing here widens
it, and nothing here waits for it.

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
                              │        │
                              │        └─> P19 Rules (triggers/actions)
                              │
                              └─> P16 LAN remote client   (P10 SQLite: deferred)
                                       │
                                       └── P12 Settings
                                       v
                                 P13 Packaging
                                       │
                                       v
                                 P18 Windows port
```

P19 hangs off P17 because it generalises it rather than adding to it: the
matcher, the per-bubble counting and the two reactions all become one case of a
rule, and its outbound action needs P12's file to write it down in.

P18 hangs below everything because it reopens three packages rather than
following them: P1's capture, P4's GPU backend and P13's packaging, each for a
second platform. Nothing between them changes.

**Milestone A (technical backbone):** P0 → P1 → P2 → P4 → P5. *Reached; what P5
still owes is a load test on live audio.*

**Milestone B (usable MVP):** + P6, P7, P11, P12, P16, P17, P19. *What is left
is the settings proper — every threshold is still a constant in the code.*

**Milestone C (language quality & production readiness):** + P14 fixtures, P15,
P8, P9, P13. *P10 left it on 2026-09-18; P3b was dropped.*

**Milestone D (second platform):** + P18. *Deliberately after C: a Windows
build of an app whose own numbers are not yet measured would be measured
against nothing.*

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
   guesses. The latency/accuracy sweep described under P14 is what turns the
   two segmenter constants from guesses into a measured trade-off, and it needs
   step 1's fixture set to run at all.
3. **P15** — develop vocabulary against the measurable baseline from step 1,
   not by feel. The control exists and is global; what is missing is the
   evidence that any particular list is better than another.
4. **The remote, on a real network.** P16 has not had a second machine, a house
   network and somebody working from it. That comes before authentication —
   the obvious next thing to want, and the wrong thing to build against an
   arrangement nobody has used yet.
5. **P13, packaging and signing.** With P6, P7, P9 and P19 in, what is left
   between this and a machine that is not this one is a notarised build.
6. **The webhook against a real Companion.** Built and tested against its own
   unit tests on 2026-09-19; what it has not had is a desk. The two things
   only a house can answer are whether a Companion press URL fires from the
   body the editor produces, and whether three seconds and one retry are the
   right numbers on a network with a show on it.
7. **P18, the Windows port.** Last, and not because it is hard: it reopens
   capture, the GPU backend and packaging, and each of the three is cheaper to
   redo once the macOS answer it is being ported from is measured rather than
   assumed. The one part worth pulling forward is the hardware question in its
   **Risk** — which channels the interface exposes through WASAPI — because it
   can be answered on the target machine in an afternoon and decides the shape
   of the rest.

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
5. Enter a search term → the hits are marked across every stream in the view,
   ↑ and ↓ walk them and reach one from the top of a long session, "only
   matches" leaves the rest out, and the channel picker narrows to one of them.
   None of it changes what a second window or a browser on the LAN is showing
   (P9).
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
13. Write a rule on `cue` that highlights and asks urgently, unfocus the main
    window and speak "Cue fünf" → the exact word is highlighted and the window
    requests attention; "rescue" does neither. Repeat from a channel in an open
    group to verify its separate window. Then narrow the same rule to one
    channel → it fires there and nowhere else. Write a second rule with no
    words, one group and the flash action, and speak on a member channel → the
    whole window floods dark orange, focused or not; a channel outside the
    group does not flood a window that is not showing it. Give that rule a
    two-second limit and speak twice in a second → it fires once. Delete the
    channel the first rule named → the rule is still there and switched off,
    never widened to every channel. Verify that the browser highlights the same
    characters, floods the same way, and flashes its own surface and tab title
    (P19, P17, P16).
    1. Add the webhook action to the `cue` rule, pointed at a Companion button
       (`POST http://<companion>:8000/api/location/1/0/2/press`), and speak the
       word with two group windows and a browser open → the button presses
       **once**. Put `{{text}}` in the body of a second rule pointed at
       `/api/custom-variable/last_call/value` → what was said appears on the
       button face, umlauts intact.
    2. Point a third rule at a host that does not exist and speak → the
       transcript arrives on time, and a few seconds later the meter strip
       says one webhook was dropped, with the reason in its tooltip.
    3. Open the same Settings page from the browser → the rule shows that it
       calls a webhook and offers no URL, method, header or body box. Untick
       the action there and speak → the button does not press. Tick it again
       on the machine and the destination is still the one that was typed.
14. On Windows, with a DAW holding the same interface: start a session on eight
    channels and let the DAW run through it — the transcripts appear and the
    DAW is undisturbed. Then switch the capture route to ASIO on a driver that
    is single-client and confirm the app says the interface is busy instead of
    taking it (P18).
15. With a long chat open, switch to **Newest at top** while reading an older
    bubble → the same bubble stays under the cursor, older pages load below and
    a new transcript appears at the top. Switch back to **Newest at bottom**
    and verify the inverse; the main window, an open group window and the LAN
    browser retain their own chosen direction (P7).

There is deliberately no "restart and the session loads" step: with P10 set
aside the chat lives in the window, and killing the app loses it by design.

---

## Deliberately out of scope

- Export (TXT/SRT/JSON). With P10 set aside it would have to read the window's
  own messages.
- Markers and bookmarks.
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

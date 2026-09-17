# Plan and progress

The sixteen work packages of the MVP, and where each one actually stands.

Plan and status live in the same file on purpose. Kept apart they drift, and a
plan that no longer describes the code is worse than no plan — it reads as
authority while being fiction.

Evidence for the architecture decisions is in
[measurements.md](measurements.md); this file references it rather than
repeating numbers, so there is one place to correct when a measurement changes.

**Status of this document:** 2026-09-17.

---

## Context

Audio engineers monitor several intercom channels during a production and
inevitably lose information — nobody can follow eight conversations in
parallel, and what was said is gone. This app transcribes up to eight intercom
streams locally and near-live and shows them as a chat log, so what was missed
stays readable and searchable.

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
| UI scope, MVP | Chat view with a group sidebar, live search and filter, levels and stream health, vocabulary editor, and a separate Settings page for channel setup |
| Not in the MVP | Export, markers, diarisation, cloud, multi-user |

---

## Status at a glance

**Done** — meets its acceptance criteria. **Partial** — usable, but something
named below is missing. **Open** — not started.

| | Package | Status | What is missing |
|---|---|---|---|
| P0 | Scaffolding & toolchain | **Done** | — |
| P1 | Audio capture (macOS, multichannel) | **Partial** | Channel mapping unverified on real hardware |
| P2 | Ring buffer & resampling | **Done** | — |
| P3 | VAD & segment scheduler | **Partial** | Speech detection is energy-based; Silero sits in the sidecar, not in the segmenter |
| P3b | Language stabiliser | **Open** | — |
| P4 | Whisper sidecar | **Partial** | Not registered as a Tauri `externalBin` |
| P5 | Worker pool & backpressure | **Partial** | No reference load test; still fed by a single stream |
| P6 | Tauri API | **Partial** | No generated shared types; level events not bundled across streams |
| P7 | Chat UI | **Partial** | One stream, no virtualisation, no language picker from the sidecar's list |
| P8 | Levels & stream health | **Partial** | Level, overrun count and overload count only |
| P9 | Live search & filter | **Open** | — |
| P10 | SQLite persistence | **Open** | — |
| P11 | Model manager | **Open** | Models discovered from `.scratch/models` or environment variables |
| P12 | Settings & presets | **Open** | Every threshold is a constant in the code |
| P13 | Packaging & signing | **Open** | `Info.plist` with the microphone usage description exists |
| P14 | Performance & latency budget | **Partial** | Latency measured on one stream; no reference load test, no WER baseline |
| P15 | Vocabulary & prompt management | **Partial** | A single text box, not stored, not named |

**What runs today:** one channel end to end — microphone, ring buffer,
resampling, segmentation, speech gate, worker pool, chat list — at a measured
~330 ms from the end of a spoken segment to the text appearing. The pool
behind the segmenter is already indifferent to how many streams feed it; what
is still single-stream is everything in front of it.

---

## Deviation from the planned order

The plan sequences P2 → P3 → P5 → P6 → P7. After P2 the work switched to a
**narrow vertical slice** instead: one channel through every stage, with a
deliberately crude implementation of each.

The reason was that "latency first" was, at that point, an assumption. Nothing
had ever run end to end, so the 1–2 s target could not be checked against
anything. Building P3 and P5 properly first would have meant several more days
of work resting on a number nobody had measured.

It paid for itself twice:

- The measurement came out at ~330 ms end to end, of which 300–340 ms is the
  decode. **Everything else in the pipeline costs 1–2 ms.** That settles where
  tuning is worth doing — model size and segment length — and where it is not.
- Running it on real audio exposed problems no unit test would have found: a
  quiet channel filled the chat with `[BREAK]`, `(music)` and, later,
  hallucinated sentences. That is now the three-layer defence in P3/P4, and it
  would otherwise have surfaced during acceptance rather than during
  development.

What the slice does provisionally is listed per package below and marked in the
code.

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
apart from quiet, not speech from a bell. See P3.

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
3. **Vocabulary prompt per stream.** The strongest quality lever for code
   switching — measured, see measurements.md §4. Package P15.
4. **Auto-detect as opt-in, restricted to a candidate list.** Not free
   detection across all ~99 whisper languages, but argmax over the probability
   vector restricted to the languages declared for that stream. Package P3b.

**Extensibility is a hard requirement:** languages are hard-coded nowhere.
Modelled as `LanguageCode(String)` against the full whisper language list
queried from the sidecar. A third language costs configuration, not code.

---

## Packages

### P0 — Scaffolding & toolchain — **Done**

Cargo workspace with four crates: `app` (Tauri backend), `whisper-worker`
(sidecar binary), `whisper-protocol` (shared frame format), `audio-core`
(capture/VAD/segmentation, UI-free and therefore unit-testable). Frontend in
`ui/` with Vite + Svelte + TS, Tauri v2, pnpm, pinned Rust toolchain,
`justfile`, CI on `macos-14`.

**Accepted when:** `just dev` opens a window on Apple Silicon, CI is green. ✔

### P1 — Audio capture (macOS, multichannel) — **Partial**

`AudioBackend` trait as the platform seam, macOS implementation on `cpal`.
Device enumeration, one input stream on the chosen multichannel device,
de-interleaving into N mono channels inside the callback, configurable
stream→channel mapping, disconnect handling.

The callback is a realtime context: **no allocation, no lock, no logging**. The
`Demuxer` therefore owns its sinks and its scratch buffer.

**Accepted when:** an integration test writes 10 s of two channels of an
aggregate device into separate WAVs and the channels are provably not swapped.

**Outstanding:** the de-interleaving is covered by unit tests including the
case where a device buffer spans several scratch passes, which is where an
off-by-one would hide. But on the 32-channel interface no signal was present
during testing, so the end-to-end mapping is **unconfirmed on real hardware**.
Patching a known source to one specific channel and checking that only the
matching WAV carries it remains a manual step.

**Known limitation:** `cpal` exposes only `name()`, no stable device id. Two
identical interfaces produce the same id. Affects P12.

### P2 — Ring buffer & resampling — **Done**

Lock-free SPSC ring buffer per stream between callback and processing thread.
Resampling to 16 kHz mono f32 with `rubato` (sinc) in the processing thread,
never in the callback. Per-stream overrun counters. f32 normalised to [-1, 1].

**Accepted when:** 48 → 16 kHz is spectrally verified against a sine; an
artificially blocked consumer produces overrun counts rather than a panic or an
audio glitch. ✔ — a 12 kHz tone that would alias onto 4 kHz is suppressed by
92 dB; see measurements.md §7.

### P3 — VAD & segment scheduler — **Partial**

`Vad` trait with an energy-based first implementation. Segmentation rules per
stream: start on speech onset with ~300 ms pre-roll, end after ~500 ms of
silence, force-flush at `max_segment_len` (6 s) **cut at the quietest frame of
the recent past**, discard anything under ~300 ms.

**Accepted when:** unit tests on a WAV fixture with known pauses produce the
expected segment count and boundaries (±100 ms). ✔ for the segmenter.

**Outstanding — the detector.** `EnergyVad` measures loudness, and loudness is
all it can measure: a bell, a chair scrape and a spoken word are
indistinguishable to it. It works in a quiet room and will not survive an
intercom feed with constant background noise.

Silero *is* now in use, but as a gate inside the sidecar (see P4) — after the
segment has already been cut and sent. Moving it in front of the segmenter,
behind the existing `Vad` trait, would additionally save the IPC round trip and
the segments that are created only to be rejected. Measured: 2 of 11 segments
in a live session passed the gate, cost a full ~320 ms decode, and were then
discarded by the text filter.

### P3b — Language stabiliser (auto mode only) — **Open**

No-op for streams with a fixed language. Evaluates the probability vector
**restricted to the stream's candidate languages** — not a global argmax —
with a sticky language and hysteresis, a minimum segment length for a detection
decision, and a manual override from the UI.

**Accepted when:** no language jump within one continuous contribution on a
mixed-language fixture; a genuine DE→EN speaker change is adopted within two
segments.

### P4 — Whisper sidecar — **Partial**

Own binary linking `whisper-rs` with `metal`. Loads the model once, reports
`ready`, then serves length-prefixed frames: JSON header plus raw f32 PCM,
binary rather than base64 to save copying on the latency path. `list_languages`
exposes the full whisper language list so none is hard-coded. Greedy decoding,
`no_context`, `single_segment`, no temperature fallback, `translate = false`.

Logging hooks are installed as the very first action — otherwise whisper.cpp
writes on stdout and corrupts the frame protocol.

**Speech gate.** A Silero VAD context is loaded alongside the model and run
*before* decoding. No speech segments means empty text and no decoder call.
This is the only component in the chain that models speech rather than
loudness. Three earlier attempts failed and are documented in measurements.md
§8, including one that appeared to work and did nothing at all.

**Accepted when:** a CLI test pipes a WAV fixture in and gets correct German
and English text back; a mixed-language fixture returns the English terms
verbatim; the model is provably loaded only once. ✔

**Outstanding:** registration as a Tauri `externalBin` so the binary is bundled
and co-signed. Belongs with P13.

### P5 — Worker pool & backpressure — **Partial**

Pool of M sidecar processes (default 2) on **one** shared queue, oldest segment
first so no stream starves. Hard bound of 32 segments; on overflow the
*oldest* is dropped and an `overload` event fired per stream — **silent loss is
explicitly forbidden**. Supervision: crash or timeout kills and respawns the
process, requeues the affected segment once, then discards it. A result
aggregator orders results by `seq` per stream.

**Accepted when:** a load test with eight synthetic continuous-speech streams
runs 10 minutes without memory growth; a `kill -9` on a worker is healed within
~2 s without a UI error. ✔ for the second — `tests/pool.rs` kills the pool's
own sidecars mid-session and asserts that they come back and keep decoding.

**The invariant everything else rests on:** every segment the pool accepts
produces **exactly one** result, including the ones it gives up on. Without it
the reorder buffer would stall on the missing sequence number and lose that
stream for the rest of the session — a far worse failure than the dropped
segment itself. It is asserted over a whole run rather than per segment,
because that is the only way it can be asserted at all.

Three things fell out of building it that were not in the plan:

- **Dropping the oldest is the opposite end from the one work is taken from.**
  Workers pop the oldest so a busy stream cannot starve a quiet one; overflow
  drops the oldest because by then its transcript is the one worth least. Both
  rules are right and they point in opposite directions, which is worth
  knowing before someone "fixes" one of them.
- **A wedged worker is killed, not signalled.** The decode is blocked reading a
  pipe that will never deliver again, so there is nothing to interrupt.
  Killing the process closes the pipe, and the read then fails exactly like a
  crash — so timeout and crash are healed by one code path rather than two.
- **Stop has to drain the queue.** Stop is pressed *after* the last thing worth
  hearing was said, so discarding the backlog reliably loses the end of every
  session. The queue now drains within a 5 s grace period before it closes.
  Found by a test asserting the invariant above, not by reading the code.

A failed respawn is retried three times before the pool gives up on that
worker: a replacement can be killed while it is still loading its model, and
treating that as permanent turns a passing problem into a permanently smaller
pool.

**Outstanding:** the reference load test, which belongs with P14 and needs the
multi-stream session from P6/P7 to be worth running. Until then the pool size,
the queue bound and the timeouts are reasoned defaults, not measured ones.

### P6 — Tauri API — **Partial**

Commands `list_devices`, `start_session`, `stop_session` exist; events
`transcript`, `level`, `overload` and `session-state` are emitted.

Two things learned here, both in the code as comments:

- **Tauri v2 gates the core plugins behind capabilities.** Commands defined by
  the app need no permission, but `listen` belongs to the core event plugin.
  Without `capabilities/default.json` every subscription is rejected — and an
  unhandled rejection is silent, which makes a working backend look like a hung
  one.
- **`start_session` returns the running state** rather than only emitting it,
  so the transition the operator waits for after pressing the single global
  Start button arrives as the answer to that click. Events carry only what
  happens afterwards.
- **Start is non-destructive.** It starts or resumes capture without clearing
  the transcripts already displayed or stored for the current session. Clearing
  history, if offered at all, is a separate explicit action and never a side
  effect of Start or Stop.

**Outstanding:** shared types generated from the Rust structs (`ts-rs` or
`specta`) instead of hand-written TypeScript; level events bundled across all
eight streams at 20 Hz rather than one event per stream; `update_stream_config`,
group CRUD and channel-to-group assignment commands, and `search_session`.

**Accepted when:** the frontend receives typed events; eight streams under full
load keep the WebView main thread under ~30 %.

### P7 — Chat UI (Svelte) — **Partial**

Today: one stream, a device and channel picker, a label, a language field, a
vocabulary prompt, and a chat list with timestamp, language badge and measured
latency per message. Follow behaviour with a "jump to latest" button. Channels
are numbered from 1, the way they are numbered on an interface; the backend
stays zero-based and the conversion happens at that one boundary.

**Target interaction model.** Channel configuration belongs on the dedicated
Settings page, not in the live chat view: input mapping, label, language,
vocabulary assignment and channel colour are edited there. Each channel has a
user-editable colour; a deterministic palette colour is preselected when the
channel is created, with enough contrast for labels, badges and level meters.

The chat page has a left sidebar for groups. An operator can create, rename and
delete groups, and select a group to focus the chat on its member channels.
Groups are non-exclusive views: a channel may belong to any number of groups,
and removing it from a group never removes the channel itself. The sidebar also
offers an all-channels view. Starting and stopping is session-wide, through one
global Start/Stop control; there is deliberately no per-channel Start button.
Start leaves the existing chat history visible and appends new transcripts to
it; neither Start nor Stop clears the list.

**Outstanding:** several streams side by side with their configured colours and
free labels; group sidebar and membership management; **virtualised list** — an
hour across eight streams is tens of thousands of nodes; Settings page for
channel setup with per-stream language configuration from the sidecar's language
list rather than a text field; one global session Start/Stop control; one-click
language override.

**Accepted when:** eight streams run visibly live; scrolling stays smooth at
10,000 segments; a third language can be chosen without a code change; a
channel has a preselected but editable colour; a channel can appear in two
groups and is visible in both; all configured channels start from one action.
Starting again after a Stop retains the preceding chat history and appends new
segments to it.

### P8 — Levels & stream health — **Partial**

Today: one level meter (RMS and peak, computed in Rust), the per-stream
overrun count from P2, and the per-stream count of segments the decode queue
had to give up on (P5). The two counts are deliberately separate: one is audio
lost on the way *into* the pipeline, the other is audio that was captured
cleanly and then could not be kept up with, and they call for different fixes.

**Outstanding:** peak hold, silence indicator, clipping warning, worker queue
depth, and a global status bar with active workers, queue length and
throughput. `WorkerPool` already exposes `workers()` and `queue_depth()` for
it.

**Accepted when:** a deliberately clipped channel and a silent one are both
immediately recognisable in the UI.

### P9 — Live search & filter — **Open**

Full-text search over the running session (SQLite FTS5, see P10), incremental
while typing, with highlighting and a jump into the chat position. Filters for
streams and time range.

**Accepted when:** search over a two-hour session answers in <100 ms and does
not disturb ongoing writes (WAL mode).

### P10 — SQLite persistence — **Open**

Schema: `sessions`, `streams`, `segments`, `segments_fts`, `vocabularies`,
`devices`. Language fields are written **from day one**, including for streams
with a fixed language — it costs nothing and makes misdetections analysable
later. WAL mode with batched writes, not one commit per segment. Versioned
migrations. Crash recovery marks an incomplete session as ended and keeps the
data.

**Accepted when:** a hard-killed app loses at most the last write batch and the
session loads completely afterwards.

### P11 — Model manager — **Open**

Curated model list with URL, SHA256, size and expected latency. Download with
progress, resume and **checksum verification** into the app data directory.
Settings UI for selection; switching restarts the workers. A clear "no model"
state instead of silent malfunction. Manual path to a `.bin` as an offline
fallback.

**Recommendation matching "latency first": `ggml-small-q5_1` as the default**,
`base` as the faster and `large-v3-turbo` as the more accurate option.

**Today:** `just model` and `just vad-model` download into `.scratch/models`;
the app searches upward from both the working directory and its own binary,
picks the largest non-Silero `.bin` as the speech model and the newest
`ggml-silero-*.bin` as the VAD model. `INTERCOM2CHAT_MODEL`,
`INTERCOM2CHAT_VAD_MODEL` and `INTERCOM2CHAT_WORKER` override each.

**Accepted when:** a fresh machine without a model is guided through the
download to the first transcription; a tampered file is rejected by checksum.

### P12 — Settings & configuration persistence — **Open**

Persistent settings: worker count, model choice, VAD thresholds, maximum
segment length, hysteresis parameters from P3b, theme. Session presets: a named
channel mapping including labels, colours, language configuration and assigned
vocabulary, plus named groups and their channel memberships — an engineer does
not want to remap eight channels or rebuild their working views before every
show. Channel colours are automatically assigned from a deterministic palette
when no saved choice exists, then persist as explicit user-editable values.
The Settings page is the home for this configuration. Device recognition via a
stable id, not the display name.

**Accepted when:** a preset, including channel colours and non-exclusive group
memberships, survives a restart and a reconnect of the interface.

**Candidates already identified as needing to be settings:** the segmenter's
`silence_timeout` (it is a *latency* parameter, not only a quality one — it is
the time the segmenter must wait before it can know a segment ended), and the
Silero threshold, which trades decode time against the risk of losing quietly
spoken commands.

### P13 — Packaging & signing (macOS) — **Open**

arm64 build, sidecar correctly bundled, code signing with hardened runtime,
notarisation, DMG.

**Done already:** `Info.plist` with `NSMicrophoneUsageDescription`. Without it
macOS does not even ask — the callback silently delivers zeros, and a silent
channel is indistinguishable from a denied permission. That is why it is in
place from the start rather than at packaging time.

**Outstanding:** the `com.apple.security.device.audio-input` entitlement, which
hardened runtime gates separately, and the sidecar's own signature.

**Accepted when:** the DMG installs on a foreign Mac and runs without a
Gatekeeper warning.

**Risk:** hardened runtime plus a sidecar process is the classic stumbling
block — the sidecar needs its own signature.

### P14 — Performance & latency budget — **Partial**

**Measured** (see measurements.md §7 and §8): ~330 ms end to end on one stream,
of which 300–340 ms is the decode; resampling runs at ~2200× realtime; the
speech gate rejects non-speech in 3–8 ms instead of a ~320 ms decode.

**Outstanding:** instrumented measuring points across all stages, a written
latency budget held against the measurements, a repeatable "8 streams,
30 minutes" reference load test, and the **code-switching fixture set** with
reference transcripts for measuring word error rate separately over German and
English passages.

**Accepted when:** a reproducible benchmark shows p95 latency within the target
on defined hardware; the WER baseline for mixed-language material is
documented.

**Caveat on what exists:** the current fixture is macOS text-to-speech in a
quiet room. Real intercom traffic with noise, compression and crosstalk will do
worse. The numbers are an upper bound, not a promise.

### P15 — Vocabulary & prompt management — **Partial**

Today: one free-text prompt field in the UI, passed through to the sidecar's
`initial_prompt`. Its effect is measured — `Q5` becomes `Cue`, `Fadeout`
becomes `Fade out` (measurements.md §4).

**Outstanding:** named vocabulary lists assignable to one or more streams and
stored in SQLite; assembly into the prompt with **whisper's ~224 token limit**
handled by sensible shortening rather than silent truncation, with the UI
showing the usage; a warning that over-long or unsuitable prompts encourage
hallucination; a shipped starter list.

**Accepted when:** on the P14 fixture set an enabled vocabulary list measurably
improves the word error rate of the technical terms against the baseline.

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
              P14 Fixtures ──> P3b Language stabiliser ──┐
                                       │                 │
                                       v                 v
                            P6 Tauri API (commands/events)
                                       v
        P7 Chat UI ──> P8 Health/levels ──> P9 Search/filter
                                       │
                            P10 SQLite │  P12 Settings
                                       v
                                 P13 Packaging
```

**Milestone A (technical backbone):** P0 → P1 → P2 → P4 → P5. One channel, text
in the log. *Reached, including the pool — what P5 still owes is a load test
that cannot be run until several streams exist to load it with.*

**Milestone B (usable MVP):** + P3, P6, P7, P11, P12. Eight channels, chat UI,
model download.

**Milestone C (language quality & production readiness):** + P14 fixtures, P15,
P3b, P8, P9, P10, P13.

---

## Suggested next steps

In this order, with the reason each one comes where it does.

1. **P3 — move Silero in front of the segmenter.** Now the first thing worth
   doing: the pool made every wasted decode a decode a real segment could have
   had, and 2 of 11 segments in a live session were decoded only to be thrown
   away by the text filter. It also replaces the energy detector, which will
   not survive a real intercom feed.
2. **P6 → P7** — generated shared types, bundled level events, several streams
   with a virtualised list. The pool is already indifferent to the stream
   count, so this is what finally puts more than one stream behind it.
3. **P14 reference load test** — the numbers in measurements.md §8 were all
   taken on one stream through one worker and none of them has been re-measured
   against the queue. Possible only once step 2 exists.
4. **P11 → P12** — model download and persisted presets, which together make
   the app usable by someone who is not holding a terminal.
5. **P15** — develop vocabulary against the measurable baseline from step 3,
   not by feel.

---

## Whole-system verification

1. `just dev` on Apple Silicon with a multichannel interface connected.
2. Start a session with eight channels; some streams fixed to German, one on
   auto with DE/EN as candidates.
3. Speak on all channels at once — text appears in the correct stream, latency
   within target.
   1. Speak mixed-language ("standby für Cue fünf, dann fade out") — the
      English terms stand verbatim in the German text, nothing was translated.
   2. Disable the vocabulary list and repeat the same passage — the difference
      must be visible, otherwise P15 is not working.
   3. Select a third language in the setup and put a stream on it — possible
      without a code change.
4. Overdrive one channel and leave another silent → P8 shows both.
5. Enter a search term → hits across all streams, jumping works.
6. `kill -9` a sidecar → the app keeps running, the worker comes back.
7. Kill the app hard, restart → the session loads completely from SQLite.
8. Run the P14 reference load test, record p95 latency.

---

## Deliberately out of scope

- Export (TXT/SRT/JSON) — the P10 schema is already prepared for it.
- Markers and bookmarks.
- Windows port: only P1 needs reimplementing, provided the `AudioBackend` trait
  is respected; plus the GPU backend choice in P4.
- Speaker diarisation.
- Second decode of low-confidence segments in the alternative language, better
  result wins. Doubles the latency for those segments.

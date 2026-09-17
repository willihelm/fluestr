# Plan and progress

The sixteen work packages of the MVP, and where each one actually stands.

Plan and status live in the same file on purpose. Kept apart they drift, and a
plan that no longer describes the code is worse than no plan — it reads as
authority while being fiction.

Evidence for the architecture decisions is in
[measurements.md](measurements.md); this file references it rather than
repeating numbers, so there is one place to correct when a measurement changes.

**Status of this document:** 2026-09-17, after the Settings page, the group
sidebar and muting the channels the device does not have.

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
| P3 | VAD & segment scheduler | **Done** | — |
| P3b | Language stabiliser | **Open** | — |
| P4 | Whisper sidecar | **Partial** | Not registered as a Tauri `externalBin` |
| P5 | Worker pool & backpressure | **Partial** | No reference load test on live audio |
| P6 | Tauri API | **Partial** | No generated shared types; no `update_stream_config`, no persisted group CRUD, no `clear_communication` in the backend, no `search_session` |
| P7 | Chat UI | **Partial** | No language picker from the sidecar's list; the window is a window, not a jump to an arbitrary position |
| P8 | Levels & stream health | **Partial** | Level, overrun count and overload count only |
| P9 | Live search & filter | **Open** | — |
| P10 | SQLite persistence | **Open** | — |
| P11 | Model manager | **Open** | Models discovered from `.scratch/models` or environment variables |
| P12 | Settings & presets | **Open** | Every threshold is a constant in the code |
| P13 | Packaging & signing | **Open** | `Info.plist` with the microphone usage description exists |
| P14 | Performance & latency budget | **Partial** | Latency measured on one stream; no reference load test, no WER baseline |
| P15 | Vocabulary & prompt management | **Partial** | One global box on Settings, but not stored, not named, and not yet measured against a baseline |

**What runs today:** up to eight channels of one multichannel device end to
end — microphone, one ring buffer and segmenter per channel, Silero speech
detection, the shared worker pool, one colour-coded chat — at a measured
~330 ms on one stream from the end of a spoken segment to the text appearing.
Around it: a Settings page holding the device, the channel map, the one global
vocabulary and the groups; a channel map that may reach past the device, whose
surplus channels start muted instead of blocking Start; a chat page with a
group sidebar that narrows the view to a group's channels; consecutive segments merged into one bubble per
utterance; and Clear all communication as the only action that empties the
chat. What the eight-channel case still lacks is a measurement of its own: the
latency and CPU numbers were all taken with one stream running.

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
  hallucinated sentences. That is now the three-layer defence in P3/P4 —
  Silero in front of the segmenter, the sidecar's gate behind it, the text
  filter last — and it would otherwise have surfaced during acceptance rather
  than during development.

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
apart from quiet, not speech from a bell. Silero therefore decides what the
segmenter cuts, in the app process rather than only in the sidecar: a segment
that was never speech now costs 0.4 ms of detection instead of a ~320 ms decode
plus an IPC round trip. See P3 and measurements.md §10.

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
(capture/VAD/segmentation, UI-free and therefore unit-testable; whisper.cpp
enters it only through the optional `silero` feature, so the Windows port and
the crate's own tests never have to build it). Frontend in
`ui/` with Vite + Svelte + TS, Tauri v2, pnpm, pinned Rust toolchain,
`justfile`, CI on `macos-14`. The frontend's non-component logic — bubble
grouping, group membership, the vocabulary prompt — is tested with vitest, run
by `just check` and by CI alongside the Rust tests: a type checker says the
shapes line up, not that the rules hold.

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

### P3 — VAD & segment scheduler — **Done**

`Vad` trait, with `SileroVad` as the implementation the app uses and
`EnergyVad` kept as the fallback for a machine without the VAD model.
Segmentation rules per stream: start on speech onset with ~300 ms pre-roll, end
after ~500 ms of silence, force-flush at `max_segment_len` (6 s) **cut at the
quietest frame of the recent past**, discard anything under ~300 ms.

**Accepted when:** unit tests on a WAV fixture with known pauses produce the
expected segment count and boundaries (±100 ms). ✔ — and, for the detector,
loud noise and a loud tone produce no segment at all while the spoken passage
in the same signal produces exactly one. See measurements.md §10.

**What the detector does now.** Silero runs in the app process, in front of the
segmenter, so a bell or a chair scrape never opens a segment. Measured on 3 s
of each signal, as a share of frames called speech: loud broadband noise and a
loud 440 Hz tone read 0.00 against the energy detector's 1.00; the speech
fixture reads 0.86.

Three things about it are worth knowing before changing it:

- **The LSTM state is cleared on every call.** `whisper_vad_detect_speech`
  resets it, so a call containing only the current frame judges that frame with
  no memory of the syllable it belongs to. Each call therefore replays the
  preceding four windows and reads only the last probability. It costs 0.41 ms
  per 32 ms frame — about 1.3 % of a core per stream.
- **The frame size comes from the model.** whisper.cpp reads the window length
  out of the model file and exposes it nowhere, and the whole scheme rests on
  one probability meaning exactly one frame. `SileroVad` probes it once at
  startup instead of compiling in 512.
- **It fails open.** A frame the model refuses to judge counts as speech. The
  sidecar's gate is still behind it, so failing open costs a wasted decode
  while failing closed would cost an operator who spoke and was never
  transcribed.

**Outstanding:** nothing for the MVP. The threshold and the pre-roll/timeout
values are still constants in the code — they belong in P12 with the rest of
the settings, and `silence_timeout` in particular is a *latency* parameter.

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
Since P3 the same model also runs in front of the segmenter, so this is the
backstop rather than the first line: it is the only layer that sees a segment
as a whole, and it is what still protects a session that fell back to the
energy detector. Three earlier attempts failed and are documented in
measurements.md §8, including one that appeared to work and did nothing at
all.

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

**Eight streams have now been through the queue**, in
`eight_streams_share_the_queue_without_starving_each_other`: eight streams
submitting interleaved into a queue deliberately smaller than the batch, with
every `(stream, seq)` pair accounted for exactly once and no stream coming back
empty. That is the fairness claim the oldest-first rule exists for, asserted
rather than argued. It is seconds of synthetic segments, not the 10-minute
reference load test, and it says nothing about memory or latency — but the
pool's central arrangement is no longer only exercised by one stream.

A detail found while writing it, and fixed: these tests had been skipping
themselves. `find_binary` looked only next to the running binary, and cargo
puts a test binary in `target/debug/deps` while the sidecar is in
`target/debug` — so every one of them printed "no model — skipping" and passed
without testing anything unless `INTERCOM2CHAT_WORKER` was set by hand. The
search now also looks one directory up. A test that passes by not running is
worse than a missing test, because the status table believes it.

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

**Outstanding:** the reference load test, which belongs with P14 and now has
the multi-stream session it needed — what it still needs is live audio on eight
channels rather than synthetic segments. Until then the pool size, the queue
bound and the timeouts are reasoned defaults, not measured ones.

### P6 — Tauri API — **Partial**

Commands `list_devices`, `start_session` and `stop_session` exist; events
`transcript`, `levels`, `overload` and `session-state` are emitted.

`start_session` now takes a **list** of streams on one device, and the session
builds one pipeline per channel — ring, resampler, speech detector, segmenter
— all feeding the one shared queue. The stream id is the position in that list,
assigned by the backend rather than by the UI, so two streams can never be
given the same one; `SessionState::Running` echoes the mapping back as the
authoritative answer to which channel ended up under which label.

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
  communication is the separate, explicit `clear_communication` action and is
  never a side effect of Start or Stop.

Two more things worth knowing, both in the code:

- **Levels are one event for all streams**, at 20 Hz — hence `levels` rather
  than `level`. Eight separate events would be 160 bridge crossings a second,
  each waking the WebView, to draw eight bars that were each measured at a
  slightly different moment. Bundled, the cost no longer grows with the channel
  count and the meters that are read side by side were also sampled side by
  side.
- **One processing thread serves every stream.** The work is small and bounded
  — resampling at ~2200× realtime, Silero at ~1.3 % of a core per stream — so
  eight streams are ~12 % of one core. What must not share a thread is
  decoding, and it does not.
- **Every event carries a session id, because `seq` does not identify a
  segment.** Sequence numbers are per stream and start again at zero each time
  a session starts, so `(stream, seq)` repeats as soon as a second one runs —
  and Start is deliberately non-destructive, so the chat holds lines from
  several sessions at once. `(session, stream, seq)` is the identity, here and
  in the P10 schema. Found the hard way: the chat list keyed on `(stream, seq)`
  and the duplicate key threw, so after a Stop and a Start nothing appeared at
  all. The id is counted per process, which is as long as the in-memory chat
  lives; persistence needs one that survives a restart, and that belongs with
  P10.

**A failed session has to be cleared, or Start never works again.** The
failure — the device unplugged, the sidecar lost — is discovered on the
processing thread, which reports `session-state: failed` and ends. Nothing
there can reach the session the Tauri state holds, so it stayed in that slot:
the UI, seeing the failure, offered Start, and Start answered "a session is
already running" until the app was restarted. `start_session` now asks
`Session::is_finished` and clears such a session before refusing, which also
joins its threads and releases its sidecar; and the UI calls the idempotent
`stop_session` when it sees a failure, so the model is freed then rather than
at the next Start.

**Vocabulary is session-level, not per stream.** `SessionConfig::vocabulary`
is handed to every stream's segmenter, and `StreamConfig` no longer carries a
prompt at all. Putting it in the wire format rather than leaving it to the UI
to send the same string eight times is what makes "one vocabulary per
production" a property of the system instead of a convention — a later caller
cannot half-apply it. The field is `#[serde(default)]`, so a config written
before it existed still starts a session with no prompt rather than failing.

**Outstanding:** shared types generated from the Rust structs (`ts-rs` or
`specta`) instead of hand-written TypeScript; `update_stream_config`, group
CRUD and channel-to-group assignment commands, `clear_communication` (atomic
deletion of transcript data only), and `search_session`. Groups exist in the UI
and have no backend at all yet — they live as long as the window does, and
their persistence is P12's. `clear_communication` likewise clears what exists
today, which is the in-memory chat; the stored half arrives with P10.

**Accepted when:** the frontend receives typed events; eight streams under full
load keep the WebView main thread under ~30 %. The second half is unmeasured —
it belongs with the P14 load test.

### P7 — Chat UI (Svelte) — **Partial**

Today: two pages. **Settings** holds the input device, the editable list of
channels — device channel, label, language and colour, added and removed
freely — the one global vocabulary, and the groups with their membership.
**Chat** holds a group sidebar, a meter per running stream in the selected
view, and the lines themselves, each with its channel's colour down the left
edge and its label, timestamp, language badges and measured latency. Follow
behaviour with a "jump to latest" button. Start/Stop is in the header and
belongs to neither page: an operator who has to find the right page before they
can stop capture has already lost.

A bubble is an utterance, not a segment: consecutive segments of the same
channel are merged into one, up to a silence of 1.5 s between them and ceilings
on length and duration, and a merged bubble carries every language its parts
were detected as. Merging only ever appends to the *last* bubble in the list —
a channel that has meanwhile been interrupted by another one starts a new
bubble rather than growing upwards, because the list is read as a chronology.
Editing the channel map is locked while a session runs, because it is fixed
when capture opens. Group membership is *not* locked: a group is a view, not
capture configuration, and an operator watching eight channels live is exactly
who needs to narrow them to three.

**The list renders a window, not the session.** The last 200 bubbles are in the
DOM; scrolling to the top of them adds a page at a time, with the scroll
position corrected by the height the new content added so the line being read
stays put, and returning to the bottom trims the window again. A window rather
than a virtual scroller with estimated heights: a bubble's height depends on
how its text wraps, so estimates mean a scrollbar that lies and jumps as they
are corrected. What it costs is that a point an hour back takes several steps
to reach — which is what P9's search and its jump into the chat position are
for, rather than scrolling.

Channels are numbered from 1, the way they are numbered on an interface; the
backend stays zero-based and the conversion happens at that one boundary.

**A channel the device does not have is muted, not a refusal to start.** A
setup is built for the desk and then carried to a laptop with a two-input
interface, and until P12 persists presets, rebuilding it for the small
interface means losing it. So the setup may hold more channels than the device
offers: those keep their stream id, their colour and their group membership,
are marked `muted` in the running state, and receive no audio — no ring, no
segmenter, no VAD context. The session refuses to start only when *every*
channel is above what the device has, because then it would open a device and
transcribe silence, which reads as a broken app rather than as a misconfigured
one. Muted rather than dropped is what keeps the ids stable: the stream id is
the row's position, so removing a row would move every channel after it onto
the wrong label.

Each channel gets a palette colour when it is created and the operator can
change it. The colour is configuration, not decoration: with eight channels
interleaved in one list it is what makes a line's origin readable at a glance,
faster than reading eight labels, and it is the same colour on the meter strip.

**The interaction model.** Channel configuration belongs on the dedicated
Settings page, not in the live chat view: input mapping, label, language and
channel colour are edited there. Vocabulary is likewise managed on Settings,
but globally: one active vocabulary/prompt applies to all channels and has no
per-channel assignment. Each channel has a user-editable colour; a deterministic
palette colour is preselected when the channel is created, with enough contrast
for labels, badges and level meters.

The chat page has a left sidebar for groups, and selecting one focuses the
chat and the meters on its member channels. The sidebar *selects*; groups are
created, renamed, deleted and filled on the Settings page. It is read during a
show, when the question is "show me just the stage" and not "let me reorganise
my groups", and a rename box under the cursor at that moment is a liability.

Groups are non-exclusive views: a channel may belong to any number of groups,
and removing it from a group never removes the channel itself. The sidebar also
offers an all-channels view, which is what the chat shows when no group is
selected rather than a group that happens to contain everything. A group with
no members shows nothing and says so — falling back to showing every channel
would be a different view than the one that was asked for. Starting and stopping is session-wide, through one
global Start/Stop control; there is deliberately no per-channel Start button.
Start leaves the existing chat history visible and appends new transcripts to
it; neither Start nor Stop clears the list. A global **Clear all
communication** button clears every transcript, across every group and session,
from both the visible chat and persistent storage. It does not change channel,
group or Settings configuration. Because this is irreversible, it requires a
confirmation dialog and is available only after capture has been stopped.

Two streams reading the *same* device channel is allowed — mirroring one
channel into two views is unusual but not wrong — so it is a warning rather
than an error. During setup it is nearly always a slip, and the symptom, every
line appearing twice, does not point at its cause.

**Outstanding:** per-stream language configuration from the sidecar's language
list rather than a free-text field, and a one-click language override. Beyond
the MVP line but worth naming: the window can only be walked backwards a page
at a time, so jumping to an arbitrary position needs P9's search to drive it.

**Accepted when:** eight streams run visibly live; speech that runs on is one
bubble rather than one per segment ✔; scrolling stays smooth at 10,000
segments — the DOM is now bounded by construction, but "smooth" is a claim
about a machine under load and belongs to the P14 test; a third language can be
chosen without a code change (any ISO code can be typed today; the picker is
outstanding); a channel has a preselected but editable colour ✔; a channel can
appear in two groups and is visible in both ✔; all configured channels start
from one action ✔. Starting again after a Stop retains the preceding chat
history and appends new segments to it ✔. Clear all communication requires
confirmation ✔, removes every displayed transcript across all groups and
sessions ✔ and leaves the channel, group and vocabulary configuration intact ✔;
removing the *stored* transcripts waits on P10, because there is no store yet.

### P8 — Levels & stream health — **Partial**

Today: a level meter per running stream (RMS and peak, computed in Rust, all
of them arriving in one event so the bars on screen are the same age), the
per-stream overrun count from P2, and the per-stream count of segments the
decode queue had to give up on (P5). The two counts are deliberately separate:
one is audio lost on the way *into* the pipeline, the other is audio that was
captured cleanly and then could not be kept up with, and they call for
different fixes.

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
`devices`. A segment is keyed by `(session, stream, seq)` — see P6: the
sequence number restarts with every session and does not identify anything on
its own. The session id the events carry today is counted per process and has
to become one that survives a restart here. Language fields are written **from day one**, including for streams
with a fixed language — it costs nothing and makes misdetections analysable
later. WAL mode with batched writes, not one commit per segment. Versioned
migrations. Crash recovery marks an incomplete session as ended and keeps the
data. `clear_communication` deletes transcript data for all sessions, including
the FTS index, but preserves device, channel, group, model and vocabulary
configuration.

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
channel mapping including labels, colours and language configuration, plus named
groups and their channel memberships — an engineer does not want to remap eight
channels or rebuild their working views before every show. Vocabulary lists and
the active vocabulary are global settings, not part of a channel mapping.
Channel colours are automatically assigned from a deterministic palette when no
saved choice exists, then persist as explicit user-editable values. The Settings
page is the home for all of this configuration. Device recognition via a stable
id, not the display name.

**Accepted when:** a preset, including channel colours and non-exclusive group
memberships, survives a restart and a reconnect of the interface.

**What P7 has now handed it:** groups, their names and their memberships, the
active vocabulary, and the channel map with its colours all exist and are all
lost when the window closes. Membership is keyed on the channel's own key
rather than on its position, which is the shape that has to survive the
round trip — a preset that stored positions would silently re-point every group
the first time a channel was deleted.

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

The load test is now unblocked and is the next thing to do. Eight streams run,
and `tests/pool.rs` already puts eight of them through the queue with synthetic
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
stream through `SessionConfig::vocabulary` (see P6) and passed to the sidecar
as `initial_prompt`. Its effect is measured — `Q5` becomes `Cue`, `Fadeout`
becomes `Fade out` (measurements.md §4). The box shows an estimated token count
against whisper's 224-token window and warns twice over: that an over-long
prompt loses the terms typed *first*, and that length itself makes
hallucination on quiet channels more likely, because whisper treats the prompt
as context it may continue.

The estimate is four characters to the token, which is a heuristic and not
whisper's tokeniser — German compounds and technical terms split worse than
average, so it reads low exactly where this list is densest. Hence the warning
at 80 % rather than at the limit, and hence `estimated` in the name of
everything that carries it.

**Outstanding:** named vocabulary lists stored in SQLite with exactly one
active, a shipped starter list, and shortening that is deliberate rather than
whisper's silent truncation. And the measurement: the point of a global list is
that its effect is visible on *every* channel, which needs the P14 fixture set
to show.

**Accepted when:** on the P14 fixture set the enabled global vocabulary list
measurably improves the word error rate of the technical terms against the
baseline on every applicable channel.

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
in the log. *Reached, including the pool — what P5 still owes is a load test on
live audio.*

**Milestone B (usable MVP):** + P6, P7, P11, P12. Eight channels, chat UI,
model download. *P3 is done, eight channels run, and the chat view the eight
make necessary — Settings page, group sidebar, bounded list — is in place. What
is left is the model download and the saved presets: everything configured on
the Settings page currently dies with the window.*

**Milestone C (language quality & production readiness):** + P14 fixtures, P15,
P3b, P8, P9, P10, P13.

---

## Suggested next steps

In this order, with the reason each one comes where it does.

1. **P14 reference load test** — the numbers in measurements.md §8 and §10 were
   all taken on one stream. The 10 % CPU figure for eight streams' speech
   detection is arithmetic, not a measurement, and the queue has only been put
   under contention by synthetic segments. Eight channels now run, so this is
   finally possible — and it comes first because every number the remaining
   decisions rest on is currently a single-stream number. The chat's window
   bounds the DOM, which removes the frontend from the list of things that
   could explain a bad result.
2. **P11 → P12** — model download and persisted presets, which together make
   the app usable by someone who is not holding a terminal. P12 has become the
   more urgent of the two: the Settings page now holds a channel map, eight
   colours, a vocabulary and a set of groups, and every one of them is typed in
   again after every restart. Nobody rebuilds that before a show. P12 also
   takes the constants P3 still owns: the Silero threshold, `silence_timeout`,
   `max_segment`.
3. **P15** — develop vocabulary against the measurable baseline from step 1,
   not by feel. The control exists and is global; what is missing is the
   evidence that any particular list is better than another.
4. **P7's language picker** — `list_languages` through the Tauri API and a
   picker in place of the free-text field. Small, and last of these, because a
   typed `de` works today; it is discoverability that is missing, not
   capability.

## Whole-system verification

1. `just dev` on Apple Silicon with a multichannel interface connected.
2. Start a session with eight channels; some streams fixed to German, one on
   auto with DE/EN as candidates.
3. Speak on all channels at once — text appears in the correct stream, latency
   within target.
   1. Speak mixed-language ("standby für Cue fünf, dann fade out") — the
      English terms stand verbatim in the German text, nothing was translated.
   2. Disable the global vocabulary list and repeat the same passage — the
      difference must be visible on every channel, otherwise P15 is not
      working.
   3. Select a third language in the setup and put a stream on it — possible
      without a code change.
4. Overdrive one channel and leave another silent → P8 shows both.
5. Enter a search term → hits across all streams, jumping works.
6. `kill -9` a sidecar → the app keeps running, the worker comes back.
7. Kill the app hard, restart → the session loads completely from SQLite.
8. Stop capture, press **Clear all communication**, confirm → all chat lines
   and search hits are gone after restart, while channels, groups and Settings
   remain.
9. Run the P14 reference load test, record p95 latency.

---

## Deliberately out of scope

- Export (TXT/SRT/JSON) — the P10 schema is already prepared for it.
- Markers and bookmarks.
- Windows port: only P1 needs reimplementing, provided the `AudioBackend` trait
  is respected; plus the GPU backend choice in P4.
- Speaker diarisation.
- Second decode of low-confidence segments in the alternative language, better
  result wins. Doubles the latency for those segments.

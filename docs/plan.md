# Plan and progress

The sixteen work packages of the MVP, and where each one actually stands.

Plan and status live in the same file on purpose. Kept apart they drift, and a
plan that no longer describes the code is worse than no plan — it reads as
authority while being fiction.

Evidence for the architecture decisions is in
[measurements.md](measurements.md); this file references it rather than
repeating numbers, so there is one place to correct when a measurement changes.

**Status of this document:** 2026-09-18, after the stable device id — a
profile now identifies its interface by something a rename cannot break, which
is the second half of P12's "survives a restart *and* a reconnect". Before it,
the same day: the model manager, so a fresh machine reaches its first
transcription without a terminal. P10 and P3b were set aside on that day too;
see each package for what that costs and what it changes.

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
| P3b | Language stabiliser | **Deferred** | Set aside on 2026-09-18: with a fixed language per stream the language is already stable |
| P4 | Whisper sidecar | **Partial** | Not registered as a Tauri `externalBin` |
| P5 | Worker pool & backpressure | **Partial** | No reference load test on live audio |
| P6 | Tauri API | **Partial** | No generated shared types; no `update_stream_config`, no `search_session` |
| P7 | Chat UI | **Partial** | The window is a window, not a jump to an arbitrary position |
| P8 | Levels & stream health | **Partial** | Level, overrun count and overload count only |
| P9 | Live search & filter | **Open** | Was specified on P10's FTS index, which is not coming — needs a decision on what it searches instead |
| P10 | SQLite persistence | **Deferred** | Set aside by the user on 2026-09-18: this is about live communication, not persistence |
| P11 | Model manager | **Done** | — |
| P12 | Settings & presets | **Partial** | Profiles are stored and read back, with their default language and a stable device id; every threshold is still a constant in the code |
| P13 | Packaging & signing | **Open** | `Info.plist` with the microphone usage description exists |
| P14 | Performance & latency budget | **Partial** | Latency measured on one stream; no reference load test, no WER baseline |
| P15 | Vocabulary & prompt management | **Partial** | One global box on Settings, stored since P12, but not named as a list and not yet measured against a baseline |
| P16 | LAN remote client | **Partial** | Served, driven and verified against a mock app; not yet exercised from a second machine on a real network. No authentication — by decision, see the package |
| P17 | Alert terms & visual attention | **Partial** | Unicode normalisation of terms and transcripts |

**What runs today:** up to eight channels of one multichannel device end to
end — microphone, one ring buffer and segmenter per channel, Silero speech
detection, the shared worker pool, one colour-coded chat — at a measured
~330 ms on one stream from the end of a spoken segment to the text appearing.
Around it: a Settings page holding the device, the channel map, the one global
vocabulary, the global alert terms and the groups; a channel map that may reach past the device, whose
surplus channels start muted instead of blocking Start; a chat page with a
group sidebar that narrows the view to a group's channels and opens any of them in a
window of its own; consecutive segments merged into one bubble per
utterance, with the alert terms marked where they were said and an unfocused
window asking once per bubble to be looked at; Clear all communication as the only action that empties the
chat; all of that setup stored as named profiles that come back at the next
start, keyed on an interface the app recognises after a rename and can tell
from a second unit of the same model; a model manager that fetches,
checksums and selects the models the
whole thing runs on, so a machine that has never seen this repository can be
brought to its first transcription from inside the window; and, optionally, the
same UI served to a browser on the local network, where it is a full peer —
the same chat, the same alert highlighting, the same Start, Stop and Settings —
so the session can run where the audio interface is and be worked from
somewhere else. What the
eight-channel case still lacks is a measurement of its own: the latency and CPU
numbers were all taken with one stream running.

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
   vector restricted to the languages declared for that stream. Package P3b —
   deferred, so today every stream is on a fixed language and auto mode is not
   offered at all. The data model carries it (`LanguageMode::Auto`); nothing
   constructs it.

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

### P3b — Language stabiliser (auto mode only) — **Deferred**

**Set aside by the user on 2026-09-18:** the language is stable as things
stand, so the mechanism that would stabilise it has nothing to fix yet.

That stability is not an accident and it is worth naming, because it is also
the condition under which this stays deferred. Every stream runs on a fixed
language — `session.rs` builds `LanguageMode::Fixed` and nothing else — so
there is no per-segment language decision that could oscillate in the first
place. P3b only becomes real work the day auto mode is offered in the UI, and
it must land in the same step: an auto mode without the stabiliser is worse
than no auto mode, because it flips language inside a single contribution.

What that costs: a stream on which two languages genuinely alternate has to be
switched by hand. Given DE/EN code switching within one speaker — the normal
case here — a fixed German stream already carries the English terms verbatim,
which is why this is a smaller loss than it first looks.

The specification below stands unchanged, so reviving it is not a redesign.
One thing found while scoping it belongs with it: `whisper-rs` exposes the
probability vector through `WhisperState::lang_detect`, but that is a separate
encoder pass *before* the decode, not a value that can be read off afterwards —
`full_lang_id_from_state` is whisper's global argmax and arrives too late to
restrict. So P3b costs an extra encoder pass per segment on auto streams, in
the 300 ms class rather than the 1–2 ms class. It needs a measurement of its
own before it is offered.

---

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
- **`list_languages` needs no model and no session.** It runs the sidecar
  binary with `--languages`, which prints whisper's compiled-in table and
  exits, rather than sending `ListLanguages` down a worker's pipe — a worker
  answers only once a model is loaded, and the picker this feeds is used while
  the operator is still mapping channels on a machine that may have downloaded
  nothing. Cached for the life of the process, because the table cannot change
  within it.
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
  in the P10 schema that is no longer coming. Found the hard way: the chat list
  keyed on `(stream, seq)` and the duplicate key threw, so after a Stop and a
  Start nothing appeared at all. The id is counted per process, which is
  exactly as long as the in-memory chat lives — with P10 set aside that is no
  longer a stand-in for something better, it is the whole lifetime.

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

**Configuration is two commands, not a CRUD surface.** `load_config` and
`save_config` move the whole file, because it is a few kilobytes a human edits
at human speed and a per-entity command set would need the two sides to agree
on which half is authoritative. So groups, the channel map and the profiles
around them need no commands of their own: they are edited in the UI and
stored with everything else. See P12.

`open_group_window(group, name)` opens a group in a second Tauri WebView backed
by the same session state — not a second capture or worker pool. Beside it,
`session_snapshot` hands a window that has just opened the chat as it stands,
`clear_communication` empties it for every window at once, and
`close_group_windows` retires the group windows when the active profile
changes. Those three exist because the chat is no longer read in one place:
`app::history` keeps what the bridge has emitted — the last 10 000 transcripts
and the sessions' zero points — as the one copy every view is caught up from.
That is not persistence and reaches no disk; it is what stops two windows
disagreeing about what was said and about what "Clear all communication"
cleared. `save_config` now also announces itself as a `config-changed` event,
which is how a group renamed or refilled on the Settings page reaches the
window showing it.

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

Today: two pages. **Settings** holds the input device, the editable list of
channels — device channel, label, language and colour, added and removed
freely — the one global vocabulary, the groups with their membership, and the
model manager. **Chat** holds a group sidebar, a meter per running stream in the selected
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

**The Settings page is split by scope, not by topic.** Its four scoped sections
sit in a rail on the left, under a heading naming what each belongs to: Audio
and Groups under the *profile*, Vocabulary under *all profiles*, Models under
*this machine*. Scope is the thing an operator gets wrong — what a change here
travels with, and what stays behind — and as one scrolling column of cards all
four looked alike, with the difference carried only by a paragraph of prose in
the profile card. Grouping the rail says it once, so no section has to explain
its own scope.

**The profiles are a section of their own, not a permanent strip.** Picking a
setup is the rarest thing done on this page — once at the start of a show — and
as a strip across the top it stood in front of the most frequent ones, with New,
Duplicate and Delete under the cursor all evening next to the fields they would
discard. It sits first in the rail instead, above the scope headings, because it
selects what those headings are about. What has to stay visible while the
channel map is edited is not the switcher but the *name*, and the rail carries
it: the profile-scoped sections are listed under "Profile · Oper grosses Haus".
With room for a list rather than a dropdown, each setup is one row with its own
name field, its channel and group count, its device and its own Duplicate and
Delete, so three profiles can be told apart without selecting each one in turn.
The one thing that does span the page whichever section is open is a failed
save, in a strip at the top: an operator who builds a setup believing it is
stored and finds it gone before the next show has been failed twice.

Each rail item carries a dot when its section wants attention — amber for a
setup that will not start or a vocabulary over the token budget, red for a
machine with no model at all. Those are exactly the problems that used to be
invisible from the chat, where an app that refuses to start looks like it is
refusing for no reason. The sections all stay mounted and the inactive ones are
hidden, because a model download is minutes long and a rail that unmounted it
would throw away the progress of a transfer that is still running.

Channels are numbered from 1, the way they are numbered on an interface; the
backend stays zero-based and the conversion happens at that one boundary. The
channel list names its fields once, above the columns, rather than over every
row: eight rows of "Ch / Label / Lang / Colour" is four times as much text as
the setup itself, and it buried the one thing the section is read for — which
label sits on which channel.

**A channel the device does not have is muted, not a refusal to start.** A
setup is built for the desk and then carried to a laptop with a two-input
interface, and rebuilding it for the small interface would mean losing it —
now that P12 stores it as a profile, rebuilding it would mean *overwriting* it. So the setup may hold more channels than the device
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
channel colour are edited there. Its sections — profiles, input, channels,
vocabulary, alerts, groups — are cards with the page background between them rather
than one continuous surface divided by hairlines: it asks six separate
questions, and sections that touch read as one long form in which it is easy to
answer the wrong one. The frame is one `.panel` rule in `app.css`, not five
copies in five components. Vocabulary is likewise managed on Settings,
but globally: one active vocabulary/prompt applies to all channels and has no
per-channel assignment. Each channel has a user-editable colour; a deterministic
palette colour is preselected when the channel is created, with enough contrast
for labels, badges and level meters.

The Alerts section holds one global list of terms and phrases. A match is
case-insensitive and Unicode-normalised; a one-word term matches a word, while
a multi-word entry matches that phrase, so an alert for `cue` does not fire on
`rescue`. It applies to every channel and is separate from vocabulary: one
helps Whisper choose words, the other reacts to words Whisper has already
produced.

Each group also has an independent **Flash on every new message** setting,
disabled by default. It belongs beside that group's membership controls in
Settings, because it describes how the group is watched, not how a channel is
transcribed. It is separate from Alerts: an enabled group flashes for every new
message from one of its members, whether or not that message contains an alert
term.

A group's detached window (P7) can be **pinned in front of other windows**, and
that is asked of the window itself — a pin in its own header, not a checkbox on
the Settings page. During a show the question is "keep this one where I can see
it", and it is asked of the window in front of the operator. It is remembered
per group all the same, so the next window that group opens is born in front
rather than jumping there a moment later. Per group rather than a property of
every group window, because a window that is always in front is a window
covering something: the stage manager's view is worth that, the general
talkback is not. Off by default.

The window applies it to itself the moment it is clicked and asks the main
window to store it, because **the configuration has exactly one writer**. The
main window holds it and saves it whole through its debounce (P6), so a second
writer editing the file underneath that would be overwritten by the next
keystroke on the Settings page. The request travels as a `group-on-top` event
between the windows; the main window writes it into the group and the ordinary
save carries it to disk.

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

Every alert match is highlighted in the chat bubble, including in an already
open group window and the read-only browser viewer. On the desktop, a new match
in an unfocused window requests native user attention (the platform's visible
window/Dock flash): the main window for any channel, and a group window only
when the matching channel belongs to that group. It fires once for a newly
matched bubble rather than repeatedly while that bubble is rendered.

The group-level flash setting uses the same native attention request for every
new bubble from that group's channels. It applies to an open detached group
window, and to the main window only while that group is the selected view; the
all-channels view has no implicit group setting. If a channel belongs to two
groups, each open group window follows its own setting. In all cases, one new
bubble creates at most one request per affected window.

Every group can be opened in its own window from the sidebar. This window is a
live view of that group's channels: it receives existing history, incoming
transcripts, meters and session state from the shared backend, and reflects
later membership changes. It does not own or create a capture session, so
Start/Stop remains global. Closing the window closes only that view, never the
group or its history.

**How it is one app in several windows.** The same bundle is mounted in every
WebView, and the window's *label* says which view it is — `group-3` is the
fourth group of the active profile. The label rather than a query string,
because it is readable before the first paint, so a group window never renders
the main view for a frame and then replaces itself; and because Tauri
identifies a window by it, so asking for a group that is already open focuses
that window instead of stacking a second one on it.

A group is addressed by its **position** in the active profile, which is the
same handle the configuration file stores membership by and the only one that
means the same thing in two windows — the keys the UI edits by are issued per
window and mean nothing outside it. The window then follows `config-changed`,
which is how a rename, a membership change or a flash setting made on the
Settings page reaches it.

Because there is more than one window, the running state is now **announced as
well as returned**. `start_session` hands its state back to the window that
pressed Start, which is how that window knows; every other window has only the
`session-state` event, so a group window that was open when Start was pressed
would otherwise go on saying "not running" through a running session — and,
having no zero point for it, would time every line as "+0:12" instead of as a
time of day.

The history it opens with comes from `session_snapshot`. A window subscribes to
the live events *before* it asks for that snapshot, because the other order
drops whatever is said during the round trip — which is exactly the moment a
group window is opened, since something is happening. The overlap that costs is
harmless: a segment is identified by session, stream and sequence, and one that
is in both answers is kept once. Two bubbles with one key would not be a
cosmetic problem — it throws and stops the list rendering.

**Switching profiles closes the group windows.** The stream ids a transcript
carries are positions in the active profile's channel list, so a window kept
open across a profile change would be filtering the right numbers against the
wrong channels. Closing them is not a loss worth protecting: switching profiles
is a setup action between productions, not something done during a show.

Two streams reading the *same* device channel is allowed — mirroring one
channel into two views is unusual but not wrong — so it is a warning rather
than an error. During setup it is nearly always a slip, and the symptom, every
line appearing twice, does not point at its cause.

**The language is picked, and picked once.** Every language field — the one
above the channel list and the one on each row — is a menu built from whisper's
own table, fetched through `list_languages` and never written down here. That
command runs the sidecar with `--languages` rather than asking a worker over
the protocol, because a worker only answers once a model is loaded and this
list is wanted while an operator is still mapping channels, on a machine that
may have downloaded nothing. The table cannot change while the app runs, so it
is fetched once per process.

The picker above the list sets the *profile's* default: what a channel added
afterwards is born with. It presets and does not rewrite — a map built channel
by channel is not something to change behind the operator's back — so when the
rows below it disagree, the section says which languages differ and offers to
put all of them onto the picker's in one press. Two separate acts: eight
channels in one language is the normal case and setting it eight times is eight
chances to miss one, but doing it silently on a picker change would lose a
deliberate mixed setup. A code the running whisper build does not offer is kept
as an option of its own rather than snapping to the first entry.

**Outstanding:** a one-click language override on a running stream. Beyond the
MVP line but worth naming: the window can only be walked backwards a page at a
time, so jumping to an arbitrary position needs P9's search to drive it.

**Accepted when:** eight streams run visibly live; speech that runs on is one
bubble rather than one per segment ✔; scrolling stays smooth at 10,000
segments — the DOM is now bounded by construction, but "smooth" is a claim
about a machine under load and belongs to the P14 test; a third language can be
chosen without a code change ✔ — the picker offers whisper's hundred and holds
no list of its own; a channel has a preselected but editable colour ✔; a channel can
appear in two groups and is visible in both ✔; all configured channels start
from one action ✔. Starting again after a Stop retains the preceding chat
history and appends new segments to it ✔. Clear all communication requires
confirmation ✔, removes every displayed transcript across all groups and
sessions ✔ — in every open window at once ✔ — and leaves the channel, group and
vocabulary configuration intact ✔; removing the *stored* transcripts waits on
P10, because there is no store yet. Opening a group in its own window shows the
same existing and incoming messages as the main window, limited to that group's
channels, without starting another capture session ✔.

### P8 — Levels & stream health — **Partial**

Today: a level meter per running stream (RMS and peak, computed in Rust, all
of them arriving in one event so the bars on screen are the same age), the
per-stream overrun count from P2, and the per-stream count of segments the
decode queue had to give up on (P5). The two counts are deliberately separate:
one is audio lost on the way *into* the pipeline, the other is audio that was
captured cleanly and then could not be kept up with, and they call for
different fixes.

Beside the per-stream meters there is now a **CPU and memory indicator** for
the app itself, in the status bar next to the session line. It answers the
question the overload counter only answers too late — the counter says the
machine has already stopped keeping up, this says it is about to — and it is
the figure that decides how many channels a given laptop can be given.

Three things about it are worth knowing:

- **It counts the workers too.** Decoding happens in sidecar processes, so the
  app's own process is the smallest part of what the app costs. What is
  reported is this process and everything descended from it, which is what an
  operator would see in Activity Monitor if they thought to add it up — and
  because that sum is not what any one row there shows, the indicator says so
  in its tooltip rather than letting the difference look like a bug.
- **The percentage is a share of the machine.** macOS counts one fully busy
  core as 100 %, so eight busy workers report 800 % and read as a failure. The
  figure is divided by the core count, so 100 % means there is nothing left,
  and the bar turns at 75 % rather than at 90: a warning that arrives once
  every core is saturated arrives after the segments have started dropping.
- **It runs whether or not a session does.** The load before Start is what
  decides how many channels are sensible, and the fall after Stop is how the
  memory a session held is seen to come back. Sampled once a second, not at the
  meters' 20 Hz — walking the process table is real work, and at that rate the
  indicator would be part of the load it reports.

**Outstanding:** peak hold, silence indicator, clipping warning, worker queue
depth, and the rest of a global status bar — active workers, queue length and
throughput beside the CPU and memory figures. `WorkerPool` already exposes
`workers()` and `queue_depth()` for it.

**Accepted when:** a deliberately clipped channel and a silent one are both
immediately recognisable in the UI.

### P9 — Live search & filter — **Open**

Full-text search over the running session (SQLite FTS5, see P10), incremental
while typing, with highlighting and a jump into the chat position. Filters for
streams and time range.

**Accepted when:** search over a two-hour session answers in <100 ms and does
not disturb ongoing writes (WAL mode).

### P10 — SQLite persistence — **Deferred**

**Set aside by the user on 2026-09-18**, with the reason: *this is about live
communication, not about persistence.* That is a scope decision rather than a
postponement, and it is worth writing down here because several packages were
specified against a database that is now not coming.

What it costs, stated plainly rather than left to be discovered: a session
lives in the window and dies with it. Kill the app and the chat is gone. The
configuration is unaffected — profiles, groups, colours and the model choice
are in `config.json` and were never P10's business (see P12), so an operator
loses the record of what was said, never their setup.

What it changes elsewhere:

- **P9** was specified as SQLite FTS5 over the stored session. Without a
  database it becomes a search over the messages the window is holding, which
  is a different and much smaller job — and it interacts with the chat window
  from the DOM work, since a hit may be outside the rendered range.
- **Clear all communication** is already only the in-memory half, and that half
  is now the whole action rather than the first of two.
- **P6's `search_session`** was a database command. It may not need to exist at
  all if search is a frontend concern.
- The **whole-system verification** below loses its "restart and the session
  loads" step.

The original specification is kept below, unedited, because reversing this
decision should not mean designing it again.

---

**Original specification:**

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

### P11 — Model manager — **Done**

Curated model list with URL, SHA256, size and expected latency. Download with
progress, resume and **checksum verification** into the app data directory.
Settings UI for selection; a clear "no model" state instead of silent
malfunction. A custom model — a path to a `.bin` — as the offline way in.

**Recommendation matching "latency first": `ggml-small-q5_1` as the default**,
`base` as the faster and `large-v3-turbo` as the more accurate option — which
is what shipped.

**Done:** `crates/app/src/models.rs` holds the catalogue — four entries, each
with the published SHA256 and size — and `ModelPicker.svelte` is the section on
the Settings page. Downloads go to `models/` inside the app's *data* directory,
not the configuration one: they are hundreds of megabytes that can always be
fetched again, which on macOS is the difference between something the user's
backup carries forever and something it can skip. The choice is stored beside
the profiles rather than inside one, for the same reason the vocabulary is —
which model a machine can run depends on its memory, its GPU and the latency it
can afford, not on the production, and carrying a profile to a laptop must not
carry a model that laptop cannot serve.

Four decisions worth keeping in view:

- **The checksum decides, and a failure discards.** A damaged model does not
  announce itself: whisper.cpp either refuses it with a message about magic
  numbers or loads it and transcribes noise. So nothing is renamed into place
  before it hashes correctly, and a file that fails is *deleted* rather than
  kept — keeping it would leave a corrupt model one resume away from being
  installed, surviving every later attempt.
- **Downloads resume.** 600 MB on a venue's network is not one uninterrupted
  transfer, and restarting from zero after each failure is how somebody gives
  up and goes back to the terminal. The partial file is kept as `.bin.part` and
  continued with a range request; cancelling deliberately leaves it alone, and
  the button then says "Resume" with the amount already there. A server that
  ignores the range header is detected and the write starts from the front,
  because appending in that case would splice two copies together and only the
  checksum at the end would notice.
- **Resolution has one order and it is written down** ([`models::resolve`]):
  the environment variable, then the custom model if it is the one selected,
  then the selected catalogue model, then the default one if it happens to be
  installed, then the old `.scratch/models` search. The custom model is picked
  in the same list as the catalogue ones, under the reserved id `custom`,
  rather than being a path that silently overrode the picker: an override meant
  the selected model and the running model could disagree with nothing on the
  page saying which had won, and the path is now simply remembered while
  another model runs. Its path is chosen through the system file dialog
  (`pick_model_file`, `tauri-plugin-dialog` driven from Rust so the frontend
  keeps one way of talking to the backend) and the field stays typable for a
  path that arrived in an email. A custom path that is not there is an error
  rather than a quiet fall-through, because falling back would run a different
  model than the operator chose and never say so.
- **"Not downloaded yet" is a different sentence from "no model at all".** The
  first is what a cancelled download leaves behind, and collapsing the two
  would send an operator looking for a problem they do not have.

The four network tests in `crates/app/tests/models.rs` (`just models`) run
against the real host rather than a mock: the two things most likely to be
wrong here are the URL and the checksum, and those are exactly the two a mock
replaces. They cover a fresh download, a resumed one, a cancelled one, and the
tampered file — one byte short of full size, so the download resumes, arrives
at the right size with the wrong contents, and only the hash can tell.

**Outstanding, and deliberately so:** switching the model while a session runs
does not restart the workers — the picker is locked instead, like the channel
map, because a model swap under a running show is not something to do quietly.
`just model` and `just vad-model` are kept for the integration tests and
`just slice`, which have no app directory to resolve against.

**Accepted when:** a fresh machine without a model is guided through the
download to the first transcription; a tampered file is rejected by checksum.
The second half is tested. The first needs a machine that has never run this,
which is a P13 concern as much as a P11 one.

### P12 — Settings & configuration persistence — **Partial**

Persistent settings: worker count, model choice, VAD thresholds, maximum
segment length, hysteresis parameters from P3b, theme. Session presets: a named
channel mapping including labels, colours and language configuration, plus named
groups, their channel memberships and their flash-on-message preference — an
engineer does not want to remap eight channels or rebuild their working views
before every show. Vocabulary lists and
the active vocabulary are global settings, not part of a channel mapping. Alert
terms are likewise one global setting: the vocabulary and alerts of a house
should not drift with each individual channel or profile.
Channel colours are automatically assigned from a deterministic palette when no
saved choice exists, then persist as explicit user-editable values. The Settings
page is the home for all of this configuration. Device recognition via a stable
id, not the display name.

**Accepted when:** a preset, including channel colours and non-exclusive group
memberships, and the global vocabulary and alert terms survive a restart and a
reconnect of the interface; each group's flash-on-message preference does too.
✔ for the identity half — the reconnect is verified against real hardware in
`capture_live.rs`, which is `#[ignore]`d because CI has no interface.

**Done:** the presets, as **profiles**. `config.json` in the app's
configuration directory holds a list of profiles — device, default language,
channel map with labels, languages and colours, groups with their memberships
and flash-on-message preference —
plus the active vocabulary, alert terms and which profile was last in use. It is read at
startup, written on every edit after a short debounce, and there is no Save
button: a setup an operator believes is stored and is not is worse than no
persistence at all. Profiles are created, renamed, duplicated and deleted in
their own section of the Settings page, first in the rail, and switching is
locked while a session runs, for the same reason the channel map is. The
section is not on screen while the channels are edited, because choosing a
setup is done once a show and deleting one should not be a click away from the
fields it would discard. What it governs is named in the rail rather than
explained in it: Audio and Groups are listed under the active profile's name,
which is the whole of what a profile holds — see P7.

A profile's **default language** joined them: the language a channel added to
that profile is born with. In the profile rather than beside the vocabulary,
although both are "set once and forget": a house may run a German main stage
and an English touring production, and the thing this presets — the channel map
— is profile-scoped too, so nothing here reaches across a scope boundary. A
file written before it existed loads on the default rather than on an empty
string, which would make every channel added afterwards languageless. See P7
for what the picker itself does and does not rewrite.

Three decisions worth keeping in view:

- **JSON, not the P10 database.** Configuration is read once and written when a
  human edits something, which is nothing like the write load transcripts have.
  It is also the one file an operator may have to repair by hand or send to
  someone else when it is wrong.
- **Saved membership is positions, live membership is keys.** Both are right
  where they are: the key is stable while a row is edited, which is what stops
  a deleted channel from re-pointing every group below it; the position is what
  still means something after a restart, when every key is a number the new
  window has never issued. `ui/src/lib/profiles.ts` owns that translation and
  is the only place that does, and `profiles.test.ts` pins down the case it
  exists for — a channel deleted between two saves.
- **Nothing here may stop the app from starting.** A missing file is a first
  run; an unreadable one, or one from a newer version, is moved to
  `config.bad.json` and the app comes up empty and says so, rather than
  overwriting the only copy of a setup before anyone could look at it. Writes
  go through a temporary file and a rename, so an interrupted save cannot
  truncate the real one.

**Device recognition by a stable id** is done. A profile now stores the
CoreAudio **UID** beside the display name and is keyed on it, so an interface
renamed in Audio MIDI Setup is still recognised and two units of the same model
are two entries in the picker rather than one. cpal knows a device only by its
name and keeps its `AudioDeviceID` private, so the id is read from CoreAudio
beside cpal — `audio-core/src/backend/stable_id.rs` — and matched to what cpal
enumerated. Four things about it are worth knowing:

- **The name stays, and is not a leftover.** A device that is not plugged in
  still has to be *named*: "Scarlett 18i20 is not connected" is a sentence an
  operator can act on, a UID is not. The name is what is displayed and what an
  old file is migrated from; it is never what a lookup keys on while an id is
  available.
- **The migration costs the operator nothing.** A profile written before this
  has no id. The first time the interface it names is seen connected, the id is
  filled in and saved — `adoptDeviceIds` in `profiles.ts`, and nothing is asked
  and nothing is re-entered. A profile whose device is *absent* is left alone
  rather than bound to whatever is plugged in, which is the same rule as ever:
  the profile describes the production, not the room the laptop is in.
- **Once a profile has an id, the name stops being a fallback.** A profile with
  an id has been resolved once already, so an id that is not there means the
  device is not there — the second unit of the same model must not stand in for
  it. The name fallback exists only for a profile that has never had an id, and
  only until it does.
- **It degrades rather than fails.** A device CoreAudio will not identify, and
  every device on Windows until the WASAPI port brings its endpoint id, falls
  back to the name as its id. That is exactly what the app did before, so the
  worst case is the old behaviour and never a device without identity.

**Outstanding:** the settings proper — worker count, model choice, VAD
thresholds, maximum segment length, theme — are all still constants in the
code, and none of them is in the file yet. (The P3b hysteresis parameters left
this list with the package.)

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

The one active list is stored and read back since P12 — beside the profiles
rather than inside one, because the jargon of a house outlives any one patch
and a copy per profile is a copy that drifts. An emptied box stays empty across
a restart: the file tells "never set", which keeps the default, apart from
"deliberately cleared", which turns the prompt off and is how its effect gets
measured at all.

**Outstanding:** *named* lists with exactly one active — today there is one
list and no way to keep a second — a shipped starter list, and shortening that
is deliberate rather than whisper's silent truncation. And the measurement: the point of a global list is
that its effect is visible on *every* channel, which needs the P14 fixture set
to show.

**Accepted when:** on the P14 fixture set the enabled global vocabulary list
measurably improves the word error rate of the technical terms against the
baseline on every applicable channel.

### P17 — Alert terms & visual attention — **Partial**

The global Settings alert list is compiled once when it changes and applied to
each final transcript before it reaches any view. The resulting match ranges
travel with the transcript event, so the main chat, detached group windows and
the LAN viewer highlight exactly the same text rather than each implementing a
slightly different matcher. Alert metadata lives only as long as the live chat,
like the transcript itself while P10 is deferred.

The desktop uses the platform's native attention request for a new match in an
unfocused window. The same request is also used for every new message when the
affected group has enabled its flash-on-message setting. This is deliberately
not an audible alarm: the request is for the window to flash, as requested, and
must not add noise to an intercom environment. Main chat sees every alert match;
a detached group window is alerted only for its current members and its own
group preference. The browser viewer shows highlights but cannot ask another
device's operating system to flash its window.

**Done:** the list, the matcher and the main window's attention request. The
terms are a row each in a new Settings section beside the vocabulary, stored
globally in `config.json`, and pushed to the backend when they change rather
than read from that file — the file is a debounce behind the term just typed.
`app::alerts` compiles them once and matches every finished transcript in
`TauriEvents::transcript`, which is the one place every view's copy of an event
comes from; the ranges travel with the event in UTF-16 code units, because the
consumer slices JavaScript strings. Matching is case-insensitive and on whole
words, so `Feuer` does not fire on `Feuerwehrzufahrt`. The chat marks the exact
range — piece by piece, never as markup, because a transcript is text from
outside — and rings the bubble holding it. Each group carries its
flash-on-message preference, stored with it and editable while a session runs.
The attention request is counted per bubble and not per segment: one sentence
arrives as three segments, and three flashes for one sentence is exactly the
noise this must not add.

Which window asks, and for what, is one rule in `ui/src/lib/attention.ts` that
both views call, so they cannot drift into counting differently. The main
window is urgent about every alert term whichever group is selected — it is the
view of the whole session, and a word that must not be missed must not be
missed because of what is on screen — and flashes only for the group it is
showing. A group window is the other way round: urgent only for its own
members, since it never claimed to be a view of the session, and always
flashing when that group asks for it, because it *is* that group's view and
there is no "while it is selected" to qualify.

**Outstanding:** Unicode normalisation. A term and a transcript that differ
only in composed and decomposed umlauts do not match today. The third view the
match ranges travel for — the remote client (P16) — arrived with that package
and needed no matcher of its own, which was the point of doing the matching
once in the sink.

**Accepted when:** adding a term in Settings survives a restart, a matching
incoming transcript highlights the exact match in every applicable chat view,
and an unfocused main or applicable group window requests attention once; a
non-matching word and a partial-word false positive do neither. With a group's
flash-on-message setting enabled, an unfocused view of that group requests
attention for a non-alert message; disabled groups do not. ✔ for the main
window, the detached group windows and their lists, and for the browser — which
has no window to bounce and flashes its own surface and its tab title instead.

### P16 — LAN remote client — **Partial**

The operator can optionally enable a local-network client from Settings. The
app then serves a browser build of the same UI on the selected LAN interface;
there is no cloud service and no data leaves the local network. Settings show
a shareable URL. The listener is off by default and binds only to the
explicitly selected interface.

The point of it is a second machine: the session runs where the audio
interface is, and the operator works from somewhere else. The browser is
therefore a full peer rather than an observer. It receives the complete
operational view — all chat history and new transcripts including alert-term
highlights, every channel and group, levels, stream health, session state and
the configuration — and it can do what the local app can: start and stop,
clear communication, edit channels, groups, vocabulary, alert terms and
profiles.

There is no authentication yet. Anyone who reaches the chosen interface can
control the session, which is why the listener is off by default and the
interface is picked deliberately. The transport is arranged so that a token
check is one middleware in front of the socket when it is wanted, not a
redesign. What that decision does *not* cover, and what is therefore checked
already: a WebSocket upgrade is not subject to the same-origin policy, so the
socket refuses an upgrade whose stated origin is not the address this app is
serving on — otherwise any page anybody on the network opened could drive the
session out of the operator's own browser.

Every mutation exists exactly once. The Tauri commands become thin wrappers
over a command layer that takes typed arguments and returns typed results, and
the socket dispatches into that same layer — so the remote cannot quietly fall
behind the desktop as commands are added, and no action has two
implementations to keep in agreement. Configuration is last-write-wins: a save
from either side is broadcast to every client at once, so the side that lost
sees it happen instead of holding a stale form.

Three things stay local because the browser has no equivalent. The native file
picker for models is desktop-only, so remote model management covers the
managed downloads and not an arbitrary file on the host. OS attention requests
are desktop-only, so the remote flashes in the page and in the tab title
instead. Detached group windows are desktop-only, so the remote uses in-page
panels or its own browser tabs. A group opened in a local window and the same
group on the remote are independent views of one backend state, not separate
sessions.

Initial state arrives as a snapshot over the socket, followed by ordered
incremental events; reconnecting repeats the snapshot. That snapshot is bounded
by the same in-memory history the local app gets, so a client that slept longer
than the buffer resumes from what is left rather than from nothing. Levels are
downsampled for remote clients, so a slow link falls behind on meters and never
on transcripts.

**How it was built.** The command layer came first, as the ordering above
demanded. Every mutation now lives exactly once in `crates/app/src/commands.rs`
as one arm of a `Command` enum; the Tauri bridge is a single `command` that
hands its request to `dispatch`, and the socket hands it the same request.
Adding a command is one arm and one typed wrapper in `bridge.ts` — there is
nowhere left to add it to only one of the two views. Three things stayed
`#[tauri::command]`s because a browser has no equivalent for them, and an enum
arm answering "not on this transport" would have suggested the remote was
missing something: the native file picker, the platform's attention request,
and the detached group windows.

The listener is `axum` on the one address the operator picked, serving the
frontend out of Tauri's own asset resolver — the same bundle the app runs in
its WebView, embedded in the binary, so there is no second copy on disk to fall
a version behind.

In a checkout it serves the Vite dev server instead, and it checks for that
*first*. The asset resolver answers in development too, out of whatever was
last left in `ui/dist`, so preferring it means the operator's window is running
the dev server and the remote is running a build from some earlier afternoon:
two builds of this app driving one session. That cost half a day. It does not
look like a stale build — it looks like a bug that will not be fixed, because
every fix lands in the window and none of them reaches the browser, which is
where it is being tested. A released app has no dev URL and is unaffected.

The other half of that is *when* a save happens. The Settings page has no Save
button — a setup an operator believes is stored and is not is worse than no
persistence at all — so it writes on a debounce as the state changes. With
several views onto one configuration that is no longer free: the write is what
every view rebuilds its page from, so a save landing while a label is being
typed rebuilds the field under the cursor, and the label has to be typed one
character at a time with the focus chased back into the box after each. The
save therefore waits for the field to be left. Not for a button — leaving the
field is the operator saying they are done with it, and they were going to do
it anyway. Only fields that are *typed into* wait: a colour, a language or a
device is one discrete act with nothing half-finished about it, and making it
wait would be making it wait for nothing. A window closed with the cursor still
in the box writes on the way out, as a best effort, which is the difference
between usually keeping that last edit and never keeping it.

One rule turned out to carry the whole of last-write-wins: **a save that
changes nothing is not a save.** Adopting another peer's configuration is a
state change like any other, so it schedules that view's own save — and a view
that writes back what it was just given turns one edit into a conversation
nothing ends. Two peers then bounce the configuration off each other several
times a second, and because every view rebuilds its Settings page from it, the
field being typed in is torn down under the cursor on each pass. The views hold
what they know the app already has and skip a save that equals it, but care in
one view is not a property of the system: `save_config` compares what it is
sent with what the file holds — both tidied, so the comparison has an answer —
and a configuration that changes nothing is neither written nor announced,
whoever sent it and whatever version they are running.

Whether it runs is a consequence of the configuration file and of nothing else:
`save_config` applies the remote block, at every save and once at startup.
There is deliberately no "enable the remote" command, because the configuration
has one writer and a second mutation path is how the file and the listener come
to disagree about whether the app is serving. What the configuration cannot
know — whether the listener actually came up, on a port that may be taken, and
how many browsers are connected — is reported separately as a `remote-status`
event.

The frontend learned one new seam, `transport.ts`, and nothing above it knows
which transport it is on. A browser reconnects with backoff and re-reads the
snapshot, which is the same call it makes when it opens rather than a second
mechanism that could disagree with it; while it is down it says so, because a
silent remote and a quiet intercom look identical. The main window now follows
`config-changed` like the group windows always have — it is no longer the only
writer — and carries the keys it handed out across an adopted configuration by
position, so a channel renamed on the remote does not read as a profile switch
and close the operator's group windows.

**Outstanding:** authentication, which the transport is shaped for and which
nothing yet asks for. The acceptance below has been verified against a mock app
over a real socket — commands dispatch, events arrive, and disabling the
listener drops the connections it was serving — but not yet from a second
machine on a real network, which is the one part of it that cannot be written.

**Accepted when:** a second device on the chosen LAN opens the URL, sees the
same existing and incoming communication, levels and group views as the
operator, and can start and stop the session and change configuration with the
change appearing on the operator’s screen; a remote config save reaches the
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
              P14 Fixtures ──> P3b Language stabiliser ──┐
                                       │                 │
                                       v                 v
                            P6 Tauri API (commands/events)
                              ├─> P17 Alert terms ──> P7 Chat UI ──> P8 Health/levels ──> P9 Search/filter
                              │
                              └─> P16 LAN remote client   (P10 SQLite: deferred)
                                       │
                                       └── P12 Settings
                                       v
                                 P13 Packaging
```

**Milestone A (technical backbone):** P0 → P1 → P2 → P4 → P5. One channel, text
in the log. *Reached, including the pool — what P5 still owes is a load test on
live audio.*

**Milestone B (usable MVP):** + P6, P7, P11, P12, P16, P17. Eight channels,
chat UI, model download, alert terms and the optional LAN remote client. *P3 is done, eight
channels run, the chat view the eight make necessary — Settings page, group
sidebar, bounded list — is in place, and the setup now survives the window as
a profile, the model download is done, the alert terms are in, and the remote
client serves the same UI to a browser on the house network. What is left for
this milestone is the settings proper — every threshold is still a constant in
the code.*

**Milestone C (language quality & production readiness):** + P14 fixtures, P15,
P8, P9, P13. *P10 and P3b left it on 2026-09-18.*

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
   could explain a bad result. It is also the one step here that cannot be
   written: it needs the interface and somebody to talk into it.
2. **The rest of P12 — the settings proper.** The presets are done, the model
   choice has joined them and the device is now recognised by a stable id, so
   what is left is the thresholds: the Silero threshold and the segmenter's
   `silence_timeout` and `max_segment` are still constants in the code. They
   come *after* step 1 on purpose — exposing a number as a setting fixes the
   default somebody will keep, and the current defaults are single-stream
   guesses.
3. **P15** — develop vocabulary against the measurable baseline from step 1,
   not by feel. The control exists and is global; what is missing is the
   evidence that any particular list is better than another.
4. **P9, respecified.** It was FTS5 over the stored session, and with P10 set
   aside there is no stored session to search. What it becomes is a search over
   what the window holds — smaller, and it runs into the chat's DOM window,
   since a hit may lie outside the rendered range. Worth deciding before it is
   built.
5. **P17's last piece** — Unicode normalisation of terms and transcripts
   before the match. All three views exist now and all three highlight the same
   characters, because the match happens once in the sink every view's copy of
   an event comes from. What is left is that a term and a transcript differing
   only in composed and decomposed umlauts do not match, which is a bug an
   operator would read as the alert list not working.
6. **The remote, on a real network.** P16 is built and tested against a mock
   app; what it has not had is a second machine, a house network and somebody
   working from it. That is the one part of its acceptance that cannot be
   written, and it comes before authentication — which is the obvious next
   thing to want, and the wrong thing to build against an arrangement nobody
   has used yet.

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
   3. Select a third language in the setup and put a stream on it — from the
      picker, which offers every language the installed whisper knows, without
      a code change.
4. Overdrive one channel and leave another silent → P8 shows both.
5. Enter a search term → hits across all streams, jumping works.
6. `kill -9` a sidecar → the app keeps running, the worker comes back.
7. On a machine that has never run this: no model, and the Settings page says
   so and offers the download. Fetch it, interrupt it, resume it, then
   transcribe — the first transcription without ever opening a terminal (P11).
8. Stop capture, press **Clear all communication**, confirm → all chat lines
   and search hits are gone, while channels, groups, the model choice and
   Settings remain.
9. Open a group in its own window, then speak on one member and one non-member
   channel → only the member transcript appears in that window; global
   Start/Stop is reflected there without creating another capture session.
10. Enable the remote client in Settings, pick the house interface, and open
    its URL from a second device → it sees the same history, incoming lines,
    groups, meters and session state. Start and stop the session from the
    browser and watch the operator's screen follow; rename a channel there and
    watch the Settings page on the app take it on without the operator's own
    edits being thrown away; open a group from the browser's sidebar and check
    it comes up as a tab showing only that group's channels. Pull the network
    for ten seconds → the page says it is not connected, and on its return the
    chat is complete rather than missing the gap. Switch the listener off →
    the browser drops immediately.
11. Rename the interface in Audio MIDI Setup and restart → the profile still
    finds it and starts, under its new name. With two identical interfaces
    connected, the picker offers both and each profile keeps the one it was
    set to (P12).
12. Run the P14 reference load test, record p95 latency.
13. Add `cue` to Alerts, unfocus the main window and speak “Cue fünf” → the
    exact word is highlighted in the chat and the window requests attention;
    “rescue” does neither. Repeat from a channel in an open group to verify its
    separate window. Then enable that group's **Flash on every new message**
    setting and speak a non-alert message → its window requests attention;
    disable it and repeat → it does not. Verify that the browser highlights the
    same characters and flashes its own surface and tab title rather than
    asking the platform (P17, P16).

Step 7 used to read "kill the app hard, restart → the session loads completely
from SQLite". That goes with P10: the chat lives in the window, and killing the
app loses it by design.

---

## Deliberately out of scope

- Export (TXT/SRT/JSON). The P10 schema was prepared for it, and with P10 set
  aside an export would have to read the window's own messages instead.
- Markers and bookmarks.
- Windows port: only P1 needs reimplementing, provided the `AudioBackend` trait
  is respected; plus the GPU backend choice in P4.
- Speaker diarisation.
- Second decode of low-confidence segments in the alternative language, better
  result wins. Doubles the latency for those segments.

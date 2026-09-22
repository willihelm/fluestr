# Fluestr

An offline desktop app that transcribes several intercom streams at once and
presents them as a chat log.

Audio engineers often struggle to monitor intercom channels and what was said is gone quickly. This app makes what you missed readable
and searchable, locally and without any cloud service.

**The name** — *Fluestr* is German *Flüster* / *Flüstern*, "whisper", written
the way German spells an umlaut without one (ü → ue). It is pronounced
[ˈflʏstɐ]. It allows you to turn down your shout box and still get what is said on coms.

**Status:** one channel runs end to end — microphone, ring buffer, resampling,
segmentation, sidecar, chat list — at a measured **~330 ms** from the end of a
spoken segment to the text appearing. That is a deliberately narrow vertical
slice to make the real latency measurable, not the MVP: it handles a single
stream with a single worker and a provisional voice detector.
[docs/measurements.md](docs/measurements.md) lists what is provisional and
which work package replaces it.

The sixteen work packages and where each one stands are in
[docs/plan.md](docs/plan.md) — plan and progress in one file, so they cannot
drift apart.

---

## Requirements

macOS on Apple Silicon. **Full Xcode is not needed** — the Command Line Tools
are enough, see [docs/measurements.md](docs/measurements.md).

```sh
xcode-select --install
brew install cmake pnpm just
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

## Getting started

```sh
just setup              # frontend dependencies
just model small-q5_1   # a speech model, until the model manager exists (P11)
just vad-model          # Silero, so noise and silence are not transcribed
just dev                # run the app
just check              # everything CI runs as well
```

Or, in a terminal that has never seen rustup — no `just`, nothing on `PATH`:

```sh
./scripts/run.sh              # build and start, hot reload
./scripts/run.sh --release    # build the bundle and start that instead
```

The script finds the toolchain, installs the frontend dependencies if they are
missing, builds the sidecar and opens the window. Models are downloaded in the
app itself (P11); the two `just` lines above are the older way in and what the
integration tests still read.

Without the VAD model the app still runs, but whisper captions room noise and
silence as well — as `[BREAK]`, `(music)`, or as plausible sentences that were
never spoken. The sidecar warns when it starts without one.

In the app: the first start opens a setup guide — it downloads the speech
model, fetches the Silero speech detector without asking, and walks the audio
interface, its channels and the groups. It can be skipped, and it is at the
foot of the Settings rail afterwards. Then pick a device, pick a channel, press Start.
The first start on a machine takes about eight seconds because macOS compiles
the Metal shaders once; afterwards it is a fraction of a second.

Two environment variables override the stand-ins for work packages that do not
exist yet:

| Variable | Replaces | Default |
|---|---|---|
| `FLUESTR_MODEL` | the model manager (P11) | largest non-Silero `.bin` in `.scratch/models` |
| `FLUESTR_VAD_MODEL` | the model manager (P11) | newest `ggml-silero-*.bin` in `.scratch/models` |
| `FLUESTR_WORKER` | the bundled sidecar (P13) | `whisper-worker` next to the app binary |

## Building a signed app

```sh
just build     # signed Fluestr.app and a DMG in target/release/bundle
just verify    # what the signature actually says, app and sidecar
```

The sidecar is bundled as a Tauri `externalBin`, so it lands in
`Contents/MacOS` next to the app and is signed with the same identity and the
same entitlements. Hardened Runtime is on, which is why
`Entitlements.plist` carries `com.apple.security.device.audio-input`: the
`Info.plist` key makes macOS ask for the microphone, the entitlement makes the
answer count.

Without an identity in the environment the bundle is signed **ad-hoc**. That
runs here and satisfies Hardened Runtime, but it carries no team identity — so
`spctl` rejects it, another Mac shows a Gatekeeper warning, and notarisation
cannot run. All three want a Developer ID, which needs the paid Apple Developer
Program. Once there is one, nothing here changes but the argument:

```sh
just build "Developer ID Application: Your Name (TEAMID)"
```

Notarisation is then the credentials `tauri build` already looks for and
reports missing — `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password) and
`APPLE_TEAM_ID`, or an App Store Connect key in `APPLE_API_KEY`,
`APPLE_API_ISSUER` and `APPLE_API_KEY_PATH`.

### Handing a build to testers

```sh
just beta      # target/beta: the DMG and the instructions that go with it
```

While the signature is ad-hoc, macOS blocks the app on a machine that is not
this one and the tester has to allow it through System Settings by hand — once
per build, and Control-click → Open is *not* a way around it on macOS 15 and
newer. [docs/beta-install.md](docs/beta-install.md) is written for the tester
rather than for this repository, and `just beta` copies it next to the DMG so
the two travel together.

## Layout

```
crates/
  audio-core/        capture, ring buffers, resampling, segmentation — UI-free
  whisper-protocol/  frame format between app and sidecar, shared by both
  whisper-worker/    sidecar process, keeps a whisper model warm
  app/               Tauri backend: session, worker pool, events
ui/src/lib/          bridge types, chat list, level meter
ui/                  Svelte + TypeScript
docs/plan.md         the sixteen work packages and their current status
docs/measurements.md measured evidence behind the architecture decisions
```

## Architecture in three sentences

Each intercom channel is demultiplexed individually out of a multichannel
interface, pushed through a lock-free ring buffer down to 16 kHz mono and cut
into speech segments by a VAD.

Those segments flow through **one** prioritised queue into a pool of sidecar
processes — deliberately fewer workers than streams, because parallel whisper
instances contend for the same Metal queue.

The results travel as Tauri events into a virtualised chat view and on into
SQLite.

## Languages

The app is built for **code switching**: mostly German with English technical
terms ("Standby für Cue fünf, dann fade out").

Whisper assigns exactly one language token per segment — there is no genuine
"both at once" mode. The solution is therefore a **fixed primary language per
stream plus a vocabulary prompt**, not auto-detection. Auto-detection actively
makes this case worse: a segment with a high share of English flips into
English mode and mangles the German remainder. The measurements are in
[docs/measurements.md](docs/measurements.md).

Auto-detection still exists — opt-in per stream and restricted to a candidate
list. The language list itself comes from whisper.cpp at runtime; there is no
`de`/`en` enum in the code. A third language is configuration, not a code
change.

## Whisper models

Models are not bundled (P11: download on first start). For development:

```sh
just model small-q5_1   # into .scratch/models/
```

Recommendation for "latency first": `small-q5_1` — measured at 9-14x realtime
on an M1 Pro with usable accuracy on mixed-language material.

## Tests

```sh
just test    # unit tests, no hardware or model required

# Audio hardware (P1) — needs an interface and microphone permission:
cargo test -p audio-core --test capture_live -- --ignored --nocapture list_devices

CAPTURE_DEVICE="Waves SoundGrid" CAPTURE_CHANNELS=0,1 CAPTURE_SECONDS=10 \
  cargo test -p audio-core --test capture_live -- --ignored --nocapture record
# writes one WAV per channel into .scratch/capture/ — speak into a single
# input at a time and confirm only the matching file carries the signal

# End-to-end against the real sidecar (needs a model and a fixture):
just model small-q5_1
just fixture de_mixed "Achtung Regie, Standby für Cue fünf. Auf mein Kommando dann fade out und Blackout."
WHISPER_TEST_MODEL=.scratch/models/ggml-small-q5_1.bin \
WHISPER_TEST_FIXTURE=.scratch/fixtures/de_mixed.wav \
  cargo test -p whisper-worker --test sidecar -- --ignored --nocapture

# Resampling throughput (P2) — a property of the machine, so not part of CI:
cargo test -p audio-core --release -- --ignored --nocapture resampling_cost

# The whole slice against a WAV file instead of a microphone:
just slice
```

`just fixture` generates test audio through macOS text-to-speech. That is fine
for quick regression tests but is no substitute for real recordings — the WER
baseline in P14 needs actual intercom material.

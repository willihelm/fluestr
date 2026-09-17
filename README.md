# Intercom2Chat

An offline desktop app that transcribes several intercom streams at once and
presents them as a chat log.

Audio engineers monitor several intercom channels during a production and
inevitably lose information — nobody can follow eight conversations in
parallel, and what was said is gone. This app makes what you missed readable
and searchable, locally and without any cloud service.

**Status:** P0 (scaffolding) and P1 (audio capture) are in place, P4 (whisper
sidecar) works at its core. Segmentation, worker pool and chat UI still to
come — the pieces are not yet wired together, so there is nothing to run
end-to-end.

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
just setup   # frontend dependencies
just dev     # run the app in development mode
just check   # everything CI runs as well
```

## Layout

```
crates/
  audio-core/        capture, ring buffers, resampling, segmentation — UI-free
  whisper-protocol/  frame format between app and sidecar, shared by both
  whisper-worker/    sidecar process, keeps a whisper model warm
  app/               Tauri backend: session, worker pool, events
ui/                  Svelte + TypeScript
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
```

`just fixture` generates test audio through macOS text-to-speech. That is fine
for quick regression tests but is no substitute for real recordings — the WER
baseline in P14 needs actual intercom material.

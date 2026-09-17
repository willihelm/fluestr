# Measurements

Evidence behind the architecture decisions. Every number here was measured,
not estimated.

**Hardware:** Apple M1 Pro, macOS 27.0, Command Line Tools only (no Xcode)
**Date:** 2026-09-17
**Versions:** whisper-rs 0.16.0 / whisper.cpp, models `ggml-tiny` and `ggml-small-q5_1`

---

## 1. Metal without full Xcode

**Question:** Does the whisper.cpp build need the Metal compiler from Xcode
(~15 GB)?

**Answer: no.**

```
GGML_METAL:BOOL=ON
GGML_METAL_EMBED_LIBRARY:BOOL=ON
METAL_FRAMEWORK:FILEPATH=/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/System/Library/Frameworks/Metal.framework
ggml_metal_library_init: using embedded metal library
ggml_metal_device_init: GPU name: Apple M1 Pro
```

The shader source is embedded into the binary and compiled at runtime by
`MTLDevice`. `xcrun metal` — and therefore Xcode — is never invoked. The macOS
SDK shipped with the Command Line Tools provides `Metal.framework`.

**Consequence for P13:** the CI runner needs no Xcode, only the CLT (present on
`macos-14`).

---

## 2. Cold start: one-off shader compilation

| Run | `ggml_metal_library_init` | total model load |
|---|---|---|
| first on the machine | **8.345 s** | 8.46 s |
| every subsequent one | 0.008 s | 0.12-0.14 s |

macOS caches the compiled shaders system-wide. The 8-second hit only affects
the very first start after installation or an OS update.

**Consequences:**
- **P5:** workers must be pre-warmed, not spawned on demand. A cold start in
  the middle of a session would mean eight seconds of silence.
- **P11:** the first start after installation needs a progress indicator,
  otherwise the app looks hung.
- The protocol field `Ready.load_ms` carries the value so the UI can tell the
  two cases apart.

---

## 3. Throughput

Fixture: 5.65 s of German speech (macOS TTS, voice Anna), 16 kHz mono.
Greedy decoding, `no_context`, `single_segment`, temperature 0.

| Model | Decode | Realtime factor |
|---|---|---|
| `tiny` (warm) | 125 ms | 45x |
| `small-q5_1` without prompt | 617 ms | 9.2x |
| `small-q5_1` with prompt | 384-412 ms | 13.7x |

**Consequence for P5:** at 9-14x realtime per stream, two workers cover eight
streams with a comfortable margin. The plan's assumption "worker pool != stream
count" survives the measurement.

**Still open:** these figures come from clean TTS material. Real intercom
traffic with noise, compression and crosstalk will do worse — the number is an
upper bound, not a promise. The reference load test in P14 has to be repeated
with real recordings.

---

## 4. Code switching: effect of the vocabulary prompt

This is the heart of P15. The spoken reference sentence (German, as this is
what the operators actually say):

> Achtung Regie, Standby für Cue fünf. Auf mein Kommando dann fade out und Blackout.

Prompt: `Intercom-Kommandos: Standby, Cue, Go, Fade out, Blackout, Talkback, Regie.`
Language fixed to `de` in every run, `translate = false`.

| Model | Prompt | Result |
|---|---|---|
| `tiny` | no | Achtung **Regi**, **stend bei** für **Kio** 5 … **Fahrdeaut** und Blackout |
| `tiny` | yes | Achtung Regie, **Standby** für **Kio** 5 … **Farde out** und Blackout |
| `small-q5_1` | no | Achtung Regie, Standby für **Q5** … **Fadeout** und Blackout |
| `small-q5_1` | yes | Achtung Regie, **Standby** für **Cue** 5 … **Fade out** und Blackout |

**Two separate effects, both real:**

1. **Model size** fixes the gross mishearings (`stend bei` -> `Standby`).
2. **The prompt** additionally fixes the technical terms that even `small`
   otherwise gets wrong (`Q5` -> `Cue`, `Fadeout` -> `Fade out`).

Together they produce a practically usable transcript. That confirms the plan's
decision to solve code switching through **fixed primary language + vocabulary**
rather than through auto-detection.

**Caveat:** TTS-generated material pronounces English terms with German
phonetics — arguably a harder test for the code-switching case than real
speakers, but not the same thing. The P14 fixture set needs real recordings
before it becomes a defensible WER baseline.

---

## 5. Language list

`whisper_lang_max_id()` provides the complete list at runtime, including
spelled-out names via `whisper_lang_str_full`. The sidecar exposes it through
`ListLanguages`.

That satisfies the extensibility requirement: a third language is
configuration, not a code change. There is no `de`/`en` enum in the codebase —
only the open newtype `LanguageCode`.

---

## 6. Audio capture (P1)

Verified against the hardware actually present on the development machine.

| Device | Channels | Default rate | Advertised rates |
|---|---|---|---|
| Waves SoundGrid | 32 | 44100 | 44100, 48000, 88200, 96000 |
| MacBook Pro microphone | 1 | 48000 | 44100, 48000, 88200, 96000 |
| NDI Audio | 2 | 44100 | 44100, 48000 |
| LoomAudioDevice | 2 | 48000 | 48000 |

Capturing three channels (0, 7, 31) of the 32-channel device for three seconds
produced **131584 frames at 44.1 kHz with 0 samples dropped** and three
separate WAV files. The built-in microphone delivered real signal over the same
path (peak 0.12, rms 0.010), which confirms the microphone permission is
granted and audio genuinely flows — a silent capture would otherwise be
indistinguishable from a denied permission.

**Still unverified:** that channels are not swapped *on real hardware*. The
de-interleaving logic is covered by unit tests, including the case where a
device buffer spans several scratch passes, which is where an off-by-one would
hide. But the SoundGrid channels carried no signal, so the end-to-end mapping
could not be confirmed. Patching a known source to one specific channel and
checking that only the matching WAV carries it remains a manual step.

### `cpal::Stream` is `!Send` on macOS

CoreAudio's property listener holds a plain `FnMut`, which makes the whole
stream non-transferable between threads. The app needs a session handle it can
keep in shared state, so the stream is confined to a dedicated thread that owns
it from construction to drop, and the handle communicates only through
channels.

Asserting `Send` on the stream would compile and appear to work. It was the
first thing tried and then removed: it is an unchecked claim about what
CoreAudio permits, and the failure it invites — destroying a stream on the
wrong thread — is the kind that surfaces as an intermittent crash in
production rather than in a test.

### Device identity is weaker than it looks

cpal exposes only `name()`, no stable identifier. Two identical interfaces
therefore produce the same id, and renaming a device in Audio MIDI Setup breaks
the match.

**Consequence for P12:** presets must tolerate a failed device lookup and ask
the user to re-pick, rather than silently capturing from the wrong hardware. A
CoreAudio UID would fix this properly and needs a platform-specific extension
below the `AudioBackend` trait.

---

## 7. Resampling to 16 kHz (P2)

`rubato` `SincFixedIn`, 128-tap sinc with a Blackman-Harris² window, cutoff from
`calculate_cutoff`, 1024-frame input chunks, one channel per instance.

**Anti-aliasing.** A 12 kHz tone at 48 kHz input sits above the 8 kHz Nyquist of
the 16 kHz output. Naive decimation would mirror it onto 4 kHz — the middle of
the speech band, indistinguishable from real signal afterwards.

| Measurement | Value |
|---|---|
| 12 kHz tone, energy at 4 kHz after conversion | 0.000024 (**−92 dB**) |
| 1 kHz tone, amplitude after conversion | 1.00 (±0.05) |
| Spurious energy at 0.5 / 2 / 3 / 6 kHz | < 0.02 |

**Cost.** One stream, 48 kHz → 16 kHz, release build:

| Audio | Wall clock | Realtime factor |
|---|---|---|
| 60 s | 26-28 ms | **~2200x** |

Eight streams therefore cost well under one percent of a core. Resampling is
not a latency factor; it only must not happen in the audio callback, where the
per-block cost would still be a deadline risk.

**Latency floor.** Two terms, both deliberate: the 1024-frame input chunk
(21 ms at 48 kHz) and half the filter length as group delay (64 input samples,
1.3 ms). About 22 ms in total — an order of magnitude below the segment lengths
P3 works with, so it does not shape the budget. 256 taps would roughly double
the second term for stopband attenuation nobody can use at 16 kHz.

**Overruns.** Per-stream counters live on the ring buffer (`RingStats`), not
only in the device-wide `CaptureStats`: "channel 3 is losing audio" is
actionable for an operator, "the session is losing audio" is not. A stalled
consumer raises the count and the callback keeps running — verified by unit
test, which is the P2 acceptance criterion.

### Reproducing

```sh
cargo test -p audio-core --release -- --ignored --nocapture resampling_cost
```

---

## 8. Incidental findings

- **whisper-rs 0.16 ships a Silero VAD** (`whisper_vad.rs`). Relevant for P3:
  the planned alternative to `webrtc-vad` may already be available and would
  not need a separate dependency.
- **`lang_detect(offset_ms, threads)`** returns `(lang_id, Vec<f32>)` — the
  full probability vector the language stabiliser (P3b) needs for its argmax
  over the candidate languages. Whether the separate call costs an additional
  encoder pass is still to be determined.
- **whisper.cpp logs on its own.** Without `install_logging_hooks()` the C++
  side writes unfiltered, and anything landing on stdout corrupts the frame
  protocol. The sidecar therefore installs the hooks as its first action.

## Reproducing

```sh
just model small-q5_1
just fixture de_mixed "Achtung Regie, Standby für Cue fünf. Auf mein Kommando dann fade out und Blackout."

WHISPER_TEST_MODEL=.scratch/models/ggml-small-q5_1.bin \
WHISPER_TEST_FIXTURE=.scratch/fixtures/de_mixed.wav \
  cargo test -p whisper-worker --test sidecar -- --ignored --nocapture
```

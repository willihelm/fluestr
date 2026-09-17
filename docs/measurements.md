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

## 8. First end-to-end run (vertical slice)

The narrow slice — one channel from capture through resampling, segmentation
and the sidecar into the chat view — exists so the real latency becomes
measurable before the worker pool (P5) and the full UI (P7) are built. Run
without a microphone via `just slice`, which pushes a WAV fixture down the same
path.

On the 5.65 s German fixture:

| Stage | Result |
|---|---|
| Segmentation | 1 segment, 0.00-5.64 s |
| Model load (warm) | 183 ms |
| Decode | 434 ms |
| Transcript | "Achtung Regie, Standby für Cue 5, auf mein Kommando dann Fade out und Blackout." |

The English terms come through verbatim in German mode, as section 4 predicted.

**The latency budget is dominated by neither resampling nor decode.** For a
6 s segment the decode is ~0.4 s and the conversion is ~3 ms, but the segmenter
first has to wait out `silence_timeout` (500 ms) before it can know the segment
ended. That waiting time is therefore a latency setting, not only a quality
one — which is why it belongs in P12 next to the model choice rather than
buried in P3.

### Live from the microphone

First run of the assembled app against the built-in microphone, one channel at
48 kHz, model `small-q5_1` on the GPU. Six consecutive segments, sequence
numbers 0-5 with no gaps and none discarded:

| Segment | Decode | End to end |
|---|---|---|
| 0 | 342 ms | 344 ms |
| 1 | 329 ms | 330 ms |
| 2 | 300 ms | 301 ms |
| 3 | 322 ms | 322 ms |
| 4 | 306 ms | 307 ms |
| 5 | 319 ms | 320 ms |

"End to end" is measured from the moment the segmenter closes a segment to the
transcript being ready for the UI.

**Everything except the decode costs 1-2 ms.** Ring buffer, resampling,
segmentation and the sidecar round trip together disappear next to whisper
itself. That settles where tuning is worth doing and where it is not: model
size and segment length are the only levers on latency that matter, which is
what P14 should spend its time on.

Counted from the last spoken word, add the `silence_timeout` the segmenter
waits out before it can know the segment ended — about 850 ms in total, well
inside the 1-2 s target on a single stream. Eight streams through one worker
will not hold that; the queue and pool in P5 is what the figure has to be
re-measured against.

**Still unmeasured:** real intercom material. These are short utterances from a
laptop microphone in a quiet room — the easiest possible case, and an upper
bound rather than a promise.

### Whisper captions sound, not just speech

The same run filled the chat with entries that were not communication:

```
0:09  [BREAK]
0:12  [BREAK]
0:14  (music)
0:20  (Bell)
```

An intercom channel is silent or carries only room noise for long stretches,
and whisper annotates what it hears rather than staying quiet. The energy VAD
opened segments for a bell and for background sound because it measures
loudness, and loudness is all it can measure.

Two attempts failed first, and both failures are worth keeping written down
because each one *looked* like protection.

**Failure 1: suppressing the annotations made it worse.** Setting
`suppress_nst` stopped `[BREAK]` and `(music)` — and whisper started producing
ordinary-looking German sentences for the same noise instead. It still has to
emit something; forbidding the annotation tokens only pushes it towards words.
That is strictly worse: an annotation is a *labelled* non-speech marker that
can be filtered with certainty, while a plausible sentence carries nothing that
tells an operator it was invented. The flag is now explicitly set to `false`.

**Failure 2: `params.vad` did nothing at all.** whisper.cpp applies the flag in
`whisper_full` (and `whisper_full_parallel`) but **not** in
`whisper_full_with_state`, which is what the sidecar calls. Setting it compiled,
logged nothing, and changed nothing — verified by reading `whisper.cpp` after
the noise test kept failing.

**Failure 3: `no_speech_probability` is inert under these parameters.** It
reads exactly 0.000 for digital silence, room tone, hiss and loud noise alike.
With greedy decoding at temperature 0 and `single_segment`, whisper.cpp never
fills it in. The check against it remains in the code because it is free and
correct if the value ever becomes meaningful, but it is **not** a defence
today, and describing it as one would be wrong.

**What actually works — Silero, run directly, before any decoding:**

`WhisperVadContext` from whisper-rs is driven explicitly rather than through
the parameter flag. If it reports no speech segments, the sidecar returns empty
text and never calls the decoder.

| Input, 3 s each | Result | 
|---|---|
| digital silence | no segments, no decode |
| quiet room tone (0.005) | no segments, no decode |
| audible hiss (0.05) | no segments, no decode |
| loud noise (0.3) | no segments, no decode |
| German speech fixture | passes through, 437 ms decode, full sentence |

The loud case is the one that matters: at 0.3 amplitude an energy VAD calls it
speech without hesitation. Loudness was never the question.

**It is also faster, not slower.** Four non-speech segments took 2.10 s when
they were decoded and discarded, and 0.89 s once the gate skipped the decode.
The VAD model is 0.88 MB and runs in single-digit milliseconds.

**On CPU, deliberately.** With `use_gpu(true)` ggml aborts the whole process:
it reports "no GPU found" for the VAD backend while the model is already
allocated in a Metal buffer, then trips its own assertion in
`ggml-backend.cpp`. Beyond that, a second Metal user would contend with whisper
for the same queue — the same reason the worker pool is kept small.

**The text filter stays** as a backstop for whatever still gets through
(`is_only_non_speech`). It fails *safe*: unbalanced delimiters mean "keep it",
because a stray `(music` is a much smaller harm than silently swallowing a
sentence that happened to contain a bracket.

Every rejection is logged with its reason. An operator who spoke and saw no
text has to be able to find out which layer fired — silent loss is the one
failure this project does not accept.

**Consequence for P3:** the segmenter's energy VAD stays as the cheap first
cut, but the authority on "is this speech" now sits in the sidecar. Moving
Silero in front of the segmenter would additionally save the IPC round trip,
which is the remaining argument for doing P3 properly. *That is what §10 then
did.*

### What the slice does provisionally

Marked as such in the code, listed here so none of it is mistaken for a
decision:

- **One worker, request/response, no queue.** A long segment blocks the ones
  behind it. P5.
- **Energy VAD.** Works in a quiet room, will not survive an intercom feed with
  constant background noise. P3 has since replaced it behind the `Vad` trait —
  see §10; it remains only as the fallback when the VAD model is missing.
- **Model discovered from `.scratch/models` or `INTERCOM2CHAT_MODEL`.** P11.
- **Sidecar found next to the app binary.** Bundling it as an `externalBin`
  with its own signature is P13.
- **Chat list is unvirtualised.** Fine for minutes, not for an hour across
  eight streams. P7.

### What the first launch cost, and why

Two failures that were both invisible rather than hard, which is the part worth
remembering.

The model lookup resolved `.scratch/models` against the working directory.
`tauri dev` runs the binary with the crate as its working directory rather than
the repository root, so the app reported "no model configured" to someone who
had just downloaded one — a wrong-directory problem wearing a missing-file
message. The search now walks up from both the working directory and the
binary's own location.

Then the UI sat at "loading the model" while the backend was, in fact, running
the whole pipeline — a process sample showed the processing thread resampling
and segmenting and the decode thread waiting for work. The cause was Tauri v2's
permission system: commands defined by the app need no permission, but the core
event plugin does, and there was no capability file at all. So `invoke`
succeeded and `listen` was rejected. Because the rejected promise was never
caught, the failure was completely silent, and a backend whose events go
nowhere looks exactly like a backend that hangs.

**Consequences kept in the code:**

- `crates/app/capabilities/default.json` grants `core:event:default`, and
  nothing else — a future need should fail loudly, not be silently pre-granted.
- A refused subscription now surfaces in the UI.
- `start_session` **returns** the running state instead of only emitting it, so
  the transition the operator is waiting for after pressing Start arrives as
  the answer to that click. Events carry only what happens afterwards.
- Transcripts are logged with their timings — lengths and milliseconds, never
  the words. Intercom traffic is the operator's communication and does not
  belong in a terminal log by default; the text sits at `debug` for when a run
  has to be traced deliberately (`RUST_LOG=app_lib=debug just dev`).

---

## 9. Incidental findings

- **whisper-rs 0.16 ships a Silero VAD** (`whisper_vad.rs`). This is what P3
  ended up using — no `webrtc-vad` and no additional dependency, in the sidecar
  first (§8) and in front of the segmenter since (§10).
- **`lang_detect(offset_ms, threads)`** returns `(lang_id, Vec<f32>)` — the
  full probability vector the language stabiliser (P3b) needs for its argmax
  over the candidate languages. Whether the separate call costs an additional
  encoder pass is still to be determined.
- **whisper.cpp logs on its own.** Without `install_logging_hooks()` the C++
  side writes unfiltered, and anything landing on stdout corrupts the frame
  protocol. The sidecar therefore installs the hooks as its first action.

---

## 10. Speech detection in front of the segmenter (P3)

Silero now decides what the segmenter cuts, instead of frame energy. The
sidecar's gate (§8) stays behind it as a backstop, but it is no longer the
first place in the chain that can tell speech from sound.

`cargo test -p audio-core --test silero -- --ignored --nocapture`, 3 s of each
signal, share of 32 ms frames called speech:

| Signal | Silero | Energy VAD |
|---|---|---|
| digital silence | 0.00 | 0.00 |
| quiet room tone (0.005) | 0.00 | 0.00 |
| loud broadband noise (0.3) | 0.00 | **1.00** |
| loud 440 Hz tone (0.3) | 0.00 | **1.00** |
| German speech fixture | **0.86** | — |

The two loud rows are the whole point. An energy detector opens a segment for
every one of those frames and cannot do otherwise: it measures loudness, and
both signals are loud. Each such segment previously cost a full ~320 ms decode
before the text filter discarded it.

**Through the segmenter.** Noise, a tone and a spoken passage in one signal,
each separated by silence: exactly one segment comes out, and it is the spoken
one (6.23-12.19 s, the speech starting at 6.5 s with the 300 ms pre-roll in
front of it). The energy detector produced a segment for all three.

**The window size is read from the model**, not compiled in: whisper.cpp stores
it in the model file and exposes it nowhere, so `SileroVad` probes it once with
a known number of samples and counts the probabilities returned. Silero v5.1.2
answers 512 samples — 32 ms, which is therefore the segmenter's frame.

**Cost: 0.41 ms per frame, 79x realtime per stream.** A frame is 32 ms of
audio, so one stream spends about 1.3 % of a core on speech detection and eight
spend ~10 % — against decoders that need the machine. That figure includes the
rolling context: `whisper_vad_detect_speech` clears the LSTM state on every
call, so each call replays the preceding four windows and only the last
probability is read. Judging each frame with a cold network would be five times
cheaper and is not worth it at this price.

**Still on one stream.** The 10 % is arithmetic from a single-stream
measurement, not eight streams measured together. That is the P14 load test.

---

## Reproducing

```sh
just model small-q5_1
just fixture de_mixed "Achtung Regie, Standby für Cue fünf. Auf mein Kommando dann fade out und Blackout."

WHISPER_TEST_MODEL=.scratch/models/ggml-small-q5_1.bin \
WHISPER_TEST_FIXTURE=.scratch/fixtures/de_mixed.wav \
  cargo test -p whisper-worker --test sidecar -- --ignored --nocapture

# Section 10: speech detection in front of the segmenter
just vad-model
cargo test -p audio-core --test silero -- --ignored --nocapture
```

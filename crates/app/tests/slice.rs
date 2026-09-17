//! The vertical slice without the microphone.
//!
//! Everything from a WAV file through segmentation and the sidecar to a
//! transcript, so the wiring can be checked without a device, a permission
//! dialogue and somebody talking. Ignored by default: it needs a model, which
//! CI does not have.
//!
//! ```sh
//! just model small-q5_1
//! just fixture de_mixed "Achtung Regie, Standby für Cue fünf. …"
//! INTERCOM2CHAT_MODEL=.scratch/models/ggml-small-q5_1.bin \
//! SLICE_FIXTURE=.scratch/fixtures/de_mixed.wav \
//!   cargo test -p app --test slice -- --ignored --nocapture
//! ```

use app_lib::worker::Worker;
use audio_core::{
    EnergyVad, LanguageCode, LanguageMode, Resampler, Segment, Segmenter, SegmenterConfig,
    StreamId, SAMPLE_RATE,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use whisper_protocol::Response;

/// German on purpose: this is vocabulary data, not code.
const PROMPT: &str = "Intercom-Kommandos: Standby, Cue, Go, Fade out, Blackout, Talkback, Regie.";

fn fixture() -> Option<PathBuf> {
    std::env::var_os("SLICE_FIXTURE").map(PathBuf::from)
}

/// Reads a WAV as mono f32 and resamples it to 16 kHz — the same path the
/// capture side takes, minus the device.
fn read_wav(path: &PathBuf) -> (Vec<f32>, u32) {
    let mut reader = hound::WavReader::open(path).expect("opening the fixture");
    let spec = reader.spec();

    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.unwrap() as f32 * scale)
                .collect()
        }
    };

    // Average the channels rather than take the first: a fixture recorded in
    // stereo would otherwise lose half its level.
    let channels = spec.channels as usize;
    let mono: Vec<f32> = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();

    let mut resampler = Resampler::new(spec.sample_rate).expect("resampler");
    let mut out = Vec::new();
    for block in mono.chunks(1024) {
        resampler.process(block, &mut out).unwrap();
    }
    resampler.flush(&mut out).unwrap();

    (out, spec.sample_rate)
}

#[test]
#[ignore = "needs a model and a fixture"]
fn a_wav_file_travels_all_the_way_to_a_transcript() {
    let Some(path) = fixture() else {
        panic!("set SLICE_FIXTURE to a WAV file");
    };

    let (pcm, source_rate) = read_wav(&path);
    println!(
        "fixture: {} — {source_rate} Hz, {:.2} s at {SAMPLE_RATE} Hz after resampling",
        path.display(),
        pcm.len() as f32 / SAMPLE_RATE as f32
    );

    let mut segmenter = Segmenter::new(
        StreamId(0),
        Box::new(EnergyVad::new(SAMPLE_RATE)),
        SegmenterConfig {
            language_mode: LanguageMode::Fixed {
                language: LanguageCode::german(),
            },
            initial_prompt: Some(PROMPT.into()),
            ..Default::default()
        },
    );

    // Blocks the size the processing thread actually pumps, so the segment
    // boundaries are the ones the running app would produce.
    let mut segments = Vec::new();
    for block in pcm.chunks(SAMPLE_RATE as usize / 50) {
        segments.extend(segmenter.push(block));
    }
    segments.extend(segmenter.finish());

    assert!(
        !segments.is_empty(),
        "the segmenter found no speech in the fixture"
    );
    println!("{} segment(s)", segments.len());

    let binary = app_lib::worker::find_binary().expect("sidecar binary");
    let model = app_lib::worker::find_model().expect("model");
    let vad_model = app_lib::worker::find_vad_model();
    println!(
        "vad model: {}",
        vad_model
            .as_deref()
            .map_or("none — noise will be transcribed".into(), |p| p
                .display()
                .to_string())
    );

    let mut worker =
        Worker::spawn(&binary, &model, vad_model.as_deref(), 4).expect("starting the sidecar");
    println!(
        "model {} loaded in {} ms ({})",
        worker.ready().model,
        worker.ready().load_ms,
        if worker.ready().gpu { "GPU" } else { "CPU" }
    );

    let mut total = Duration::ZERO;
    let mut any_text = false;

    for segment in &segments {
        let started = Instant::now();
        let response = worker.transcribe(segment).expect("transcribing");
        let elapsed = started.elapsed();
        total += elapsed;

        match response {
            Response::Transcript {
                seq,
                text,
                detected_language,
                decode_ms,
                ..
            } => {
                println!(
                    "  [{seq}] {:.2}-{:.2}s  {:>4} ms decode  {:<3}  {text:?}",
                    segment.t_start.as_secs_f32(),
                    segment.t_end.as_secs_f32(),
                    decode_ms,
                    detected_language.unwrap_or_default()
                );
                any_text |= !text.is_empty();
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    worker.shutdown();

    println!("total decode time: {:.2} s", total.as_secs_f32());
    assert!(any_text, "every segment came back empty");
}

/// Silence and noise must produce nothing.
///
/// This is the regression test for the failure that mattered most in practice:
/// on a quiet channel whisper does not stay quiet, it captions what it hears —
/// first as "[BREAK]" and "(music)", and, once those tokens were suppressed,
/// as ordinary-looking sentences. The second form is far worse, because
/// nothing about it tells the operator it was invented.
#[test]
#[ignore = "needs a model"]
fn noise_and_silence_produce_no_text() {
    let binary = app_lib::worker::find_binary().expect("sidecar binary");
    let model = app_lib::worker::find_model().expect("model");
    let vad_model = app_lib::worker::find_vad_model()
        .expect("this test is about the VAD — run `just vad-model`");
    println!("vad model: {}", vad_model.display());

    let mut worker =
        Worker::spawn(&binary, &model, Some(&vad_model), 4).expect("starting the sidecar");

    let cases: [(&str, Vec<f32>); 4] = [
        ("digital silence", vec![0.0; SAMPLE_RATE as usize * 3]),
        ("quiet room tone", noise(3.0, 0.005)),
        ("audible hiss", noise(3.0, 0.05)),
        // Loud enough that an energy VAD would certainly call it speech, which
        // is the whole point: loudness is not the question.
        ("loud noise", noise(3.0, 0.3)),
    ];

    let mut failures = Vec::new();
    for (name, pcm) in cases {
        let segment = Segment {
            stream_id: StreamId(0),
            seq: 0,
            t_start: Duration::ZERO,
            t_end: Duration::from_secs(3),
            pcm,
            language_mode: LanguageMode::Fixed {
                language: LanguageCode::german(),
            },
            initial_prompt: Some(PROMPT.into()),
        };

        match worker.transcribe(&segment).expect("transcribing") {
            Response::Transcript {
                text,
                no_speech_probability,
                ..
            } => {
                let verdict = app_lib::session::discard_reason(&text, no_speech_probability);
                println!(
                    "  {name:<16} no_speech={no_speech_probability:.3}  \
                     verdict={verdict:?}  {text:?}"
                );
                if verdict.is_none() {
                    failures.push(format!("{name}: kept {text:?}"));
                }
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    worker.shutdown();
    assert!(
        failures.is_empty(),
        "non-speech reached the chat:\n  {}",
        failures.join("\n  ")
    );
}

/// Deterministic white noise. A hand-rolled generator rather than a
/// dependency, and seeded so a failure is reproducible.
fn noise(seconds: f32, amplitude: f32) -> Vec<f32> {
    let mut state: u32 = 0x2545_f491;
    (0..(SAMPLE_RATE as f32 * seconds) as usize)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32 * 2.0 - 1.0) * amplitude
        })
        .collect()
}

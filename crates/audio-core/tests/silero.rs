//! What the energy detector could not do (P3).
//!
//! Needs the Silero model and a speech fixture, hence `#[ignore]` — CI
//! downloads neither. Locally:
//!
//! ```sh
//! just vad-model
//! just fixture de_mixed "Achtung Regie, Standby für Cue fünf. Dann fade out."
//! cargo test -p audio-core --test silero -- --ignored --nocapture
//! ```
//!
//! Both files are found in `.scratch` by default;
//! `INTERCOM2CHAT_VAD_MODEL` and `VAD_TEST_FIXTURE` override them.

#![cfg(feature = "silero")]

use audio_core::{
    EnergyVad, Segment, Segmenter, SegmenterConfig, SileroVad, StreamId, Vad, SAMPLE_RATE,
};
use std::f32::consts::TAU;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

fn model_path() -> PathBuf {
    if let Ok(path) = std::env::var("INTERCOM2CHAT_VAD_MODEL") {
        return PathBuf::from(path);
    }
    let dir = workspace_root().join(".scratch/models");
    let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e} — run `just vad-model`", dir.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_str()?.to_ascii_lowercase();
            (name.contains("silero") && name.ends_with(".bin")).then_some(path)
        })
        .collect();
    found.sort();
    found.pop().unwrap_or_else(|| {
        panic!(
            "no Silero model in {} — run `just vad-model`",
            dir.display()
        )
    })
}

fn vad() -> SileroVad {
    SileroVad::new(&model_path()).expect("loading the Silero model")
}

/// The spoken fixture, at 16 kHz mono.
fn speech() -> Vec<f32> {
    let path = std::env::var("VAD_TEST_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root().join(".scratch/fixtures/de_mixed.wav"));
    let mut reader = hound::WavReader::open(&path).unwrap_or_else(|e| {
        panic!(
            "opening the fixture {}: {e} — run `just fixture …`",
            path.display()
        )
    });
    assert_eq!(
        reader.spec().sample_rate,
        SAMPLE_RATE,
        "fixture must be 16 kHz"
    );
    reader
        .samples::<i16>()
        .map(|s| s.expect("sample") as f32 / 32768.0)
        .collect()
}

fn silence(seconds: f32) -> Vec<f32> {
    vec![0.0; (SAMPLE_RATE as f32 * seconds) as usize]
}

/// A loud tone — the case that defeats an energy detector.
fn tone(seconds: f32, amplitude: f32, hz: f32) -> Vec<f32> {
    let n = (SAMPLE_RATE as f32 * seconds) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            amplitude * (TAU * hz * t).sin()
        })
        .collect()
}

/// Loud broadband noise — a chair scrape, a door, a burst of applause.
fn noise(seconds: f32, amplitude: f32) -> Vec<f32> {
    let n = (SAMPLE_RATE as f32 * seconds) as usize;
    // Deterministic, so a failing run can be repeated exactly.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            amplitude * ((state >> 40) as f32 / 8388608.0 - 1.0)
        })
        .collect()
}

/// Share of frames the detector called speech.
fn speech_ratio(vad: &mut dyn Vad, signal: &[f32]) -> f32 {
    let frame = vad.frame_samples();
    let frames: Vec<&[f32]> = signal.chunks_exact(frame).collect();
    let speech = frames.iter().filter(|f| vad.is_speech(f)).count();
    speech as f32 / frames.len() as f32
}

fn segments(vad: Box<dyn Vad>, signal: &[f32]) -> Vec<Segment> {
    let mut segmenter = Segmenter::new(StreamId(0), vad, SegmenterConfig::default());
    let mut out = Vec::new();
    // Realistic block sizes: the processing thread does not get to choose.
    for block in signal.chunks(1600) {
        out.extend(segmenter.push(block));
    }
    out.extend(segmenter.finish());
    out
}

#[test]
#[ignore = "needs the Silero model, see module docs"]
fn the_frame_size_comes_from_the_model() {
    let vad = vad();
    // Silero at 16 kHz uses 512 samples. The point of the assertion is not the
    // number but that it was read from the model rather than assumed: the
    // whole scheme rests on one probability meaning exactly one frame.
    assert_eq!(vad.window(), vad.frame_samples());
    assert!(
        (64..=4096).contains(&vad.window()),
        "implausible window: {}",
        vad.window()
    );
    println!("window: {} samples", vad.window());
}

#[test]
#[ignore = "needs the Silero model, see module docs"]
fn silence_is_not_speech() {
    let mut vad = vad();
    assert_eq!(speech_ratio(&mut vad, &silence(3.0)), 0.0);
    assert_eq!(vad.failures(), 0, "the model refused a frame");
}

#[test]
#[ignore = "needs the Silero model, see module docs"]
fn loud_non_speech_is_not_speech() {
    // The whole reason for P3. At 0.3 amplitude the energy detector calls both
    // of these speech without hesitation; loudness was never the question.
    for (name, signal) in [
        ("tone", tone(3.0, 0.3, 440.0)),
        ("noise", noise(3.0, 0.3)),
        ("quiet room tone", noise(3.0, 0.005)),
    ] {
        let mut silero = vad();
        let silero_ratio = speech_ratio(&mut silero, &signal);
        let mut energy = EnergyVad::new(SAMPLE_RATE);
        let energy_ratio = speech_ratio(&mut energy, &signal);
        println!("{name}: silero {silero_ratio:.2}, energy {energy_ratio:.2}");
        assert!(
            silero_ratio < 0.1,
            "{name} read as speech in {:.0} % of frames",
            silero_ratio * 100.0
        );
    }
}

#[test]
#[ignore = "needs the Silero model and a fixture, see module docs"]
fn speech_is_speech() {
    let mut vad = vad();
    let ratio = speech_ratio(&mut vad, &speech());
    println!("speech fixture: {ratio:.2} of frames");
    // The fixture is a spoken sentence with its natural pauses, so nowhere
    // near 1.0 — but unmistakably more than the noise cases above.
    assert!(
        ratio > 0.4,
        "only {:.0} % of frames read as speech",
        ratio * 100.0
    );
}

#[test]
#[ignore = "needs the Silero model and a fixture, see module docs"]
fn the_segmenter_cuts_speech_and_ignores_noise() {
    // The acceptance criterion of P3, at the level that matters: what the pool
    // is asked to decode.
    let mut signal = silence(0.5);
    signal.extend(noise(2.0, 0.3));
    signal.extend(silence(1.0));
    signal.extend(tone(2.0, 0.3, 440.0));
    signal.extend(silence(1.0));
    let before = signal.len();
    signal.extend(speech());
    signal.extend(silence(1.0));

    let cut = segments(Box::new(vad()), &signal);
    println!(
        "segments: {:?}",
        cut.iter()
            .map(|s| (s.t_start.as_millis(), s.t_end.as_millis()))
            .collect::<Vec<_>>()
    );

    assert!(!cut.is_empty(), "the spoken passage produced no segment");
    let speech_starts = Duration::from_secs_f64(before as f64 / SAMPLE_RATE as f64);
    for segment in &cut {
        assert!(
            // The pre-roll reaches back before the onset, hence the tolerance.
            segment.t_end > speech_starts,
            "a segment at {:?} was cut out of noise, not speech",
            segment.t_start
        );
    }
}

#[test]
#[ignore = "needs the Silero model, see module docs"]
fn runs_far_faster_than_realtime() {
    // Eight streams share one machine with the decoders, so the cost of the
    // rolling context is the thing to keep an eye on.
    let mut vad = vad();
    let signal = noise(10.0, 0.05);
    let t0 = Instant::now();
    speech_ratio(&mut vad, &signal);
    let elapsed = t0.elapsed();
    let factor = 10.0 / elapsed.as_secs_f32();
    println!(
        "10 s in {:?} — {factor:.0}× realtime, {:.3} ms per frame",
        elapsed,
        elapsed.as_secs_f32() * 1000.0 / (10.0 * SAMPLE_RATE as f32 / vad.window() as f32)
    );
    assert!(
        factor > 20.0,
        "only {factor:.0}× realtime — eight streams would not fit"
    );
}

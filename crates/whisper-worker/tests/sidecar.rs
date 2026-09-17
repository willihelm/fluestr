//! End-to-end test driving the sidecar as a real subprocess.
//!
//! Needs a model and a fixture, hence `#[ignore]` — CI does not download
//! 200 MB models. Locally:
//!
//! ```sh
//! just model small-q5_1
//! just fixture de_mixed "Achtung Regie, Standby für Cue fünf. Dann fade out."
//! WHISPER_TEST_MODEL=.scratch/models/ggml-small-q5_1.bin \
//! WHISPER_TEST_FIXTURE=.scratch/fixtures/de_mixed.wav \
//!   cargo test -p whisper-worker --test sidecar -- --ignored --nocapture
//! ```

use std::io::{BufReader, BufWriter};
use std::process::{Child, Command, Stdio};
use whisper_protocol::{read_response, write_request, Request, Response};

struct Sidecar {
    child: Child,
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Resolves a path from an environment variable. `cargo test` sets the working
/// directory to the crate folder rather than the workspace root, so relative
/// values are resolved against the root — that way they can be written exactly
/// as they appear in the repository.
fn resolve(var: &str) -> std::path::PathBuf {
    let raw = std::env::var(var).unwrap_or_else(|_| panic!("{var} is not set"));
    let path = std::path::PathBuf::from(&raw);
    if path.is_absolute() {
        return path;
    }
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    workspace_root.join(path)
}

fn model_path() -> String {
    resolve("WHISPER_TEST_MODEL").to_string_lossy().into_owned()
}

fn fixture_pcm() -> Vec<f32> {
    let path = resolve("WHISPER_TEST_FIXTURE");
    let mut reader = hound::WavReader::open(&path)
        .unwrap_or_else(|e| panic!("opening fixture {}: {e}", path.display()));
    assert_eq!(reader.spec().sample_rate, 16_000, "fixture must be 16 kHz");
    reader
        .samples::<i16>()
        .map(|s| s.expect("sample") as f32 / 32768.0)
        .collect()
}

fn spawn() -> (
    Sidecar,
    BufWriter<std::process::ChildStdin>,
    BufReader<std::process::ChildStdout>,
) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_whisper-worker"))
        .args(["--model", &model_path(), "--threads", "4"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawning sidecar");

    let stdin = BufWriter::new(child.stdin.take().unwrap());
    let stdout = BufReader::new(child.stdout.take().unwrap());
    (Sidecar { child }, stdin, stdout)
}

#[test]
#[ignore = "needs a whisper model, see module docs"]
fn sidecar_reports_ready_and_serves_the_full_language_list() {
    let (_guard, mut stdin, mut stdout) = spawn();

    match read_response(&mut stdout).unwrap().unwrap() {
        Response::Ready { load_ms, gpu, .. } => {
            println!("ready after {load_ms} ms, gpu={gpu}");
            assert!(gpu, "Metal should be active");
        }
        other => panic!("expected Ready, got {other:?}"),
    }

    write_request(&mut stdin, &Request::ListLanguages, &[]).unwrap();
    match read_response(&mut stdout).unwrap().unwrap() {
        Response::Languages { languages } => {
            assert!(languages.len() > 50, "only {} languages", languages.len());
            // The extensibility point: this list comes from whisper.cpp, not
            // from our code.
            for code in ["de", "en", "fr", "ja"] {
                assert!(
                    languages.iter().any(|l| l.code == code),
                    "{code} missing from the language list"
                );
            }
        }
        other => panic!("expected Languages, got {other:?}"),
    }
}

#[test]
#[ignore = "needs a model and a fixture, see module docs"]
fn vocabulary_prompt_prevents_germanising_english_technical_terms() {
    let pcm = fixture_pcm();
    let (_guard, mut stdin, mut stdout) = spawn();

    match read_response(&mut stdout).unwrap().unwrap() {
        Response::Ready { .. } => {}
        other => panic!("expected Ready, got {other:?}"),
    }

    let transcribe = |stdin: &mut BufWriter<std::process::ChildStdin>,
                      stdout: &mut BufReader<std::process::ChildStdout>,
                      seq: u64,
                      prompt: Option<&str>| {
        write_request(
            stdin,
            &Request::Transcribe {
                stream_id: 0,
                seq,
                language_mode: audio_core::LanguageMode::default(), // fixed German
                initial_prompt: prompt.map(str::to_string),
            },
            &pcm,
        )
        .unwrap();
        match read_response(stdout).unwrap().unwrap() {
            Response::Transcript {
                seq: got_seq,
                text,
                detected_language,
                decode_ms,
                ..
            } => {
                assert_eq!(
                    got_seq, seq,
                    "the reply must be attributable to the request"
                );
                println!("[seq {seq}] {decode_ms} ms, lang={detected_language:?}: {text}");
                text
            }
            other => panic!("expected Transcript, got {other:?}"),
        }
    };

    let without = transcribe(&mut stdin, &mut stdout, 1, None);
    // The prompt terms are the operator's jargon and stay as spoken.
    let with = transcribe(
        &mut stdin,
        &mut stdout,
        2,
        Some("Intercom-Kommandos: Standby, Cue, Go, Fade out, Blackout, Talkback, Regie."),
    );

    assert!(
        !without.is_empty() && !with.is_empty(),
        "text must not be empty"
    );

    // The actual evidence for P15: with a vocabulary prompt the English
    // technical terms appear verbatim inside the German text.
    assert!(
        with.to_lowercase().contains("standby"),
        "the vocabulary prompt should preserve 'Standby', got: {with}"
    );

    // And never translate — the text stays German.
    assert!(
        with.to_lowercase().contains("regie") || with.to_lowercase().contains("kommando"),
        "text should remain German, got: {with}"
    );
}

#[test]
#[ignore = "needs a whisper model, see module docs"]
fn sidecar_survives_a_failing_segment() {
    let (_guard, mut stdin, mut stdout) = spawn();
    match read_response(&mut stdout).unwrap().unwrap() {
        Response::Ready { .. } => {}
        other => panic!("expected Ready, got {other:?}"),
    }

    // Empty PCM: whisper dislikes it, but the worker must not die of it.
    write_request(
        &mut stdin,
        &Request::Transcribe {
            stream_id: 7,
            seq: 1,
            language_mode: audio_core::LanguageMode::default(),
            initial_prompt: None,
        },
        &[],
    )
    .unwrap();
    let first = read_response(&mut stdout).unwrap().unwrap();
    println!("reply to the empty segment: {first:?}");

    // The decisive part: the worker keeps accepting work afterwards.
    write_request(&mut stdin, &Request::ListLanguages, &[]).unwrap();
    match read_response(&mut stdout).unwrap().unwrap() {
        Response::Languages { languages } => assert!(!languages.is_empty()),
        other => panic!("worker stopped responding correctly after an error: {other:?}"),
    }
}

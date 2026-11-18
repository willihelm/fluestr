//! A WAV file played in as the input device, end to end: the file backend,
//! the session's pipelines, the pool and the sidecar, as the app runs them.
//!
//! Ignored by default: it needs a model and a recording, which CI does not
//! have, and it runs in real time — as long as the file.
//!
//! ```sh
//! python3 scripts/intercom-clip.py            # .scratch/fixtures/intercom-de-en.wav
//! cargo build --release -p whisper-worker
//! FLUESTR_WORKER=target/release/whisper-worker \
//! FILE_INPUT=.scratch/fixtures/intercom-de-en.wav \
//! FILE_INPUT_MODEL=.scratch/models/parakeet-tdt-0.6b-v3-int8 \
//!   cargo test -p app --test file_input -- --ignored --nocapture
//! ```
//!
//! `FILE_INPUT_LANGUAGES=de,en` sets the channels' languages.

use app_lib::config::Tuning;
use app_lib::models::{Resolved, Source};
use app_lib::replay::ReplayStore;
use app_lib::session::{
    EventSink, LevelsEvent, OverloadEvent, Session, SessionConfig, SessionState, StreamConfig,
    TranscriptEvent,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Lines(Mutex<Vec<TranscriptEvent>>);

impl EventSink for Lines {
    fn transcript(&self, event: TranscriptEvent) {
        println!(
            "{:>6.1} s  ch {}  [{}] {:?}  ({} ms)",
            event.t_start_ms as f32 / 1000.0,
            event.stream_id + 1,
            event.detected_language.as_deref().unwrap_or("-"),
            event.text,
            event.decode_ms
        );
        self.0.lock().unwrap().push(event);
    }
    fn levels(&self, _: LevelsEvent) {}
    fn overload(&self, event: OverloadEvent) {
        println!("overload: {event:?}");
    }
    fn state(&self, _: SessionState) {}
}

#[test]
#[ignore = "needs a model and a recording, and runs as long as the file"]
fn a_wav_file_is_transcribed_like_a_desk() {
    let file = PathBuf::from(std::env::var("FILE_INPUT").expect("set FILE_INPUT to a WAV file"));
    let model = PathBuf::from(std::env::var("FILE_INPUT_MODEL").expect("set FILE_INPUT_MODEL"));
    let vad = std::env::var_os("FILE_INPUT_VAD")
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(".scratch/models/ggml-silero-v5.1.2.bin")))
        .filter(|p| p.is_file());
    let languages: Vec<String> = std::env::var("FILE_INPUT_LANGUAGES")
        .unwrap_or_else(|_| "de,en".into())
        .split(',')
        .map(str::to_string)
        .collect();

    let id = audio_core::file_backend::id_for(&std::fs::canonicalize(&file).unwrap());
    let info = app_lib::session::probe_audio_file(&file.to_string_lossy()).unwrap();
    let seconds =
        hound::WavReader::open(&file).unwrap().duration() as f32 / info.default_sample_rate as f32;

    let streams = (0..info.channels)
        .map(|channel| StreamConfig {
            channel,
            label: format!("ch {}", channel + 1),
            language: languages
                .get(channel as usize)
                .or(languages.last())
                .cloned()
                .unwrap(),
            gain_db: 0,
            muted: false,
            preview: false,
            id: None,
        })
        .collect();
    let tuning = Tuning {
        live_preview: false,
        ..Tuning::default()
    };

    let lines = Arc::new(Lines::default());
    let (session, _) = Session::start(
        SessionConfig {
            device_id: id.0,
            streams,
            vocabulary: None,
            tuning,
        },
        lines.clone(),
        Some(Resolved {
            speech: model,
            vad,
            source: Source::Environment,
        }),
        Some(2),
        Arc::new(ReplayStore::default()),
        Arc::new(app_lib::solo::Bus::default()),
    )
    .expect("starting the session on the file");

    std::thread::sleep(Duration::from_secs_f32(seconds + 2.0));
    session.stop();

    let lines = lines.0.lock().unwrap();
    assert!(!lines.is_empty(), "nothing was transcribed from the file");
}

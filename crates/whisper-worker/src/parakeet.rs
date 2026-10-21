//! The second engine: NVIDIA Parakeet TDT 0.6B v3, run through sherpa-onnx.
//!
//! Selected by handing `--model` a *directory* rather than a `.bin` — the
//! export is four files (encoder, decoder, joiner, tokens) and has no single
//! file to point at. See [`is_model_dir`].
//!
//! What it does not have, and what the rest of the app therefore cannot rely
//! on when it runs:
//!
//! - **No prompt.** A transducer has no `initial_prompt`; the vocabulary (P15)
//!   and the carried sentence (P25) are dropped here, not approximated. The
//!   Settings page says so beside the vocabulary.
//! - **No fixed language.** v3 decides the language itself from the audio, so
//!   a stream's language setting has no effect on it.
//! - **No `no_speech` probability.** Reported as 0; the Silero gate in front
//!   of it is what keeps silence out.

use anyhow::{Context, Result};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use std::path::Path;
use std::time::Instant;

/// The files the export consists of, as they are named in the directory.
const ENCODER: &str = "encoder.int8.onnx";
const DECODER: &str = "decoder.int8.onnx";
const JOINER: &str = "joiner.int8.onnx";
const TOKENS: &str = "tokens.txt";

/// The shortest segment that is decoded: 100 ms at 16 kHz. Below a single
/// feature frame onnxruntime throws a C++ exception on the empty input, which
/// Rust cannot catch and which aborts the process — and nothing that short
/// holds a word anyway.
const MIN_SAMPLES: usize = 1_600;

/// Whether `path` is a Parakeet export rather than a whisper model.
pub fn is_model_dir(path: &Path) -> bool {
    path.is_dir() && path.join(TOKENS).is_file()
}

pub struct Parakeet {
    recognizer: OfflineRecognizer,
}

impl Parakeet {
    pub fn load(dir: &Path, threads: usize) -> Result<Self> {
        let file = |name: &str| -> Result<Option<String>> {
            let path = dir.join(name);
            anyhow::ensure!(path.is_file(), "{} is missing", path.display());
            Ok(Some(path.to_string_lossy().into_owned()))
        };

        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: file(ENCODER)?,
            decoder: file(DECODER)?,
            joiner: file(JOINER)?,
        };
        config.model_config.tokens = file(TOKENS)?;
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.num_threads = threads as i32;
        // CPU, like the VAD: Metal is whisper's, and onnxruntime's CoreML
        // provider compiles the graph on every start, which is a long wait for
        // a sidecar the pool respawns after a crash.
        config.model_config.provider = Some("cpu".into());
        config.decoding_method = Some("greedy_search".into());

        let recognizer = OfflineRecognizer::create(&config)
            .with_context(|| format!("loading the Parakeet model in {}", dir.display()))?;
        Ok(Self { recognizer })
    }

    /// Decodes one segment of 16 kHz mono. Returns the text and the time taken.
    pub fn transcribe(&self, pcm: &[f32]) -> (String, u32) {
        let t0 = Instant::now();
        if pcm.len() < MIN_SAMPLES {
            return (String::new(), 0);
        }
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(16_000, pcm);
        self.recognizer.decode(&stream);
        let text = stream
            .get_result()
            .map(|r| r.text.trim().to_string())
            .unwrap_or_default();
        (text, t0.elapsed().as_millis() as u32)
    }
}

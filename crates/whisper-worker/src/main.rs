//! Whisper sidecar.
//!
//! Loads the model **once** and keeps it warm. Spawning a process per segment
//! would be incompatible with the 1-2 s latency target: the first-ever Metal
//! shader compilation alone was measured at ~8 s (cached system-wide
//! afterwards, ~0.1 s).
//!
//! The process is deliberately separate from the main one: if whisper.cpp
//! crashes, the pool (P5) respawns the worker and the app keeps running.

use anyhow::{Context, Result};
use audio_core::LanguageMode;
use std::io::{BufReader, BufWriter};
use std::time::Instant;
use whisper_protocol as protocol;
use whisper_protocol::{Language, Request, Response};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

struct Args {
    model: String,
    threads: usize,
    gpu: bool,
}

fn parse_args() -> Result<Args> {
    let mut model = None;
    let mut threads = 4usize;
    let mut gpu = true;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--model" => model = it.next(),
            "--threads" => {
                threads = it
                    .next()
                    .context("--threads without a value")?
                    .parse()
                    .context("--threads is not a number")?
            }
            "--no-gpu" => gpu = false,
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    Ok(Args {
        model: model.context("--model <path> is required")?,
        threads: threads.max(1),
        gpu,
    })
}

fn main() -> Result<()> {
    // Logs go to stderr — stdout belongs exclusively to the protocol.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    // Otherwise whisper.cpp and ggml write on their own. Anything of theirs
    // that ends up on stdout would corrupt the length-prefixed frame protocol;
    // these hooks route it into `tracing` and thus to stderr.
    whisper_rs::install_logging_hooks();

    let args = parse_args()?;

    let t0 = Instant::now();
    let mut ctx_params = WhisperContextParameters::default();
    ctx_params.use_gpu(args.gpu);
    let ctx = WhisperContext::new_with_params(&args.model, ctx_params)
        .with_context(|| format!("loading model: {}", args.model))?;
    let load_ms = t0.elapsed().as_millis() as u32;
    tracing::info!(model = %args.model, load_ms, gpu = args.gpu, "model loaded");

    let mut state = ctx.create_state().context("creating whisper state")?;

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());

    protocol::write_response(
        &mut writer,
        &Response::Ready {
            model: args.model.clone(),
            load_ms,
            gpu: args.gpu,
        },
    )?;

    while let Some((request, pcm)) = protocol::read_request(&mut reader)? {
        match request {
            Request::Shutdown => {
                tracing::info!("shutdown requested");
                break;
            }

            Request::ListLanguages => {
                protocol::write_response(
                    &mut writer,
                    &Response::Languages {
                        languages: list_languages(),
                    },
                )?;
            }

            Request::Transcribe {
                stream_id,
                seq,
                language_mode,
                initial_prompt,
            } => {
                let response = match transcribe(
                    &mut state,
                    &pcm,
                    &language_mode,
                    initial_prompt.as_deref(),
                    args.threads,
                ) {
                    Ok(decoded) => Response::Transcript {
                        stream_id,
                        seq,
                        text: decoded.text,
                        detected_language: decoded.detected_language,
                        language_probabilities: None, // P3b
                        no_speech_probability: decoded.no_speech_probability,
                        decode_ms: decoded.decode_ms,
                    },
                    // A failed segment must not take the worker down — the app
                    // needs the attribution in order to drop it.
                    Err(err) => {
                        tracing::warn!(stream_id, seq, error = %err, "segment failed");
                        Response::Error {
                            stream_id: Some(stream_id),
                            seq: Some(seq),
                            message: err.to_string(),
                        }
                    }
                };
                protocol::write_response(&mut writer, &response)?;
            }
        }
    }

    Ok(())
}

/// The full language list from whisper.cpp. Nothing here is limited to de/en.
fn list_languages() -> Vec<Language> {
    (0..=whisper_rs::get_lang_max_id())
        .filter_map(|id| {
            let code = whisper_rs::get_lang_str(id)?;
            let name = whisper_rs::get_lang_str_full(id).unwrap_or(code);
            Some(Language {
                code: code.to_string(),
                name: name.to_string(),
            })
        })
        .collect()
}

fn transcribe(
    state: &mut WhisperState,
    pcm: &[f32],
    language_mode: &LanguageMode,
    initial_prompt: Option<&str>,
    threads: usize,
) -> Result<Decoded> {
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(threads as i32);

    match language_mode.whisper_language() {
        Some(lang) => params.set_language(Some(lang)),
        None => {
            // Auto mode: whisper detects on its own. Restricting to the
            // candidate languages and applying hysteresis is the stabiliser's
            // job (P3b) on the app side.
            params.set_language(None);
            params.set_detect_language(true);
        }
    }

    // Never translate: silent translation is the most dangerous failure mode
    // because the output looks plausible.
    params.set_translate(false);

    // No sound-event annotations. An intercom channel is silent or carries
    // only room noise for long stretches, and whisper's habit of captioning
    // that as "[BREAK]", "(music)" or "(Bell)" fills the chat with entries
    // that are not communication. The operator wants what was *said*.
    params.set_suppress_nst(true);
    // Nothing at all rather than an invented blank line.
    params.set_suppress_blank(true);

    // One stream's context must not contaminate another — the state is shared
    // per worker, the streams are not.
    params.set_no_context(true);
    params.set_single_segment(true);

    // Latency over robustness: no temperature fallback, no retries.
    params.set_temperature(0.0);
    params.set_temperature_inc(0.0);

    // Decoder chatter does not belong on stdout.
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    if let Some(prompt) = initial_prompt.filter(|p| !p.trim().is_empty()) {
        params.set_initial_prompt(prompt);
    }

    let t0 = Instant::now();
    state.full(params, pcm).context("transcription")?;
    let decode_ms = t0.elapsed().as_millis() as u32;

    let mut text = String::new();
    // Highest across the segments rather than an average: with
    // `single_segment` there is normally just one, and if there ever is more
    // than one, the most doubtful of them is what should be reported.
    let mut no_speech: f32 = 0.0;
    for i in 0..state.full_n_segments() {
        if let Some(segment) = state.get_segment(i) {
            if let Ok(s) = segment.to_str_lossy() {
                text.push_str(&s);
            }
            no_speech = no_speech.max(segment.no_speech_probability());
        }
    }

    let detected = whisper_rs::get_lang_str(state.full_lang_id_from_state()).map(str::to_string);

    Ok(Decoded {
        text: text.trim().to_string(),
        detected_language: detected,
        no_speech_probability: no_speech,
        decode_ms,
    })
}

/// What one decode produced.
struct Decoded {
    text: String,
    detected_language: Option<String>,
    no_speech_probability: f32,
    decode_ms: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_list_is_not_empty_and_contains_de_and_en() {
        let langs = list_languages();
        assert!(
            langs.len() > 50,
            "whisper supports ~99 languages, found: {}",
            langs.len()
        );
        assert!(langs.iter().any(|l| l.code == "de"));
        assert!(langs.iter().any(|l| l.code == "en"));
        // The extensibility point: everything else is there too.
        assert!(langs.iter().any(|l| l.code == "fr"));
    }

    #[test]
    fn language_list_has_spelled_out_names() {
        let langs = list_languages();
        let de = langs.iter().find(|l| l.code == "de").unwrap();
        assert!(!de.name.is_empty());
        assert_ne!(de.name, de.code, "name should be spelled out");
    }
}

//! Whisper sidecar.
//!
//! Loads the model **once** and keeps it warm. Spawning a process per segment
//! would be incompatible with the 1-2 s latency target: the first-ever Metal
//! shader compilation alone was measured at ~8 s (cached system-wide
//! afterwards, ~0.1 s).
//!
//! The process is deliberately separate from the main one: if whisper.cpp
//! crashes, the pool (P5) respawns the worker and the app keeps running.
//!
//! A `--model` that is a directory runs Parakeet instead of whisper — see
//! [`parakeet`]. The protocol, the gate and the pool are the same for both.

use anyhow::{Context, Result};
use audio_core::{silero, LanguageMode};
use std::io::{BufReader, BufWriter};
use std::time::Instant;
use whisper_protocol as protocol;
use whisper_protocol::{Language, Request, Response};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
    WhisperVadContext, WhisperVadContextParams,
};

mod parakeet;
#[cfg(all(windows, target_env = "msvc"))]
mod vulkan_guard;

/// The speech model the worker keeps warm.
enum Engine {
    Whisper(WhisperState),
    Parakeet(parakeet::Parakeet),
}

/// What the process was asked to do.
///
/// `--languages` is not a request on the protocol even though `ListLanguages`
/// exists there: the list is a table compiled into whisper.cpp, and asking for
/// it through the pipe would mean loading a model first. The app needs it
/// while the operator is still setting up channels, on a machine that may not
/// have downloaded a model yet.
enum Mode {
    Serve(Args),
    /// Print the language list as JSON on stdout and exit.
    Languages,
}

struct Args {
    model: String,
    /// Silero VAD model. Optional, but without it whisper transcribes whatever
    /// it is handed — see [`transcribe`].
    vad_model: Option<String>,
    threads: usize,
    gpu: bool,
}

fn parse_args() -> Result<Mode> {
    let mut model = None;
    let mut vad_model = None;
    let mut threads = 4usize;
    let mut gpu = true;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--model" => model = it.next(),
            "--vad-model" => vad_model = it.next(),
            "--threads" => {
                threads = it
                    .next()
                    .context("--threads without a value")?
                    .parse()
                    .context("--threads is not a number")?
            }
            "--no-gpu" => gpu = false,
            // Answered before anything else is validated, because it needs
            // none of it — in particular not `--model`.
            "--languages" => return Ok(Mode::Languages),
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    Ok(Mode::Serve(Args {
        model: model.context("--model <path> is required")?,
        vad_model,
        threads: threads.max(1),
        gpu,
    }))
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

    let args = match parse_args()? {
        Mode::Serve(args) => args,
        // Straight to stdout as plain JSON rather than as a protocol frame:
        // the caller here is a one-shot `Command`, not a worker holding a
        // pipe, and it has no reason to speak the framing to read one answer.
        Mode::Languages => {
            serde_json::to_writer(std::io::stdout(), &list_languages())?;
            return Ok(());
        }
    };

    let t0 = Instant::now();
    // Kept alive for the whole loop: whisper.cpp frees the weights with the
    // context, and the state only works while they are there.
    let mut _ctx = None;
    let mut engine = if parakeet::is_model_dir(std::path::Path::new(&args.model)) {
        Engine::Parakeet(parakeet::Parakeet::load(
            std::path::Path::new(&args.model),
            args.threads,
        )?)
    } else {
        let mut ctx_params = WhisperContextParameters::default();
        ctx_params.use_gpu(args.gpu);
        let ctx = WhisperContext::new_with_params(&args.model, ctx_params)
            .with_context(|| format!("loading model: {}", args.model))?;
        let state = ctx.create_state().context("creating whisper state")?;
        _ctx = Some(ctx);
        Engine::Whisper(state)
    };
    let load_ms = t0.elapsed().as_millis() as u32;
    #[cfg(all(windows, target_env = "msvc"))]
    vulkan_guard::report();
    // Whether the VAD is armed decides what reaches the chat on a quiet
    // channel, so it belongs in the log rather than being inferred.
    // Parakeet runs on the CPU whatever was asked for.
    let gpu = args.gpu && matches!(engine, Engine::Whisper(_));
    tracing::info!(
        model = %args.model,
        load_ms,
        gpu,
        vad = args.vad_model.as_deref().unwrap_or("none"),
        "model loaded"
    );
    // The speech gate. Loaded once and kept warm like the speech model — it is
    // tiny (under a megabyte), so this costs nothing worth measuring.
    let mut vad = match args.vad_model.as_deref() {
        Some(path) => {
            let mut vad_ctx_params = WhisperVadContextParams::default();
            vad_ctx_params.set_n_threads(args.threads as i32);
            // CPU, deliberately. On GPU, ggml aborts the process outright —
            // it reports "no GPU found" for the VAD backend while the model is
            // already allocated in a Metal buffer, and then trips its own
            // assertion. Beyond that a second Metal user would contend with
            // whisper for the same queue, which is the reason the worker pool
            // is kept small in the first place. The model is under a megabyte;
            // the CPU handles it in single-digit milliseconds.
            vad_ctx_params.set_use_gpu(false);
            Some(
                WhisperVadContext::new(path, vad_ctx_params)
                    .map_err(|e| anyhow::anyhow!("loading the VAD model {path}: {e:?}"))?,
            )
        }
        None => {
            tracing::warn!(
                "no VAD model: whisper will transcribe noise and silence as well — \
                 pass --vad-model, or run `just vad-model`"
            );
            None
        }
    };

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());

    protocol::write_response(
        &mut writer,
        &Response::Ready {
            model: args.model.clone(),
            load_ms,
            gpu,
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
                    &mut engine,
                    &pcm,
                    &language_mode,
                    initial_prompt.as_deref(),
                    args.threads,
                    vad.as_mut(),
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
    engine: &mut Engine,
    pcm: &[f32],
    language_mode: &LanguageMode,
    initial_prompt: Option<&str>,
    threads: usize,
    vad: Option<&mut WhisperVadContext>,
) -> Result<Decoded> {
    // The gate, before any decoding happens.
    //
    // Since P3 the same model also runs in front of the segmenter, so most of
    // what arrives here has already been judged to be speech. This layer stays
    // because it is the only one that sees the segment as a whole, and because
    // the app falls back to an energy detector when the VAD model is missing.
    //
    // Note this is *not* whisper's `params.vad` flag. That one is only honoured
    // by `whisper_full`; the sidecar uses `whisper_full_with_state`, which
    // skips it entirely — setting the flag there looks like protection and
    // provides none. Running Silero directly is both explicit and cheaper: a
    // stretch of noise costs a few milliseconds of VAD instead of a few
    // hundred of transcription.
    if let Some(vad) = vad {
        let t0 = Instant::now();
        let segments = vad
            .segments_from_samples(silero::segment_params(), pcm)
            .map_err(|e| anyhow::anyhow!("VAD: {e:?}"))?;
        let vad_ms = t0.elapsed().as_millis() as u32;

        if segments.num_segments() == 0 {
            tracing::debug!(vad_ms, samples = pcm.len(), "no speech, not decoded");
            return Ok(Decoded {
                text: String::new(),
                detected_language: None,
                no_speech_probability: 1.0,
                decode_ms: vad_ms,
            });
        }
    }

    let state = match engine {
        Engine::Whisper(state) => state,
        // No prompt and no language to pass — see the module.
        Engine::Parakeet(parakeet) => {
            let (text, decode_ms) = parakeet.transcribe(pcm);
            return Ok(Decoded {
                text,
                detected_language: None,
                no_speech_probability: 0.0,
                decode_ms,
            });
        }
    };

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

    // Nothing at all rather than an invented blank line.
    params.set_suppress_blank(true);

    // Sound-event annotations are deliberately *left on*.
    //
    // Suppressing them was tried and made things worse. Whisper still has to
    // emit something for a stretch of noise; forbidding "[BREAK]" only pushes
    // it towards plausible words instead. An annotation is a non-speech marker
    // the app can filter with certainty — a hallucinated sentence is not. The
    // suppression removed the label, not the problem.
    params.set_suppress_nst(false);

    // One stream's context must not contaminate another — the state is shared
    // per worker, the streams are not.
    params.set_no_context(true);
    params.set_single_segment(true);

    // Latency over robustness: no temperature fallback, no retries.
    params.set_temperature(0.0);
    params.set_temperature_inc(0.0);

    // The price of that decision is whisper's repetition loop: greedy sampling
    // with nothing to fall back to sometimes cycles on a phrase until the
    // token budget runs out. It is cut from the text on the app side
    // (`audio_core::collapse_repetitions`), and it is *not* bounded here.
    //
    // A per-segment token cap was tried and had to be taken out again: it did
    // not truncate where it was set. When whisper stops on `max_tokens` it
    // keeps only the tokens up to the last closing timestamp it emitted and
    // throws the rest away (`whisper_full_with_state`, the `result_len`
    // resize), so a cap that lands mid-sentence deletes that whole sentence,
    // and one that lands before the first timestamp fails the decode and
    // deletes the segment. Both look exactly like speech that was never
    // spoken — the silent damage this project does not accept — while the
    // loop itself is only ever a latency cost, bounded anyway by whisper's own
    // ceiling of half the text context.

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

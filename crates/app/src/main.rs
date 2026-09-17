// For the later Windows port (P1): no console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            // whisper.cpp logs three info lines per VAD call, and the speech
            // detector in front of the segmenter runs fifty times a second per
            // stream. `RUST_LOG` overrides this when that is what is wanted.
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,whisper_rs=warn".into()),
        )
        .init();

    app_lib::run();
}

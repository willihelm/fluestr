# Intercom2Chat — development tasks
#
# Install `just` with: brew install just

set shell := ["bash", "-uc"]

ui_dir := "ui"
scratch := justfile_directory() / ".scratch"

# The Tauri CLI looks for tauri.conf.json below the working directory. The
# config lives in crates/app and the frontend in ui, so it has to run from the
# repository root rather than from ui.
tauri := justfile_directory() / "ui/node_modules/.bin/tauri"

default:
    @just --list

# --- Development ---------------------------------------------------------

# Run the app in development mode (Vite + Tauri, hot reload)
dev:
    {{tauri}} dev

# Frontend only, without the Tauri window
dev-ui:
    cd {{ui_dir}} && pnpm dev

# --- Dependencies --------------------------------------------------------

setup:
    cd {{ui_dir}} && pnpm install

# --- Checks --------------------------------------------------------------

# Everything CI runs as well
check: fmt-check lint test check-ui

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

check-ui:
    cd {{ui_dir}} && pnpm check

# --- Build ---------------------------------------------------------------

build:
    {{tauri}} build

# --- Whisper tooling (P4/P14) --------------------------------------------

# Download a model into .scratch/models. Example: just model small-q5_1
model name="small-q5_1":
    mkdir -p {{scratch}}/models
    curl -fL --progress-bar \
        -o {{scratch}}/models/ggml-{{name}}.bin \
        https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{{name}}.bin
    @ls -lh {{scratch}}/models/ggml-{{name}}.bin

# Generate a code-switching test fixture via macOS text-to-speech (P14).
# The spoken text is German on purpose — it is speech data, not code.
fixture name text:
    mkdir -p {{scratch}}/fixtures
    say -v Anna -o {{scratch}}/fixtures/{{name}}.aiff {{quote(text)}}
    afconvert -f WAVE -d LEI16@16000 -c 1 \
        {{scratch}}/fixtures/{{name}}.aiff {{scratch}}/fixtures/{{name}}.wav
    rm {{scratch}}/fixtures/{{name}}.aiff
    @echo "{{scratch}}/fixtures/{{name}}.wav"

clean:
    cargo clean
    rm -rf {{ui_dir}}/dist {{ui_dir}}/node_modules

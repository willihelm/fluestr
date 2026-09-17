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

# Run the app in development mode (Vite + Tauri, hot reload).
#
# The sidecar is looked up next to the app binary, which is where cargo puts
# it — so it has to be built too. Packaging it as a Tauri `externalBin` is P13.
dev:
    cargo build -p whisper-worker
    {{tauri}} dev

# Frontend only, without the Tauri window
dev-ui:
    cd {{ui_dir}} && pnpm dev

# --- Dependencies --------------------------------------------------------

setup:
    cd {{ui_dir}} && pnpm install

# --- Checks --------------------------------------------------------------

# Everything CI runs as well
check: fmt-check lint test check-ui test-ui

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

# The frontend logic that is not a component: bubble grouping, group
# membership, the vocabulary prompt.
test-ui:
    cd {{ui_dir}} && pnpm test

# --- Build ---------------------------------------------------------------

build:
    {{tauri}} build

# --- Whisper tooling (P4/P14) --------------------------------------------

# The model download, against the real host. Needs a network; fetches the
# 885 KB VAD model several times, which is why it is that one and not `small`.
# These are what check the URLs and the checksums in the catalogue — the two
# things a mocked server would replace and therefore never test.
models:
    cargo test -p app --test models -- --ignored --nocapture

# Download a model into .scratch/models. Example: just model small-q5_1
#
# Superseded by the model manager in the app (P11), which downloads into the
# app's own directory with a checksum and a progress bar. Kept because it is
# what the integration tests and `just slice` find — they have no app
# directory to resolve against.
model name="small-q5_1":
    mkdir -p {{scratch}}/models
    curl -fL --progress-bar \
        -o {{scratch}}/models/ggml-{{name}}.bin \
        https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{{name}}.bin
    @ls -lh {{scratch}}/models/ggml-{{name}}.bin

# Download the Silero VAD model. Without it whisper transcribes noise and
# silence too — it is what decides whether a stretch of audio is speech at all.
vad-model name="v5.1.2":
    mkdir -p {{scratch}}/models
    curl -fL --progress-bar \
        -o {{scratch}}/models/ggml-silero-{{name}}.bin \
        https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-{{name}}.bin
    @ls -lh {{scratch}}/models/ggml-silero-{{name}}.bin

# Generate a code-switching test fixture via macOS text-to-speech (P14).
# The spoken text is German on purpose — it is speech data, not code.
fixture name text:
    mkdir -p {{scratch}}/fixtures
    say -v Anna -o {{scratch}}/fixtures/{{name}}.aiff {{quote(text)}}
    afconvert -f WAVE -d LEI16@16000 -c 1 \
        {{scratch}}/fixtures/{{name}}.aiff {{scratch}}/fixtures/{{name}}.wav
    rm {{scratch}}/fixtures/{{name}}.aiff
    @echo "{{scratch}}/fixtures/{{name}}.wav"

# Run the vertical slice against a WAV file instead of a microphone: fixture ->
# resampling -> segmentation -> sidecar -> transcript. Needs a model.
slice fixture=(scratch / "fixtures/de_mixed.wav") model=(scratch / "models/ggml-small-q5_1.bin"):
    cargo build -p whisper-worker
    INTERCOM2CHAT_WORKER={{justfile_directory()}}/target/debug/whisper-worker \
    INTERCOM2CHAT_MODEL={{model}} \
    SLICE_FIXTURE={{fixture}} \
        cargo test -p app --test slice -- --ignored --nocapture

clean:
    cargo clean
    rm -rf {{ui_dir}}/dist {{ui_dir}}/node_modules

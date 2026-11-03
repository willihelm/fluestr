# Fluestr — development tasks
#
# Install `just` with: brew install just

set shell := ["bash", "-uc"]

ui_dir := "ui"
scratch := justfile_directory() / ".scratch"

# The Tauri CLI looks for tauri.conf.json below the working directory. The
# config lives in crates/app and the frontend in ui, so it has to run from the
# repository root rather than from ui.
tauri := justfile_directory() / "ui/node_modules/.bin/tauri"

# Where Tauri looks for the bundled sidecar (P13). It insists on the target
# triple in the file name and strips it off again when it copies the binary
# into `Contents/MacOS` — which is exactly where `worker::find_binary` looks,
# so nothing in the app has to know whether it is running packaged or not.
triple := "aarch64-apple-darwin"
sidecar := justfile_directory() / "crates/app/binaries/whisper-worker-" + triple

# Who signs the bundle. `-` is an ad-hoc signature: enough for Hardened Runtime
# and for running here, not enough for another Mac — Gatekeeper wants a
# Developer ID, which needs the paid programme. Pass one in once there is one:
#
#     just build "Developer ID Application: Name (TEAMID)"
identity := env("APPLE_SIGNING_IDENTITY", "-")

# The oldest macOS the bundle claims to run on. Read out of the Tauri config
# rather than written down twice: `tauri build` exports it as the deployment
# target for the app, and the sidecar has to be built against the same one. A
# sidecar built against the host's macOS refuses to start on anything older —
# the app would launch on macOS 13 and then fail at its first transcription.
minimum_macos := `sed -n 's/.*"minimumSystemVersion": "\(.*\)".*/\1/p' crates/app/tauri.conf.json`

default:
    @just --list

# --- Development ---------------------------------------------------------

# Run the app in development mode (Vite + Tauri, hot reload).
#
# The sidecar is staged for Tauri rather than only built: `externalBin` is
# checked in development too, and a missing one fails the run before the
# window opens.
dev: (_sidecar "debug")
    {{tauri}} dev

# Frontend only, without the Tauri window
dev-ui:
    cd {{ui_dir}} && pnpm dev

# --- Dependencies --------------------------------------------------------

setup:
    cd {{ui_dir}} && pnpm install

# --- Checks --------------------------------------------------------------

# Everything CI runs as well
check: fmt-check lint test types-check check-ui test-ui

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

# --- Generated types (P6) ------------------------------------------------

# Write the frontend's types from the Rust structs that cross the bridge.
#
# `cargo test` does this anyway — the export is a test, which is how ts-rs
# works — so this exists to be *called by name* when only the types changed.
types:
    cargo test --workspace export_bindings

# The committed bindings are what the current structs produce.
#
# They are committed so the frontend builds without a Rust toolchain, which
# means they can also go stale — a field renamed in Rust and not regenerated
# would leave TypeScript agreeing with a backend that no longer exists. This is
# what stops that reaching a machine.
types-check: types
    git diff --exit-code -- ui/src/lib/generated

check-ui:
    cd {{ui_dir}} && pnpm check

# The frontend logic that is not a component: bubble grouping, group
# membership, the vocabulary prompt.
test-ui:
    cd {{ui_dir}} && pnpm test

# --- Build ---------------------------------------------------------------

# Build, sign and package the app (P13).
#
# `tauri build` signs the bundle *and* the sidecar inside it with one identity
# and the entitlements next to the config. Hardened Runtime is on by default in
# Tauri 2, and it is what makes the microphone entitlement necessary rather
# than decorative.
#
# Extra arguments go to `tauri build`. The release workflow passes
# `--config crates/app/tauri.release.conf.json`, which adds the signed update
# bundle (`Fluestr.app.tar.gz` and its `.sig`) — and needs the updater's
# private key in `TAURI_SIGNING_PRIVATE_KEY`, which is why a plain build here
# leaves it out.
build *args: (_sidecar "release" "--release")
    APPLE_SIGNING_IDENTITY={{quote(identity)}} {{tauri}} build {{args}}

# What the signature actually says, on the bundle and on the sidecar. Read this
# rather than trusting that the build printed no errors: an unsigned sidecar
# inside a signed app is the P13 failure that only shows up on a foreign Mac.
verify app=(justfile_directory() / "target/release/bundle/macos/Fluestr.app"):
    codesign --verify --deep --strict --verbose=2 {{quote(app)}}
    @echo
    codesign --display --entitlements - --verbose=4 {{quote(app)}}
    @echo
    codesign --display --verbose=2 {{quote(app)}}/Contents/MacOS/whisper-worker
    @echo
    spctl --assess --type execute --verbose=4 {{quote(app)}} || \
        echo "^ expected while the identity is ad-hoc: Gatekeeper wants a Developer ID"

# Everything a beta tester needs, in one folder to hand over.
#
# The DMG is signed ad-hoc, so macOS blocks it on their machine and they have
# to let it through by hand — which is why the instructions travel *with* the
# file rather than in the covering message, where they get skimmed and then
# asked about anyway.
beta: build
    #!/usr/bin/env bash
    set -euo pipefail
    out="{{justfile_directory()}}/target/beta"
    rm -rf "$out" && mkdir -p "$out"
    cp {{justfile_directory()}}/target/release/bundle/dmg/*.dmg "$out/"
    cp {{justfile_directory()}}/docs/beta-install.md "$out/Read me first — installing Fluestr.md"
    echo
    echo "Ready to send:"
    ls -1sh "$out"
    echo
    echo "Signed with: {{identity}}"
    if [ "{{identity}}" = "-" ]; then
        echo "  ^ ad-hoc. Testers will have to unblock it in System Settings;"
        echo "    that is what the instructions next to the DMG are for."
    fi

# Copy the sidecar to the name Tauri bundles it under.
_sidecar dir flags="":
    MACOSX_DEPLOYMENT_TARGET={{minimum_macos}} cargo build -p whisper-worker {{flags}}
    mkdir -p {{parent_directory(sidecar)}}
    cp {{justfile_directory()}}/target/{{dir}}/whisper-worker {{sidecar}}

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
    FLUESTR_WORKER={{justfile_directory()}}/target/debug/whisper-worker \
    FLUESTR_MODEL={{model}} \
    SLICE_FIXTURE={{fixture}} \
        cargo test -p app --test slice -- --ignored --nocapture

clean:
    cargo clean
    rm -rf {{ui_dir}}/dist {{ui_dir}}/node_modules

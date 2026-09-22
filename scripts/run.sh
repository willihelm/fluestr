#!/usr/bin/env bash
#
# Build the app and start it.
#
# `just dev` already does this for somebody whose shell is set up for it. This
# script is for every other case: a fresh checkout, a terminal that rustup never
# touched, a double-click. It needs nothing on PATH but bash — it finds the
# toolchain, installs the frontend dependencies if they are missing, builds the
# sidecar and starts the window.
#
# Usage:
#   scripts/run.sh              development mode, hot reload (the default)
#   scripts/run.sh --release    build the bundle and start that instead
#
# The two modes are genuinely different programs. Development mode runs Vite and
# reloads the frontend as it is edited, and it is what you want while working.
# The release bundle is what a machine that is not this one would run: it takes
# minutes to build and has no hot reload, and it is the honest answer to "does
# the shipped thing work".

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

mode="dev"
case "${1:-}" in
"") ;;
--release) mode="release" ;;
-h | --help)
    sed -n '3,20p' "${BASH_SOURCE[0]}" | sed 's|^# \{0,1\}||'
    exit 0
    ;;
*)
    echo "run.sh: unknown argument '$1' — try --release or --help" >&2
    exit 2
    ;;
esac

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

# rustup installs into ~/.cargo/bin and adds it to PATH from the shell profile,
# which a non-interactive shell, an IDE terminal or a launcher never reads. That
# is one `cargo: command not found` for a toolchain that is sitting right there,
# so look before complaining.
if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
    export PATH
fi

missing=""
command -v cargo >/dev/null 2>&1 || missing="$missing rust"
command -v pnpm >/dev/null 2>&1 || missing="$missing pnpm"
command -v cmake >/dev/null 2>&1 || missing="$missing cmake"
if [ -n "$missing" ]; then
    echo "run.sh: missing:$missing" >&2
    echo "  brew install cmake pnpm" >&2
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh" >&2
    exit 1
fi

tauri="$root/ui/node_modules/.bin/tauri"
if [ ! -x "$tauri" ]; then
    step "Installing frontend dependencies"
    (cd ui && pnpm install)
fi

# The sidecar is a separate binary the app looks for next to its own. Packaging
# it into the bundle is P13, so until then it has to be built here — and in
# release mode it has to be *found* here too, see below.
step "Building the transcription sidecar"
if [ "$mode" = "release" ]; then
    cargo build --release -p whisper-worker
    worker="$root/target/release/whisper-worker"
else
    cargo build -p whisper-worker
fi

if [ "$mode" = "dev" ]; then
    # From the repository root, not from ui: the Tauri CLI looks for
    # tauri.conf.json below the working directory and it lives in crates/app.
    step "Starting Fluestr (development)"
    exec "$tauri" dev
fi

step "Building the release bundle — this takes a few minutes"
"$tauri" build

app="$root/target/release/bundle/macos/Fluestr.app"
binary="$app/Contents/MacOS/Fluestr"
if [ ! -x "$binary" ]; then
    echo "run.sh: the build finished but $app is not there" >&2
    exit 1
fi

step "Starting $app"
# Started as a binary rather than with `open`, for two reasons: the log goes to
# this terminal, where a failure to start can be read, and the environment
# reaches the process — which until P13 packages the sidecar is the only way the
# bundled app finds it at all.
exec env FLUESTR_WORKER="$worker" "$binary"

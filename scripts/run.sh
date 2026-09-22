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

# The sidecar is a separate binary the app looks for next to its own. Since P13
# it is a Tauri `externalBin`: Tauri copies it into the bundle and co-signs it,
# but only if it is staged under the name it expects — the target triple is
# part of that name and Tauri strips it again on the way in. Staged in both
# modes, because `tauri dev` checks for it too and fails before the window
# opens if it is not there.
#
# Built against the bundle's own minimum macOS rather than this machine's,
# which is read out of the Tauri config so it is not written down twice. A
# sidecar built against a newer macOS than the app claims refuses to start on
# an older one — the app would launch and fail at its first transcription.
triple="aarch64-apple-darwin"
sidecar="$root/crates/app/binaries/whisper-worker-$triple"
minimum_macos="$(sed -n 's/.*"minimumSystemVersion": "\(.*\)".*/\1/p' crates/app/tauri.conf.json)"

step "Building the transcription sidecar"
if [ "$mode" = "release" ]; then
    MACOSX_DEPLOYMENT_TARGET="$minimum_macos" cargo build --release -p whisper-worker
    built="$root/target/release/whisper-worker"
else
    MACOSX_DEPLOYMENT_TARGET="$minimum_macos" cargo build -p whisper-worker
    built="$root/target/debug/whisper-worker"
fi
mkdir -p "$(dirname "$sidecar")"
cp "$built" "$sidecar"

if [ "$mode" = "dev" ]; then
    # From the repository root, not from ui: the Tauri CLI looks for
    # tauri.conf.json below the working directory and it lives in crates/app.
    step "Starting Fluestr (development)"
    exec "$tauri" dev
fi

# Signed ad-hoc unless an identity is in the environment. That is enough for
# Hardened Runtime and for running here; a Developer ID — which Gatekeeper and
# notarisation want, and which this project does not have yet — is the same
# variable with a real name in it. See P13.
step "Building the release bundle — this takes a few minutes"
APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:--}" "$tauri" build

app="$root/target/release/bundle/macos/Fluestr.app"
binary="$app/Contents/MacOS/fluestr"
if [ ! -x "$binary" ]; then
    echo "run.sh: the build finished but $app is not there" >&2
    exit 1
fi

step "Starting $app"
# Started as a binary rather than with `open` so the log goes to this terminal,
# where a failure to start can be read. No FLUESTR_WORKER: the sidecar is in
# the bundle next to this binary now, which is where the app looks anyway.
exec "$binary"

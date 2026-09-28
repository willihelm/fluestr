# Build Fluestr on Windows (P18) — the counterpart of `just build` / `scripts/run.sh`.
#
# The justfile stays the macOS entry point: it is bash, signs with codesign and
# makes fixtures with `say`. This does the same steps a Windows machine needs,
# and CI runs it so it cannot quietly rot.
#
# Usage (from any directory):
#   scripts\build-windows.ps1           unsigned NSIS installer, release build
#   scripts\build-windows.ps1 -Dev      development mode, hot reload
#
# Needs: Rust (rustup, MSVC toolchain), Visual Studio Build Tools with the
# "Desktop development with C++" workload, CMake, Node 22 and pnpm.

param([switch]$Dev)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Step($text) { Write-Host "`n==> $text" -ForegroundColor Cyan }

# Native commands do not throw on a non-zero exit in Windows PowerShell 5.
function Run {
    & $args[0] @($args | Select-Object -Skip 1)
    if ($LASTEXITCODE -ne 0) { throw "$($args -join ' ') failed with exit code $LASTEXITCODE" }
}

$missing = @()
foreach ($tool in "cargo", "pnpm", "cmake") {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { $missing += $tool }
}
if ($missing) {
    throw "missing: $($missing -join ', ') — install Rust from https://rustup.rs, then: winget install Kitware.CMake; npm install -g pnpm"
}

$tauri = Join-Path $root "ui\node_modules\.bin\tauri.cmd"
if (-not (Test-Path $tauri)) {
    Step "Installing frontend dependencies"
    Push-Location ui
    try { Run pnpm install --frozen-lockfile } finally { Pop-Location }
}

# Tauri bundles the sidecar as an `externalBin` and wants the target triple in
# its file name, which it strips again on the way into the install directory —
# where `worker::find_binary` looks for `whisper-worker.exe`.
$triple = "x86_64-pc-windows-msvc"
$profileDir = if ($Dev) { "debug" } else { "release" }

Step "Building the transcription sidecar"
if ($Dev) { Run cargo build -p whisper-worker } else { Run cargo build --release -p whisper-worker }
New-Item -ItemType Directory -Force -Path "crates\app\binaries" | Out-Null
Copy-Item "target\$profileDir\whisper-worker.exe" "crates\app\binaries\whisper-worker-$triple.exe" -Force

# From the repository root: the Tauri CLI looks for tauri.conf.json below the
# working directory, and it lives in crates\app.
if ($Dev) {
    Step "Starting Fluestr (development)"
    Run $tauri dev
    exit 0
}

Step "Building the installer — this takes a few minutes"
Run $tauri build

$installer = Get-ChildItem "target\release\bundle\nsis\*.exe" | Select-Object -First 1
if (-not $installer) { throw "the build finished but no installer is in target\release\bundle\nsis" }
Step "Installer: $($installer.FullName)"
Write-Host "Unsigned: SmartScreen will warn on first start — More info, then Run anyway."

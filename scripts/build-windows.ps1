# Build Fluestr on Windows (P18) - the counterpart of `just build` / `scripts/run.sh`.
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
# "Desktop development with C++" workload, CMake, LLVM (libclang, for the
# whisper.cpp bindings), the Vulkan SDK (whisper on the GPU), Node 22 and pnpm.
#
# Keep this file ASCII: Windows PowerShell 5.1 reads a BOM-less script as
# Windows-1252, and the last byte of an em dash is a closing quote there.

param([switch]$Dev)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Step($text) { Write-Host "`n==> $text" -ForegroundColor Cyan }

# Native commands do not throw on a non-zero exit in Windows PowerShell 5, and
# when the output is redirected (`> build.log`) it turns every stderr line into
# an error record - which "Stop" makes fatal at cargo's first "Compiling". So
# the exit code alone decides.
function Run {
    $ErrorActionPreference = "Continue"
    & $args[0] @($args | Select-Object -Skip 1)
    if ($LASTEXITCODE -ne 0) { throw "$($args -join ' ') failed with exit code $LASTEXITCODE" }
}

$missing = @()
foreach ($tool in "cargo", "pnpm", "cmake") {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { $missing += $tool }
}
if ($missing) {
    throw "missing: $($missing -join ', ') - install Rust from https://rustup.rs, then: winget install Kitware.CMake; npm install -g pnpm"
}

# whisper-rs-sys generates its bindings with bindgen, which loads libclang at
# build time. The LLVM installer does not put itself on PATH, so point at its
# default location rather than make everyone edit their environment. The
# crate's bundled fallback bindings are not an option: they come from Linux
# glibc headers, where `int64_t` is `long` - which on Windows is 32-bit.
if (-not $env:LIBCLANG_PATH) {
    $llvm = "C:\Program Files\LLVM\bin"
    if (Test-Path "$llvm\libclang.dll") {
        $env:LIBCLANG_PATH = $llvm
    } elseif (-not (Get-Command clang -ErrorAction SilentlyContinue)) {
        throw "missing: libclang - winget install LLVM.LLVM, or set LIBCLANG_PATH to the folder holding libclang.dll"
    }
}

# The sidecar runs whisper on Vulkan (P18 stage 3), and building ggml-vulkan
# needs the SDK: headers, vulkan-1.lib and glslc for the shaders. Its installer
# sets VULKAN_SDK machine-wide, which a shell opened before the install does
# not see yet - so fall back to the newest version under the default folder.
if (-not $env:VULKAN_SDK) {
    $sdk = Get-ChildItem "C:\VulkanSDK" -Directory -ErrorAction SilentlyContinue |
        Where-Object { Test-Path "$($_.FullName)\Bin\glslc.exe" } |
        Sort-Object { [version]$_.Name } | Select-Object -Last 1
    if ($sdk) {
        $env:VULKAN_SDK = $sdk.FullName
    } else {
        throw "missing: Vulkan SDK - winget install KhronosGroup.VulkanSDK, or set VULKAN_SDK"
    }
}

$tauri = Join-Path $root "ui\node_modules\.bin\tauri.cmd"
if (-not (Test-Path $tauri)) {
    Step "Installing frontend dependencies"
    Push-Location ui
    try { Run pnpm install --frozen-lockfile } finally { Pop-Location }
}

# Tauri bundles the sidecar as an `externalBin` and wants the target triple in
# its file name, which it strips again on the way into the install directory -
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

Step "Building the installer - this takes a few minutes"
Run $tauri build

$installer = Get-ChildItem "target\release\bundle\nsis\*.exe" | Select-Object -First 1
if (-not $installer) { throw "the build finished but no installer is in target\release\bundle\nsis" }
Step "Installer: $($installer.FullName)"
Write-Host "Unsigned: SmartScreen will warn on first start - More info, then Run anyway."

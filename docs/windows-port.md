# Windows port — working plan

The execution checklist for **P18** in [plan.md](plan.md#p18--windows-port-wasapi-shared-asio-opt-in--in-progress).
P18 holds the *design* and its reasons (shared-mode WASAPI, ASIO opt-in, the
never-touch-the-DAW constraint); this file holds the *order of work* and where
each piece currently stands, so the port can be picked up at any step without
re-surveying the tree. When a step is done, tick it here and keep P18's status
line in plan.md in sync.

## Start here (handoff, 2026-09-23)

**Picked up on the Windows PC the same day:** steps 1–4 below are done, and
the app runs and transcribes in development mode. What that turned up is in
*Findings*; what is left of the list is step 5 (the installer and its smoke
test), then Stage 3.

Work continues on the `windows-port` branch, on a Windows PC. Stages 1, 2 and
4 were written on macOS and have now been built and tested on Windows.

GitHub Actions is **not available** right now: jobs are refused with "recent
account payments have failed or your spending limit needs to be increased".
The workflow also only triggers on pushes to `main` and on pull requests, so
the branch pushes never ran it. Until billing is fixed, the Windows PC is the
build loop.

On the Windows PC, in order:

1. Install: Rust via rustup (MSVC toolchain), Visual Studio Build Tools with
   "Desktop development with C++", CMake, **LLVM** (libclang for bindgen),
   Node 22, pnpm. Then `git checkout windows-port`. All of it by winget:
   `Rustlang.Rustup`, `Microsoft.VisualStudio.2022.BuildTools` (with
   `--override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"`),
   `Kitware.CMake`, `LLVM.LLVM`, `OpenJS.NodeJS.22`, `KhronosGroup.VulkanSDK`
   (Stage 3; its installer needs an elevated prompt); then `npm install -g pnpm`.
2. `cargo build --workspace --release` — Stage 1. Expect whisper.cpp's cmake
   build to be the likely first failure; fix it here.
3. `cargo clippy --workspace --all-targets -- -D warnings` and
   `cargo test --workspace` — the `cfg(windows)` paths (`worker::hide_console`,
   the `ping` child in `resources.rs`, `stable_id::wasapi`) have only been
   type-checked, never run.
4. `cargo test -p audio-core catalogue_on_this_machine -- --ignored --nocapture`
   — lists each capture endpoint with its id. Paste the output into
   *Findings*: it is the hardware check Stage 5 depends on (how many endpoints
   the interface shows, and with how many channels each).
5. `scripts\build-windows.ps1` — the installer — then the Stage 2 smoke test
   below. `scripts\build-windows.ps1 -Dev` for hot reload.
6. Record everything surprising in *Findings*, tick the plan, then Stage 3
   (Vulkan) and Stage 5.

## Where we build

Development happens on a Mac, and whisper.cpp (cmake + MSVC) does not
cross-compile from macOS without a fragile toolchain (`cargo-xwin` plus a
hand-fed cmake), so the **build loop is a `windows-latest` GitHub Actions job**
that uploads the installer as an artifact. A real Windows machine is only
needed from Stage 3 on — to run the installer and, for acceptance, with a
multichannel interface and a DAW holding it.

Target: `x86_64-pc-windows-msvc`, Windows 10 22H2 / Windows 11. ARM64 Windows
is out of scope for now.

## Stages

Each stage ends in something that can be checked, and each is shippable on its
own: Stage 2 already gives a working (slow, CPU-only, name-identified) Windows
build. Later stages make it good, not possible.

### Stage 0 — Survey (done)

Everything platform-specific in the tree, found by grepping for `cfg(`,
`objc2`, `unix`, hard-coded names and paths. Nothing outside this list is known
to depend on macOS.

| Where | What | Blocks the build? |
|---|---|---|
| `Cargo.toml` (workspace) | `whisper-rs` pinned to `features = ["metal"]` unconditionally | **yes** |
| `Cargo.toml`, `crates/app/Cargo.toml` | `objc2-io-kit`, `objc2-core-foundation` as plain dependencies of `app` | **yes** |
| `crates/app/src/gpu.rs` | GPU utilisation via IOKit, used by `resources.rs` (`Gpu::open() -> Option<Self>`) | **yes** |
| `rust-toolchain.toml` | `targets` lists only `aarch64-apple-darwin` | CI only |
| `crates/app/src/worker.rs:274` | `find_binary` looks for `whisper-worker` without `.exe` | runtime |
| `crates/app/src/worker.rs:51,234` | sidecar spawned without `CREATE_NO_WINDOW` → a console window flashes up per worker and on every Settings open | runtime (ugly) |
| `crates/app/tauri.conf.json` | bundle targets `dmg`/`app` only; `externalBin` needs the `-x86_64-pc-windows-msvc.exe` name staged | packaging |
| `justfile`, `scripts/run.sh` | bash, `aarch64-apple-darwin` triple, `MACOSX_DEPLOYMENT_TARGET`, `.app` paths, `say`/`afconvert` | tooling |
| `.github/workflows/ci.yml` | one `macos-14` job | CI |
| `crates/app/src/resources.rs:240` | test spawns `/bin/sh` | tests |
| `crates/app/tests/pool.rs:70` | `pgrep -P` / `kill -9` | tests |
| `crates/app/src/remote.rs:417` | secret file `0o600` under `cfg(unix)`; on Windows it inherits the profile ACL, which is per-user already — acceptable, note it | no |
| `crates/audio-core/src/backend/stable_id.rs:40` | non-macOS `catalogue()` is an empty stub → devices identified by name | quality |
| `crates/audio-core/src/backend/cpal_backend.rs` | one cpal stream per `CaptureSession`; WASAPI endpoints are usually stereo pairs | quality (channel count) |
| `crates/audio-core/build.rs` | compiler-rt link, already gated on macOS | no |
| `crates/app/src/main.rs` | `windows_subsystem = "windows"` already set for release | no |
| `ui/src/App.svelte` and others | shortcut hints read "⌘T"; handler already accepts Ctrl | cosmetic |
| `ui/src/lib/SetupGuide.svelte:450` | microphone-permission copy mentions only macOS | copy |

Already fine on Windows: config/data dirs (Tauri path API), window
attention and always-on-top (Tauri), TLS (rustls), the LAN remote and
webhooks, stdout framing (Rust does no CRLF translation on pipes).

### Stage 1 — It compiles on Windows (done, 2026-09-23 on the PC; CI still pending billing)

Goal: `cargo build --workspace --release` succeeds on the CI Windows runner,
CPU-only whisper.

- [x] **whisper-rs per target.** Drop `metal` from the workspace entry; add it
      back per target where it is consumed (`audio-core`, `whisper-worker`) via
      `[target.'cfg(target_os = "macos")'.dependencies] whisper-rs = { workspace = true, features = ["metal"] }`.
      Windows gets no GPU feature yet. Check that macOS still builds with Metal
      (`just check`; the ~8 s shader-compile log line on first start is the tell).
- [x] **IOKit behind `cfg(target_os = "macos")`.** Move `objc2-io-kit` /
      `objc2-core-foundation` under a macOS target table in `crates/app/Cargo.toml`.
      Split `gpu.rs` so the IOKit reader is `#[cfg(target_os = "macos")]` and
      other targets get a `Gpu::open()` that returns `None` — `resources.rs`
      already degrades to "memory only" on `None`.
- [x] **Toolchain.** Add `x86_64-pc-windows-msvc` to `rust-toolchain.toml`
      `targets` (harmless on the Mac, needed for rustup on the runner).
- [x] **CI job `windows`** on `windows-latest`, alongside the macOS one:
      checkout, `rustup show`, rust-cache, cmake (preinstalled on the image —
      verify), `cargo build --workspace --release`. No tests yet; this stage is
      only about the build going green.

Done when: the Windows job is green and the macOS job is unchanged.

### Stage 2 — It runs and packages (runs in dev mode; installer and its smoke test next)

Implemented as: `gpu_none.rs` swapped in by `#[path]` off macOS;
`worker::hide_console`; `tauri.windows.conf.json` (merged over the main config
on Windows only, so the macOS build never attempts NSIS; WebView2 by download
bootstrapper and a per-user install for now); `scripts/build-windows.ps1`; the
`windows` job in `ci.yml` runs clippy, tests and the script, and uploads the
installer as `fluestr-windows-installer`.

Goal: an unsigned NSIS installer as a CI artifact that installs, starts,
lists devices and transcribes one stereo input on CPU.

- [x] **Sidecar lookup.** `find_binary` joins
      `format!("whisper-worker{}", std::env::consts::EXE_SUFFIX)`. Adjust the
      error message and any test that names the file.
- [x] **No console windows.** On Windows, set
      `CommandExt::creation_flags(CREATE_NO_WINDOW)` (`0x0800_0000`) on both
      spawns in `worker.rs`, behind `#[cfg(windows)]`. Check `stderr(inherit)`
      still reaches the dev terminal under `tauri dev`.
- [x] **Unix-only tests.** `resources.rs` test: spawn a child that works on both
      (`cmd /C timeout` / `ping -n` on Windows, or re-spawn the test binary
      itself). `tests/pool.rs`: replace `pgrep`/`kill` with a `sysinfo`
      children-of-our-pid walk + `Process::kill()` — portable and already a
      dependency. Then add `cargo test --workspace` to the Windows CI job.
- [x] **Bundle config.** Add `nsis` to `bundle.targets` (keep `dmg`/`app`; the
      CLI skips targets that do not apply). Add `bundle.windows` with
      `webviewInstallMode: { type: "downloadBootstrapper" }` (or
      `embedBootstrapper` if offline installs matter — the target machines may
      have no internet; **decide**). Consider moving the macOS-only keys into a
      `tauri.macos.conf.json` / `tauri.windows.conf.json` split if the shared
      file gets awkward.
- [x] **Build script.** `scripts/build-windows.ps1`: frontend install, sidecar
      build, stage to `crates/app/binaries/whisper-worker-x86_64-pc-windows-msvc.exe`,
      `tauri build`. Same steps as `just build`, no signing. CI calls this
      script so it cannot rot. (`justfile` stays the macOS entry point; making
      it cross-platform is not worth it while one platform has signing and
      fixtures via `say`.)
- [x] **CI artifact.** Upload `target/release/bundle/nsis/*.exe`. Add the
      frontend checks (`pnpm check`, `pnpm test`) to the Windows job only if
      they ever differ — they do not touch the platform, so the macOS job
      covering them is enough.
- [ ] **Smoke test on a Windows machine:** install, first-run guide, model
      download, pick the default input, start, see a transcript. Note what
      broke in *Findings* below. *In dev mode all of it but "install" passed
      on 2026-09-23; the installer itself has not been built yet.*

Done when: the smoke test passes from the CI-built installer.

### Stage 3 — GPU (Vulkan)

Goal: whisper on the GPU on Windows, with its own measured defaults.

- [x] Enable `whisper-rs` feature `vulkan` for `cfg(windows)` — on
      `whisper-worker` only; the app's own whisper (Silero VAD, CPU) stays
      without it under `tauri build`. CI installs the SDK
      (`humbletim/install-vulkan-sdk`, 1.4.309.0); locally
      `winget install KhronosGroup.VulkanSDK` (needs admin), and
      `build-windows.ps1` finds it under `C:\VulkanSDK` if `VULKAN_SDK` is not
      set yet (listed with the other prerequisites in its header — the README
      has no Windows section). Built and run on 2026-09-24, see *Findings*.
- [x] Confirm the fallback. Loader present, no usable driver: ggml's
      `ggml_backend_vk_reg` catches the failed `vkCreateInstance` and registers
      no device → CPU. **No loader at all** was the real problem:
      `vkGetInstanceProcAddr` is a hard import, so `vulkan-1.dll` would stop
      the sidecar from starting, and ggml registers backends before `use_gpu`
      is read. Now delay-loaded (`whisper-worker/build.rs`) with a failure hook
      (`src/vulkan_guard.rs`) that hands ggml a stand-in whose
      `vkEnumerateInstanceVersion` fails — ggml's catch does the rest.
      Verified on 2026-09-24 in a stand-alone harness (loader hidden via
      `SetDefaultDllDirectories`): starts, returns `VK_ERROR_INITIALIZATION_FAILED`,
      logs the CPU warning; with the loader visible, binds the real one (1.3).
      In the real sidecar, the no-driver case (`VK_DRIVER_FILES` pointed at
      nothing): one device (CPU), "no GPU found", transcript unchanged at CPU
      speed. The missing-loader case has only the harness behind it — this
      PC always has `vulkan-1.dll` in System32.
- [ ] **Measure on the target hardware** and write the numbers into
      measurements.md next to the Metal ones: per-segment latency, first-start
      time, how many workers before latency climbs. Set the Windows pool size
      and the first-start message from that, not from the Metal numbers.
      Without a GPU the pool default (2 workers × 4 threads, sized for Metal)
      oversubscribes a 4-core CPU; the CPU fallback should size workers ×
      threads from `available_parallelism`, not from the GPU default.
- [ ] GPU utilisation for the resource meter: PDH counter
      `\GPU Engine(*engtype_3D)\Utilization Percentage` summed — optional; the
      meter already works without it.
- [ ] CUDA: only if Vulkan proves too slow on NVIDIA hardware that the users
      actually have. Not planned.

### Stage 4 — Device identity (WASAPI endpoint id) (runs on Windows; rename check pending)

- [x] Fill the non-macOS `catalogue()` in `stable_id.rs` with
      `IMMDeviceEnumerator::EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)` →
      `(PKEY_Device_FriendlyName, IMMDevice::GetId)`, via the `windows` crate
      (0.54, the version cpal binds WASAPI with — no second copy in the tree).
      cpal's WASAPI device name is the friendly name, so the existing
      name-plus-position matching in `by_name` applies unchanged.
- [x] Unit test stays on `by_name`; add a live test gated like `capture_live.rs`.
      On the Windows PC: `cargo test -p audio-core catalogue_on_this_machine -- --ignored --nocapture`.

Done when: renaming an endpoint in Sound settings keeps the stored profile
working, and two identical interfaces are told apart.

### Stage 5 — Many endpoints behind one `CaptureSession`

The actual P18 capture design: an interface that exposes 8 inputs as four
stereo WASAPI endpoints must still be one "device" with 8 channels.

- [ ] Hardware check first (P18's stated risk): on the target interface, list
      what WASAPI exposes — endpoint count, channels each, shared-mode rates.
      Record it in *Findings*. This decides how much of this stage matters.
- [ ] A stored device becomes an ordered set of endpoint ids; the channel map
      indexes across them in that order. Config migration: a single id is a set
      of one.
- [ ] The Windows backend opens one cpal stream per endpoint, each on its own
      thread, and fans them into the one `CaptureSession`'s sinks. The seam
      (`AudioBackend` → one `CaptureSession`, `sinks` ordered like
      `channel_map`) does not change.
- [ ] Settings UI: choose several endpoints as one source; show which route
      (shared WASAPI) is running.
- [ ] Denied microphone vs. silent channel: detect the privacy-setting denial
      (cpal/WASAPI error `E_ACCESSDENIED` on stream build) and surface it with
      the Windows wording; update the setup guide copy.

Done when (P18 acceptance, part 1): eight channels transcribe while a DAW holds
the same interface, with no dropout, rate change or lost device in the DAW.

### Stage 6 — ASIO (opt-in cargo feature)

- [ ] `asio` cargo feature → `cpal/asio`. CI builds **without** it (SDK not
      redistributable). Document the SDK + LLVM setup for whoever builds with it.
- [ ] The three rules from P18: never call `ASIOSetSampleRate` (request the
      driver's current rate), input only with the driver's buffer size, release
      on stop.
- [ ] Enumeration survives a busy driver: listed as present-and-busy, not
      dropped.
- [ ] Settings: ASIO as a deliberate route choice, never a fallback.

Done when (P18 acceptance, part 2): runs on a multi-client driver; on a
single-client one, fails naming the interface as busy.

### Stage 7 — Signing and distribution

- [ ] Azure Trusted Signing (or an EV certificate) wired into `tauri build`
      via `bundle.windows.signCommand`. Blocked on an account, like P13.
- [ ] Until then: a `docs/beta-install-windows.md` for the SmartScreen
      "More info → Run anyway" step, shipped next to the installer the way
      `beta-install.md` is for macOS.

### Polish (any time after Stage 2)

- [x] Shortcut hints: "Ctrl+T" instead of "⌘T" when not on macOS (the handler
      already accepts both) — `ui/src/lib/shortcut.ts`, decided by the reader's
      browser so the LAN remote gets its own keyboard's label.
- [x] Setup guide: the no-device message names the Windows microphone privacy
      setting instead of "when macOS asks".
- [ ] Test fixtures: `just fixture` uses `say`; on Windows either reuse the
      committed/Mac-generated WAVs or document `System.Speech`. Low priority.

## Decisions to confirm

- **WebView2 install mode** — download bootstrapper (small installer, needs
  internet at install) vs. embedded/offline (bigger, works on air-gapped
  machines). Leaning offline, given the app is built to run without a network.
- **Minimum Windows version** — assumed Windows 10 22H2+.
- **Test hardware** — which Windows machine and which interface Stage 3+ is
  measured on.

## Cross-checking from the Mac

`cargo clippy -p audio-core --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
works on the Mac (audio-core without `silero` links no C++), so the capture
and device-identity code for Windows can be checked without CI. Everything
that pulls in whisper.cpp — `app`, `whisper-worker` — needs the Windows job.

## Findings

Record here what only a real run could tell (driver behaviour, surprises in CI,
numbers), with the date.

### 2026-09-23 — first run on Windows

Machine: Windows 11 Pro 26100, AMD Ryzen 3 4300U (4 cores / 4 threads, AVX2,
Radeon iGPU), Rust 1.98.1, VS 2022 Build Tools 17.14, CMake 4.4.3, LLVM 23.

- **bindgen needs libclang.** whisper-rs-sys generates its bindings at build
  time and fails without LLVM. Its bundled fallback (`WHISPER_DONT_GENERATE_BINDINGS`)
  is *not* an option: it comes from Linux glibc headers, where `int64_t` is
  `long`, which is 32-bit on Windows. `build-windows.ps1` finds the LLVM
  default install by itself; plain `cargo` needs
  `LIBCLANG_PATH=C:\Program Files\LLVM\bin` (the installer does not touch PATH).
- **The release build of whisper.cpp was unoptimised.** With the Visual
  Studio generator the `cmake` crate overwrites `CMAKE_<LANG>_FLAGS_RELEASE`
  with cc's flags and strips every `/O…` from them, so `ggml-cpu` was built
  with `<Optimization></Optimization>`. The dev tree escaped by accident (it
  defines `RelWithDebInfo`, so the crate overwrote that variable instead, while
  the VS build still used `Release`). Fixed by
  `.cargo/msvc-release-flags.cmake`, injected via `CMAKE_PROJECT_INCLUDE`,
  which appends `/O2 /Ob2 /DNDEBUG` under `if(MSVC)` only. Slice test, one
  worker × 4 threads, `small-q5_1`, 14.5 s German fixture:
  **77 s → 6.3 s per segment** (decode time is flat in segment length — it is
  the 30 s encoder window).
- **CPU-only is far from real time on this machine** even optimised: ~6 s per
  segment with one worker. In the app (2 workers × 4 threads on 4 cores) the
  first decodes ran past the 30 s watchdog and workers were replaced. Stage 3
  (Vulkan; the Radeon iGPU is a candidate) and a CPU-sized pool are what make
  it usable. whisper.cpp with `use_gpu = true` and no GPU backend falls back to
  CPU on its own ("no GPU found"), which answers half of Stage 3's fallback
  question.
- **Integration tests that link the Tauri runtime** died with
  `STATUS_ENTRYPOINT_NOT_FOUND`: `TaskDialogIndirect` needs Common Controls v6,
  which only a manifest brings in, and tauri-build manifests the app, not the
  tests. `crates/app/build.rs` now embeds that dependency into test targets
  on MSVC. After it: fmt, clippy `-D warnings`, `cargo test --workspace` all
  green (195 app, 8 remote, 89 audio-core, 8 protocol, 2 worker).
- **CI never staged the sidecar.** `tauri-build` refuses to build `app` without
  `crates/app/binaries/whisper-worker-<triple>`; it only ever worked on the Mac
  because `just dev` had left one in the ignored folder. Both CI jobs now
  build and stage it first.
- **`build-windows.ps1` under Windows PowerShell 5.1:** it did not parse (em
  dashes in a BOM-less file are read as Windows-1252, where their last byte is
  a closing quote), and with output redirected, cargo's first stderr line was
  fatal under `$ErrorActionPreference = "Stop"`. Both fixed; the file is
  ASCII now.
- **Capture:** WASAPI shared mode works; the default input was metered and
  transcribed. The first-run guide, model download (managed, into
  `%APPDATA%\io.github.willihelm.fluestr\models`) and config all work
  unchanged. No GPU utilisation, as expected: the meter shows memory only.
- **Endpoints on this PC** (`catalogue_on_this_machine`, `list_devices`) —
  two, each with a distinct id, each stereo, each offering exactly one
  shared-mode rate:

  ```
  Line In /Microphone (Waves SoundGrid)    {0.0.1.00000000}.{25ec8b49-a294-44b6-996f-ab4bb8a36288}  2 ch, 44100 Hz
  Mikrofon (Realtek High Definition Audio) {0.0.1.00000000}.{84bc2c30-aa53-4a8a-b65f-bff9607f2810}  2 ch, 48000 Hz
  ```

  SoundGrid appears as one stereo endpoint here — how a real SoundGrid rig
  with more I/O shows up is Stage 5's hardware check.
- Cosmetic: MSVC's German `link.exe` prints "Bibliothek … werden erstellt"
  for the cdylib, which rustc's `linker_messages` lint reports as a warning
  (it only recognises the English text). Harmless; `-D warnings` in clippy
  does not link, so it does not fail the build.

### 2026-09-24 — Vulkan (Stage 3)

Same machine; Vulkan SDK 1.4.357.0, AMD Radeon iGPU driver (Vulkan 1.3).

- **Per-segment latency, `small-q5_1`, one worker, slice test** (14.5 s
  German fixture, five segments, same release sidecar both times):

  | Backend | First segment | Following segments | Total |
  |---|---|---|---|
  | Vulkan (Radeon iGPU) | 3.6 s | 1.4–1.7 s | 9.8 s |
  | CPU fallback (4 threads) | 6.2 s | 6.4–6.7 s | 32.5 s |

  The first Vulkan segment carries the pipeline compilation. Model load
  0.7 s (189 MB into `Vulkan0`). Not yet measured: several workers on the one
  iGPU, and first start after a driver update (shader cache cold).
- **MAX_PATH in the ggml-vulkan build.** The shader generator is an
  ExternalProject under `…\ggml-vulkan\vulkan-shaders-gen-prefix\src\vulkan-shaders-gen-build\`,
  and MSBuild's `VCTargetsPath` probe below it reached 262 characters from
  `C:\Users\ICF Licht\Documents\git\fluestr` (MSB3491). Fixed in
  `.cargo/msvc-release-flags.cmake` with `EP_BASE`, which moves it to
  `…\build\ep\Build\vulkan-shaders-gen`. CI's `D:\a\fluestr\fluestr` is short
  enough either way.
- **A failed CMake run poisons the next build.** whisper-rs-sys passes *every*
  directory under its CMake build tree to rustc as `-L`, including the
  over-long leftovers of the failed configure; with those, rustc reported
  `can't find crate for tracing` while compiling whisper-rs.
  `cargo clean --release -p whisper-rs-sys` clears it.
- **Fallback, verified:** no usable driver → one device (CPU), "no GPU found",
  same transcript at CPU speed. `vulkan-1.dll` is a delay-load dependency of
  `whisper-worker.exe` (`dumpbin /dependents`).
- The slice test's "loaded in … (GPU)" reflects the requested `use_gpu`, not
  the backend whisper ended up on; the sidecar log line
  `whisper_backend_init_gpu: using Vulkan0 backend` / `no GPU found` is the
  authority.

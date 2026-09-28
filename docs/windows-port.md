# Windows port — working plan

The execution checklist for **P18** in [plan.md](plan.md#p18--windows-port-wasapi-shared-asio-opt-in--open).
P18 holds the *design* and its reasons (shared-mode WASAPI, ASIO opt-in, the
never-touch-the-DAW constraint); this file holds the *order of work* and where
each piece currently stands, so the port can be picked up at any step without
re-surveying the tree. When a step is done, tick it here and keep P18's status
line in plan.md in sync.

## Start here (handoff, 2026-09-23)

Work continues on the `windows-port` branch, on a Windows PC. Nothing on this
branch has been compiled on Windows yet. Stages 1, 2 and 4 are written and
pass on macOS (clippy, tests, and `audio-core` clippy for the Windows target).

GitHub Actions is **not available** right now: jobs are refused with "recent
account payments have failed or your spending limit needs to be increased".
The workflow also only triggers on pushes to `main` and on pull requests, so
the branch pushes never ran it. Until billing is fixed, the Windows PC is the
build loop.

On the Windows PC, in order:

1. Install: Rust via rustup (MSVC toolchain), Visual Studio Build Tools with
   "Desktop development with C++", CMake, Node 22, pnpm. Then
   `git checkout windows-port`.
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

### Stage 1 — It compiles on Windows (code done, awaiting first CI run)

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

### Stage 2 — It runs and packages (code done, awaiting CI + smoke test)

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
      broke in *Findings* below.

Done when: the smoke test passes from the CI-built installer.

### Stage 3 — GPU (Vulkan)

Goal: whisper on the GPU on Windows, with its own measured defaults.

- [ ] Enable `whisper-rs` feature `vulkan` for `cfg(windows)`. CI needs the
      Vulkan SDK (a setup action, pinned version); document the local
      equivalent in the README.
- [ ] Confirm the fallback: a machine without a Vulkan driver must fall back to
      CPU, not fail to start the sidecar. If whisper.cpp does not do that on its
      own, the sidecar has to detect and retry with `use_gpu = false`.
- [ ] **Measure on the target hardware** and write the numbers into
      measurements.md next to the Metal ones: per-segment latency, first-start
      time, how many workers before latency climbs. Set the Windows pool size
      and the first-start message from that, not from the Metal numbers.
- [ ] GPU utilisation for the resource meter: PDH counter
      `\GPU Engine(*engtype_3D)\Utilization Percentage` summed — optional; the
      meter already works without it.
- [ ] CUDA: only if Vulkan proves too slow on NVIDIA hardware that the users
      actually have. Not planned.

### Stage 4 — Device identity (WASAPI endpoint id) (code done, awaiting a Windows run)

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

- _none yet_

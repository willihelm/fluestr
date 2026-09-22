//! Links clang's compiler-rt builtins next to whisper.cpp's Metal backend.
//!
//! `ggml-metal.m` guards macOS 14 API behind `@available`. Compiled against a
//! deployment target below that, clang cannot fold the guard away and emits a
//! call to `__isPlatformVersionAtLeast` instead — a symbol that lives in
//! compiler-rt. rustc links with `-nodefaultlibs`, so nothing else drags
//! compiler-rt in and the link fails with an undefined symbol.
//!
//! That only happens in the packaged build: `tauri build` exports the bundle's
//! `minimumSystemVersion` (13.0) as the deployment target, while a plain
//! `cargo build` inherits the host's and folds the guard to a constant. So
//! this is invisible until the day someone actually runs `just build` — which
//! is why it is fixed here rather than worked around by raising the minimum.
//!
//! Gated on the Silero feature because that is what puts whisper.cpp, and
//! therefore its Metal backend, into the link at all.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    let silero = std::env::var_os("CARGO_FEATURE_SILERO").is_some();
    if !macos || !silero {
        return;
    }

    // Ask the toolchain where its builtins are: the path carries the clang
    // major version, so anything written down here would break on the next
    // Command Line Tools update.
    let out = std::process::Command::new("xcrun")
        .args(["clang", "-print-runtime-dir"])
        .output();

    let dir = match out {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        // Not fatal: without a deployment target below macOS 14 the symbol is
        // never referenced, and failing the build of every `cargo test` over a
        // library it does not need would be the worse trade.
        _ => {
            println!("cargo:warning=could not locate the clang runtime directory");
            return;
        }
    };

    println!("cargo:rustc-link-search=native={dir}");
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}

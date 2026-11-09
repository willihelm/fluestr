fn main() {
    // Tauri imports `TaskDialogIndirect`, which only exists in version 6 of the
    // Common Controls, and Windows loads that one only for a process whose
    // manifest asks for it. tauri-build embeds such a manifest as a resource
    // into the app, but not into test executables, so a test that pulls in the
    // runtime dies before `main` with STATUS_ENTRYPOINT_NOT_FOUND (P18).
    //
    // `rustc-link-arg-tests` would cover only the integration tests in
    // `tests/`, not the lib's own unit tests (`sources.rs` builds a mock app).
    // So the linker writes the manifest for every target of this crate, and
    // tauri-build leaves its copy out. Both at once would be two manifest
    // resources and a failed link.
    let mut windows = tauri_build::WindowsAttributes::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' \
             name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
             processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
        windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    }
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}

fn main() {
    // Tauri imports `TaskDialogIndirect`, which only exists in version 6 of the
    // Common Controls, and Windows loads that one only for a process whose
    // manifest asks for it. tauri-build embeds such a manifest into the app,
    // not into test executables — so an integration test that pulls in the
    // runtime dies before `main` with STATUS_ENTRYPOINT_NOT_FOUND (P18).
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTDEPENDENCY:type='win32' \
             name='Microsoft.Windows.Common-Controls' version='6.0.0.0' \
             processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
    tauri_build::build()
}

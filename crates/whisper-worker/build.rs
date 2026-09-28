fn main() {
    // Load the Vulkan loader on first use instead of at process start, so a
    // Windows machine without one still starts the sidecar and transcribes on
    // the CPU. `vulkan_guard.rs` is the other half (P18 stage 3).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:vulkan-1.dll");
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
    }
}

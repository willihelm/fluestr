//! The GPU reading on platforms without one (P18).
//!
//! Same surface as the IOKit reader in `gpu.rs`, which is macOS-only. Windows
//! has the figure behind a PDH counter, and until that is read here the
//! resource meter shows memory alone — the degradation it already has for a
//! Mac that presents no GPU in the registry.

/// A reader that never finds a GPU.
pub struct Gpu {
    /// Shader cores, for the tooltip. Never known here.
    pub cores: Option<usize>,
}

impl Gpu {
    /// Always `None`: there is no reading on this platform yet.
    pub fn open() -> Option<Self> {
        None
    }

    /// Unreachable in practice, since [`Gpu::open`] never returns a reader.
    pub fn sample(&mut self) -> Option<f32> {
        None
    }
}

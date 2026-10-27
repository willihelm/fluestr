//! Whether the machine is about to stop keeping up (P8).
//!
//! An operator running eight channels on a laptop has one question this app
//! never answered: is the machine keeping up, or is it about to stop keeping
//! up? The overload counter says it has already happened; this says it is
//! coming.
//!
//! One figure: the GPU share, and it is the whole machine's. Decoding runs on
//! Metal, so the GPU is the thing that runs out first, and macOS will not break
//! its utilisation down per process. See [`crate::gpu`] for what that means and
//! why it is still the right number.

use crate::gpu;
use serde::Serialize;
use std::thread;
use std::time::Duration;

/// How often the usage is sampled and sent.
///
/// A second, not the meters' 20 Hz: this is a trend, not a signal, and the
/// GPU counter has to be read on a steady interval — see [`crate::gpu`].
pub const INTERVAL: Duration = Duration::from_secs(1);

/// One sample of what the machine's GPU is doing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "resources.ts", rename = "ResourceUsage")]
pub struct Usage {
    /// GPU utilisation in percent, smoothed, or `None` while the counter has
    /// nothing to report yet. The whole machine's, not this app's — see
    /// [`crate::gpu`], which is also where the smoothing and its reasons live.
    pub gpu: Option<f32>,
    /// The GPU's shader cores, when it says. Not a denominator — the share
    /// above is already one — but the scale an operator reads it against.
    pub gpu_cores: Option<usize>,
}

/// Samples the GPU forever, handing each sample to `emit`.
///
/// Blocks; [`spawn`] is what the app calls. Split out so the loop is readable
/// without a thread around it.
fn run(mut gpu: gpu::Gpu, mut emit: impl FnMut(Usage)) {
    let gpu_cores = gpu.cores;
    loop {
        thread::sleep(INTERVAL);
        emit(Usage {
            gpu: gpu.sample(),
            gpu_cores,
        });
    }
}

/// Starts the sampler on a thread of its own.
///
/// Its own thread rather than Tauri's async runtime: the work is a blocking
/// registry read once a second, and it has no business sharing a pool with
/// the downloads.
///
/// A machine that reports no GPU gets no sampler at all, and so no indicator:
/// there is nothing to show, and an empty strip would read as zero.
pub fn spawn(emit: impl FnMut(Usage) + Send + 'static) {
    // The one reader of the GPU counter, for as long as the app runs. It has
    // to be exactly one, on a steady interval — see [`crate::gpu`].
    let Some(gpu) = gpu::Gpu::open() else {
        tracing::info!("no GPU utilisation available; the indicator stays hidden");
        return;
    };

    thread::Builder::new()
        .name("resource-sampler".into())
        .spawn(move || run(gpu, emit))
        // A machine that cannot start one more thread has larger problems than
        // a missing indicator, and refusing to start the app over it would be
        // the wrong trade.
        .map(|_| ())
        .unwrap_or_else(|err| tracing::warn!(error = %err, "no resource sampler"));
}

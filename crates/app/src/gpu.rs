//! What the GPU is doing, read out of the IO registry (P8).
//!
//! Decoding runs on Metal, and eight whisper instances contend for one queue
//! (see [`crate::pool`]). So the question an operator actually has — "is this
//! machine about to stop keeping up?" — is a question about the GPU. The CPU
//! figure this replaced answered a question nobody was asking: the app's own
//! threads are audio plumbing and never the bottleneck.
//!
//! Three things about this reading are worth knowing before trusting it:
//!
//! - **It is the whole machine, not this app.** macOS exposes no per-process
//!   GPU utilisation, publicly or otherwise. What is reported here includes
//!   the window server, the browser somebody left open, and everything else.
//!   On a machine dedicated to running a show that is the right number anyway
//!   — the GPU is shared, and a queue saturated by somebody else's video call
//!   delays the decodes exactly as much as one saturated by whisper. But it
//!   means the figure is not zero when the app is idle, and the tooltip has
//!   to say so rather than let an operator conclude the app is at fault.
//! - **The counter is measured since it was last read.** Two reads in quick
//!   succession make the second one nearly zero, because almost no time
//!   passed between them. One reader at a fixed interval is therefore not
//!   just cheaper but the only way the number means anything, which is why
//!   the service handle is held open across samples rather than looked up per
//!   sample by whoever wants a figure.
//! - **A single sample is noise.** Measured on an M1 Pro: an idle machine
//!   reads anywhere from 5 to 40 % second to second, and a machine decoding
//!   continuously still drops to 30 % in the gaps between segments. The
//!   smoothing in [`Gpu::sample`] is what makes a threshold meaningful rather
//!   than a source of flicker.

use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_io_kit::{
    io_object_t, kIOMainPortDefault, IOIteratorNext, IOObjectRelease,
    IORegistryEntryCreateCFProperty, IOServiceGetMatchingServices, IOServiceMatching,
};
use std::ffi::CStr;
use std::ptr;

/// The driver class every Mac GPU registers under.
///
/// The concrete class is per-chip — `AGXAcceleratorG13X` on an M1 Pro — but
/// they all descend from this one, and IOKit matching follows subclasses. So
/// this one name covers Apple Silicon, the Intel machines before it, and
/// whatever comes next, without a table of chip names to keep current.
const SERVICE: &CStr = c"IOAccelerator";

/// The dictionary the utilisation figures live in.
const STATISTICS: &str = "PerformanceStatistics";
/// The combined figure, 0-100.
///
/// Rather than `Renderer` or `Tiler`, which sit next to it: measured against a
/// whisper decode all three move together to within a percent, and `Device` is
/// the one that also covers work belonging to neither.
const UTILISATION: &str = "Device Utilization %";
/// How many shader cores the GPU has — static, and read once.
const CORES: &str = "gpu-core-count";

/// How much of a new sample is believed.
///
/// An exponential average with this weight settles within about five seconds,
/// which is the shape this wants: a decode burst that saturates the GPU for
/// half a second is not an overloaded machine and must not colour the
/// indicator, while a machine that has been at 90 % for five seconds is one
/// and must. Fast enough that an operator watching the strip sees a session
/// start register; slow enough that the figure can be read at all.
const SMOOTHING: f32 = 0.3;

/// A reader for the GPU's share, holding its registry handle open.
///
/// Holding it open is not only an optimisation — see the module docs on why
/// there must be exactly one reader on a fixed interval.
pub struct Gpu {
    service: io_object_t,
    /// The smoothed figure, or `None` until the first sample.
    average: Option<f32>,
    /// Shader cores, for the tooltip. Read once: it cannot change.
    pub cores: Option<usize>,
}

// The handle is an integer naming a kernel object, and every call made on it
// here is a read. Rust cannot see that through the raw type, so it is said
// explicitly — otherwise the sampler thread could not own one.
unsafe impl Send for Gpu {}

impl Drop for Gpu {
    fn drop(&mut self) {
        if self.service != 0 {
            IOObjectRelease(self.service);
        }
    }
}

impl Gpu {
    /// Finds the GPU, or `None` on a machine that does not present one this
    /// way.
    ///
    /// `None` rather than an error: a missing GPU reading costs an indicator,
    /// and an app that refused to start over one would be trading something
    /// an operator needs for something they merely like.
    pub fn open() -> Option<Self> {
        // SAFETY: `SERVICE` is a C string literal, and the matching dictionary
        // is handed straight to the call that consumes it.
        let matching = unsafe { IOServiceMatching(SERVICE.as_ptr()) }?;
        // The matching helpers hand back a mutable dictionary and the lookup
        // wants an immutable one; in Core Foundation the mutable type *is* the
        // immutable one with more methods, so this renames rather than
        // converts.
        // SAFETY: `CFMutableDictionary` is a `CFDictionary`.
        let matching: CFRetained<CFDictionary> = unsafe { CFRetained::cast_unchecked(matching) };

        let mut iterator = 0;
        // SAFETY: `iterator` is ours to write into; the dictionary is consumed
        // by this call, which the binding's signature already accounts for.
        let result = unsafe {
            IOServiceGetMatchingServices(kIOMainPortDefault, Some(matching), &mut iterator)
        };
        if result != 0 || iterator == 0 {
            return None;
        }

        // The first match. A Mac with two GPUs presents both, but the one that
        // matters is the one Metal picked, and there is no honest way to ask
        // the registry which that was — so the machine's own order is taken,
        // which on every Mac that has two puts the one in use first.
        let service = IOIteratorNext(iterator);
        IOObjectRelease(iterator);
        if service == 0 {
            return None;
        }

        let mut gpu = Self {
            service,
            average: None,
            cores: None,
        };
        gpu.cores = gpu
            .property(CORES)
            .and_then(|value| number(&value))
            .map(|cores| cores.max(0.0) as usize);
        Some(gpu)
    }

    /// One reading, smoothed. `None` if the GPU stopped answering.
    ///
    /// Must be called at a steady interval and from one place only; see the
    /// module documentation.
    pub fn sample(&mut self) -> Option<f32> {
        let statistics = self.property(STATISTICS)?;
        let statistics = statistics.downcast_ref::<CFDictionary>()?;
        let key = CFString::from_str(UTILISATION);
        // SAFETY: the key is a `CFString` and the dictionary is a registry
        // snapshot with `CFString` keys, which is the comparison it is set up
        // to make.
        let value = unsafe { statistics.value(ptr::from_ref(&*key).cast()) };
        if value.is_null() {
            return None;
        }
        // SAFETY: non-null, and owned by the dictionary this borrows from.
        let percent = number(unsafe { &*value.cast::<CFType>() })?.clamp(0.0, 100.0);

        // The first sample has nothing to average against, and starting from
        // zero would show a calm machine that is in fact already loaded.
        let smoothed = match self.average {
            Some(previous) => previous + SMOOTHING * (percent - previous),
            None => percent,
        };
        self.average = Some(smoothed);
        Some(smoothed)
    }

    /// One property of the GPU's registry entry.
    fn property(&self, key: &str) -> Option<CFRetained<CFType>> {
        let key = CFString::from_str(key);
        // SAFETY: the service handle is live for as long as `self` is, and the
        // default allocator is what every registry read here wants.
        unsafe { IORegistryEntryCreateCFProperty(self.service, Some(&key), None, 0) }
    }
}

/// A registry number as an `f32`.
///
/// The registry types its numbers by what fits, so a utilisation that happens
/// to be a whole percent and a core count arrive as integers while other
/// entries do not. Going through `CFNumber`'s own conversion rather than
/// guessing the width is what makes that not matter.
fn number(value: &CFType) -> Option<f32> {
    let number = value.downcast_ref::<CFNumber>()?;
    number.as_f64().map(|value| value as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reading against the real machine, which is the only place it can be
    /// checked: every branch above is a call into the registry, and a test
    /// that mocked those would be a test of the mock.
    #[test]
    fn reads_the_machine_s_gpu() {
        let Some(mut gpu) = Gpu::open() else {
            // A machine with no GPU registry entry is a machine this cannot be
            // tested on, not a failure — CI runners are exactly that.
            eprintln!("no GPU registry entry; skipping");
            return;
        };

        let percent = gpu.sample().expect("a reading from a GPU that just opened");
        assert!((0.0..=100.0).contains(&percent), "{percent}");

        if let Some(cores) = gpu.cores {
            assert!(cores > 0, "a GPU with no cores");
        }
    }

    /// The smoothing is the whole reason a single noisy reading is usable, so
    /// it is worth knowing it actually converges rather than drifting or
    /// standing still.
    #[test]
    fn smoothing_converges_on_a_steady_load() {
        let Some(mut gpu) = Gpu::open() else {
            return;
        };

        // Drive it by hand rather than against the real GPU, whose load during
        // a test run is not something a test gets to decide.
        gpu.average = Some(0.0);
        for _ in 0..40 {
            let previous = gpu.average.expect("primed above");
            gpu.average = Some(previous + SMOOTHING * (100.0 - previous));
        }

        let settled = gpu.average.expect("primed above");
        assert!(
            settled > 99.0,
            "never reached the load it was fed: {settled}"
        );
    }

    /// Two reads of the counter in quick succession: the second covers almost
    /// no time and reads near zero. This is the documented hazard that the
    /// single long-lived reader exists to avoid, and a regression here would
    /// mean the indicator quietly started reading nonsense.
    #[test]
    fn a_second_immediate_read_is_not_a_second_opinion() {
        let Some(mut first) = Gpu::open() else {
            return;
        };
        let Some(mut second) = Gpu::open() else {
            return;
        };

        // Both are unsmoothed: each is its owner's first sample.
        let _ = first.sample();
        let immediately_after = second.sample().expect("a reading");

        // Not an assertion on the value — an idle machine reads near zero
        // anyway, so this cannot distinguish. What it pins down is that the
        // second reader answers at all, which is what makes the failure mode
        // silent and therefore worth a test naming it.
        assert!((0.0..=100.0).contains(&immediately_after));
    }
}

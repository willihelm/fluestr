//! Whether the machine is about to stop keeping up (P8).
//!
//! An operator running eight channels on a laptop has one question this app
//! never answered: is the machine keeping up, or is it about to stop keeping
//! up? The overload counter says it has already happened; this says it is
//! coming.
//!
//! Two figures, and they are measured against different things — which the
//! UI has to say out loud, because side by side they look like one pair:
//!
//! - **The GPU share is the whole machine.** Decoding runs on Metal, so the
//!   GPU is the thing that runs out first, and macOS will not break its
//!   utilisation down per process. See [`crate::gpu`] for what that means and
//!   why it is still the right number.
//! - **The memory is this app and its workers.** Every whisper worker is a
//!   sidecar (see [`crate::worker`]), so the app's own process is the smallest
//!   part of what the app holds. What is reported is the whole family — this
//!   process and everything descended from it — because that is what an
//!   operator would see in Activity Monitor if they thought to add it up.

use crate::gpu;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// How often the usage is sampled and sent.
///
/// A second, not the meters' 20 Hz: this is a trend, not a signal, and
/// enumerating every process on the machine is real work — at the meters' rate
/// the indicator would be part of the load it reports.
pub const INTERVAL: Duration = Duration::from_secs(1);

/// One sample of what the app and its workers are using.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "resources.ts", rename = "ResourceUsage")]
pub struct Usage {
    /// GPU utilisation in percent, smoothed, or `None` on a machine that does
    /// not report it. The whole machine's, not this app's — see [`crate::gpu`],
    /// which is also where the smoothing and its reasons live.
    pub gpu: Option<f32>,
    /// Resident memory of the app and its sidecars, in bytes. Resident rather
    /// than virtual because a whisper model is only a cost once it is actually
    /// in RAM, and virtual size on macOS is a number nobody can act on.
    pub memory: u64,
    /// The machine's physical memory, so the UI can say "of 16 GB" rather than
    /// leaving a gigabyte figure without a scale.
    pub total_memory: u64,
    /// How many processes the figures cover: the app plus one per running
    /// worker. Shown because "2.1 GB" reads differently when it is one process
    /// than when it is five.
    pub processes: usize,
    /// The GPU's shader cores, when it says. Not a denominator — the share
    /// above is already one — but the scale an operator reads it against, the
    /// way the memory figure has the machine's total next to it.
    pub gpu_cores: Option<usize>,
}

/// The pids of `root` and everything descended from it.
///
/// Descendants rather than only children: a worker that one day starts a
/// helper of its own would otherwise drop out of the figures silently, and a
/// resource indicator that quietly stops counting part of the load is worse
/// than none. Cycles cannot happen in a real process table, but a parent
/// pointer that has been recycled between two samples can look like one, so
/// each pid is visited once.
fn family(parents: &HashMap<Pid, Option<Pid>>, root: Pid) -> HashSet<Pid> {
    let mut found = HashSet::new();
    if parents.contains_key(&root) {
        found.insert(root);
    }

    // Repeated passes rather than recursion from the root, because the table
    // is a child -> parent map: a pass that adds nothing means every
    // descendant has been reached.
    loop {
        let before = found.len();
        for (&pid, &parent) in parents {
            if let Some(parent) = parent {
                if found.contains(&parent) {
                    found.insert(pid);
                }
            }
        }
        if found.len() == before {
            return found;
        }
    }
}

/// Samples usage forever, handing each sample to `emit`.
///
/// Blocks; [`spawn`] is what the app calls. Split out so the loop is readable
/// without a thread around it.
fn run(mut emit: impl FnMut(Usage)) {
    // Memory only: the per-process CPU figure this used to report is gone, and
    // asking sysinfo to keep computing it would be work once a second that
    // nothing reads.
    let wanted = ProcessRefreshKind::nothing().with_memory();
    let mut system = System::new();
    let us = Pid::from_u32(std::process::id());

    system.refresh_memory();
    let total_memory = system.total_memory();

    // The one reader of the GPU counter, for as long as the app runs. It has
    // to be exactly one, on a steady interval — see [`crate::gpu`].
    let mut gpu = gpu::Gpu::open();
    if gpu.is_none() {
        tracing::info!("no GPU utilisation available; the indicator will show memory only");
    }
    let gpu_cores = gpu.as_ref().and_then(|gpu| gpu.cores);

    loop {
        thread::sleep(INTERVAL);

        system.refresh_processes_specifics(ProcessesToUpdate::All, true, wanted);

        let parents: HashMap<Pid, Option<Pid>> = system
            .processes()
            .iter()
            .map(|(&pid, process)| (pid, process.parent()))
            .collect();
        let ours = family(&parents, us);

        let mut memory = 0;
        for pid in &ours {
            if let Some(process) = system.process(*pid) {
                memory += process.memory();
            }
        }

        emit(Usage {
            gpu: gpu.as_mut().and_then(|gpu| gpu.sample()),
            memory,
            total_memory,
            processes: ours.len(),
            gpu_cores,
        });
    }
}

/// Starts the sampler on a thread of its own.
///
/// Its own thread rather than Tauri's async runtime: the work is a blocking
/// walk of the process table and a registry read once a second, and it has no
/// business sharing a pool with the downloads.
pub fn spawn(emit: impl FnMut(Usage) + Send + 'static) {
    thread::Builder::new()
        .name("resource-sampler".into())
        .spawn(move || run(emit))
        // A machine that cannot start one more thread has larger problems than
        // a missing indicator, and refusing to start the app over it would be
        // the wrong trade.
        .map(|_| ())
        .unwrap_or_else(|err| tracing::warn!(error = %err, "no resource sampler"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(n: u32) -> Pid {
        Pid::from_u32(n)
    }

    /// The shape this actually runs against: the app, its sidecars, and the
    /// rest of the machine's processes, which must not be counted.
    #[test]
    fn counts_the_app_and_its_workers_only() {
        let parents = HashMap::from([
            (pid(1), None),
            (pid(10), Some(pid(1))),  // us
            (pid(11), Some(pid(10))), // a worker
            (pid(12), Some(pid(10))), // another worker
            (pid(20), Some(pid(1))),  // somebody else's process
        ]);

        assert_eq!(
            family(&parents, pid(10)),
            HashSet::from([pid(10), pid(11), pid(12)])
        );
    }

    #[test]
    fn reaches_a_worker_s_own_children() {
        let parents = HashMap::from([
            (pid(10), None),
            (pid(11), Some(pid(10))),
            (pid(12), Some(pid(11))),
        ]);

        assert_eq!(
            family(&parents, pid(10)),
            HashSet::from([pid(10), pid(11), pid(12)])
        );
    }

    /// A parent pointer that has been recycled between two samples can close a
    /// loop. The walk has to end anyway.
    #[test]
    fn a_cycle_terminates() {
        let parents = HashMap::from([(pid(10), Some(pid(11))), (pid(11), Some(pid(10)))]);

        assert_eq!(family(&parents, pid(10)), HashSet::from([pid(10), pid(11)]));
    }

    /// The sampler against the real machine: what it reports has to include
    /// the test process itself, which is the one thing a unit test of the
    /// tree-walk cannot check. Takes a second, because that is the interval.
    #[test]
    fn samples_this_very_process() {
        let (tx, rx) = std::sync::mpsc::channel();
        spawn(move |usage| {
            let _ = tx.send(usage);
        });

        let usage = rx
            .recv_timeout(INTERVAL * 5)
            .expect("a sample within five intervals");

        assert!(usage.processes >= 1, "the test process is missing");
        assert!(usage.memory > 0, "a running process occupies memory");
        // `None` is allowed — a machine that does not report a GPU is a
        // machine the indicator degrades on, not one the sampler fails on.
        if let Some(gpu) = usage.gpu {
            assert!((0.0..=100.0).contains(&gpu), "{gpu}");
        }
    }

    /// The whole point of walking the tree: a whisper worker is a child
    /// process, so a sampler that reported only our own process would report a
    /// fraction of the memory the app actually holds. A child stands in for
    /// one — `sh` rather than a real sidecar, which would need a model and a
    /// minute.
    #[test]
    fn counts_a_child_process() {
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("while :; do sleep 1; done")
            .spawn()
            .expect("a shell");

        let (tx, rx) = std::sync::mpsc::channel();
        spawn(move |usage| {
            let _ = tx.send(usage);
        });
        let usage = rx
            .recv_timeout(INTERVAL * 5)
            .expect("a sample within five intervals");

        let _ = child.kill();
        let _ = child.wait();

        assert!(usage.processes >= 2, "the child is missing: {usage:?}");
        assert!(usage.memory > 0, "the family holds no memory: {usage:?}");
    }

    /// Our own pid is always in the table in practice; if a refresh ever
    /// misses it, the answer is "nothing", not "the whole machine".
    #[test]
    fn an_unknown_root_owns_nothing() {
        let parents = HashMap::from([(pid(1), None), (pid(20), Some(pid(1)))]);

        assert!(family(&parents, pid(10)).is_empty());
    }
}

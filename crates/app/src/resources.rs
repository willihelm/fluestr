//! What the app is costing the machine (P8).
//!
//! An operator running eight channels on a laptop has one question this app
//! never answered: is the machine keeping up, or is it about to stop keeping
//! up? The overload counter says it has already happened; this says it is
//! coming.
//!
//! Two things make the measurement less obvious than "ask for our own
//! process":
//!
//! - **The decoding happens in child processes.** Every whisper worker is a
//!   sidecar (see [`crate::worker`]), so the app's own process is the smallest
//!   part of what the app costs. What is reported is the whole family — this
//!   process and everything descended from it — because that is what an
//!   operator would see in Activity Monitor if they thought to add it up.
//! - **A percentage needs a denominator.** The operating system counts one
//!   fully busy core as 100 %, so eight busy workers report 800 % and look
//!   like a failure. This divides by the number of cores, so the number on
//!   screen is the share of the whole machine and 100 % means there is nothing
//!   left.

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
    /// Share of the whole machine, in percent: 100 is every core busy. Summed
    /// over the app and its sidecars and divided by the core count — see the
    /// module documentation for why the raw figure would mislead.
    pub cpu: f32,
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
    /// Logical cores, which is what the CPU share is measured against.
    pub cores: usize,
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
    let wanted = ProcessRefreshKind::nothing().with_cpu().with_memory();
    let mut system = System::new();
    let us = Pid::from_u32(std::process::id());

    // The first CPU reading of a process is meaningless — a percentage needs
    // two points in time. So the first refresh is a priming one and nothing is
    // sent from it; the loop below always has a previous sample to subtract.
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, wanted);
    system.refresh_memory();
    let total_memory = system.total_memory();
    // `available_parallelism` rather than sysinfo's CPU list: it is the number
    // the operating system will actually schedule us on, and it needs no
    // second refresh of its own. Never zero, but the fallback keeps the
    // division honest if it ever were.
    let cores = thread::available_parallelism()
        .map_or(1, |n| n.get())
        .max(1);

    loop {
        thread::sleep(INTERVAL);

        system.refresh_processes_specifics(ProcessesToUpdate::All, true, wanted);

        let parents: HashMap<Pid, Option<Pid>> = system
            .processes()
            .iter()
            .map(|(&pid, process)| (pid, process.parent()))
            .collect();
        let ours = family(&parents, us);

        let mut cpu = 0.0;
        let mut memory = 0;
        for pid in &ours {
            if let Some(process) = system.process(*pid) {
                cpu += process.cpu_usage();
                memory += process.memory();
            }
        }

        emit(Usage {
            cpu: cpu / cores as f32,
            memory,
            total_memory,
            processes: ours.len(),
            cores,
        });
    }
}

/// Starts the sampler on a thread of its own.
///
/// Its own thread rather than Tauri's async runtime: the work is a blocking
/// walk of the process table once a second, and it has no business sharing a
/// pool with the downloads.
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
        assert!(usage.cores >= 1);
        // A share of the machine, not the per-core figure the operating
        // system reports: a single-threaded test cannot exceed one core, so
        // even a busy one stays well under 100 on any real machine.
        assert!(usage.cpu >= 0.0 && usage.cpu <= 100.0, "{}", usage.cpu);
    }

    /// The whole point of walking the tree: a whisper worker is a child
    /// process, so a sampler that reported only our own process would report
    /// close to nothing exactly while the machine is busiest. A busy child
    /// stands in for one — `sh` rather than a real sidecar, which would need a
    /// model and a minute.
    #[test]
    fn counts_a_child_process() {
        // Started before the sampler, so the first sample it sends already
        // covers a full interval of the child being busy.
        let mut child = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("while :; do :; done")
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
        // Back into cores: the child saturates one, and the test process is
        // idle, so anything under half a core means the child was not counted.
        // Phrased in cores rather than in percent because the percentage
        // depends on how many the machine has.
        let cores_busy = usage.cpu * usage.cores as f32 / 100.0;
        assert!(cores_busy > 0.5, "the child's CPU is missing: {usage:?}");
    }

    /// Our own pid is always in the table in practice; if a refresh ever
    /// misses it, the answer is "nothing", not "the whole machine".
    #[test]
    fn an_unknown_root_owns_nothing() {
        let parents = HashMap::from([(pid(1), None), (pid(20), Some(pid(1)))]);

        assert!(family(&parents, pid(10)).is_empty());
    }
}

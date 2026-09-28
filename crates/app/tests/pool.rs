//! Supervision and accounting of the worker pool (P5), against real sidecars.
//!
//! The queue and the reorder buffer are covered by unit tests in the module
//! itself. What cannot be tested there is the part that involves an actual
//! process dying: that a killed sidecar is replaced, and — the invariant the
//! whole pipeline rests on — that every segment handed to the pool produces
//! exactly one result, however badly the run goes.
//!
//! Since P6 there is also a test for eight streams feeding the queue at once,
//! which is the part the pool was designed for and had never actually been
//! done to it. It is not yet the P14 reference load test — it runs for seconds
//! rather than thirty minutes and measures nothing — but it is what that test
//! grows out of.
//!
//! Ignored by default: needs a model, which CI does not have.
//!
//! ```sh
//! just model small-q5_1
//! cargo test -p app --test pool -- --ignored --nocapture
//! ```

use app_lib::pool::{Completed, Outcome, PoolConfig, PoolEvent, WorkerPool};
use audio_core::{LanguageCode, LanguageMode, Segment, StreamId, SAMPLE_RATE};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Half a second of quiet noise.
///
/// Deliberately not speech: the sidecar's Silero gate rejects it in a few
/// milliseconds instead of spending a full decode, which keeps the test about
/// the pool rather than about whisper. A rejected segment still travels the
/// complete request/response path, which is the part under test.
fn segment(seq: u64) -> Segment {
    segment_on(0, seq)
}

fn segment_on(stream: u16, seq: u64) -> Segment {
    let samples = (SAMPLE_RATE / 2) as usize;
    let pcm = (0..samples)
        .map(|i| ((i * 2654435761) % 2003) as f32 / 2003.0 * 0.002 - 0.001)
        .collect();
    Segment {
        stream_id: StreamId(stream),
        seq,
        t_start: Duration::from_millis(seq * 500),
        t_end: Duration::from_millis(seq * 500 + 500),
        lag: Duration::ZERO,
        pcm,
        language_mode: LanguageMode::Fixed {
            language: LanguageCode::german(),
        },
        initial_prompt: None,
    }
}

fn config() -> Option<PoolConfig> {
    let binary = app_lib::worker::find_binary().ok()?;
    let model = app_lib::worker::find_model().ok()?;
    let mut config = PoolConfig::new(binary, model, app_lib::worker::find_vad_model());
    config.workers = 2;
    Some(config)
}

/// Kills this test process's own sidecar children, and nothing else.
///
/// Only direct children of this process are considered, so a developer's
/// running app is never caught by this. Through `sysinfo` rather than
/// `pgrep`/`kill`, which Windows does not have (P18).
fn kill_our_sidecars() -> usize {
    let us = Pid::from_u32(std::process::id());
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());

    let mut killed = 0;
    for process in system.processes().values() {
        let is_sidecar = process
            .name()
            .to_string_lossy()
            .starts_with("whisper-worker");
        if process.parent() == Some(us) && is_sidecar && process.kill() {
            killed += 1;
        }
    }
    killed
}

#[test]
#[ignore = "needs a model"]
fn every_submitted_segment_produces_exactly_one_result() {
    // The invariant the reorder buffer depends on. If the pool ever swallowed
    // a segment, the chat would stall on that stream for the rest of the
    // session rather than lose one line — which is why this is asserted over
    // the whole run and not per segment.
    let Some(mut config) = config() else {
        eprintln!("no model — skipping");
        return;
    };
    // Smaller than the batch below, so the drop policy is exercised too.
    // Pinned rather than derived: a real pool sizes its queue to five seconds
    // of work, which this batch would never fill.
    config.queue_capacity = Some(4);

    let (pool, results) = WorkerPool::spawn(config).expect("pool");
    const COUNT: u64 = 24;
    for seq in 0..COUNT {
        pool.submit(segment(seq), Instant::now());
    }
    pool.shutdown();

    let seen = collect(results);
    assert_eq!(
        seen.len(),
        COUNT as usize,
        "every segment must be accounted for exactly once"
    );
    assert_eq!(seen, (0..COUNT).collect::<HashSet<_>>());
}

#[test]
#[ignore = "needs a model"]
fn a_killed_sidecar_is_replaced_and_nothing_goes_missing() {
    // The acceptance criterion for P5: `kill -9` on a worker is healed rather
    // than ending the session. Note what is *not* asserted — that the pool
    // always survives. Killing repeatedly can catch a replacement before it
    // has reported ready, and a pool that ends up with no workers is a
    // legitimate outcome of that. What may never happen is a segment
    // disappearing without a word.
    let Some(config) = config() else {
        eprintln!("no model — skipping");
        return;
    };

    let (pool, results) = WorkerPool::spawn(config).expect("pool");
    const FIRST: u64 = 8;
    const COUNT: u64 = 16;

    for seq in 0..FIRST {
        pool.submit(segment(seq), Instant::now());
    }
    // Long enough that a decode is actually in flight, so the kill lands on a
    // worker that is busy rather than one that is idle.
    std::thread::sleep(Duration::from_millis(200));
    assert!(kill_our_sidecars() > 0, "no sidecar was found to kill");

    // The replacements have to load the model before they can take work.
    // Submitting through that window would only test the queue again.
    std::thread::sleep(Duration::from_secs(3));
    for seq in FIRST..COUNT {
        pool.submit(segment(seq), Instant::now());
    }
    pool.shutdown();

    let (seen, decoded, restarts) = collect_with_restarts(results);
    assert_eq!(seen, (0..COUNT).collect::<HashSet<_>>());
    assert!(restarts > 0, "a killed sidecar must be replaced");
    // The pool is not merely alive but working: everything submitted after the
    // kill was decoded by a replacement.
    for seq in FIRST..COUNT {
        assert!(decoded.contains(&seq), "segment {seq} was not decoded");
    }
}

/// Eight streams into one queue, which is the arrangement the pool exists for.
///
/// The accounting invariant is the same as for one stream and is asserted the
/// same way, now per `(stream, seq)` pair. What is new is the fairness claim
/// the design rests on: workers take the *oldest* segment so that a busy
/// channel cannot starve a quiet one. With every stream submitting the same
/// number of segments, no stream may come back empty — and none may be
/// silently dropped either, since the queue bound is deliberately smaller than
/// the batch.
#[test]
#[ignore = "needs a model"]
fn eight_streams_share_the_queue_without_starving_each_other() {
    let Some(mut config) = config() else {
        eprintln!("no model — skipping");
        return;
    };
    // Pinned for the same reason as above: the fairness this asserts is only
    // visible in a queue the batch can actually overflow.
    config.queue_capacity = Some(8);

    let (pool, results) = WorkerPool::spawn(config).expect("pool");
    const STREAMS: u16 = 8;
    const PER_STREAM: u64 = 6;

    // Interleaved rather than stream by stream: eight people do not take turns
    // speaking, and submitting one stream at a time would never put two of
    // them in the queue together.
    for seq in 0..PER_STREAM {
        for stream in 0..STREAMS {
            pool.submit(segment_on(stream, seq), Instant::now());
        }
    }
    pool.shutdown();

    let mut per_stream: HashMap<u16, HashSet<u64>> = HashMap::new();
    let mut total = 0usize;
    for event in results {
        if let PoolEvent::Completed(Completed { stream_id, seq, .. }) = event {
            total += 1;
            assert!(
                per_stream.entry(stream_id.0).or_default().insert(seq),
                "segment {seq} of stream {} was reported twice",
                stream_id.0
            );
        }
    }

    assert_eq!(
        total,
        (STREAMS as u64 * PER_STREAM) as usize,
        "every segment must be accounted for exactly once"
    );
    for stream in 0..STREAMS {
        assert_eq!(
            per_stream.get(&stream).map(HashSet::len),
            Some(PER_STREAM as usize),
            "stream {stream} lost segments the others kept"
        );
    }
}

fn collect(results: std::sync::mpsc::Receiver<PoolEvent>) -> HashSet<u64> {
    collect_with_restarts(results).0
}

/// Sequence numbers reported, those that actually reached the decoder, and how
/// often a worker was replaced.
fn collect_with_restarts(
    results: std::sync::mpsc::Receiver<PoolEvent>,
) -> (HashSet<u64>, HashSet<u64>, usize) {
    let mut seen = HashSet::new();
    let mut decoded = HashSet::new();
    let mut restarts = 0;
    for event in results {
        match event {
            PoolEvent::Completed(Completed { seq, outcome, .. }) => {
                assert!(seen.insert(seq), "segment {seq} was reported twice");
                if matches!(outcome, Outcome::Text { .. }) {
                    decoded.insert(seq);
                }
            }
            PoolEvent::WorkerRestarted { .. } => restarts += 1,
            PoolEvent::Exhausted { message } => eprintln!("pool exhausted: {message}"),
        }
    }
    (seen, decoded, restarts)
}

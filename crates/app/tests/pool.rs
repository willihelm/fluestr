//! Supervision and accounting of the worker pool (P5), against real sidecars.
//!
//! The queue and the reorder buffer are covered by unit tests in the module
//! itself. What cannot be tested there is the part that involves an actual
//! process dying: that a killed sidecar is replaced, and — the invariant the
//! whole pipeline rests on — that every segment handed to the pool produces
//! exactly one result, however badly the run goes.
//!
//! Ignored by default: needs a model, which CI does not have.
//!
//! ```sh
//! just model small-q5_1
//! cargo test -p app --test pool -- --ignored --nocapture
//! ```

use app_lib::pool::{Completed, Outcome, PoolConfig, PoolEvent, WorkerPool};
use audio_core::{LanguageCode, LanguageMode, Segment, StreamId, SAMPLE_RATE};
use std::collections::HashSet;
use std::process::Command;
use std::time::{Duration, Instant};

/// Half a second of quiet noise.
///
/// Deliberately not speech: the sidecar's Silero gate rejects it in a few
/// milliseconds instead of spending a full decode, which keeps the test about
/// the pool rather than about whisper. A rejected segment still travels the
/// complete request/response path, which is the part under test.
fn segment(seq: u64) -> Segment {
    let samples = (SAMPLE_RATE / 2) as usize;
    let pcm = (0..samples)
        .map(|i| ((i * 2654435761) % 2003) as f32 / 2003.0 * 0.002 - 0.001)
        .collect();
    Segment {
        stream_id: StreamId(0),
        seq,
        t_start: Duration::from_millis(seq * 500),
        t_end: Duration::from_millis(seq * 500 + 500),
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
/// `pgrep -P` restricts the search to direct children, so a developer's
/// running app is never caught by this.
fn kill_our_sidecars() -> usize {
    let Ok(output) = Command::new("pgrep")
        .args(["-P", &std::process::id().to_string(), "whisper-worker"])
        .output()
    else {
        return 0;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let pids: Vec<&str> = stdout.split_whitespace().collect();
    for pid in &pids {
        let _ = Command::new("kill").args(["-9", pid]).status();
    }
    pids.len()
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
    config.queue_capacity = 4;

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

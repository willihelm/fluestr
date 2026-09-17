//! Worker pool, bounded queue and supervision (P5).
//!
//! A [`crate::worker::Worker`] on its own is a blocking request/response pipe
//! to one sidecar. This module turns a handful of them into something that can
//! carry eight streams:
//!
//! - **Several workers on one queue.** Decodes run in parallel, and no stream
//!   owns a worker — a channel that is quiet costs nothing.
//! - **A hard bound with a drop policy.** The queue cannot grow without limit,
//!   and what it gives up it reports.
//! - **Supervision.** A worker that crashes or wedges is replaced, and the
//!   segment it took with it is tried once more.
//!
//! # Why the pool is not one worker per stream
//!
//! Eight whisper instances contend for the same Metal queue and make each
//! other slower; the total throughput does not improve, only the number of
//! processes holding a model does. The pool size is therefore decoupled from
//! the stream count — see [`DEFAULT_WORKERS`].
//!
//! # Why the *oldest* segment is the one that gets dropped
//!
//! Under overload something has to go, and for a live transcript the stalest
//! segment is worth the least: by the time the queue has backed up far enough
//! to matter, the conversation has moved on. Dropping the newest would keep
//! the log tidy and useless. Note that this is the opposite end from the one
//! decodes are taken from — workers pop the oldest first, so that a busy
//! stream cannot starve a quiet one.
//!
//! Nothing here drops anything quietly. Every segment the pool accepts
//! produces exactly one [`Completed`], including the ones it gives up on —
//! which is also what keeps [`Reorder`] from stalling behind a segment that
//! never produced text.

use crate::worker::{Killer, Ready, Worker};
use anyhow::{bail, Context, Result};
use audio_core::{Segment, StreamId};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use whisper_protocol::Response;

/// Workers in the pool unless configured otherwise.
///
/// Two, not eight: parallel whisper instances share one GPU and start to slow
/// each other down well before the stream count is reached. This is the number
/// to revisit against the P14 reference load test, not by intuition.
pub const DEFAULT_WORKERS: usize = 2;

/// Segments that may wait for a worker before the pool starts dropping.
///
/// At ~320 ms a decode and two workers that is roughly five seconds of
/// backlog. A transcript that is five seconds late has already lost most of
/// its value on an intercom channel, so a deeper queue would not buy anything
/// an operator could use.
pub const DEFAULT_QUEUE_CAPACITY: usize = 32;

/// How long one decode may take before its worker counts as wedged.
///
/// A six-second segment — the segmenter's maximum — decodes in ~320 ms on
/// `small-q5_1`. Two orders of magnitude of headroom means this catches a
/// process that has stopped answering, never one that is merely slow on a
/// larger model or without a GPU.
pub const DEFAULT_DECODE_TIMEOUT: Duration = Duration::from_secs(30);

/// CPU threads handed to each sidecar.
pub const DEFAULT_THREADS_PER_WORKER: usize = 4;

/// How often the watchdog looks for an overdue decode.
const WATCHDOG_INTERVAL: Duration = Duration::from_millis(250);

/// How long [`WorkerPool::shutdown`] lets the queue drain before giving up on
/// what is left in it.
///
/// Those segments are words somebody actually spoke; discarding them the
/// moment Stop is pressed loses the end of every session, which is the same
/// silent loss the queue's drop policy exists to avoid. The bound is what
/// keeps Stop from hanging behind a backlog that is not clearing — about what
/// a full queue needs at the measured decode rate.
const DRAIN_GRACE: Duration = Duration::from_secs(5);

/// How often the drain is rechecked. A shutdown path, so polling is cheaper
/// than a second condvar on the hot one.
const DRAIN_POLL: Duration = Duration::from_millis(20);

/// How often a dead worker is replaced before the pool gives up on it.
///
/// One failed start is not a reason to shrink the pool for the rest of the
/// session. A replacement can be killed while it is still loading its model —
/// by the watchdog, or by whatever took its predecessor down — and the next
/// attempt a moment later comes up fine. Giving up on the first failure turns
/// a passing problem into a permanently smaller pool.
const RESPAWN_ATTEMPTS: u32 = 3;

/// Long enough for a model load to finish, so the retries are not three
/// attempts at the same bad moment.
const RESPAWN_BACKOFF: Duration = Duration::from_millis(500);

/// How often the watchdog checks whether it should stop, so that shutting the
/// pool down does not wait out a full [`WATCHDOG_INTERVAL`].
const WATCHDOG_TICK: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub binary: PathBuf,
    pub model: PathBuf,
    /// Absent means the sidecar transcribes noise and silence too; it warns
    /// about that itself rather than refusing to start.
    pub vad_model: Option<PathBuf>,
    pub workers: usize,
    pub threads_per_worker: usize,
    pub queue_capacity: usize,
    pub decode_timeout: Duration,
}

impl PoolConfig {
    pub fn new(binary: PathBuf, model: PathBuf, vad_model: Option<PathBuf>) -> Self {
        Self {
            binary,
            model,
            vad_model,
            workers: DEFAULT_WORKERS,
            threads_per_worker: DEFAULT_THREADS_PER_WORKER,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            decode_timeout: DEFAULT_DECODE_TIMEOUT,
        }
    }
}

/// What became of one segment.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// The sidecar decoded it. Whether the text belongs in the chat is a
    /// separate question, decided by the session.
    Text {
        text: String,
        detected_language: Option<String>,
        no_speech_probability: f32,
        decode_ms: u32,
    },
    /// The sidecar rejected this one segment, or it was lost together with the
    /// worker that was decoding it.
    Failed(String),
    /// The queue was full and this was the stalest segment in it.
    Overloaded,
}

/// One segment's result, on its way back to the session.
#[derive(Debug, Clone)]
pub struct Completed {
    pub stream_id: StreamId,
    pub seq: u64,
    pub t_start: Duration,
    pub t_end: Duration,
    /// When the segmenter closed this segment — the zero point of the latency
    /// the chat shows.
    pub closed_at: Instant,
    pub outcome: Outcome,
}

/// Everything the pool tells the session about.
#[derive(Debug, Clone)]
pub enum PoolEvent {
    Completed(Completed),
    /// A worker died and was replaced. The only thing that explains a gap in
    /// the transcript, so it is reported rather than only logged.
    WorkerRestarted {
        worker: usize,
        reason: String,
    },
    /// The last worker could not be replaced. Nothing will be decoded after
    /// this, so the session is over.
    Exhausted {
        message: String,
    },
}

struct Job {
    segment: Segment,
    closed_at: Instant,
    /// Counts how often this segment has been handed to a worker that then
    /// died under it. A segment that wedges a sidecar would otherwise take
    /// down every worker in turn.
    attempt: u32,
}

impl Job {
    fn completed(&self, outcome: Outcome) -> Completed {
        Completed {
            stream_id: self.segment.stream_id,
            seq: self.segment.seq,
            t_start: self.segment.t_start,
            t_end: self.segment.t_end,
            closed_at: self.closed_at,
            outcome,
        }
    }
}

/// A job the queue would not take, and why.
enum Rejected {
    /// Evicted to stay within the bound.
    Overloaded(Job),
    /// The session stopped while this one was still waiting.
    Closed(Job),
}

impl Rejected {
    fn into_completed(self) -> Completed {
        match self {
            Rejected::Overloaded(job) => job.completed(Outcome::Overloaded),
            Rejected::Closed(job) => job.completed(Outcome::Failed(
                "the session stopped before this segment was decoded".into(),
            )),
        }
    }
}

struct Queue {
    inner: Mutex<QueueInner>,
    ready: Condvar,
    capacity: usize,
}

struct QueueInner {
    jobs: VecDeque<Job>,
    closed: bool,
}

impl Queue {
    fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(QueueInner {
                jobs: VecDeque::new(),
                closed: false,
            }),
            ready: Condvar::new(),
            capacity: capacity.max(1),
        }
    }

    /// Adds a job and returns whatever had to give way for it.
    ///
    /// `front` is for a segment being retried after its worker died: it is
    /// older than everything already waiting, and putting it at the back would
    /// break the oldest-first ordering the workers rely on.
    #[must_use]
    fn push(&self, job: Job, front: bool) -> Vec<Rejected> {
        let mut inner = self.inner.lock().expect("queue poisoned");
        if inner.closed {
            return vec![Rejected::Closed(job)];
        }

        if front {
            inner.jobs.push_front(job);
        } else {
            inner.jobs.push_back(job);
        }

        let mut rejected = Vec::new();
        while inner.jobs.len() > self.capacity {
            let stalest = inner.jobs.pop_front().expect("over capacity, so non-empty");
            rejected.push(Rejected::Overloaded(stalest));
        }

        self.ready.notify_one();
        rejected
    }

    /// Blocks until a job is available. `None` once the queue is closed and
    /// drained, which is how a worker thread learns that the session is over.
    fn pop(&self) -> Option<Job> {
        let mut inner = self.inner.lock().expect("queue poisoned");
        loop {
            if let Some(job) = inner.jobs.pop_front() {
                return Some(job);
            }
            if inner.closed {
                return None;
            }
            inner = self.ready.wait(inner).expect("queue poisoned");
        }
    }

    /// Closes the queue and hands back everything still waiting.
    fn close(&self) -> Vec<Rejected> {
        let mut inner = self.inner.lock().expect("queue poisoned");
        inner.closed = true;
        let abandoned = inner.jobs.drain(..).map(Rejected::Closed).collect();
        self.ready.notify_all();
        abandoned
    }

    fn is_closed(&self) -> bool {
        self.inner.lock().expect("queue poisoned").closed
    }

    fn depth(&self) -> usize {
        self.inner.lock().expect("queue poisoned").jobs.len()
    }

    /// Blocks until nothing is waiting any more, or the deadline passes.
    /// Reports whether it drained.
    fn wait_until_empty(&self, deadline: Instant) -> bool {
        loop {
            if self.depth() == 0 {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(DRAIN_POLL);
        }
    }
}

/// The watchdog's view of one worker.
struct WorkerSlot {
    inflight: Mutex<Option<InFlight>>,
    /// Replaced whenever the worker is, so the watchdog never holds a handle
    /// to a process that has already been reaped.
    killer: Mutex<Option<Killer>>,
}

struct InFlight {
    deadline: Instant,
    /// Set by the watchdog. Tells the worker thread that the pipe closing was
    /// its own doing, not the sidecar's, so the log says which.
    killed: bool,
}

impl WorkerSlot {
    fn new(killer: Killer) -> Self {
        Self {
            inflight: Mutex::new(None),
            killer: Mutex::new(Some(killer)),
        }
    }

    fn begin(&self, deadline: Instant) {
        *self.inflight.lock().expect("slot poisoned") = Some(InFlight {
            deadline,
            killed: false,
        });
    }

    /// Clears the deadline and reports whether the watchdog intervened.
    fn end(&self) -> bool {
        self.inflight
            .lock()
            .expect("slot poisoned")
            .take()
            .is_some_and(|f| f.killed)
    }

    fn replace_killer(&self, killer: Killer) {
        *self.killer.lock().expect("slot poisoned") = Some(killer);
    }
}

pub struct WorkerPool {
    queue: Arc<Queue>,
    /// Dropped at the end of [`WorkerPool::shutdown`]. The worker threads hold
    /// the only other senders, so this is what finally closes the results
    /// channel — and the session's aggregator loop ends when it does.
    outcomes: Mutex<Option<Sender<PoolEvent>>>,
    /// Behind a lock so that shutting the pool down needs only `&self`. The
    /// processing thread holds an `Arc` to the pool for as long as it runs, so
    /// there is no moment at which the session owns it outright.
    threads: Mutex<Vec<JoinHandle<()>>>,
    watchdog: Mutex<Option<JoinHandle<()>>>,
    watchdog_stop: Arc<AtomicBool>,
    ready: Ready,
    workers: usize,
}

impl WorkerPool {
    /// Starts the workers and returns the channel their results arrive on.
    ///
    /// The first worker is started synchronously: it is the one whose failure
    /// means the session cannot run at all, and its error — a missing model, a
    /// sidecar that will not execute — belongs to the caller rather than to a
    /// log line. It also absorbs the one-off Metal shader compilation alone,
    /// so the rest find a warm cache. Those are started in parallel, and one
    /// that does not come up costs throughput rather than the session.
    pub fn spawn(config: PoolConfig) -> Result<(Self, Receiver<PoolEvent>)> {
        if config.workers == 0 {
            bail!("a pool needs at least one worker");
        }
        let config = Arc::new(config);

        let first = Worker::spawn(
            &config.binary,
            &config.model,
            config.vad_model.as_deref(),
            config.threads_per_worker,
        )
        .context("starting the first sidecar")?;
        let ready = first.ready().clone();

        let rest: Vec<JoinHandle<Result<Worker>>> = (1..config.workers)
            .map(|index| {
                let config = Arc::clone(&config);
                std::thread::Builder::new()
                    .name(format!("worker-spawn-{index}"))
                    .spawn(move || {
                        Worker::spawn(
                            &config.binary,
                            &config.model,
                            config.vad_model.as_deref(),
                            config.threads_per_worker,
                        )
                    })
                    .expect("spawning a worker-start thread")
            })
            .collect();

        let mut workers = vec![first];
        for handle in rest {
            match handle.join() {
                Ok(Ok(worker)) => workers.push(worker),
                Ok(Err(err)) => {
                    tracing::warn!(error = ?err, "a sidecar did not start — the pool runs smaller")
                }
                Err(_) => tracing::warn!("a worker-start thread panicked"),
            }
        }
        tracing::info!(
            workers = workers.len(),
            requested = config.workers,
            model = %ready.model,
            gpu = ready.gpu,
            "worker pool up"
        );

        let queue = Arc::new(Queue::new(config.queue_capacity));
        let (outcomes, results) = mpsc::channel();
        let live = Arc::new(AtomicUsize::new(workers.len()));

        let mut slots = Vec::with_capacity(workers.len());
        let mut threads = Vec::with_capacity(workers.len());
        for (index, worker) in workers.into_iter().enumerate() {
            let slot = Arc::new(WorkerSlot::new(worker.killer()));
            slots.push(Arc::clone(&slot));
            threads.push(spawn_worker_thread(
                index,
                worker,
                Arc::clone(&config),
                Arc::clone(&queue),
                slot,
                outcomes.clone(),
                Arc::clone(&live),
            ));
        }

        let watchdog_stop = Arc::new(AtomicBool::new(false));
        let watchdog = spawn_watchdog(slots, Arc::clone(&watchdog_stop));

        let workers = threads.len();
        Ok((
            Self {
                queue,
                outcomes: Mutex::new(Some(outcomes)),
                threads: Mutex::new(threads),
                watchdog: Mutex::new(Some(watchdog)),
                watchdog_stop,
                ready,
                workers,
            },
            results,
        ))
    }

    pub fn ready(&self) -> &Ready {
        &self.ready
    }

    pub fn workers(&self) -> usize {
        self.workers
    }

    /// Segments currently waiting for a worker. Feeds the status bar in P8.
    pub fn queue_depth(&self) -> usize {
        self.queue.depth()
    }

    /// Queues a segment. Never blocks — the processing thread that calls this
    /// also has a ring buffer to drain, and making it wait here is how audio
    /// gets lost while the app is busy transcribing the audio before it.
    pub fn submit(&self, segment: Segment, closed_at: Instant) {
        let rejected = self.queue.push(
            Job {
                segment,
                closed_at,
                attempt: 0,
            },
            false,
        );
        self.report(rejected);
    }

    fn report(&self, rejected: Vec<Rejected>) {
        if rejected.is_empty() {
            return;
        }
        let outcomes = self.outcomes.lock().expect("pool poisoned");
        for item in rejected {
            let completed = item.into_completed();
            tracing::warn!(
                stream_id = completed.stream_id.0,
                seq = completed.seq,
                "segment dropped before it was decoded"
            );
            if let Some(tx) = outcomes.as_ref() {
                let _ = tx.send(PoolEvent::Completed(completed));
            }
        }
    }

    /// Closes the queue, lets the workers finish what they hold, and stops
    /// their sidecars. Segments still waiting are reported, not forgotten.
    ///
    /// The backlog is decoded first, within [`DRAIN_GRACE`]. Stop is pressed
    /// after the last thing worth hearing was said, not before it, so throwing
    /// the queue away here would reliably lose the end of every session.
    ///
    /// Idempotent, and takes `&self` so that [`Drop`] can run it too: a
    /// session that fails to start after the pool is up would otherwise leave
    /// the worker threads blocked on an empty queue forever, each holding a
    /// sidecar with a loaded model.
    pub fn shutdown(&self) {
        if !self.queue.wait_until_empty(Instant::now() + DRAIN_GRACE) {
            tracing::warn!(
                queued = self.queue.depth(),
                "stopping with segments still waiting — they will be reported as lost"
            );
        }
        let abandoned = self.queue.close();
        self.report(abandoned);

        self.watchdog_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.watchdog.lock().expect("pool poisoned").take() {
            let _ = handle.join();
        }
        for handle in self.threads.lock().expect("pool poisoned").drain(..) {
            let _ = handle.join();
        }

        // Last, and only once the workers are gone: their senders and this one
        // are what hold the results channel open.
        self.outcomes.lock().expect("pool poisoned").take();
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Replaces a dead worker. See [`RESPAWN_ATTEMPTS`].
fn respawn(config: &PoolConfig, index: usize) -> Result<Worker> {
    let mut last = None;
    for attempt in 0..RESPAWN_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(RESPAWN_BACKOFF);
        }
        match Worker::spawn(
            &config.binary,
            &config.model,
            config.vad_model.as_deref(),
            config.threads_per_worker,
        ) {
            Ok(worker) => return Ok(worker),
            Err(err) => {
                tracing::warn!(worker = index, attempt, error = %err, "restarting a sidecar failed");
                last = Some(err);
            }
        }
    }
    Err(last.expect("at least one attempt was made"))
}

fn spawn_worker_thread(
    index: usize,
    worker: Worker,
    config: Arc<PoolConfig>,
    queue: Arc<Queue>,
    slot: Arc<WorkerSlot>,
    outcomes: Sender<PoolEvent>,
    live: Arc<AtomicUsize>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name(format!("decode-{index}"))
        .spawn(move || worker_loop(index, worker, config, queue, slot, outcomes, live))
        .expect("spawning a decode thread")
}

fn worker_loop(
    index: usize,
    mut worker: Worker,
    config: Arc<PoolConfig>,
    queue: Arc<Queue>,
    slot: Arc<WorkerSlot>,
    outcomes: Sender<PoolEvent>,
    live: Arc<AtomicUsize>,
) {
    while let Some(mut job) = queue.pop() {
        slot.begin(Instant::now() + config.decode_timeout);
        let result = worker.transcribe(&job.segment);
        let killed = slot.end();

        let outcome = match result {
            Ok(Response::Transcript {
                text,
                detected_language,
                no_speech_probability,
                decode_ms,
                ..
            }) => Outcome::Text {
                text,
                detected_language,
                no_speech_probability,
                decode_ms,
            },
            // A segment-level complaint. The sidecar is still alive, so only
            // this one segment is lost.
            Ok(Response::Error { message, .. }) => Outcome::Failed(message),
            Ok(other) => Outcome::Failed(format!("unexpected response: {other:?}")),
            Err(err) => {
                // The pipe is gone: the sidecar crashed under this segment, or
                // the watchdog killed it for running past its deadline. Both
                // are healed the same way, which is the whole reason the
                // watchdog works by killing rather than by signalling.
                let reason = if killed {
                    format!(
                        "decode ran past {} s and the worker was killed",
                        config.decode_timeout.as_secs()
                    )
                } else {
                    format!("sidecar lost: {err}")
                };
                tracing::warn!(
                    worker = index,
                    stream_id = job.segment.stream_id.0,
                    seq = job.segment.seq,
                    reason,
                    "replacing a worker"
                );

                worker.shutdown();
                match respawn(&config, index) {
                    Ok(replacement) => {
                        slot.replace_killer(replacement.killer());
                        worker = replacement;
                        let _ = outcomes.send(PoolEvent::WorkerRestarted {
                            worker: index,
                            reason: reason.clone(),
                        });

                        // Once. A segment that wedges a sidecar would
                        // otherwise take every worker in the pool down in
                        // turn, and a pool that kills itself on one bad
                        // segment is worse than a lost segment.
                        if job.attempt == 0 {
                            job.attempt += 1;
                            let rejected = queue.push(job, true);
                            for item in rejected {
                                let _ = outcomes.send(PoolEvent::Completed(item.into_completed()));
                            }
                            continue;
                        }
                        Outcome::Failed(format!("{reason} (already retried once)"))
                    }
                    Err(spawn_err) => {
                        let _ = outcomes
                            .send(PoolEvent::Completed(job.completed(Outcome::Failed(reason))));
                        // Last one out reports it: with no worker left nothing
                        // will ever be decoded again, and a session that
                        // silently stops transcribing looks exactly like a
                        // quiet room. Not while shutting down, though — there
                        // an empty pool is the goal, and reporting it would
                        // turn a clean Stop into a failed session.
                        if live.fetch_sub(1, Ordering::AcqRel) == 1 && !queue.is_closed() {
                            let _ = outcomes.send(PoolEvent::Exhausted {
                                message: format!("no sidecar could be restarted: {spawn_err}"),
                            });
                        }
                        return;
                    }
                }
            }
        };

        let _ = outcomes.send(PoolEvent::Completed(job.completed(outcome)));
    }

    worker.shutdown();
    live.fetch_sub(1, Ordering::AcqRel);
}

/// Kills workers whose decode has run past its deadline.
///
/// It cannot interrupt them any other way: the decode is blocked in a read on
/// a pipe that a wedged sidecar will never write to again.
fn spawn_watchdog(slots: Vec<Arc<WorkerSlot>>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("decode-watchdog".into())
        .spawn(move || {
            let mut last_sweep = Instant::now();
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(WATCHDOG_TICK);
                if last_sweep.elapsed() < WATCHDOG_INTERVAL {
                    continue;
                }
                last_sweep = Instant::now();

                for (index, slot) in slots.iter().enumerate() {
                    let mut inflight = slot.inflight.lock().expect("slot poisoned");
                    let Some(state) = inflight.as_mut() else {
                        continue;
                    };
                    if state.killed || Instant::now() < state.deadline {
                        continue;
                    }
                    state.killed = true;
                    drop(inflight);

                    tracing::warn!(worker = index, "decode overdue — killing the sidecar");
                    if let Some(killer) = slot.killer.lock().expect("slot poisoned").as_ref() {
                        killer.kill();
                    }
                }
            }
        })
        .expect("spawning the watchdog thread")
}

/// Restores per-stream ordering of results.
///
/// Workers finish out of order — two segments of the same stream can be in
/// flight at once, and a short one overtakes a long one. Read out of order a
/// chat log is actively misleading, so results wait here until their
/// predecessors have arrived.
///
/// This only terminates because the pool produces exactly one [`Completed`]
/// per segment it accepts, including for the ones it drops. A segment that
/// produced nothing at all would hold back every later one on its stream for
/// the rest of the session.
#[derive(Default)]
pub struct Reorder {
    streams: HashMap<StreamId, StreamOrder>,
}

#[derive(Default)]
struct StreamOrder {
    next: u64,
    pending: BTreeMap<u64, Completed>,
}

impl Reorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes one result and returns everything that is now in order.
    pub fn accept(&mut self, completed: Completed) -> Vec<Completed> {
        let order = self.streams.entry(completed.stream_id).or_default();

        // Cannot happen while every segment yields exactly one result, but a
        // duplicate would sit in `pending` forever and stall the stream behind
        // it. Passing it straight through trades an impossible deadlock for a
        // harmless out-of-order line.
        if completed.seq < order.next {
            return vec![completed];
        }

        order.pending.insert(completed.seq, completed);
        let mut ready = Vec::new();
        while let Some(next) = order.pending.remove(&order.next) {
            order.next += 1;
            ready.push(next);
        }
        ready
    }

    /// Everything still held back, in sequence order per stream.
    ///
    /// The end of a session is the one moment a gap is expected: the pool has
    /// stopped, so the missing predecessor is never going to arrive, and
    /// holding its successors back would lose the last words spoken.
    pub fn drain(&mut self) -> Vec<Completed> {
        let mut rest: Vec<Completed> = self
            .streams
            .drain()
            .flat_map(|(_, order)| order.pending.into_values())
            .collect();
        rest.sort_by_key(|c| (c.stream_id, c.seq));
        rest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_core::LanguageMode;

    fn job(stream: u16, seq: u64) -> Job {
        Job {
            segment: Segment {
                stream_id: StreamId(stream),
                seq,
                t_start: Duration::from_secs(seq),
                t_end: Duration::from_secs(seq + 1),
                pcm: Vec::new(),
                language_mode: LanguageMode::default(),
                initial_prompt: None,
            },
            closed_at: Instant::now(),
            attempt: 0,
        }
    }

    fn completed(stream: u16, seq: u64) -> Completed {
        job(stream, seq).completed(Outcome::Text {
            text: format!("segment {seq}"),
            detected_language: Some("de".into()),
            no_speech_probability: 0.01,
            decode_ms: 320,
        })
    }

    fn text_of(c: &Completed) -> &str {
        match &c.outcome {
            Outcome::Text { text, .. } => text,
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn the_queue_hands_out_the_oldest_segment_first() {
        // Oldest-first is what keeps a busy stream from starving a quiet one:
        // every segment waits behind the ones that arrived before it,
        // regardless of which stream produced them.
        let queue = Queue::new(8);
        for seq in 0..3 {
            assert!(queue.push(job(0, seq), false).is_empty());
        }
        assert!(queue.push(job(1, 0), false).is_empty());

        assert_eq!(queue.pop().unwrap().segment.seq, 0);
        assert_eq!(queue.pop().unwrap().segment.seq, 1);
        assert_eq!(queue.pop().unwrap().segment.seq, 2);
        assert_eq!(queue.pop().unwrap().segment.stream_id, StreamId(1));
    }

    #[test]
    fn overflow_drops_the_stalest_segment_and_says_so() {
        // The bound is the point: without it a backlog grows until the process
        // runs out of memory. Which end gives way is the second decision — the
        // oldest segment is the one whose transcript is worth the least by the
        // time anyone would see it.
        let queue = Queue::new(2);
        assert!(queue.push(job(0, 0), false).is_empty());
        assert!(queue.push(job(0, 1), false).is_empty());

        let rejected = queue.push(job(0, 2), false);
        assert_eq!(rejected.len(), 1);
        let dropped = rejected.into_iter().next().unwrap().into_completed();
        assert_eq!(dropped.seq, 0);
        assert!(matches!(dropped.outcome, Outcome::Overloaded));

        // And the two newer ones survived, in order.
        assert_eq!(queue.pop().unwrap().segment.seq, 1);
        assert_eq!(queue.pop().unwrap().segment.seq, 2);
    }

    #[test]
    fn a_retried_segment_keeps_its_place_in_time() {
        // A segment whose worker died is older than everything waiting. Put
        // back at the end it would be decoded after segments that were spoken
        // later, and the reorder buffer would hold those back until it landed.
        let queue = Queue::new(8);
        assert!(queue.push(job(0, 5), false).is_empty());
        assert!(queue.push(job(0, 3), true).is_empty());
        assert_eq!(queue.pop().unwrap().segment.seq, 3);
        assert_eq!(queue.pop().unwrap().segment.seq, 5);
    }

    #[test]
    fn closing_the_queue_hands_back_what_was_still_waiting() {
        // Silent loss is the one failure this project does not accept, and
        // "the session was stopped" is no excuse for it.
        let queue = Queue::new(8);
        assert!(queue.push(job(0, 0), false).is_empty());
        assert!(queue.push(job(0, 1), false).is_empty());

        let abandoned = queue.close();
        assert_eq!(abandoned.len(), 2);
        assert!(abandoned
            .into_iter()
            .all(|r| matches!(r.into_completed().outcome, Outcome::Failed(_))));

        // And a worker blocked on pop() is released rather than left hanging.
        assert!(queue.pop().is_none());

        // A segment that arrives after the close is reported too.
        let late = queue.push(job(0, 2), false);
        assert_eq!(late.len(), 1);
        assert!(matches!(
            late.into_iter().next().unwrap(),
            Rejected::Closed(_)
        ));
    }

    #[test]
    fn results_are_put_back_in_order_per_stream() {
        let mut reorder = Reorder::new();

        // Segment 1 finished first; it has to wait for 0.
        assert!(reorder.accept(completed(0, 1)).is_empty());
        let released = reorder.accept(completed(0, 0));
        assert_eq!(released.len(), 2);
        assert_eq!(text_of(&released[0]), "segment 0");
        assert_eq!(text_of(&released[1]), "segment 1");
    }

    #[test]
    fn one_stream_never_waits_for_another() {
        // Ordering is per stream. A slow decode on the intercom channel nobody
        // is using must not hold up the one the show is being called on.
        let mut reorder = Reorder::new();
        assert!(reorder.accept(completed(0, 1)).is_empty());

        let released = reorder.accept(completed(1, 0));
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].stream_id, StreamId(1));
    }

    #[test]
    fn a_dropped_segment_does_not_hold_back_the_ones_behind_it() {
        // Why the pool reports an outcome even for segments it gives up on: a
        // seq that never arrives would stall its stream for the rest of the
        // session.
        let mut reorder = Reorder::new();
        assert!(reorder.accept(completed(0, 1)).is_empty());
        assert!(reorder.accept(completed(0, 2)).is_empty());

        let released = reorder.accept(job(0, 0).completed(Outcome::Overloaded));
        assert_eq!(released.len(), 3);
        assert!(matches!(released[0].outcome, Outcome::Overloaded));
        assert_eq!(text_of(&released[1]), "segment 1");
        assert_eq!(text_of(&released[2]), "segment 2");
    }

    #[test]
    fn the_last_words_of_a_session_are_not_swallowed_by_a_gap() {
        // At shutdown the missing predecessor is never going to arrive.
        let mut reorder = Reorder::new();
        assert!(reorder.accept(completed(0, 3)).is_empty());
        assert!(reorder.accept(completed(1, 7)).is_empty());

        let rest = reorder.drain();
        assert_eq!(rest.len(), 2);
        assert_eq!(rest[0].stream_id, StreamId(0));
        assert_eq!(rest[1].stream_id, StreamId(1));
        assert!(reorder.drain().is_empty());
    }

    #[test]
    fn a_duplicate_result_does_not_deadlock_the_stream() {
        let mut reorder = Reorder::new();
        assert_eq!(reorder.accept(completed(0, 0)).len(), 1);
        // Impossible while the pool holds its invariant; harmless if it ever
        // does not, which is the point.
        assert_eq!(reorder.accept(completed(0, 0)).len(), 1);
        assert_eq!(reorder.accept(completed(0, 1)).len(), 1);
    }
}

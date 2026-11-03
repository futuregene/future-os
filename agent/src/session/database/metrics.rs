//! Fixed-size process memory observations; no SQL, paths or per-token records.
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicI64, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

const UPPER_US: [u64; 11] = [
    100, 500, 1000, 2000, 5000, 10000, 25000, 50000, 100000, 250000, 1000000,
];
#[derive(Default)]
pub(super) struct Timing {
    count: AtomicU64,
    failures: AtomicU64,
    total: AtomicU64,
    last: AtomicU64,
    max: AtomicU64,
    buckets: [AtomicU64; 12],
}
impl Timing {
    pub(super) fn record(&self, elapsed: Duration, ok: bool) {
        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        self.count.fetch_add(1, Ordering::Relaxed);
        self.total.fetch_add(micros, Ordering::Relaxed);
        self.last.store(micros, Ordering::Relaxed);
        self.max.fetch_max(micros, Ordering::Relaxed);
        if !ok {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
        self.buckets[UPPER_US
            .iter()
            .position(|upper| micros <= *upper)
            .unwrap_or(UPPER_US.len())]
        .fetch_add(1, Ordering::Relaxed);
    }
    fn snapshot(&self) -> Value {
        let read = |value: &AtomicU64| value.load(Ordering::Relaxed);
        json!({"count":read(&self.count),"failures":read(&self.failures),"totalMicros":read(&self.total),"lastMicros":read(&self.last),"maxMicros":read(&self.max),"durationUpperBoundsMicros":UPPER_US,"durationCounts":self.buckets.iter().map(read).collect::<Vec<_>>()})
    }
}

#[derive(Default)]
pub(super) struct Metrics {
    pub writer_queue: Timing,
    pub writer_job: Timing,
    pub reader_queue: Timing,
    pub reader_job: Timing,
    pub commit: Timing,
    pub checkpoint: Timing,
    pub checkpoint_busy: AtomicU64,
    pub wal_frames: AtomicI64,
    pub checkpointed_frames: AtomicI64,
    pub database_bytes: AtomicU64,
    pub wal_bytes: AtomicU64,
    pub max_wal_bytes: AtomicU64,
}
impl Metrics {
    pub(super) fn snapshot(&self) -> Value {
        json!({"writerQueue":self.writer_queue.snapshot(),"writerJob":self.writer_job.snapshot(),
            "readerQueue":self.reader_queue.snapshot(),"readerJob":self.reader_job.snapshot(),
            "foregroundCommit":self.commit.snapshot(),"checkpoint":self.checkpoint.snapshot(),
            "commitTiming":"SQLite COMMIT elapsed including FULL synchronization; not an isolated fsync measurement",
            "commitCoverage":"begin_write_transaction helper; writerJob covers complete queued work",
            "fileSamplePeriodMs":250,"walPeak":"sampled maximum, not every filesystem write",
            "checkpointMode":"PASSIVE on a separate connection/thread", "checkpointBusy":self.checkpoint_busy.load(Ordering::Relaxed),
            "walFrames":self.wal_frames.load(Ordering::Relaxed),"checkpointedFrames":self.checkpointed_frames.load(Ordering::Relaxed),
            "databaseBytes":self.database_bytes.load(Ordering::Relaxed),"walBytes":self.wal_bytes.load(Ordering::Relaxed),"maxWalBytes":self.max_wal_bytes.load(Ordering::Relaxed)})
    }
}

thread_local! {static CURRENT:RefCell<Option<Arc<Metrics>>>=const {RefCell::new(None)};}
pub(super) struct Scope(Option<Arc<Metrics>>);
pub(super) fn install(metrics: Arc<Metrics>) -> Scope {
    Scope(CURRENT.with(|current| current.replace(Some(metrics))))
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            current.replace(self.0.take());
        });
    }
}
pub(super) fn commit(elapsed: Duration, ok: bool) {
    CURRENT.with(|current| {
        if let Some(metrics) = current.borrow().as_ref() {
            metrics.commit.record(elapsed, ok);
        }
    });
}

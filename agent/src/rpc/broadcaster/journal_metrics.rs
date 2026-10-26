//! Fixed-size, in-memory counters. Never persist per-token diagnostics.
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub(super) struct JournalMetrics {
    attempts: AtomicU64,
    failures: AtomicU64,
    events: AtomicU64,
    bytes: AtomicU64,
    max_events: AtomicU64,
    max_bytes: AtomicU64,
    max_micros: AtomicU64,
    last_micros: AtomicU64,
    /// Noncumulative upper bounds in milliseconds; the last bucket is overflow.
    durations: [AtomicU64; 10],
}
const BOUNDS_MS: [u64; 9] = [1, 2, 5, 10, 25, 50, 100, 250, 1000];

impl JournalMetrics {
    pub(super) fn record(
        &self,
        events: usize,
        bytes: usize,
        elapsed: std::time::Duration,
        ok: bool,
    ) {
        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        self.attempts.fetch_add(1, Ordering::Relaxed);
        if !ok {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
        self.events.fetch_add(events as u64, Ordering::Relaxed);
        self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
        self.max_events.fetch_max(events as u64, Ordering::Relaxed);
        self.max_bytes.fetch_max(bytes as u64, Ordering::Relaxed);
        self.max_micros.fetch_max(micros, Ordering::Relaxed);
        self.last_micros.store(micros, Ordering::Relaxed);
        let bucket = BOUNDS_MS
            .iter()
            .position(|bound| micros <= bound * 1000)
            .unwrap_or(BOUNDS_MS.len());
        self.durations[bucket].fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn snapshot(&self) -> serde_json::Value {
        let read = |value: &AtomicU64| value.load(Ordering::Relaxed);
        serde_json::json!({
            "batchAttempts":read(&self.attempts),"batchFailures":read(&self.failures),
            "attemptedEvents":read(&self.events),"attemptedBytes":read(&self.bytes),
            "maxBatchEvents":read(&self.max_events),"maxBatchBytes":read(&self.max_bytes),
            "lastBatchMicros":read(&self.last_micros),"maxBatchMicros":read(&self.max_micros),
            "durationUpperBoundsMs":BOUNDS_MS,
            "durationCounts":self.durations.iter().map(read).collect::<Vec<_>>()
        })
    }
}

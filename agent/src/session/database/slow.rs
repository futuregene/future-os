//! Storage latency evidence without SQL or transcript payloads. Both queue-only
//! stalls and executing jobs are reported, including work still pending. All
//! durations are wall time: I/O, lock waits and process suspension can contribute.
use std::time::Duration;

/// Below this a job is ordinary traffic and is not worth a log row.
pub(super) const SLOW_JOB_MS: u128 = 1_000;

/// The log row for one lane job, or `None` when it was ordinary traffic.
///
/// `elapsed` measures execution wall time; `queued` measures the wait before
/// execution. Neither alone identifies the cause of an OS or disk stall.
pub(super) fn slow_line(job: &str, elapsed: Duration, queued: Duration) -> Option<String> {
    (elapsed.as_millis() >= SLOW_JOB_MS || queued.as_millis() >= SLOW_JOB_MS).then(|| {
        format!(
            "storage job slow: job={job} elapsed={}ms queued={}ms",
            elapsed.as_millis(),
            queued.as_millis()
        )
    })
}

/// Write that row into the agent's log, when the cut is crossed.
static NEXT_JOB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// One identity joins pending and completed rows, even for concurrent requests
/// from the same session and source location.
#[derive(Clone, Copy)]
pub(super) struct Job {
    id: u64,
    lane: &'static str,
    name: &'static str,
    caller: &'static std::panic::Location<'static>,
}
impl Job {
    #[track_caller]
    pub(super) fn new(lane: &'static str, name: &'static str) -> Self {
        Self {
            id: NEXT_JOB.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            lane,
            name,
            caller: std::panic::Location::caller(),
        }
    }
    pub(super) fn report(
        &self,
        elapsed: Duration,
        queued: Duration,
        success: bool,
        request: Option<&super::request::Context>,
    ) {
        if slow_line(self.name, elapsed, queued).is_some() {
            tracing::warn!(event = "storage_job_slow", job_id = self.id, lane = self.lane, job = self.name,
                operation = %self.caller, elapsed_ms = elapsed.as_millis() as u64,
                queued_ms = queued.as_millis() as u64, total_ms = (elapsed + queued).as_millis() as u64, success,
                command = request.map_or("", |request| request.command.as_str()),
                session_id = request.map_or("", |request| request.session.as_str()),
                "storage operation exceeded its latency budget");
        }
    }
    /// Emit evidence while work is still stuck, not only after it returns.
    pub(super) fn wait<T>(
        &self,
        reply: &std::sync::mpsc::Receiver<anyhow::Result<T>>,
        running: std::sync::Arc<std::sync::atomic::AtomicBool>,
        request: Option<&super::request::Context>,
    ) -> anyhow::Result<T> {
        let started = std::time::Instant::now();
        let mut next_report = Duration::from_secs(1);
        loop {
            if let Some(request) = request {
                request.check()?;
            }
            match reply.recv_timeout(Duration::from_millis(50)) {
                Ok(result) => return result,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("SQLite operation interrupted")
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
            let elapsed = started.elapsed();
            if elapsed >= next_report {
                tracing::warn!(event = "storage_job_waiting", job_id = self.id, lane = self.lane, job = self.name, operation = %self.caller,
                    phase = if running.load(std::sync::atomic::Ordering::Acquire) { "running" } else { "queued" },
                    waited_ms = elapsed.as_millis() as u64,
                    command = request.map_or("", |request| request.command.as_str()),
                    session_id = request.map_or("", |request| request.session.as_str()),
                    "still waiting for storage operation");
                next_report = elapsed + Duration::from_secs(30);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(job: &str, elapsed_ms: u64, queued_ms: u64) -> Option<String> {
        slow_line(
            job,
            Duration::from_millis(elapsed_ms),
            Duration::from_millis(queued_ms),
        )
    }

    #[test]
    fn queued_stall_records_identity_source_and_request_without_sql() {
        #[derive(Clone)]
        struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = bytes.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || Capture(writer.clone()))
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let request = crate::session::database::request::Context::new(
                "list_sessions".into(),
                "diagnostic-session".into(),
                None,
            );
            Job::new("reader", "session_summaries").report(
                Duration::from_millis(2),
                Duration::from_millis(1500),
                true,
                Some(&request),
            );
        });
        let text = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        for field in [
            "storage_job_slow",
            "job_id=",
            "session_summaries",
            "operation=",
            "slow.rs:",
            "queued_ms=1500",
            "elapsed_ms=2",
            "total_ms=1502",
            "list_sessions",
            "diagnostic-session",
        ] {
            assert!(text.contains(field), "missing {field}: {text}");
        }
    }

    #[test]
    fn only_jobs_at_or_above_the_cut_are_reported() {
        assert!(line("commit", 20, 0).is_none());
        assert!(line("commit", 999, 0).is_none());
        assert!(line("commit", 1_000, 0).is_some());
        assert!(line("commit", 38_120, 0).is_some());
        // Queue-only stalls were previously invisible.
        assert!(line("commit", 5, 120_000).is_some());
        assert!(line("commit", 0, 1_000).is_some());
    }

    #[test]
    fn the_row_names_the_job_and_both_halves_of_the_wait() {
        assert_eq!(
            line("journal_compaction", 38_120, 0).unwrap(),
            "storage job slow: job=journal_compaction elapsed=38120ms queued=0ms"
        );
        assert_eq!(
            line("reclaim", 1_250, 30).unwrap(),
            "storage job slow: job=reclaim elapsed=1250ms queued=30ms"
        );
        assert_eq!(
            line("wal_checkpoint", 2_000, 0).unwrap(),
            "storage job slow: job=wal_checkpoint elapsed=2000ms queued=0ms"
        );
    }
}

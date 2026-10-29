//! One line of evidence when a job holds a storage lane long enough to explain
//! a caller that will not come back.
//!
//! A remote command that times out is answered by a *different* process: the
//! desktop logs only its own wait (`remote command slow: type=… elapsed=…`),
//! which says the caller waited but not what it waited for. The writer and the
//! checkpoint share no clock with it, so this is the other half: the lane's own
//! row, naming the job and separating the two worlds a long wait can come from —
//! `elapsed` (the job itself ran that long) versus `queued` (the job started
//! immediately and the thread was not running). Measured 2026-10-29: six remote
//! commands released within 400 ms of each other after 18–30 s, which is one
//! blocking job; which one, and whether the process was merely descheduled, was
//! not answerable from the desktop log.
//!
//! Same cut as the command-side diagnostics (`SLOW_COMMAND_MS` in
//! `host/src/remote_host/command_diagnostics.rs`, 1 s): both sides answer the
//! same question about the same incident, so a reader comparing the two files
//! must not have to convert between two thresholds. A row is formatted only
//! when the cut is crossed, so the ordinary job pays one comparison.
use std::time::Duration;

/// Below this a job is ordinary traffic and is not worth a log row.
pub(super) const SLOW_JOB_MS: u128 = 1_000;

/// The log row for one lane job, or `None` when it was ordinary traffic.
///
/// `elapsed` is the work; `queued` is how long it waited for its turn. The two
/// together are the point: a long `elapsed` names a job that must be split, a
/// long `queued` with a short `elapsed` names a process that was not scheduled
/// (sleep, app-nap, paging) or a lane held by an earlier job.
pub(super) fn slow_line(job: &str, elapsed: Duration, queued: Duration) -> Option<String> {
    (elapsed.as_millis() >= SLOW_JOB_MS).then(|| {
        format!(
            "storage job slow: job={job} elapsed={}ms queued={}ms",
            elapsed.as_millis(),
            queued.as_millis()
        )
    })
}

/// Write that row into the agent's log, when the cut is crossed.
pub(super) fn report(job: &str, elapsed: Duration, queued: Duration) {
    if let Some(line) = slow_line(job, elapsed, queued) {
        tracing::warn!("{line}");
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
    fn only_jobs_at_or_above_the_cut_are_reported() {
        assert!(line("commit", 20, 0).is_none());
        assert!(line("commit", 999, 0).is_none());
        assert!(line("commit", 1_000, 0).is_some());
        assert!(line("commit", 38_120, 0).is_some());
        // The cut is on the work, not on the combined wait: a job that queued
        // two minutes and then ran for 5 ms is a descheduled process, and it is
        // reported by the *lane* that was waiting, not by this row.
        assert!(line("commit", 5, 120_000).is_none());
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

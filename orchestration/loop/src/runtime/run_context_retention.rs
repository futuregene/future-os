//! Run context retention (G-5) — the retention policy over run-history
//! files, mirroring LoopX `control_plane/runtime/run_context_retention.py`:
//! keep the latest N runs (plus a TTL window when configured); older files
//! become retention candidates that compaction ARCHIVES (never deletes).

use anyhow::Result;
use serde::Serialize;

use crate::runtime::run_history::row_epoch;

pub const DEFAULT_RUN_RETENTION_LIMIT: usize = 50;

#[derive(Debug, Clone, Serialize)]
pub struct RetentionPolicy {
    pub keep_latest: usize,
    pub ttl_secs: Option<u64>,
}

/// Build a retention policy (LoopX retention knobs).
pub fn retention_policy(keep_latest: usize, ttl_secs: Option<u64>) -> RetentionPolicy {
    RetentionPolicy {
        keep_latest: keep_latest.max(1),
        ttl_secs,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RetentionReport {
    pub goal_id: String,
    pub policy: RetentionPolicy,
    pub total: usize,
    pub retained: usize,
    /// Paths that are retention candidates (archivable, never deleted).
    pub candidates: Vec<String>,
}

/// Compute which runs stay vs become retention candidates (LoopX
/// `latest_runs_with_agent_context` in minimal form: keep the latest N and
/// anything inside the TTL; the rest are candidates).
pub fn retention_report(
    runtime_root: &str,
    goal_id: &str,
    policy: &RetentionPolicy,
    now_epoch: u64,
) -> Result<RetentionReport> {
    // The index is a pure projection: derive it from the run files on disk.
    let mut rows = crate::runtime::run_index::load_run_index(runtime_root, goal_id)?;
    rows.sort_by_key(|b| std::cmp::Reverse(row_epoch(b)));

    let cutoff_epoch = policy
        .ttl_secs
        .map(|ttl| now_epoch.saturating_sub(ttl))
        .unwrap_or(0);
    let mut candidates: Vec<String> = vec![];
    let mut retained = 0usize;
    for (position, row) in rows.iter().enumerate() {
        let within_ttl = policy
            .ttl_secs
            .map(|_| row_epoch(row).map(|e| e >= cutoff_epoch).unwrap_or(false))
            .unwrap_or(false);
        if position < policy.keep_latest || within_ttl {
            retained += 1;
        } else {
            candidates.push(row.path.clone());
        }
    }
    Ok(RetentionReport {
        goal_id: goal_id.to_string(),
        policy: policy.clone(),
        total: rows.len(),
        retained,
        candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_keeps_latest_and_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("goals").join("g1").join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        // 5 run files on distinct August days; now = 08-31, TTL = 7d → cutoff 08-24.
        for day in [20, 25, 26, 27, 28] {
            let payload = format!(
                "{{\"timestamp\":\"2026-08-{day:02}T00:00:00+00:00\",\"turn\":1,\"terminal_state\":\"run_recorded\"}}"
            );
            std::fs::write(runs.join(format!("d{day}.json")), payload).unwrap();
        }

        let now = crate::scheduler::state::parse_epoch("2026-08-31T00:00:00+00:00").unwrap();
        let policy = retention_policy(2, Some(7 * 86400));
        let report = retention_report(dir.path().to_str().unwrap(), "g1", &policy, now).unwrap();
        assert_eq!(report.total, 5);
        // keep 2 latest (28, 27) + 26, 25 inside the 7d TTL; 20 is a candidate.
        assert_eq!(report.retained, 4);
        assert_eq!(report.candidates.len(), 1);
        assert!(report.candidates[0].contains("d20"));
    }

    #[test]
    fn missing_run_dir_is_empty_report() {
        let dir = tempfile::tempdir().unwrap();
        let report = retention_report(
            dir.path().to_str().unwrap(),
            "g1",
            &retention_policy(DEFAULT_RUN_RETENTION_LIMIT, None),
            1_784_000_000,
        )
        .unwrap();
        assert_eq!(report.total, 0);
        assert!(report.candidates.is_empty());
    }
}

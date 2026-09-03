//! Run history projection (G-5) — reads the append-only run index
//! (`<runtime>/goals/<id>/runs/index.jsonl`), mirroring LoopX
//! `control_plane/runtime/run_history.py` + the `event_ledger` proxy:
//! event-class counts in 24h/7d windows plus the latest run. Read-only.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

pub const RUN_HISTORY_PROXY_NOTE: &str =
    "append-only run-history projection; compact event-class counts only";

#[derive(Debug, Clone, Serialize)]
pub struct RunIndexRow {
    pub goal_id: String,
    pub timestamp: String,
    pub path: String,
    pub turn: u32,
    pub classification: String,
}

/// Read + parse the run index rows (skipping unparsable lines).
pub fn read_index_rows(index_path: &Path) -> Result<Vec<RunIndexRow>> {
    if !index_path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(index_path).context("read run index")?;
    let mut rows = vec![];
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        rows.push(RunIndexRow {
            goal_id: value
                .get("goal_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            timestamp: value
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            path: value
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            turn: value.get("turn").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            classification: value
                .get("classification")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        });
    }
    Ok(rows)
}

/// Row epoch (None when the timestamp does not parse).
pub fn row_epoch(row: &RunIndexRow) -> Option<u64> {
    crate::scheduler::state::parse_epoch(&row.timestamp)
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RunHistoryTotals {
    pub events_24h: u64,
    pub events_7d: u64,
    pub by_class_24h: BTreeMap<String, u64>,
    pub by_class_7d: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunHistoryProjection {
    pub available: bool,
    pub source: String,
    pub goal_id: String,
    pub sample_run_count: usize,
    pub proxy_note: String,
    pub totals: RunHistoryTotals,
    pub latest: Option<RunIndexRow>,
}

/// Build the run-history projection for a goal (LoopX `build_event_ledger_summary`,
/// proxy form). `now_epoch` injectable for deterministic tests.
pub fn build_run_history(
    runtime_root: &str,
    goal_id: &str,
    now_epoch: u64,
) -> Result<Option<RunHistoryProjection>> {
    let rows = crate::runtime::run_index::load_run_index(runtime_root, goal_id)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let cutoff_24h = now_epoch.saturating_sub(24 * 60 * 60);
    let cutoff_7d = now_epoch.saturating_sub(7 * 24 * 60 * 60);
    let mut totals = RunHistoryTotals::default();
    for row in &rows {
        let Some(epoch) = row_epoch(row) else {
            continue;
        };
        let class = if row.classification.is_empty() {
            "work".to_string()
        } else {
            row.classification.clone()
        };
        if epoch >= cutoff_7d {
            totals.events_7d += 1;
            *totals.by_class_7d.entry(class.clone()).or_insert(0) += 1;
        }
        if epoch >= cutoff_24h {
            totals.events_24h += 1;
            *totals.by_class_24h.entry(class).or_insert(0) += 1;
        }
    }
    Ok(Some(RunHistoryProjection {
        available: true,
        source: "run_history".to_string(),
        goal_id: goal_id.to_string(),
        sample_run_count: rows.len(),
        proxy_note: RUN_HISTORY_PROXY_NOTE.to_string(),
        totals,
        latest: rows.last().cloned(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_row(ts: &str, classification: &str) -> String {
        format!("{{\"timestamp\":\"{ts}\",\"turn\":1,\"terminal_state\":\"{classification}\"}}")
    }

    #[test]
    fn buckets_by_class_in_windows() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("goals").join("g1").join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        // The index is derived from the run files on disk.
        std::fs::write(
            runs.join("a.json"),
            index_row("2026-08-05T12:00:00+00:00", "run_recorded"),
        )
        .unwrap();
        std::fs::write(
            runs.join("b.json"),
            index_row("2026-08-04T12:00:00+00:00", "run_recorded"),
        )
        .unwrap();
        std::fs::write(
            runs.join("c.json"),
            index_row("2026-07-20T12:00:00+00:00", "quota_monitor_poll"),
        )
        .unwrap();
        let now = crate::scheduler::state::parse_epoch("2026-08-05T13:00:00+00:00").unwrap();
        let projection = build_run_history(dir.path().to_str().unwrap(), "g1", now)
            .unwrap()
            .unwrap();
        assert_eq!(projection.sample_run_count, 3);
        assert_eq!(projection.totals.events_24h, 1);
        assert_eq!(projection.totals.events_7d, 2);
        assert_eq!(
            projection.totals.by_class_7d.get("run_recorded"),
            Some(&2u64)
        );
        assert_eq!(
            projection.totals.by_class_24h.get("run_recorded"),
            Some(&1u64)
        );
        assert_eq!(
            projection.latest.unwrap().classification,
            "quota_monitor_poll"
        );
    }

    #[test]
    fn missing_index_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let projection =
            build_run_history(dir.path().to_str().unwrap(), "g1", 1_784_000_000).unwrap();
        assert!(projection.is_none());
    }
}

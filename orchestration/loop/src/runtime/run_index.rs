//! Run index dedup + rebuild (G-5 minimal) — mirroring LoopX
//! `run_index_duplicates.py` (plain-duplicate detection; a duplicate row is
//! byte-equivalent after the reward overlay is ignored) and
//! `run_index_rebuild.py` (non-destructive rebuild: backup + tmp + rename,
//! never deleting rows).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::runtime::run_history::{read_index_rows, RunIndexRow};

/// Identity key of an index row (LoopX `index_identity`):
/// generated_at | json_path | markdown_path. We use timestamp + path.
fn index_identity(row: &RunIndexRow) -> String {
    format!("{}|{}", row.timestamp, row.path)
}

#[derive(Debug, Clone, Serialize)]
pub struct DuplicateGroup {
    pub identity: String,
    pub line_numbers: Vec<usize>,
    pub duplicate_kind: String,
    pub severity: String,
    pub repairable: bool,
    pub action: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexDedupReport {
    pub index_path: String,
    pub total_rows: usize,
    pub duplicate_groups: Vec<DuplicateGroup>,
    pub repairable: bool,
}

/// Detect duplicate index rows (LoopX `classify_index_duplicate_records`,
/// plain-duplicate case only): rows sharing timestamp+path are identical
/// projections of one artifact — the extra rows are repairable duplicates.
pub fn detect_duplicates(index_path: &Path) -> Result<IndexDedupReport> {
    let rows = read_index_rows(index_path)?;
    let mut by_identity: std::collections::BTreeMap<String, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (line_number, row) in rows.iter().enumerate() {
        by_identity
            .entry(index_identity(row))
            .or_default()
            .push(line_number + 1);
    }
    let mut groups = vec![];
    for (identity, line_numbers) in by_identity {
        if line_numbers.len() <= 1 {
            continue;
        }
        groups.push(DuplicateGroup {
            identity,
            line_numbers,
            duplicate_kind: "plain_duplicate".to_string(),
            severity: "warning".to_string(),
            repairable: true,
            action: "drop_plain_duplicate_rows".to_string(),
        });
    }
    let repairable = !groups.is_empty();
    Ok(IndexDedupReport {
        index_path: index_path.to_string_lossy().into_owned(),
        total_rows: rows.len(),
        duplicate_groups: groups,
        repairable,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct RebuildReport {
    pub index_path: String,
    pub backup_path: String,
    pub rows_written: usize,
    pub non_destructive: bool,
}

/// P1-2③: drift count at which the run path self-heals the index — any
/// observed drift (a missing, stale, or duplicate row) triggers the
/// non-destructive rebuild (LoopX `run_ingest_health` → `run_index_rebuild`).
pub const INDEX_DRIFT_AUTO_REPAIR_THRESHOLD: usize = 1;

/// P1-2①: run-index drift report — the read-model self-diagnosis (LoopX
/// `run_ingest_health.py` / `run_index_duplicates.py`). The run files on
/// disk are the source of truth; the append-only index is a rebuildable
/// projection of them.
#[derive(Debug, Clone, Serialize)]
pub struct IndexDriftReport {
    pub goal_id: String,
    pub index_path: String,
    pub index_rows: usize,
    pub run_files: usize,
    /// Run files with no index row (index lags the write path).
    pub missing_rows: usize,
    /// Index rows whose run file vanished (external deletion / compaction).
    pub stale_rows: usize,
    /// Extra rows beyond the first per identity (double ingest).
    pub duplicate_rows: usize,
    pub drift_count: usize,
    pub repair_recommended: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_identities: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stale_identities: Vec<String>,
}

/// Detect drift between the run index and the run files on disk (the
/// rebuild source of truth). Identity is the same `timestamp|path` key the
/// dedup detector uses, so a rebuilt index always reports zero drift.
pub fn detect_index_drift(runtime_root: &str, goal_id: &str) -> Result<IndexDriftReport> {
    let runs = crate::runtime::runs_dir(runtime_root, goal_id);
    let index = runs.join("index.jsonl");
    let rows = read_index_rows(&index)?;
    let mut files: Vec<RunIndexRow> = vec![];
    if runs.exists() {
        collect_run_files(&runs, goal_id, &mut files)?;
    }

    let mut index_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for row in &rows {
        *index_counts.entry(index_identity(row)).or_default() += 1;
    }
    let file_identities: std::collections::BTreeSet<String> =
        files.iter().map(index_identity).collect();

    let missing: Vec<String> = file_identities
        .iter()
        .filter(|id| !index_counts.contains_key(*id))
        .cloned()
        .collect();
    let stale: Vec<String> = index_counts
        .keys()
        .filter(|id| !file_identities.contains(*id))
        .cloned()
        .collect();
    let duplicate_rows: usize = index_counts.values().map(|n| n.saturating_sub(1)).sum();
    let drift_count = missing.len() + stale.len() + duplicate_rows;
    Ok(IndexDriftReport {
        goal_id: goal_id.to_string(),
        index_path: index.to_string_lossy().into_owned(),
        index_rows: rows.len(),
        run_files: files.len(),
        missing_rows: missing.len(),
        stale_rows: stale.len(),
        duplicate_rows,
        drift_count,
        repair_recommended: drift_count >= INDEX_DRIFT_AUTO_REPAIR_THRESHOLD,
        missing_identities: missing,
        stale_identities: stale,
    })
}

/// Outcome of a drift-triggered index repair (drift report + rebuild report).
#[derive(Debug, Clone, Serialize)]
pub struct IndexRepairOutcome {
    pub drift: IndexDriftReport,
    pub rebuilt: RebuildReport,
}

/// P1-2③: projection self-healing — when the run index drifts past
/// [`INDEX_DRIFT_AUTO_REPAIR_THRESHOLD`], rebuild it from the run files
/// (non-destructive: backup + tmp + rename) and record a
/// `ProjectionRepaired` audit event in the ledger. Returns `None` when the
/// projection is clean. Used by the run path (automatic) and by
/// `store verify --repair` (operator-triggered).
pub fn repair_index_if_drifted(
    store: &mut crate::store::Store,
    goal_id: &str,
) -> Result<Option<IndexRepairOutcome>> {
    let drift = detect_index_drift(&store.root_path(), goal_id)?;
    if !drift.repair_recommended {
        return Ok(None);
    }
    let rebuilt = rebuild_index(&store.root_path(), goal_id)?;
    store.append(crate::store::Event::ProjectionRepaired {
        goal_id: goal_id.to_string(),
        projection: "run_index".to_string(),
        drift_count: drift.drift_count,
        missing_rows: drift.missing_rows,
        stale_rows: drift.stale_rows,
        duplicate_rows: drift.duplicate_rows,
        rows_written: rebuilt.rows_written,
        backup_path: rebuilt.backup_path.clone(),
        ts: crate::state::now_epoch(),
    })?;
    Ok(Some(IndexRepairOutcome { drift, rebuilt }))
}

/// Rebuild the run index by rescanning the run files on disk (LoopX
/// `run_index_rebuild`): the old index is backed up, the new index is
/// written via tmp+rename, and rows are never deleted. Handles the case
/// where the index is missing or has drifted from the run files.
pub fn rebuild_index(runtime_root: &str, goal_id: &str) -> Result<RebuildReport> {
    let runs = crate::runtime::runs_dir(runtime_root, goal_id);
    if !runs.exists() {
        anyhow::bail!("goal {goal_id} has no run-history dir");
    }
    let index = runs.join("index.jsonl");
    let mut rows: Vec<RunIndexRow> = vec![];
    collect_run_files(&runs, goal_id, &mut rows)?;
    rows.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    let backup_path = if index.exists() {
        let backup = runs.join(format!(
            "index.pre-rebuild-{}.jsonl",
            crate::state::now_epoch()
        ));
        std::fs::copy(&index, &backup).context("backup run index")?;
        backup
    } else {
        PathBuf::new()
    };

    let mut text = String::new();
    for row in &rows {
        text.push_str(&serde_json::to_string(&serde_json::json!({
            "goal_id": row.goal_id,
            "timestamp": row.timestamp,
            "path": row.path,
            "turn": row.turn,
            "classification": row.classification,
        }))?);
        text.push('\n');
    }
    let tmp = index.with_extension("jsonl.tmp");
    std::fs::write(&tmp, text).context("write rebuilt index tmp")?;
    std::fs::rename(&tmp, &index).context("rename rebuilt index")?;

    Ok(RebuildReport {
        index_path: index.to_string_lossy().into_owned(),
        backup_path: backup_path.to_string_lossy().into_owned(),
        rows_written: rows.len(),
        non_destructive: true,
    })
}

/// Walk run files (including the archive dir) and derive index rows from the
/// JSON payloads (timestamp field) — the rebuild source of truth. Row paths
/// match the writer's format (`goals/<id>/runs/<file>.json`, archive files
/// `goals/<id>/runs/archive/<file>.json` — see `compat::write_run` and
/// `run_compaction`) so a rebuilt index is identity-consistent with
/// writer-appended rows.
fn collect_run_files(dir: &Path, goal_id: &str, out: &mut Vec<RunIndexRow>) -> Result<()> {
    collect_run_files_under(dir, dir, goal_id, out)
}

fn collect_run_files_under(
    runs_root: &Path,
    dir: &Path,
    goal_id: &str,
    out: &mut Vec<RunIndexRow>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_run_files_under(runs_root, &path, goal_id, out)?;
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let relative = path.strip_prefix(runs_root).unwrap_or(&path);
        // Join with `/` explicitly: index row paths are platform-neutral
        // (the writer and the compaction re-pointing both use `/`).
        let relative = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        out.push(RunIndexRow {
            goal_id: goal_id.to_string(),
            timestamp: value
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            path: format!("goals/{goal_id}/runs/{relative}"),
            turn: value.get("turn").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            classification: value
                .get("terminal_state")
                .and_then(|v| v.as_str())
                .unwrap_or("run_recorded")
                .to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_plain_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("index.jsonl");
        std::fs::write(
            &index,
            "{\"goal_id\":\"g1\",\"timestamp\":\"2026-08-05T00:00:00+00:00\",\"path\":\"goals/g1/runs/a.json\",\"turn\":1,\"classification\":\"run_recorded\"}\n\
             {\"goal_id\":\"g1\",\"timestamp\":\"2026-08-05T00:00:00+00:00\",\"path\":\"goals/g1/runs/a.json\",\"turn\":1,\"classification\":\"run_recorded\"}\n\
             {\"goal_id\":\"g1\",\"timestamp\":\"2026-08-04T00:00:00+00:00\",\"path\":\"goals/g1/runs/b.json\",\"turn\":2,\"classification\":\"run_recorded\"}\n",
        )
        .unwrap();
        let report = detect_duplicates(&index).unwrap();
        assert_eq!(report.total_rows, 3);
        assert_eq!(report.duplicate_groups.len(), 1);
        assert_eq!(report.duplicate_groups[0].line_numbers, vec![1, 2]);
        assert!(report.duplicate_groups[0].repairable);
        assert!(report.repairable);
    }

    #[test]
    fn rebuild_requires_a_runs_dir() {
        let dir = tempfile::tempdir().unwrap();
        let err = rebuild_index(dir.path().to_str().unwrap(), "g-missing").unwrap_err();
        assert!(format!("{err:#}").contains("no run-history dir"), "{err:#}");
    }

    #[test]
    fn rebuild_recurses_subdirs_and_skips_unreadable_and_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("goals").join("g1").join("runs");
        let archive = runs.join("archive");
        std::fs::create_dir_all(&archive).unwrap();
        std::fs::write(
            archive.join("old.json"),
            "{\"goal_id\":\"g1\",\"timestamp\":\"2026-07-01T00:00:00+00:00\",\"turn\":1}",
        )
        .unwrap();
        // An unreadable *.json file fails read_to_string → skipped.
        let unreadable = runs.join("unreadable.json");
        std::fs::write(&unreadable, "{}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&unreadable).unwrap().permissions();
            perms.set_mode(0o000);
            std::fs::set_permissions(&unreadable, perms).unwrap();
        }
        #[cfg(not(unix))]
        std::fs::remove_file(&unreadable).unwrap();
        // Invalid JSON → skipped.
        std::fs::write(runs.join("garbage.json"), "not json").unwrap();
        // Non-json extension → skipped.
        std::fs::write(runs.join("notes.txt"), "ignored").unwrap();
        let report = rebuild_index(dir.path().to_str().unwrap(), "g1").unwrap();
        assert_eq!(report.rows_written, 1);
    }

    #[test]
    fn rebuild_rescans_run_files() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("goals").join("g1").join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        std::fs::write(
            runs.join("a.json"),
            "{\"goal_id\":\"g1\",\"timestamp\":\"2026-08-05T00:00:00+00:00\",\"turn\":1,\"terminal_state\":\"completed\"}",
        )
        .unwrap();
        std::fs::write(
            runs.join("b.json"),
            "{\"goal_id\":\"g1\",\"timestamp\":\"2026-08-04T00:00:00+00:00\",\"turn\":2,\"terminal_state\":\"failed\"}",
        )
        .unwrap();
        let report = rebuild_index(dir.path().to_str().unwrap(), "g1").unwrap();
        assert_eq!(report.rows_written, 2);
        assert!(report.non_destructive);
        let rows = read_index_rows(&runs.join("index.jsonl")).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].timestamp, "2026-08-05T00:00:00+00:00",
            "sorted desc"
        );
        // Classification derives from terminal_state.
        assert_eq!(rows[1].classification, "failed");
        // Row paths match the writer's format (`compat::write_run`) so a
        // rebuilt index is identity-consistent with writer-appended rows.
        assert_eq!(rows[0].path, "goals/g1/runs/a.json");
    }

    // ── P1-2: drift detection + self-healing repair ──────────────────────

    fn write_run(runs: &Path, name: &str, timestamp: &str, terminal_state: &str) {
        std::fs::write(
            runs.join(name),
            format!(
                "{{\"goal_id\":\"g1\",\"timestamp\":\"{timestamp}\",\"turn\":1,\"terminal_state\":\"{terminal_state}\"}}"
            ),
        )
        .unwrap();
    }

    #[test]
    fn drift_detects_missing_stale_and_duplicate_rows() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap().to_string();
        let runs = dir.path().join("goals").join("g1").join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        write_run(&runs, "a.json", "2026-08-05T00:00:00+00:00", "completed");
        write_run(&runs, "b.json", "2026-08-04T00:00:00+00:00", "failed");
        // Index: a.json indexed twice (duplicate), a stale row for a deleted
        // run file, and b.json not indexed at all (missing).
        std::fs::write(
            runs.join("index.jsonl"),
            "{\"goal_id\":\"g1\",\"timestamp\":\"2026-08-05T00:00:00+00:00\",\"path\":\"goals/g1/runs/a.json\",\"turn\":1,\"classification\":\"completed\"}\n\
             {\"goal_id\":\"g1\",\"timestamp\":\"2026-08-05T00:00:00+00:00\",\"path\":\"goals/g1/runs/a.json\",\"turn\":1,\"classification\":\"completed\"}\n\
             {\"goal_id\":\"g1\",\"timestamp\":\"2026-08-03T00:00:00+00:00\",\"path\":\"goals/g1/runs/gone.json\",\"turn\":3,\"classification\":\"failed\"}\n",
        )
        .unwrap();
        let drift = detect_index_drift(&root, "g1").unwrap();
        assert_eq!(drift.index_rows, 3);
        assert_eq!(drift.run_files, 2);
        assert_eq!(drift.missing_rows, 1, "b.json is not indexed");
        assert_eq!(drift.stale_rows, 1, "gone.json has no run file");
        assert_eq!(drift.duplicate_rows, 1, "a.json indexed twice");
        assert_eq!(drift.drift_count, 3);
        assert!(drift.repair_recommended);
        assert_eq!(drift.missing_identities.len(), 1);
        assert_eq!(drift.stale_identities.len(), 1);
    }

    #[test]
    fn drift_is_zero_after_a_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap().to_string();
        let runs = dir.path().join("goals").join("g1").join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        write_run(&runs, "a.json", "2026-08-05T00:00:00+00:00", "completed");
        rebuild_index(&root, "g1").unwrap();
        let drift = detect_index_drift(&root, "g1").unwrap();
        assert_eq!(drift.drift_count, 0);
        assert!(!drift.repair_recommended);
        // No runs dir at all → no drift, no repair.
        let drift = detect_index_drift(&root, "g-empty").unwrap();
        assert_eq!(drift.drift_count, 0);
    }

    fn drifted_store(tag: &str) -> (crate::store::Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "future-loop-p12-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let runs = dir.join("goals").join("g1").join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        // A run file with no index row → drift.
        write_run(&runs, "a.json", "2026-08-05T00:00:00+00:00", "completed");
        let mut store = crate::store::Store::open(dir.to_string_lossy().as_ref()).unwrap();
        let goal = crate::state::Goal::new("g1", "objective", "/tmp");
        store.register(&goal).unwrap();
        (store, dir)
    }

    #[test]
    fn repair_rebuilds_and_records_the_audit_event() {
        let (mut store, dir) = drifted_store("repair");
        // A non-projection event in the ledger exercises the `_ => None` arm
        // of the audit-event filter below.
        store
            .append(crate::store::Event::GoalStarted {
                goal_id: "g1".to_string(),
                ts: 0,
            })
            .unwrap();
        let outcome = repair_index_if_drifted(&mut store, "g1")
            .unwrap()
            .expect("drift should trigger repair");
        assert_eq!(outcome.drift.missing_rows, 1);
        assert_eq!(outcome.rebuilt.rows_written, 1);
        assert!(outcome.rebuilt.non_destructive);
        assert!(
            dir.join("goals/g1/runs/index.jsonl").exists(),
            "index rebuilt"
        );
        // ProjectionRepaired audit event landed in the ledger.
        let events = store.events("g1").unwrap();
        let repairs: Vec<_> = events
            .iter()
            .filter_map(|stored| match &stored.event {
                crate::store::Event::ProjectionRepaired {
                    projection,
                    drift_count,
                    rows_written,
                    ..
                } => Some((projection.clone(), *drift_count, *rows_written)),
                _ => None,
            })
            .collect();
        assert_eq!(repairs.len(), 1);
        assert_eq!(repairs[0], ("run_index".to_string(), 1, 1));
        // Repair is idempotent: the rebuilt index reports zero drift.
        assert!(repair_index_if_drifted(&mut store, "g1").unwrap().is_none());
        // The audit event is projection-only: replay ignores it.
        let goal = store.replay("g1").unwrap().unwrap();
        assert!(goal.todos.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn repair_is_a_noop_without_drift() {
        let (mut store, dir) = drifted_store("noop");
        // Bring the index in sync first.
        rebuild_index(&store.root_path(), "g1").unwrap();
        assert!(repair_index_if_drifted(&mut store, "g1").unwrap().is_none());
        assert!(store.events("g1").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

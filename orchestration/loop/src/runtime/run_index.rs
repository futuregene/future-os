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
/// JSON payloads (timestamp field) — the rebuild source of truth.
fn collect_run_files(dir: &Path, goal_id: &str, out: &mut Vec<RunIndexRow>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_run_files(&path, goal_id, out)?;
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
        let relative = path
            .strip_prefix(
                path.parent()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.parent())
                    .unwrap_or(dir),
            )
            .unwrap_or(&path);
        out.push(RunIndexRow {
            goal_id: goal_id.to_string(),
            timestamp: value
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            path: format!("goals/{goal_id}/runs/{}", relative.display()),
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
    }
}

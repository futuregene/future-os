//! Run compaction (G-5) — ARCHIVE (never delete) older run files under
//! `<runtime>/goals/<id>/runs/archive/` and re-point the index, mirroring
//! LoopX `control_plane/runtime/run_compaction.py` (compact run base fields)
//! plus the run-artifact lifecycle. The authoritative spend ledger
//! (`runs.jsonl`) is untouched — compaction only touches the run-history
//! file projection, and every archived file stays recoverable.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::runtime::run_history::{read_index_rows, row_epoch, RunIndexRow};

pub const RUN_ARCHIVE_DIR: &str = "archive";

/// The compact field projection for one run record (LoopX
/// `RUN_BASE_COMPACT_FIELDS` subset — the fields our RunRecord carries).
pub fn compact_run_record(record: &crate::state::RunRecord) -> serde_json::Value {
    serde_json::json!({
        "turn": record.turn,
        "todo_id": record.todo_id,
        "run_id": record.run_id,
        "terminal_state": record.terminal_state,
        "tools": record.tools,
        "tokens_in": record.tokens_in_delta,
        "tokens_out": record.tokens_out_delta,
        "cost": record.cost_delta,
        "evidence": crate::decision::truncate(&record.evidence, 240),
        "recorded_at": record.recorded_at,
        "spend_source": record.spend_source,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct CompactionReport {
    pub goal_id: String,
    pub cutoff: u64,
    pub archived: Vec<String>,
    pub kept: usize,
    pub archive_dir: String,
    pub recoverable: bool,
}

/// Move run files older than `cutoff_epoch` into `runs/archive/` and
/// re-point the index rows at their archived paths. The old index is backed
/// up first (rollback), the rewrite is tmp+rename, and nothing is deleted.
pub fn archive_runs_before(
    runtime_root: &str,
    goal_id: &str,
    cutoff_epoch: u64,
) -> Result<CompactionReport> {
    let runs = crate::runtime::runs_dir(runtime_root, goal_id);
    let index = runs.join("index.jsonl");
    if !index.exists() {
        anyhow::bail!("goal {goal_id} has no run index to compact");
    }
    let rows = read_index_rows(&index)?;
    let archive_dir = runs.join(RUN_ARCHIVE_DIR);

    let mut archived: Vec<String> = vec![];
    let mut rewritten: Vec<RunIndexRow> = vec![];
    for row in rows {
        let Some(epoch) = row_epoch(&row) else {
            rewritten.push(row);
            continue;
        };
        if epoch >= cutoff_epoch {
            rewritten.push(row);
            continue;
        }
        // Move the artifact files (json + md) into archive/, preserving
        // recoverability; then re-point the index row.
        let file_name = Path::new(&row.path)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = file_name.trim_end_matches(".json");
        let mut moved = false;
        for suffix in [".json", ".md"] {
            let src = runs.join(format!("{stem}{suffix}"));
            if src.exists() {
                std::fs::create_dir_all(&archive_dir).context("create archive dir")?;
                let dest = archive_dir.join(format!("{stem}{suffix}"));
                if !dest.exists() {
                    std::fs::rename(&src, &dest).context("move run file to archive")?;
                }
                moved = true;
            }
        }
        let archived_path = format!("goals/{goal_id}/runs/archive/{stem}.json");
        if moved {
            archived.push(archived_path.clone());
        }
        let mut rewritten_row = row;
        rewritten_row.path = archived_path;
        rewritten.push(rewritten_row);
    }

    if !archived.is_empty() {
        // Backup the pre-compaction index (rollback), then rewrite.
        let backup = runs.join(format!(
            "index.pre-compaction-{}.jsonl",
            crate::state::now_epoch()
        ));
        std::fs::copy(&index, &backup).context("backup run index")?;
        let payload = rewritten
            .iter()
            .map(|row| {
                serde_json::json!({
                    "goal_id": row.goal_id,
                    "timestamp": row.timestamp,
                    "path": row.path,
                    "turn": row.turn,
                    "classification": row.classification,
                })
            })
            .collect::<Vec<_>>();
        let mut text = String::new();
        for value in payload {
            text.push_str(&serde_json::to_string(&value)?);
            text.push('\n');
        }
        let tmp = index.with_extension("jsonl.tmp");
        std::fs::write(&tmp, text).context("write compacted index tmp")?;
        std::fs::rename(&tmp, &index).context("rename compacted index")?;
    }

    Ok(CompactionReport {
        goal_id: goal_id.to_string(),
        cutoff: cutoff_epoch,
        archived,
        kept: rewritten.len(),
        archive_dir: archive_dir.to_string_lossy().into_owned(),
        recoverable: true,
    })
}

/// Archive everything except the latest `keep` runs (LoopX retention-driven
/// compaction helper).
pub fn archive_keeping_latest(
    runtime_root: &str,
    goal_id: &str,
    keep: usize,
) -> Result<CompactionReport> {
    let index = crate::runtime::index_path(runtime_root, goal_id);
    let mut rows = read_index_rows(&index)?;
    if rows.len() <= keep {
        return Ok(CompactionReport {
            goal_id: goal_id.to_string(),
            cutoff: 0,
            archived: vec![],
            kept: rows.len(),
            archive_dir: crate::runtime::runs_dir(runtime_root, goal_id)
                .join(RUN_ARCHIVE_DIR)
                .to_string_lossy()
                .into_owned(),
            recoverable: true,
        });
    }
    rows.sort_by_key(|b| std::cmp::Reverse(row_epoch(b)));
    let cutoff_epoch = row_epoch(&rows[keep]).unwrap_or(0);
    archive_runs_before(runtime_root, goal_id, cutoff_epoch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_record_is_a_field_subset() {
        let record = crate::state::RunRecord {
            turn: 1,
            todo_id: "t1".to_string(),
            run_id: "r1".to_string(),
            terminal_state: "completed".to_string(),
            error: Some("nope".to_string()),
            tokens_in_delta: 10,
            tokens_out_delta: 20,
            cost_delta: 0.5,
            tools: vec!["shell".to_string()],
            evidence: "did work".to_string(),
            recorded_at: 1_700_000_000,
            spend_source: Some("run".to_string()),
            validation: None,
            failure_kind: None,
            truncation: None,
        };
        let compact = compact_run_record(&record);
        assert_eq!(compact["terminal_state"], "completed");
        assert!(
            compact.get("error").is_none(),
            "error is not a compact field"
        );
        assert_eq!(compact["spend_source"], "run");
    }

    #[test]
    fn archive_keeps_unparseable_rows_and_existing_destinations() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("goals").join("g1").join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        let index = runs.join("index.jsonl");
        // A row whose timestamp does not parse is kept verbatim...
        std::fs::write(
            &index,
            "{\"goal_id\":\"g1\",\"timestamp\":\"not-a-date\",\"path\":\"goals/g1/runs/x.json\",\"turn\":0,\"classification\":\"run_recorded\"}\n\
             {\"goal_id\":\"g1\",\"timestamp\":\"2026-07-01T00:00:00+00:00\",\"path\":\"goals/g1/runs/2026-07-01T00-00-00-00-00.json\",\"turn\":1,\"classification\":\"run_recorded\"}\n",
        )
        .unwrap();
        std::fs::write(runs.join("2026-07-01T00-00-00-00-00.json"), "{}").unwrap();
        // ...and a pre-existing archive destination is left in place (no rename).
        let archive = runs.join("archive");
        std::fs::create_dir_all(&archive).unwrap();
        std::fs::write(
            archive.join("2026-07-01T00-00-00-00-00.json"),
            "{\"old\":true}",
        )
        .unwrap();
        let cutoff = crate::scheduler::state::parse_epoch("2026-08-05T00:00:00+00:00").unwrap();
        let report = archive_runs_before(dir.path().to_str().unwrap(), "g1", cutoff).unwrap();
        assert_eq!(report.archived.len(), 1);
        let rows = read_index_rows(&index).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "goals/g1/runs/x.json", "unparseable row kept");
        assert!(rows[1].path.contains("archive/"));
        // The old run file stays put because the destination already existed.
        assert!(runs.join("2026-07-01T00-00-00-00-00.json").exists());
        let kept = std::fs::read_to_string(archive.join("2026-07-01T00-00-00-00-00.json")).unwrap();
        assert_eq!(kept, "{\"old\":true}");
    }

    #[test]
    fn archive_moves_old_runs_and_repoints_index() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("goals").join("g1").join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        let index = runs.join("index.jsonl");
        std::fs::write(
            &index,
            "{\"goal_id\":\"g1\",\"timestamp\":\"2026-07-01T00:00:00+00:00\",\"path\":\"goals/g1/runs/2026-07-01T00-00-00-00-00.json\",\"turn\":1,\"classification\":\"run_recorded\"}\n\
             {\"goal_id\":\"g1\",\"timestamp\":\"2026-08-05T00:00:00+00:00\",\"path\":\"goals/g1/runs/2026-08-05T00-00-00-00-00.json\",\"turn\":2,\"classification\":\"run_recorded\"}\n",
        )
        .unwrap();
        std::fs::write(runs.join("2026-07-01T00-00-00-00-00.json"), "{}").unwrap();
        std::fs::write(runs.join("2026-08-05T00-00-00-00-00.json"), "{}").unwrap();

        let now = crate::scheduler::state::parse_epoch("2026-08-05T00:00:00+00:00").unwrap();
        let cutoff = now.saturating_sub(3 * 86400);
        let report = archive_runs_before(dir.path().to_str().unwrap(), "g1", cutoff).unwrap();
        assert_eq!(report.archived.len(), 1);
        assert!(report.archived[0].contains("archive/"));
        assert!(runs.join("archive/2026-07-01T00-00-00-00-00.json").exists());
        // The archived row is re-pointed; the fresh row is untouched.
        let rows = read_index_rows(&index).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].path.contains("archive/"));
        assert_eq!(rows[1].path, "goals/g1/runs/2026-08-05T00-00-00-00-00.json");
        // Index backup exists (rollback).
        let backups: Vec<_> = std::fs::read_dir(&runs)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("index.pre-compaction")
            })
            .collect();
        assert_eq!(backups.len(), 1);
    }
}

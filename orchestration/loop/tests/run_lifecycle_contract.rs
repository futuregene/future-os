//! G-5 run-lifecycle contract tests: run history projection (24h/7d by
//! class), compaction (archive never delete), index dedup + rebuild, context
//! retention policy, and the stale-latest-run warning.

use future_loop::runtime::run_compaction;
use future_loop::runtime::run_context_retention;
use future_loop::runtime::run_history;
use future_loop::runtime::run_index;
use future_loop::runtime::stale_latest_run;

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-p2-runs-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn index_row(ts: &str, path: &str, classification: &str) -> String {
    format!(
        "{{\"goal_id\":\"g1\",\"timestamp\":\"{ts}\",\"path\":\"{path}\",\"turn\":1,\"classification\":\"{classification}\"}}\n"
    )
}

/// Seed run FILES (the source of truth). The run index is a derived
/// projection (no persisted index.jsonl), so readers scan these files.
/// Each file carries a real `timestamp` + `terminal_state` payload.
fn seed_run_files(root: &str, rows: &[(&str, &str, &str)]) {
    let runs = future_loop::runtime::runs_dir(root, "g1");
    std::fs::create_dir_all(&runs).unwrap();
    for (ts, path, classification) in rows {
        let file_name = path.rsplit('/').next().unwrap();
        std::fs::write(
            runs.join(file_name),
            format!(
                "{{\"timestamp\":\"{ts}\",\"turn\":1,\"terminal_state\":\"{classification}\"}}"
            ),
        )
        .unwrap();
    }
}

/// ── Run history buckets 24h/7d by class (event-ledger proxy) ──────────────
#[test]
fn run_history_buckets_by_class() {
    let root = tmp_root("history");
    seed_run_files(
        &root,
        &[
            (
                "2026-08-05T12:00:00+00:00",
                "goals/g1/runs/a.json",
                "run_recorded",
            ),
            (
                "2026-08-04T12:00:00+00:00",
                "goals/g1/runs/b.json",
                "run_recorded",
            ),
            (
                "2026-07-20T12:00:00+00:00",
                "goals/g1/runs/c.json",
                "quota_monitor_poll",
            ),
        ],
    );
    let now = future_loop::scheduler::state::parse_epoch("2026-08-05T13:00:00+00:00").unwrap();
    let projection = run_history::build_run_history(&root, "g1", now)
        .unwrap()
        .expect("history exists");
    assert_eq!(projection.sample_run_count, 3);
    assert_eq!(projection.totals.events_24h, 1);
    assert_eq!(projection.totals.events_7d, 2);
    assert_eq!(
        projection.totals.by_class_7d.get("run_recorded"),
        Some(&2u64)
    );
    assert_eq!(
        projection.latest.unwrap().classification,
        "quota_monitor_poll"
    );
}

/// ── Compaction archives (never deletes) and re-points the index ───────────
#[test]
fn compaction_archives_old_runs_recoverably() {
    let root = tmp_root("compact");
    seed_run_files(
        &root,
        &[
            (
                "2026-07-01T00:00:00+00:00",
                "goals/g1/runs/old.json",
                "run_recorded",
            ),
            (
                "2026-08-05T00:00:00+00:00",
                "goals/g1/runs/new.json",
                "run_recorded",
            ),
        ],
    );
    let now = future_loop::scheduler::state::parse_epoch("2026-08-05T00:00:00+00:00").unwrap();
    let report =
        run_compaction::archive_runs_before(&root, "g1", now.saturating_sub(3 * 86400)).unwrap();
    assert_eq!(report.archived.len(), 1);
    assert!(report.recoverable);
    let runs = future_loop::runtime::runs_dir(&root, "g1");
    assert!(
        runs.join("archive/old.json").exists(),
        "archived, not deleted"
    );
    assert!(runs.join("new.json").exists());
    // The index re-derives the archived path on the next read (no persistent
    // index.jsonl to re-point).
    let rows = run_index::load_run_index(&root, "g1").unwrap();
    let old = rows.iter().find(|r| r.path.contains("old.json")).unwrap();
    assert!(old.path.contains("archive/"));
}

/// ── Index dedup detection + non-destructive rebuild ───────────────────────
#[test]
fn index_dedup_and_rebuild() {
    let root = tmp_root("index");
    // Seed run FILES (source of truth) + a hand-written index.jsonl carrying
    // a duplicate row — this exercises the `runs index` on-demand surface.
    seed_run_files(
        &root,
        &[
            (
                "2026-08-05T00:00:00+00:00",
                "goals/g1/runs/a.json",
                "run_recorded",
            ),
            (
                "2026-08-04T00:00:00+00:00",
                "goals/g1/runs/b.json",
                "run_recorded",
            ),
        ],
    );
    let index = future_loop::runtime::index_path(&root, "g1");
    std::fs::write(
        &index,
        format!(
            "{}{}",
            index_row(
                "2026-08-05T00:00:00+00:00",
                "goals/g1/runs/a.json",
                "run_recorded"
            ),
            index_row(
                "2026-08-05T00:00:00+00:00",
                "goals/g1/runs/a.json",
                "run_recorded"
            ),
        ),
    )
    .unwrap();
    let report = run_index::detect_duplicates(&index).unwrap();
    assert_eq!(report.duplicate_groups.len(), 1);
    assert_eq!(report.duplicate_groups[0].line_numbers, vec![1, 2]);
    assert!(report.repairable);

    // Rebuild rescans run files and rewrites the index (non-destructive).
    // Only 2 distinct files exist on disk, so the rebuilt index has 2 rows.
    std::fs::write(&index, "garbage\n").unwrap();
    let rebuild = run_index::rebuild_index(&root, "g1").unwrap();
    assert_eq!(rebuild.rows_written, 2);
    assert!(rebuild.non_destructive);
    let rows = run_history::read_index_rows(&index).unwrap();
    assert_eq!(rows.len(), 2);
    // The corrupt pre-rebuild index was backed up.
    let backups: Vec<_> = std::fs::read_dir(future_loop::runtime::runs_dir(&root, "g1"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("index.pre-rebuild")
        })
        .collect();
    assert_eq!(backups.len(), 1);
}

/// ── Retention policy: keep latest N + TTL; older are candidates ───────────
#[test]
fn retention_keeps_latest_and_ttl() {
    let root = tmp_root("retention");
    seed_run_files(
        &root,
        &[
            (
                "2026-08-28T00:00:00+00:00",
                "goals/g1/runs/d28.json",
                "run_recorded",
            ),
            (
                "2026-08-27T00:00:00+00:00",
                "goals/g1/runs/d27.json",
                "run_recorded",
            ),
            (
                "2026-08-26T00:00:00+00:00",
                "goals/g1/runs/d26.json",
                "run_recorded",
            ),
            (
                "2026-08-25T00:00:00+00:00",
                "goals/g1/runs/d25.json",
                "run_recorded",
            ),
            (
                "2026-08-20T00:00:00+00:00",
                "goals/g1/runs/d20.json",
                "run_recorded",
            ),
        ],
    );
    let now = future_loop::scheduler::state::parse_epoch("2026-08-31T00:00:00+00:00").unwrap();
    let policy = run_context_retention::retention_policy(2, Some(7 * 86400));
    let report = run_context_retention::retention_report(&root, "g1", &policy, now).unwrap();
    assert_eq!(report.total, 5);
    assert_eq!(report.retained, 4, "2 latest + 26/25 inside 7d TTL");
    assert_eq!(report.candidates.len(), 1);
    assert!(report.candidates[0].contains("d20"));
}

/// ── Stale-latest-run warns when state is newer than the latest run ────────
#[test]
fn stale_latest_run_warns_and_clears() {
    let root = tmp_root("stale");
    let mut store = future_loop::store::Store::open(&root).unwrap();
    let goal = future_loop::state::Goal::new("g1", "objective", "/tmp");
    store.register(&goal).unwrap();
    let ts = goal.created_at;
    store
        .append(future_loop::store::Event::GoalStarted {
            goal_id: "g1".into(),
            ts,
        })
        .unwrap();
    let mut todo = future_loop::state::Todo::advancement("t1", "work");
    todo.updated_at = 2_000;
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: "g1".into(),
            todo,
            ts,
        })
        .unwrap();
    let mut record = future_loop::state::RunRecord {
        turn: 1,
        todo_id: "t1".to_string(),
        run_id: "r1".to_string(),
        terminal_state: "completed".to_string(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec![],
        evidence: String::new(),
        recorded_at: 1_000,
        spend_source: Some("run".to_string()),
        validation: None,
        failure_kind: None,
        truncation: None,
    };
    record.recorded_at = 1_000;
    store.append_run("g1", &record).unwrap();

    let goal = store.replay("g1").unwrap().unwrap();
    let warning = stale_latest_run::stale_latest_run(&goal, &store.goal_dir("g1"));
    assert!(warning.is_some());
    assert_eq!(
        warning.unwrap().reason,
        "active_state_updated_after_latest_run"
    );

    // A newer run clears the warning.
    record.recorded_at = 3_000;
    store.append_run("g1", &record).unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    assert!(stale_latest_run::stale_latest_run(&goal, &store.goal_dir("g1")).is_none());
}

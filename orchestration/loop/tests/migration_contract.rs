//! G-6 schema-migration contract tests: legacy (pre-G-3) ledgers migrate on
//! the read path, the write-path migration is non-destructive + reversible,
//! and the migration bridge is fail-closed until parity/rollback/idempotency/
//! public-boundary checks are clean.

use future_loop::state::{Goal, Todo};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "loopx-p2-migration-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

/// Serialize an event WITHOUT the G-3 envelope (simulates a pre-G-3 line).
fn legacy_line(event: &Event) -> String {
    serde_json::to_string(event).unwrap() + "\n"
}

/// ── Legacy ledger (no event ids, no schema stamp) replays after read-path
/// migration, and `store verify` derives stable ids for legacy lines ────────
#[test]
fn legacy_ledger_reads_after_migration_and_verify_derives_ids() {
    let root = tmp_root("legacy");
    let mut store = Store::open(&root).unwrap();
    let goal = Goal::new("g1", "objective", "/tmp");
    store.register(&goal).unwrap();
    // Simulate a pre-G-3 ledger: raw lines WITHOUT event_id and no schema.json.
    let goal_dir = store.goal_dir("g1");
    std::fs::create_dir_all(&goal_dir).unwrap();
    std::fs::write(
        goal_dir.join("events.jsonl"),
        format!(
            "{}{}",
            legacy_line(&Event::GoalStarted {
                goal_id: "g1".into(),
                ts: 100,
            }),
            legacy_line(&Event::TodoAdded {
                goal_id: "g1".into(),
                todo: Todo::advancement("t1", "work"),
                ts: 200,
            }),
        ),
    )
    .unwrap();

    // Replay works through the migration read path.
    let goal = store.replay("g1").unwrap().expect("goal replays");
    assert_eq!(goal.todos.len(), 1);
    assert_eq!(goal.todo("t1").unwrap().text, "work");

    // verify() is clean and counts the legacy lines (ids derived, no conflict).
    let report = store.verify("g1").unwrap();
    assert!(report.ok);
    assert_eq!(report.legacy_lines_without_id, 2);
    assert_eq!(report.total_events, 2);
    assert_eq!(report.unique_events, 2);

    // events() exposes content-derived ids for legacy lines.
    let events = store.events("g1").unwrap();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|e| e.effective_id().starts_with("evt-")));
    // No schema stamp yet (read-path migration is in-memory only).
    assert!(store.goal_schema_version("g1").is_none());
}

/// ── Write-path migration: backup + rewrite + stamp, then replay ───────────
#[test]
fn write_path_migration_is_reversible_and_replay_is_stable() {
    let root = tmp_root("write-migrate");
    let mut store = Store::open(&root).unwrap();
    let goal = Goal::new("g1", "objective", "/tmp");
    store.register(&goal).unwrap();
    let goal_dir = store.goal_dir("g1");
    std::fs::create_dir_all(&goal_dir).unwrap();
    let legacy = format!(
        "{}{}",
        legacy_line(&Event::GoalStarted {
            goal_id: "g1".into(),
            ts: 100,
        }),
        legacy_line(&Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "work"),
            ts: 200,
        }),
    );
    std::fs::write(goal_dir.join("events.jsonl"), &legacy).unwrap();

    let report = future_loop::migration::apply_migrations(&goal_dir, "g1").unwrap();
    assert_eq!(report.migrated_lines, 2);
    assert!(report.non_destructive);
    // Backup is byte-identical (rollback plan).
    let backup = std::fs::read_to_string(&report.backup_path).unwrap();
    assert_eq!(backup, legacy);
    // Stamp bumped.
    assert_eq!(
        store.goal_schema_version("g1").as_deref(),
        Some(future_loop::store::EVENT_STORE_SCHEMA_VERSION)
    );
    // Post-migration replay is unchanged.
    let goal = store.replay("g1").unwrap().unwrap();
    assert_eq!(goal.todos.len(), 1);
    // verify() now sees ids on every line.
    let report = store.verify("g1").unwrap();
    assert_eq!(report.legacy_lines_without_id, 0);
    assert!(report.ok);

    // Restore the backup (rollback) → legacy replay still works.
    std::fs::write(goal_dir.join("events.jsonl"), legacy).unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    assert_eq!(goal.todos.len(), 1);
}

/// ── Bridge is fail-closed with no prerequisites and never auto-promotes ───
#[test]
fn migration_bridge_is_fail_closed() {
    let root = tmp_root("bridge");
    let mut store = Store::open(&root).unwrap();
    let goal = Goal::new("g1", "objective", "/tmp");
    store.register(&goal).unwrap();
    let ts = goal.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "work"),
            ts,
        })
        .unwrap();
    store.set_next_action("g1", "do work").unwrap();

    let bridge =
        future_loop::migration::migration_bridge_status(&store, "g1", &store.goal_dir("g1"));
    assert_eq!(bridge.schema_version, "event_store_migration_bridge_v0");
    assert!(!bridge.promotion_allowed, "promotion always fail-closed");
    assert!(bridge.checks.event_read_path_ready);
    assert!(bridge.checks.active_state_projection_ready);
    assert!(bridge.checks.idempotency_conflicts_clean);
    // Rollback not recorded until a backup exists.
    assert!(!bridge.checks.rollback_plan_recorded);
    assert_eq!(
        bridge.stage, "dual_read_shadow",
        "shadow prerequisites missing"
    );

    // After a backup the rollback check passes, but parity is still missing
    // (no ACTIVE_GOAL_STATE.md), so the bridge stays in dual_read_shadow —
    // fail-closed until the Markdown/event parity is clean.
    store.backup_goal("g1").unwrap();
    let bridge =
        future_loop::migration::migration_bridge_status(&store, "g1", &store.goal_dir("g1"));
    assert!(bridge.checks.rollback_plan_recorded);
    assert_eq!(bridge.stage, "dual_read_shadow");
    assert!(!bridge.promotion_allowed);
    assert!(bridge
        .missing_for_canary
        .contains(&"dual_read_parity_clean".to_string()));
}

/// ── Bridge helper: all checks pass → promotion_candidate, still closed ────
#[test]
fn bridge_candidate_never_promotes() {
    let checks = future_loop::migration::MigrationChecks {
        event_read_path_ready: true,
        active_state_projection_ready: true,
        dual_read_parity_clean: true,
        rollback_plan_recorded: true,
        bounded_canary_passed: true,
        idempotency_conflicts_clean: true,
        public_boundary_clean: true,
        event_projection_head_matches_store: true,
    };
    let bridge = future_loop::migration::build_migration_bridge("g1", checks, true);
    assert_eq!(bridge.stage, "promotion_candidate");
    assert!(bridge.promotion_candidate);
    assert!(!bridge.promotion_allowed, "fail-closed by contract");
    assert!(bridge.missing_for_promotion.is_empty());
}

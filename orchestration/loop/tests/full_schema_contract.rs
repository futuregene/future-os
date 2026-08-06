//! Full-schema contract tests: every todo_item_v0 field round-trips through
//! the event ledger; todo_summary aggregation; heartbeat_recommendation and
//! automation pause_policy in the decision packet.

use std::time::{Duration, SystemTime};

use future_loop::decision::decide;
use future_loop::state::{Goal, Todo, TodoRole, TodoStatus};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("future-loop-schema-full-{tag}-{}", nano()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}
fn nano() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

// ── Full todo_item_v0 field surface ────────────────────────────────────────
#[test]
fn todo_exposes_all_future_loop_fields() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(
        Todo::advancement("t1", "Run the benchmark")
            .with_title("Run bench")
            .at_priority(future_loop::state::Priority::P0)
            .with_action_kind("benchmark")
            .with_repository("github.com/acme/repo")
            .with_continuation_policy("independent_handoff")
            .with_write_scopes(&["results/", "logs/"])
            .with_capability_binding("bench:run")
            .requiring("bench"),
    );
    let t = goal.todo("t1").unwrap();
    assert_eq!(t.title, "Run bench");
    assert_eq!(t.role, TodoRole::Agent);
    assert!(t.index >= 1, "index assigned by goal");
    assert_eq!(t.action_kind.as_deref(), Some("benchmark"));
    assert_eq!(t.task_repository.as_deref(), Some("github.com/acme/repo"));
    assert_eq!(
        t.continuation_policy.as_deref(),
        Some("independent_handoff")
    );
    assert_eq!(t.required_write_scope, vec!["results/", "logs/"]);
    assert_eq!(t.capability_binding_ref.as_deref(), Some("bench:run"));
    assert_eq!(t.archive_state, "active");
    assert_eq!(t.updated_at, t.updated_at); // audit timestamps present
}

// ── All fields survive the event ledger round-trip ─────────────────────────
#[test]
fn full_schema_survives_replay() {
    let root = tmp_root("roundtrip");
    let mut store = Store::open(&root).unwrap();
    let g = Goal::new("g1", "objective", "/tmp");
    store.register(&g).unwrap();
    let ts = g.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "Work")
                .with_action_kind("shell")
                .with_repository("github.com/acme/repo")
                .with_continuation_policy("independent_handoff")
                .with_capability_binding("shell:exec"),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoCompleted {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            no_follow_up: true,
            successor_ids: vec![],
            evidence: None,
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoArchived {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            ts,
        })
        .unwrap();

    let rebuilt = Store::open(&root).unwrap().replay("g1").unwrap().unwrap();
    let t = rebuilt.todo("t1").unwrap();
    assert_eq!(t.action_kind.as_deref(), Some("shell"));
    assert_eq!(t.task_repository.as_deref(), Some("github.com/acme/repo"));
    assert_eq!(
        t.continuation_policy.as_deref(),
        Some("independent_handoff")
    );
    assert_eq!(t.capability_binding_ref.as_deref(), Some("shell:exec"));
    assert_eq!(t.status, TodoStatus::Done);
    assert!(t.completed_at.is_some(), "completed_at recorded");
    assert_eq!(t.archive_state, "archived");
}

// ── todo_summary aggregation ───────────────────────────────────────────────
#[test]
fn todo_summary_aggregates_by_role_with_proofs() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("a1", "agent work 1"));
    goal.add(Todo::advancement("a2", "agent work 2"));
    goal.add(Todo::user_gate("g1", "Approve X", &[]));
    goal.todo_mut("a1").unwrap().complete(true, vec![]);

    let s = goal.todo_summary();
    assert_eq!(s.schema_version, "todo_summary_v0");
    assert_eq!(s.user_open, 1);
    assert_eq!(s.user_done, 0);
    assert_eq!(s.agent_open, 1);
    assert_eq!(s.agent_done, 1);
    assert_eq!(s.source_proof.schema_version, "todo_source_proof_v0");
    assert_eq!(s.source_proof.item_count, 3);
    assert!(!s.terminal_closure_proof.all_todos_done);
    assert_eq!(
        s.terminal_closure_proof.schema_version,
        "todo_terminal_closure_proof_v0"
    );
}

#[test]
fn todo_summary_terminal_proof_valid_at_closure() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("a1", "work"));
    goal.todo_mut("a1").unwrap().complete(true, vec![]);
    let s = goal.todo_summary();
    assert!(s.terminal_closure_proof.all_todos_done);
    assert_eq!(s.terminal_closure_proof.no_followup_count, 1);
    assert_eq!(s.terminal_closure_proof.successor_gap_count, 0);
    assert!(goal.terminal_closure().is_some());
}

// ── heartbeat_recommendation + pause_policy in the packet ──────────────────
#[test]
fn packet_carries_heartbeat_and_pause_policy() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("t1", "Work"));
    let p = decide(&goal, SystemTime::now());
    assert_eq!(
        p.heartbeat_recommendation.recommended_mode,
        "steering_audit_then_one_step"
    );
    assert!(p
        .heartbeat_recommendation
        .spend_policy
        .contains("validated"));
    assert!(p
        .automation_liveness
        .pause_policy
        .contains("pause/delete only after"));
}

#[test]
fn terminal_packet_heartbeat_mode() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("t1", "Work"));
    goal.todo_mut("t1").unwrap().complete(true, vec![]);
    let p = decide(&goal, SystemTime::now());
    assert_eq!(
        p.heartbeat_recommendation.recommended_mode,
        "terminal_no_followup"
    );
    assert!(!p.automation_liveness.keep_active);
    assert!(p.automation_liveness.pause_allowed);
}

// ── deferred + monitor keep the summary honest ─────────────────────────────
#[test]
fn deferred_counts_as_pending_not_terminal() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::deferred("d1", "later", Duration::from_secs(3600)));
    assert!(!goal.is_terminal());
    assert!(!goal.todo_summary().terminal_closure_proof.all_todos_done);
}

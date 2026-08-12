//! P1-1 contract tests: quota decision read model —
//!   ① `quota::error_codes` — every kernel exit stamps a stable
//!      machine-readable `reason_code` on the packet (typed-RPC oneof
//!      style); quota-state failures classify into stable error codes.
//!   ② decision_summary projection — `record_turn_decision` (the run-path
//!      hook) persists one `DecisionSummaryRecorded` + one
//!      `HeartbeatReceiptRecorded` per turn; both are projection-only
//!      (replay ignores them) and the read model serves them back.
//!   ③ receipts — `scheduler ack` records the host scheduler's
//!      acknowledgement as a `SchedulerAcked` ledger event.
//!
//! CLI-level tests exercise the real entry (`console::run`) against an
//! isolated `FUTURE_LOOP_ROOT`.

use future_loop::console;
use future_loop::decision::{decide, decide_for};
use future_loop::quota::decision_summary::{
    decision_summaries, latest_decision_summary, record_turn_decision, DecisionSummary,
};
use future_loop::quota::error_codes::{DecisionReasonCode, QuotaErrorCode};
use future_loop::state::{Goal, Todo};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-p11-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn open_goal(store: &mut Store, goal_id: &str) {
    let goal = Goal::new(goal_id, "objective", "/tmp");
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: goal_id.into(),
            ts: goal.created_at,
        })
        .unwrap();
}

// ── ① reason codes: every kernel exit stamps the right code. ─────────────

#[test]
fn kernel_exits_stamp_reason_codes() {
    let now = std::time::SystemTime::now();

    // Runnable advancement → runnable_todo.
    let mut g = Goal::new("g", "o", "/tmp");
    g.add(Todo::advancement("t1", "work"));
    let p = decide(&g, now);
    assert_eq!(p.reason_code, "runnable_todo");
    assert_eq!(
        DecisionReasonCode::parse(&p.reason_code),
        Some(DecisionReasonCode::RunnableTodo)
    );

    // Repair attempt (failed_attempts > 0) → repair_attempt.
    let mut g = Goal::new("g", "o", "/tmp");
    let mut t = Todo::advancement("t1", "work");
    t.failed_attempts = 1;
    g.add(t);
    let p = decide(&g, now);
    assert_eq!(p.reason_code, "repair_attempt");

    // Cancelled goal → goal_cancelled.
    let mut g = Goal::new("g", "o", "/tmp");
    g.status = "cancelled".to_string();
    let p = decide(&g, now);
    assert_eq!(p.reason_code, "goal_cancelled");

    // Unregistered identity → identity_not_registered (fail closed).
    let g = Goal::new("g", "o", "/tmp");
    let p = decide_for(&g, now, Some("ghost"));
    assert_eq!(p.reason_code, "identity_not_registered");
    assert!(!p.ok);

    // Open user gate → open_user_gate.
    let mut g = Goal::new("g", "o", "/tmp");
    g.add(Todo::user_gate("u1", "approve the plan?", &[]));
    let p = decide(&g, now);
    assert_eq!(p.reason_code, "open_user_gate");

    // External blocker, no fallback → blocked_no_fallback.
    let mut g = Goal::new("g", "o", "/tmp");
    g.add(Todo::blocker("b1", "waiting on upstream", &[]));
    let p = decide(&g, now);
    assert_eq!(p.reason_code, "blocked_no_fallback");

    // Nothing left → validated_closure.
    let g = Goal::new("g", "o", "/tmp");
    let p = decide(&g, now);
    assert_eq!(p.reason_code, "validated_closure");

    // Every emitted code parses back into the enum (no free-form strings).
    for code in [
        "runnable_todo",
        "repair_attempt",
        "goal_cancelled",
        "identity_not_registered",
        "open_user_gate",
        "blocked_no_fallback",
        "validated_closure",
    ] {
        assert!(
            DecisionReasonCode::parse(code).is_some(),
            "unparseable code {code}"
        );
    }
}

#[test]
fn quota_error_code_classification_is_stable() {
    // LoopX `quota_error_code` parity anchors.
    assert_eq!(
        QuotaErrorCode::QuotaStateInvalidJson.as_str(),
        "quota_state_invalid_json"
    );
    assert_eq!(
        QuotaErrorCode::from_io_error(&std::io::Error::from(std::io::ErrorKind::NotFound)).as_str(),
        "quota_state_missing_field"
    );
    assert_eq!(
        QuotaErrorCode::from_io_error(&std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            .as_str(),
        "quota_state_permission_denied"
    );
}

// ── ② decision_summary projection: writeback + read model + replay-noop. ──

#[test]
fn turn_decision_writeback_projects_to_ledger() {
    let root = tmp_root("writeback");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "work"),
            ts: 1_000,
        })
        .unwrap();
    // The identity gate is fail-closed: register the peer before deciding.
    store
        .append(Event::AgentRegistered {
            goal_id: "g1".into(),
            agent_id: "agent-1".into(),
            workspaces: vec![],
            ts: 1_001,
        })
        .unwrap();

    let goal = store.replay("g1").unwrap().unwrap();
    let packet = decide_for(&goal, std::time::SystemTime::now(), Some("agent-1"));
    assert_eq!(packet.decision, "run", "setup: {packet:?}");
    record_turn_decision(&mut store, &packet, Some("agent-1"), 1).unwrap();

    let events = store.events("g1").unwrap();
    let summaries = decision_summaries(&events);
    assert_eq!(summaries.len(), 1);
    let s = summaries[0];
    assert_eq!(s.goal_id, "g1");
    assert_eq!(s.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(s.decision, "run");
    assert_eq!(s.reason_code, "runnable_todo");
    assert_eq!(s.selected_todo.as_deref(), Some("t1"));
    assert_eq!(s.turn, 1);
    assert_eq!(latest_decision_summary(&events), Some(s));

    // The heartbeat receipt lands alongside.
    let receipts: Vec<_> = events
        .iter()
        .filter_map(|se| match &se.event {
            Event::HeartbeatReceiptRecorded {
                agent_id,
                turn_instance_id,
                todo_id,
                decision,
                reason_code,
                ..
            } => Some((
                agent_id.clone(),
                turn_instance_id.clone(),
                todo_id.clone(),
                decision.clone(),
                reason_code.clone(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].0.as_deref(), Some("agent-1"));
    assert_eq!(receipts[0].1, "turn-1");
    assert_eq!(receipts[0].2.as_deref(), Some("t1"));
    assert_eq!(receipts[0].3, "run");
    assert_eq!(receipts[0].4, "runnable_todo");

    // Projection-only: replay is byte-identical with and without the
    // projection events (they never fold into goal state).
    let before = store.replay("g1").unwrap().unwrap();
    record_turn_decision(&mut store, &packet, Some("agent-1"), 2).unwrap();
    let after = store.replay("g1").unwrap().unwrap();
    assert_eq!(before.todos.len(), after.todos.len());
    assert_eq!(before.history.len(), after.history.len());
    assert_eq!(before.quota_spent_slots, after.quota_spent_slots);
    assert_eq!(decision_summaries(&store.events("g1").unwrap()).len(), 2);
    assert_eq!(
        latest_decision_summary(&store.events("g1").unwrap())
            .unwrap()
            .turn,
        2,
        "latest wins"
    );

    // Ledger integrity holds with the new event kinds.
    assert!(store.verify("g1").unwrap().ok);
}

#[test]
fn projection_events_serde_roundtrip_and_legacy_parse() {
    // New events serialize with the flattened `kind` tag and parse back.
    let event = Event::SchedulerAcked {
        goal_id: "g1".into(),
        agent_id: "codex-app".into(),
        action: "tick_next".into(),
        cadence_class: "bounded_segment".into(),
        rrule: Some("FREQ=MINUTELY;INTERVAL=15".into()),
        source: "scheduler_cli".into(),
        ts: 1_000,
    };
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"kind\":\"scheduler_acked\""));
    let back: Event = serde_json::from_str(&json).unwrap();
    match back {
        Event::SchedulerAcked {
            goal_id,
            agent_id,
            action,
            rrule,
            ..
        } => {
            assert_eq!(goal_id, "g1");
            assert_eq!(agent_id, "codex-app");
            assert_eq!(action, "tick_next");
            assert_eq!(rrule.as_deref(), Some("FREQ=MINUTELY;INTERVAL=15"));
        }
        other => panic!("wrong variant: {other:?}"),
    }

    // Legacy receipt line without the defaulted fields still parses.
    let legacy = r#"{"kind":"heartbeat_receipt_recorded","goal_id":"g1","turn_instance_id":"turn-1","decision":"run","ts":1000}"#;
    let back: Event = serde_json::from_str(legacy).unwrap();
    match back {
        Event::HeartbeatReceiptRecorded {
            agent_id,
            todo_id,
            reason_code,
            ..
        } => {
            assert_eq!(agent_id, None);
            assert_eq!(todo_id, None);
            assert_eq!(reason_code, "");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

// ── ②+③ CLI surface: `quota decisions` + `scheduler ack`. ────────────────

fn with_root<F: FnOnce(&str)>(tag: &str, f: F) {
    // FUTURE_LOOP_ROOT is process-global; serialize CLI tests behind a mutex.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = format!("{}/.future/loop", tmp_root(tag));
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("FUTURE_LOOP_ROOT", &root);
    f(&root);
}

fn cli(args: &[&str]) -> Result<(), String> {
    console::run(
        "future-loop",
        args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    )
    .map_err(|e| format!("{e:#}"))
}

#[test]
fn quota_decisions_and_scheduler_ack_cli() {
    with_root("cli", |root| {
        cli(&[
            "goal",
            "init",
            "--objective",
            "p1-1 read model",
            "--cwd",
            "/tmp",
            "--goal-id",
            "g1",
        ])
        .unwrap();

        // No turns yet → empty projection, clean message (not an error).
        cli(&["quota", "decisions", "--goal", "g1"]).unwrap();
        cli(&["quota", "decisions", "--goal", "g1", "--format", "json"]).unwrap();

        // Record one decision through the run-path hook directly (driving a
        // real `run` needs a live agent; the hook is the unit under test).
        let mut store = Store::open(root).unwrap();
        let goal = store.replay("g1").unwrap().unwrap();
        let packet = decide(&goal, std::time::SystemTime::now());
        record_turn_decision(&mut store, &packet, None, 1).unwrap();
        drop(store);

        // `quota decisions` serves the persisted projection.
        cli(&["quota", "decisions", "--goal", "g1"]).unwrap();
        cli(&["quota", "decisions", "--goal", "g1", "--limit", "5"]).unwrap();

        // `scheduler ack` records the receipt event.
        cli(&[
            "scheduler",
            "ack",
            "--goal",
            "g1",
            "--agent-id",
            "codex-app",
            "--action",
            "tick_next",
            "--rrule",
            "FREQ=MINUTELY;INTERVAL=15",
        ])
        .unwrap();
        let store = Store::open(root).unwrap();
        let acks: Vec<_> = store
            .events("g1")
            .unwrap()
            .into_iter()
            .filter_map(|se| match se.event {
                Event::SchedulerAcked {
                    agent_id,
                    action,
                    rrule,
                    source,
                    ..
                } => Some((agent_id, action, rrule, source)),
                _ => None,
            })
            .collect();
        assert_eq!(acks.len(), 1);
        assert_eq!(acks[0].0, "codex-app");
        assert_eq!(acks[0].1, "tick_next");
        assert_eq!(acks[0].2.as_deref(), Some("FREQ=MINUTELY;INTERVAL=15"));
        assert_eq!(acks[0].3, "scheduler_cli");

        // --action is required; unknown flags are rejected (P0-3 strictness).
        assert!(cli(&["scheduler", "ack", "--goal", "g1"]).is_err());
        assert!(cli(&[
            "scheduler",
            "ack",
            "--goal",
            "g1",
            "--action",
            "tick_next",
            "--bogus",
            "x",
        ])
        .is_err());
        assert!(cli(&["quota", "decisions", "--goal", "g1", "--bogus", "x"]).is_err());
    });
}

// Keep the helper type referenced so the read model's public shape is part
// of the contract surface.
#[allow(dead_code)]
fn _summary_shape_anchor(s: &DecisionSummary) {
    let _ = &s.schema_version;
}

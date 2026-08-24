//! G-16 multi-agent contract tests: identity-scoped frontiers (two agents
//! under one goal never cross), the supervisor proposal/receipt event
//! surface (through the event store), and the agent lane recommendation.

use future_loop::agents::scope::identity_scoped_frontier;
use future_loop::agents::supervisor::{
    build_supervisor_event_projection, record_supervisor_proposal, record_supervisor_receipt,
    SupervisorDecision, SupervisorReceipt, SupervisorReceiptOutcome,
};
use future_loop::state::{Goal, Todo};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-p3-agents-{tag}-{}",
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

/// ── P3 acceptance: two agent sessions hold identity-scoped frontiers ─────
#[test]
fn two_agents_hold_disjoint_identity_scoped_frontiers() {
    let mut t1 = Todo::advancement("t1", "agent A work");
    t1.claimed_by = Some("agent-a".into());
    let mut t2 = Todo::advancement("t2", "agent B work");
    t2.claimed_by = Some("agent-b".into());
    let t3 = Todo::advancement("t3", "unclaimed work");
    let gate = Todo::user_gate("g1", "approve?", &["t1"]);
    let mut goal = Goal::new("g1", "objective", "/tmp");
    goal.todos = vec![t1, t2, t3, gate];

    let a = identity_scoped_frontier(&goal, "agent-a", &[]);
    let b = identity_scoped_frontier(&goal, "agent-b", &[]);

    // Each agent's visible frontier: own claim + unclaimed + gates — never
    // the other agent's claim.
    assert!(a.contains("t1") && a.contains("t3") && a.contains("g1"));
    assert!(!a.contains("t2"), "A must never see B's claimed slice");
    assert_eq!(a.other_agent_claimed_ids, vec!["t2"]);
    assert_eq!(a.unclaimed_advancement_count, 1);

    assert!(b.contains("t2") && b.contains("t3") && b.contains("g1"));
    assert!(!b.contains("t1"), "B must never see A's claimed slice");
    assert_eq!(b.other_agent_claimed_ids, vec!["t1"]);

    // Excluded agent sees nothing (supervisor routing table).
    let excluded = identity_scoped_frontier(&goal, "agent-a", &["agent-a".to_string()]);
    assert!(excluded.visible_agent_todo_ids.is_empty());
}

/// ── Supervisor events: proposal + receipt + projection through the store ─
#[test]
fn supervisor_proposal_receipt_projection_loop() {
    let root = tmp_root("supervisor");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");

    let decision = SupervisorDecision::execute(
        "d-1",
        "agent-b",
        vec!["github".into()],
        "review and merge the change",
    );
    let event = record_supervisor_proposal(&mut store, "g1", "supervisor-1", &decision).unwrap();
    assert!(event.starts_with("evt-"));
    // Idempotent re-proposal (same content → same event id, no new line).
    let again = record_supervisor_proposal(&mut store, "g1", "supervisor-1", &decision).unwrap();
    assert_eq!(event, again);
    let report = store.verify("g1").unwrap();
    assert!(report.ok);

    let receipt = SupervisorReceipt {
        receipt_id: "r-1".into(),
        decision_id: "d-1".into(),
        adapter_id: "adapter-x".into(),
        outcome: SupervisorReceiptOutcome::Executed,
        authority_ref: Some("auth-1".into()),
        rollback_ref: Some("rb-1".into()),
        evidence_refs: vec!["ev-1".into()],
        reason_codes: vec!["merge_verified".into()],
    };
    record_supervisor_receipt(&mut store, "g1", &receipt, &["github".into()]).unwrap();

    // Ledger stays consistent: goal_started + proposal (+ idempotent dup) + receipt.
    let report = store.verify("g1").unwrap();
    assert!(report.ok);
    assert_eq!(report.total_events, 3, "goal_started + proposal + receipt");

    let projection = build_supervisor_event_projection(&store, "g1").unwrap();
    assert_eq!(projection["proposal_count"], 1);
    assert_eq!(projection["receipt_count"], 1);
    assert_eq!(projection["items"][0]["execution_status"], "executed");
    assert_eq!(projection["items"][0]["target_agent_id"], "agent-b");
    assert_eq!(projection["items"][0]["kind"], "execute");
}

/// ── Supervisor receipt rules fail closed ─────────────────────────────────
#[test]
fn supervisor_receipt_rules_fail_closed() {
    let root = tmp_root("rules");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");

    let decision = SupervisorDecision::execute("d-2", "agent-b", vec!["github".into()], "merge");
    record_supervisor_proposal(&mut store, "g1", "supervisor-1", &decision).unwrap();

    // Executed without authority → fail.
    let no_auth = SupervisorReceipt {
        receipt_id: "r-2".into(),
        decision_id: "d-2".into(),
        adapter_id: "a".into(),
        outcome: SupervisorReceiptOutcome::Executed,
        authority_ref: None,
        rollback_ref: None,
        evidence_refs: vec![],
        reason_codes: vec![],
    };
    assert!(record_supervisor_receipt(&mut store, "g1", &no_auth, &["github".into()]).is_err());
    // Executed without declared host capability → fail.
    assert!(record_supervisor_receipt(&mut store, "g1", &no_auth, &[]).is_err());
    // Executed with everything → ok; second executed receipt rejected.
    let ok = SupervisorReceipt {
        receipt_id: "r-3".into(),
        decision_id: "d-2".into(),
        adapter_id: "a".into(),
        outcome: SupervisorReceiptOutcome::Executed,
        authority_ref: Some("auth".into()),
        rollback_ref: None,
        evidence_refs: vec![],
        reason_codes: vec![],
    };
    assert!(record_supervisor_receipt(&mut store, "g1", &ok, &["github".into()]).is_ok());
    let second = SupervisorReceipt {
        receipt_id: "r-4".into(),
        decision_id: "d-2".into(),
        adapter_id: "a".into(),
        outcome: SupervisorReceiptOutcome::Executed,
        authority_ref: Some("auth".into()),
        rollback_ref: None,
        evidence_refs: vec![],
        reason_codes: vec![],
    };
    assert!(record_supervisor_receipt(&mut store, "g1", &second, &["github".into()]).is_err());
}

/// ── Observe decisions never accept receipts; orphan receipts fail ────────
#[test]
fn observe_decisions_and_orphan_receipts_fail_closed() {
    let root = tmp_root("observe");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");

    let observe = SupervisorDecision::observe("d-3", "agent-b", "observe the target");
    record_supervisor_proposal(&mut store, "g1", "supervisor-1", &observe).unwrap();
    let receipt = SupervisorReceipt {
        receipt_id: "r-5".into(),
        decision_id: "d-3".into(),
        adapter_id: "a".into(),
        outcome: SupervisorReceiptOutcome::Executed,
        authority_ref: Some("auth".into()),
        rollback_ref: None,
        evidence_refs: vec![],
        reason_codes: vec![],
    };
    let err = record_supervisor_receipt(&mut store, "g1", &receipt, &[]).unwrap_err();
    assert!(err.to_string().contains("observe decisions"));

    // Receipt without any proposal → fail closed.
    let orphan = SupervisorReceipt {
        receipt_id: "r-6".into(),
        decision_id: "d-missing".into(),
        adapter_id: "a".into(),
        outcome: SupervisorReceiptOutcome::Rejected,
        authority_ref: None,
        rollback_ref: None,
        evidence_refs: vec![],
        reason_codes: vec![],
    };
    assert!(record_supervisor_receipt(&mut store, "g1", &orphan, &[]).is_err());
}

/// ── Agent lane recommendation attributes runs by claim ───────────────────
#[test]
fn agent_lane_recommendation_attributes_runs_to_claiming_agent() {
    use future_loop::agents::lane::compact_agent_lane_recommendation;
    let mut todo = Todo::advancement("t1", "work");
    todo.claimed_by = Some("agent-a".into());
    let mut goal = Goal::new("g1", "objective", "/tmp");
    goal.todos = vec![todo];
    goal.history = vec![future_loop::state::RunRecord {
        turn: 1,
        todo_id: "t1".into(),
        run_id: "run-1".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec![],
        evidence: "merged the change".into(),
        recorded_at: 100,
        spend_source: Some("run".into()),
        validation: None,
        failure_kind: None,
    }];
    let rec = compact_agent_lane_recommendation(&goal, "agent-a").unwrap();
    assert_eq!(rec.agent_id, "agent-a");
    assert_eq!(rec.progress_scope, "agent_lane");
    assert_eq!(rec.classification, "completed");
    assert_eq!(rec.generated_at, 100);
    assert!(rec
        .recommended_action
        .as_deref()
        .unwrap()
        .contains("merged"));
    // Agent B has no lane run (todo not claimed by B).
    assert!(compact_agent_lane_recommendation(&goal, "agent-b").is_none());
}

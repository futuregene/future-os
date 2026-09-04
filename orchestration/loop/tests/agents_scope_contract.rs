//! G-16 multi-worker contract tests: identity-scoped frontiers (two agents
//! under one goal never cross) and the agent lane recommendation.

use future_loop::agents::scope::identity_scoped_frontier;
use future_loop::state::{Goal, Todo};

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
        truncation: None,
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

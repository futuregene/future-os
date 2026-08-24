//! P0 contract tests: execution profile / outcome floor (rejects
//! surface-only progress loops) and replan ACK (must carry a frontier delta
//! to clear a replan obligation). Deterministic.

use std::time::SystemTime;

use future_loop::contract::TurnMode;
use future_loop::decision::decide;
use future_loop::executor::writeback;
use future_loop::state::{delta_kind_changes_frontier, ExecutionProfile, Goal, RunRecord, Todo};

fn run_record(turn: u32, todo_id: &str, state: &str, tools: usize, evidence: &str) -> RunRecord {
    RunRecord {
        turn,
        todo_id: todo_id.to_string(),
        run_id: format!("run-{turn}"),
        terminal_state: state.to_string(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: (0..tools).map(|i| format!("tool{i}")).collect(),
        evidence: evidence.to_string(),
        recorded_at: 0,
        spend_source: None,
        validation: None,
        failure_kind: None,
    }
}

// ── Contract: outcome floor rejects surface-only progress loops ────────────
#[test]
fn outcome_floor_stops_surface_only_loops() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.execution_profile = ExecutionProfile {
        outcome_floor_streak_threshold: 3,
        ..Default::default()
    };
    goal.add(Todo::advancement("t1", "Do the thing"));

    // Two surface-only turns (no tools, no evidence): still delivering.
    let surface = run_record(1, "t1", "completed", 0, "");
    writeback(&mut goal, &surface, None, Some((false, vec!["t2".into()])));
    goal.todo_mut("t1").unwrap().status = future_loop::state::TodoStatus::Open;
    writeback(&mut goal, &surface, None, Some((false, vec!["t2".into()])));
    goal.todo_mut("t1").unwrap().status = future_loop::state::TodoStatus::Open;
    assert_eq!(goal.outcome_streak, 2);

    // Third surface-only turn crosses the floor → replan, not delivery.
    writeback(&mut goal, &surface, None, Some((false, vec!["t2".into()])));
    goal.todo_mut("t1").unwrap().status = future_loop::state::TodoStatus::Open;
    let p = decide(&goal, SystemTime::now());
    assert_eq!(
        p.interaction_contract.mode,
        TurnMode::Replan,
        "outcome floor must reject the surface-only progress loop"
    );
    assert!(p.reason.contains("outcome floor"));
}

#[test]
fn material_turn_resets_the_streak() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.execution_profile = ExecutionProfile {
        outcome_floor_streak_threshold: 3,
        ..Default::default()
    };
    goal.add(Todo::advancement("t1", "Work"));

    let surface = run_record(1, "t1", "completed", 0, "");
    writeback(&mut goal, &surface, None, Some((true, vec![])));
    goal.todo_mut("t1").unwrap().status = future_loop::state::TodoStatus::Open;
    assert_eq!(goal.outcome_streak, 1);

    // Material turn (tools + evidence) resets the streak.
    let material = run_record(2, "t1", "completed", 2, "wrote hello.txt, verify passed");
    writeback(&mut goal, &material, None, Some((true, vec![])));
    goal.todo_mut("t1").unwrap().status = future_loop::state::TodoStatus::Open;
    assert_eq!(goal.outcome_streak, 0);
}

#[test]
fn outcome_floor_disabled_by_default() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("t1", "Work"));
    let surface = run_record(1, "t1", "completed", 0, "");
    for _ in 0..10 {
        writeback(&mut goal, &surface, None, Some((true, vec![])));
        goal.todo_mut("t1").unwrap().status = future_loop::state::TodoStatus::Open;
    }
    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
}

// ── Contract: replan ACK requires a frontier-changing delta ────────────────
#[test]
fn frontier_delta_kinds_are_recognized() {
    assert!(delta_kind_changes_frontier("vision_patch"));
    assert!(delta_kind_changes_frontier("no_followup"));
    assert!(delta_kind_changes_frontier("successor_or_supersede"));
    assert!(delta_kind_changes_frontier("runnable_todo_set"));
    assert!(!delta_kind_changes_frontier("surface_only_comment"));
    assert!(!delta_kind_changes_frontier(""));
}

#[test]
fn ack_without_delta_does_not_clear_obligation() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    // Succession obligation: silently completed todo.
    goal.add(Todo::advancement("t1", "Work"));
    goal.todo_mut("t1").unwrap().status = future_loop::state::TodoStatus::Done;

    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::Replan);

    // A non-frontier ACK must not change the decision.
    goal.replan_ack = Some(future_loop::state::ReplanAck {
        recorded: true,
        delta_kinds: vec!["surface_only_comment".into()],
        at: 0,
    });
    let p2 = decide(&goal, SystemTime::now());
    assert_eq!(p2.interaction_contract.mode, TurnMode::Replan);
    assert!(!p2.replan_ack.frontier_delta_present);
}

#[test]
fn replan_ack_with_frontier_delta_is_reported() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.replan_ack = Some(future_loop::state::ReplanAck {
        recorded: true,
        delta_kinds: vec!["vision_patch".into()],
        at: 0,
    });
    let p = decide(&goal, SystemTime::now());
    assert!(p.replan_ack.recorded);
    assert!(p.replan_ack.frontier_delta_present);
    assert_eq!(p.replan_ack.delta_kinds, vec!["vision_patch"]);
}

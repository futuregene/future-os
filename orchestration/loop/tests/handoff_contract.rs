//! G-17 handoff contract tests: the project handoff document derived from
//! the durable projections, and the delivery contract derived from
//! run-history delivery signals.

use future_loop::handoff::delivery_contract::handoff_delivery_contract;
use future_loop::handoff::project_handoff::{
    build_project_handoff, render_project_handoff_markdown, write_project_handoff,
};
use future_loop::state::{Goal, RunRecord, Todo};

fn goal_with(runs: Vec<RunRecord>) -> Goal {
    let mut goal = Goal::new("g1", "objective", "/tmp");
    goal.todos = vec![
        Todo::advancement("t1", "open work"),
        Todo::user_gate("g1", "approve?", &["t1"]),
    ];
    goal.history = runs;
    goal
}

fn run(evidence: &str, recorded_at: u64) -> RunRecord {
    RunRecord {
        turn: 1,
        todo_id: "t1".into(),
        run_id: format!("run-{recorded_at}"),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec![],
        evidence: evidence.into(),
        recorded_at,
        spend_source: Some("run".into()),
        validation: None,
    }
}

#[test]
fn handoff_document_reflects_frontier_and_active_state() {
    let goal = goal_with(vec![]);
    let handoff = build_project_handoff(&goal, None);
    assert_eq!(handoff.goal_id, "g1");
    assert_eq!(handoff.open_advancement_count, 1);
    assert_eq!(handoff.open_gate_count, 1);
    assert!(handoff
        .active_state_markdown
        .contains("# Active Goal State"));
    let md = render_project_handoff_markdown(&handoff);
    assert!(md.contains("# Project Handoff"));
    assert!(md.contains("open advancement todos: `1`"));
    assert!(md.contains("## Active State"));
}

#[test]
fn handoff_carries_delivery_contract_when_degraded() {
    let goal = goal_with(vec![run("small tweak", 1), run("another tweak", 2)]);
    let contract = handoff_delivery_contract(&goal, &goal.history);
    assert!(contract.is_some());
    let handoff = build_project_handoff(&goal, contract.as_ref().map(|c| c.summary.as_str()));
    assert!(handoff.delivery_contract.is_some());
    let md = render_project_handoff_markdown(&handoff);
    assert!(md.contains("## Delivery Contract"));
    assert!(md.contains("expand_after_repeated_small_delivery"));
}

#[test]
fn no_degradation_no_delivery_contract() {
    let goal = goal_with(vec![run("implemented the fix with tests and writeback", 1)]);
    assert!(handoff_delivery_contract(&goal, &goal.history).is_none());
    let handoff = build_project_handoff(&goal, None);
    assert!(handoff.delivery_contract.is_none());
}

#[test]
fn delivery_contract_modes_and_streaks() {
    // Repeated small delivery → expand_after_repeated_small_delivery.
    let goal = goal_with(vec![run("small tweak", 1), run("unit test only", 2)]);
    let contract = handoff_delivery_contract(&goal, &goal.history).unwrap();
    assert_eq!(contract.mode, "expand_after_repeated_small_delivery");
    assert_eq!(contract.post_handoff_small_scale_streak, 2);
    assert_eq!(contract.if_blocked, "report_blocker_without_spend");
    assert!(contract
        .must_include
        .contains(&"coherent_artifact".to_string()));
    assert!(contract
        .must_include
        .contains(&"state_writeback".to_string()));
    // Surface-only loop at multi-surface scale → surface mode.
    let goal = goal_with(vec![
        run("multi-surface docs-only change", 1),
        run("multi-surface surface-only propagation", 2),
    ]);
    let contract = handoff_delivery_contract(&goal, &goal.history).unwrap();
    assert_eq!(contract.mode, "expand_after_surface_progress_loop");
    assert_eq!(contract.post_handoff_outcome_gap_streak, 2);
}

#[test]
fn handoff_writes_to_projection_dir() {
    let goal = goal_with(vec![]);
    let project = std::env::temp_dir().join(format!(
        "future-loop-p3-handoff-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&project).unwrap();
    let handoff = build_project_handoff(&goal, None);
    write_project_handoff(&project.join(".future/loop/goals/g1"), &goal, &handoff).unwrap();
    let path = project.join(".future/loop/goals/g1/HANDOFF.md");
    assert!(path.exists());
    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.contains("# Project Handoff"));
}

#[test]
fn handoff_latest_run_evidence_is_carried() {
    let goal = goal_with(vec![run("merged the change with validation", 42)]);
    let handoff = build_project_handoff(&goal, None);
    assert!(handoff.latest_run.is_some());
    assert_eq!(handoff.run_count, 1);
    let json = handoff.latest_run.unwrap();
    assert_eq!(json["run_id"], "run-42");
}

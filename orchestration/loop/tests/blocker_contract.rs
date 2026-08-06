//! P0 contract tests: blocker task class — external blockers gate dependent
//! todos exactly like user gates (LoopX: blocker + blocked successor).

use std::time::SystemTime;

use future_loop::contract::TurnMode;
use future_loop::decision::decide;
use future_loop::state::{Goal, Todo, TodoStatus};

#[test]
fn open_blocker_blocks_dependent_todo() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::blocker(
        "B1",
        "Large local dependency not yet acquired",
        &["T2"],
    ));
    goal.add(Todo::advancement(
        "T1",
        "Safe fallback work, independent of B1",
    ));
    goal.add(Todo::advancement("T2", "Depends on the large dependency").blocking(&["B1"]));

    // Blocker is an external wait (not a user decision): it gates T2 but the
    // independent T1 still delivers.
    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("T1")
    );
    assert_eq!(goal.runnable_advancement().count(), 1, "T2 must be hidden");
}

#[test]
fn resolving_blocker_unblocks_dependent_todo() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::blocker("B1", "dependency", &["T2"]));
    goal.add(Todo::advancement("T2", "Depends").blocking(&["B1"]));
    goal.todo_mut("B1").unwrap().status = TodoStatus::Done;

    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("T2")
    );
}

#[test]
fn blocker_without_dependents_does_not_freeze_goal() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::blocker("B1", "Nothing depends on this", &[]));
    goal.add(Todo::advancement("T1", "Independent work"));
    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("T1")
    );
}

#[test]
fn blocker_without_fallback_waits_quietly() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::blocker("B1", "Waiting for CI access", &["T2"]));
    goal.add(Todo::advancement("T2", "Needs CI access").blocking(&["B1"]));
    let p = decide(&goal, SystemTime::now());
    assert_eq!(p.interaction_contract.mode, TurnMode::WaitMonitor);
    assert!(p.reason.contains("blocker"));
    assert!(
        p.automation_liveness.keep_active,
        "wait keeps automation alive"
    );
}

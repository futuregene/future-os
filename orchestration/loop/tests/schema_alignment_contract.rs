//! Schema-alignment tests: the high-impact todo-schema gaps vs LoopX —
//! priority-sorted frontier, action_kind, non-blocking user_action, and
//! deferred + resume_when. Deterministic.

use std::time::{Duration, SystemTime};

use future_loop::contract::TurnMode;
use future_loop::decision::decide;
use future_loop::state::{Goal, Priority, Todo};

// ── priority: the frontier sorts P0 before P1 before P2 ────────────────────
#[test]
fn frontier_sorts_by_priority() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("p2", "low").at_priority(Priority::P2));
    goal.add(Todo::advancement("p0", "urgent").at_priority(Priority::P0));
    goal.add(Todo::advancement("p1", "mid").at_priority(Priority::P1));

    let p = decide(&goal, SystemTime::now());
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("p0"),
        "P0 must be selected before P1/P2"
    );
}

#[test]
fn priority_defaults_to_p1() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("a", "default"));
    goal.add(Todo::advancement("b", "p0").at_priority(Priority::P0));
    let p = decide(&goal, SystemTime::now());
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("b")
    );
}

// ── action_kind: declared and survives the ledger ──────────────────────────
#[test]
fn action_kind_is_recorded() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("t1", "Run the bench").with_action_kind("benchmark"));
    let t = goal.todo("t1").unwrap();
    assert_eq!(t.action_kind.as_deref(), Some("benchmark"));
    assert_eq!(t.priority, Priority::P1);
}

// ── user_action: surfaces in the user channel but NEVER freezes the agent ──
#[test]
fn user_action_does_not_freeze_delivery() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::user_action(
        "u1",
        "Review the weekly summary when convenient",
    ));
    goal.add(Todo::advancement("t1", "Agent work continues"));

    let p = decide(&goal, SystemTime::now());
    assert_eq!(
        p.interaction_contract.mode,
        TurnMode::BoundedDelivery,
        "user_action must not block agent delivery (unlike user_gate)"
    );
    assert_eq!(
        p.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("t1")
    );
    assert!(
        p.interaction_contract.user_channel.action_required,
        "user channel still surfaces the action"
    );
    assert!(p
        .interaction_contract
        .user_channel
        .question
        .unwrap()
        .contains("Review"));
}

// ── deferred: not runnable until resume_when passes, then returns ──────────
#[test]
fn deferred_todo_returns_to_frontier_after_resume() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("t1", "Immediate work"));
    goal.add(Todo::deferred(
        "d1",
        "Deferred work",
        Duration::from_millis(50),
    ));

    // Not yet due: d1 hidden, t1 selected.
    let p1 = decide(&goal, SystemTime::now());
    assert_eq!(
        p1.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("t1")
    );

    // After resume_when passes, d1 rejoins the frontier (t1 still open but
    // FIFO keeps t1 first; mark t1 done to observe d1 runnable).
    std::thread::sleep(Duration::from_millis(80));
    goal.todo_mut("t1").unwrap().status = future_loop::state::TodoStatus::Done;
    goal.todo_mut("t1").unwrap().no_follow_up = true;
    let p2 = decide(&goal, SystemTime::now());
    assert_eq!(
        p2.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("d1")
    );
    assert_eq!(goal.runnable_advancement().count(), 1);
}

#[test]
fn deferred_not_due_is_not_runnable() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::deferred("d1", "Later", Duration::from_secs(3600)));
    assert_eq!(goal.runnable_advancement().count(), 0);
    let p = decide(&goal, SystemTime::now());
    assert_ne!(
        p.interaction_contract.mode,
        TurnMode::Terminal,
        "deferred work keeps automation alive"
    );
}

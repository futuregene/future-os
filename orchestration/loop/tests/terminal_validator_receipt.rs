//! Terminal-closure validator-receipt floor: a done validator-todo blocks
//! terminal until it has a passed receipt in history OR an explicit
//! `delivery record --outcome verified`. Covers the pre-receipt-era goals
//! that could otherwise never close.

use future_loop::state::{task_validation_receipt, FailureKind, Goal, RunRecord, ValidationStatus};
use future_loop::store::{Event, Store};

fn store_with_validator_todo(root: &str) -> (Store, String, String) {
    let mut store = Store::open(root).unwrap();
    let goal_id = "g_receipt".to_string();
    let goal = Goal::new(&goal_id, "receipt floor", "/tmp");
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: goal_id.clone(),
            ts: 1,
        })
        .unwrap();
    // The todo enters the ledger the normal way (TodoAdded), already done
    // with a validator gate and no-follow-up — a pre-receipt-era completion.
    let mut todo = future_loop::state::Todo::advancement("t1", "work with a gate");
    todo.validator = Some("test -f out.txt".to_string());
    store
        .append(Event::TodoAdded {
            goal_id: goal_id.clone(),
            todo: todo.clone(),
            ts: 2,
        })
        .unwrap();
    store
        .append(Event::TodoCompleted {
            goal_id: goal_id.clone(),
            todo_id: todo.id.clone(),
            no_follow_up: true,
            successor_ids: vec![],
            evidence: Some("work landed".into()),
            ts: 3,
        })
        .unwrap();
    (store, goal_id, "t1".to_string())
}

fn run_record(todo_id: &str, passed: bool) -> RunRecord {
    RunRecord {
        turn: 1,
        todo_id: todo_id.to_string(),
        run_id: "r1".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec![],
        evidence: String::new(),
        recorded_at: 0,
        spend_source: Some("run".into()),
        validation: Some(task_validation_receipt(
            if passed {
                ValidationStatus::Passed
            } else {
                ValidationStatus::Failed
            },
            "test -f out.txt",
            "test",
            None,
            Some(0),
        )),
        failure_kind: Some(FailureKind::None),
        truncation: None,
    }
}

#[test]
fn unvalidated_delivery_blocks_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("loop-root");
    let (store, goal_id, todo_id) = store_with_validator_todo(root.to_str().unwrap());
    let goal = store.replay(&goal_id).unwrap().unwrap();
    assert!(goal
        .unvalidated_deliveries()
        .iter()
        .any(|t| t.id == todo_id));
    assert!(!goal.is_terminal());
    drop(store);
}

#[test]
fn passed_receipt_satisfies_the_floor() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("loop-root");
    let (store, goal_id, todo_id) = store_with_validator_todo(root.to_str().unwrap());
    store
        .append_run(&goal_id, &run_record(&todo_id, true))
        .unwrap();
    let goal = store.replay(&goal_id).unwrap().unwrap();
    assert!(goal.unvalidated_deliveries().is_empty());
    assert!(goal.is_terminal());
}

#[test]
fn explicit_delivery_verified_satisfies_the_floor() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("loop-root");
    let (store, goal_id, todo_id) = store_with_validator_todo(root.to_str().unwrap());
    // Pre-receipt-era close-out: the orchestrator records its judgement.
    let mut store = store;
    store
        .append(Event::DeliveryOutcomeRecorded {
            goal_id: goal_id.clone(),
            todo_id: todo_id.clone(),
            outcome: "verified".into(),
            note: Some("verified retrospectively".into()),
            delivered_turn: 0,
            seq: 1,
            ts: 2,
        })
        .unwrap();
    let goal = store.replay(&goal_id).unwrap().unwrap();
    assert!(goal.unvalidated_deliveries().is_empty());
    assert!(goal.is_terminal());
    // A `delivered` (pending) outcome does NOT satisfy the floor.
    drop(store);
    let (mut store, goal_id, todo_id) = store_with_validator_todo(root.to_str().unwrap());
    store
        .append(Event::DeliveryOutcomeRecorded {
            goal_id: goal_id.clone(),
            todo_id: todo_id.clone(),
            outcome: "delivered".into(),
            note: None,
            delivered_turn: 0,
            seq: 1,
            ts: 2,
        })
        .unwrap();
    let goal = store.replay(&goal_id).unwrap().unwrap();
    assert!(goal
        .unvalidated_deliveries()
        .iter()
        .any(|t| t.id == todo_id));
}

//! Contract tests for the productized-feature port (merged from the local
//! `main` productized line): `todo update` / `goal cancel` events, and the
//! independent-validator gating (`todo add --verify`).

use future_loop::executor::{turn_succeeded, writeback};
use future_loop::state::{
    now_epoch, Goal, Priority, RunRecord, TaskClass, Todo, TodoStatus, ValidationStatus,
};
use future_loop::store::{Event, Store};
use tempfile::TempDir;

fn tmp_store() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().to_str().unwrap()).unwrap();
    (dir, store)
}

fn todo(id: &str) -> Todo {
    Todo::advancement(id, "task")
}

fn run_record(todo_id: &str, terminal: &str) -> RunRecord {
    RunRecord {
        turn: 1,
        todo_id: todo_id.to_string(),
        run_id: "r1".to_string(),
        terminal_state: terminal.to_string(),
        error: None,
        tokens_in_delta: 1,
        tokens_out_delta: 1,
        cost_delta: 0.0,
        tools: vec!["shell".to_string()],
        evidence: "work".to_string(),
        recorded_at: now_epoch(),
        spend_source: Some("run".to_string()),
        validation: None,
        failure_kind: None,
        truncation: None,
    }
}

#[test]
fn todo_update_event_applies_field_edits() {
    let (_d, mut store) = tmp_store();
    let gid = "g1";
    store.register(&Goal::new(gid, "obj", "/tmp")).unwrap();
    let tid = "t1";
    store
        .append(Event::TodoAdded {
            goal_id: gid.into(),
            todo: todo(tid),
            ts: now_epoch(),
        })
        .unwrap();
    // update: text + priority + note
    store
        .append(Event::TodoUpdated {
            goal_id: gid.into(),
            todo_id: tid.into(),
            text: Some("renamed".into()),
            status: None,
            evidence: None,
            note: Some("note".into()),
            priority: Some("P0".into()),
            resume_when: None,
            blocks: None,
            acceptance: None,
            ts: now_epoch(),
        })
        .unwrap();
    let goal = store.replay(gid).unwrap().unwrap();
    let t = goal.todo(tid).unwrap();
    assert_eq!(t.text, "renamed");
    assert_eq!(t.priority, Priority::P0);
    assert_eq!(t.note.as_deref(), Some("note"));
}

#[test]
fn todo_update_status_done_is_rejected_by_apply() {
    let (_d, mut store) = tmp_store();
    let gid = "g1";
    store.register(&Goal::new(gid, "obj", "/tmp")).unwrap();
    let tid = "t1";
    store
        .append(Event::TodoAdded {
            goal_id: gid.into(),
            todo: todo(tid),
            ts: now_epoch(),
        })
        .unwrap();
    // `--status done` must NOT move the todo to done via update (completion
    // policy is enforced by `todo complete`).
    store
        .append(Event::TodoUpdated {
            goal_id: gid.into(),
            todo_id: tid.into(),
            text: None,
            status: Some("done".into()),
            evidence: None,
            note: None,
            priority: None,
            resume_when: None,
            blocks: None,
            acceptance: None,
            ts: now_epoch(),
        })
        .unwrap();
    let goal = store.replay(gid).unwrap().unwrap();
    assert_eq!(goal.todo(tid).unwrap().status, TodoStatus::Open);
}

#[test]
fn goal_cancel_marks_goal_status_cancelled() {
    let (_d, mut store) = tmp_store();
    let gid = "g1";
    store.register(&Goal::new(gid, "obj", "/tmp")).unwrap();
    store
        .append(Event::GoalCancelled {
            goal_id: gid.into(),
            reason: "user decision".into(),
            ts: now_epoch(),
        })
        .unwrap();
    let goal = store.replay(gid).unwrap().unwrap();
    assert_eq!(goal.status, "cancelled");
    // State retained (todos still replayable).
    assert_eq!(goal.todos.len(), 0);
}

#[test]
fn turn_succeeded_requires_validation_ok_when_attached() {
    let ok = run_record("t1", "completed");
    assert!(turn_succeeded(&ok), "no validator ⇒ not required ⇒ ok");

    let failed = RunRecord {
        validation: Some(future_loop::state::task_validation_receipt(
            ValidationStatus::Failed,
            "cmd",
            "validator exited 1",
            Some(future_loop::state::RecoveryKind::RepairRequired),
            Some(1),
        )),
        ..run_record("t1", "completed")
    };
    assert!(!turn_succeeded(&failed), "failed validator ⇒ not succeeded");

    let passed = RunRecord {
        validation: Some(future_loop::state::task_validation_receipt(
            ValidationStatus::Passed,
            "cmd",
            "validator passed (exit 0)",
            None,
            Some(0),
        )),
        ..run_record("t1", "completed")
    };
    assert!(turn_succeeded(&passed), "passed validator ⇒ succeeded");
}

#[test]
fn writeback_keeps_todo_open_on_failed_validation() {
    let mut goal = Goal::new("g1", "obj", "/tmp");
    let mut t = todo("t1");
    t.validator = Some("false".to_string());
    t.max_validation_attempts = 3;
    goal.add(t.clone());
    let record = RunRecord {
        validation: Some(future_loop::state::task_validation_receipt(
            ValidationStatus::Failed,
            "false",
            "validator exited 1",
            Some(future_loop::state::RecoveryKind::RepairRequired),
            Some(1),
        )),
        ..run_record("t1", "completed")
    };
    writeback(&mut goal, &record, None, Some((true, vec![])));
    let after = goal.todo("t1").unwrap();
    assert_eq!(
        after.status,
        TodoStatus::Open,
        "failed validation ⇒ stays open"
    );
    assert_eq!(after.failed_attempts, 1, "one repair attempt counted");
    assert_eq!(after.class, TaskClass::Advancement);
}

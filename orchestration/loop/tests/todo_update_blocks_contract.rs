//! Contract tests for `todo update --blocks`: replace / clear / leave the
//! blocking set, plus legacy-event back-compat (pre-`blocks` TodoUpdated
//! events must keep deserializing and leave the blocking set untouched).
//!
//! This is the repair path for a mis-wired dependency chain: previously the
//! only fix was `goal delete --force` + re-create; now `todo update --blocks`
//! rewires in place (see orchestration/loop/src/main.rs `todo_update`).

use future_loop::state::{Goal, Todo};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("future-loop-blocks-test-{tag}-{}", uuid_like()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    format!(
        "{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

/// Register a goal with three todos: t1 (optionally pre-blocked by t2), t2, t3.
fn setup(tag: &str, pre_blocked: bool) -> (String, Store) {
    let root = tmp_root(tag);
    let mut store = Store::open(&root).unwrap();
    let goal = Goal::new("g1", "objective", "/tmp");
    store.register(&goal).unwrap();
    let ts = goal.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts,
        })
        .unwrap();
    let t1 = if pre_blocked {
        Todo::advancement("t1", "Dep work").blocking(&["t2"])
    } else {
        Todo::advancement("t1", "Dep work")
    };
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: t1,
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t2", "Second"),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t3", "Third"),
            ts,
        })
        .unwrap();
    ("g1".into(), store)
}

fn blocks_of(store: &Store, goal_id: &str, todo_id: &str) -> Option<String> {
    store
        .replay(goal_id)
        .unwrap()
        .unwrap()
        .todo(todo_id)
        .unwrap()
        .blocked_by_gate
        .clone()
}

/// `todo update --blocks t3` replaces the blocking set (set on a plain todo).
#[test]
fn update_blocks_sets_blocking_set() {
    let (gid, mut store) = setup("set", false);
    let ts = future_loop::state::now_epoch();
    store
        .append(Event::TodoUpdated {
            goal_id: gid.clone(),
            todo_id: "t1".into(),
            text: None,
            status: None,
            evidence: None,
            note: None,
            priority: None,
            resume_when: None,
            blocks: Some(vec!["t3".into()]),
            acceptance: None,
            owner: None,
            ts,
        })
        .unwrap();
    assert_eq!(blocks_of(&store, &gid, "t1"), Some("t3".into()));
}

/// `todo update --blocks t3` REPLACES (not appends to) an existing set.
#[test]
fn update_blocks_replaces_existing_set() {
    let (gid, mut store) = setup("replace", true); // t1 pre-blocked by t2
    assert_eq!(blocks_of(&store, &gid, "t1"), Some("t2".into()));
    let ts = future_loop::state::now_epoch();
    store
        .append(Event::TodoUpdated {
            goal_id: gid.clone(),
            todo_id: "t1".into(),
            text: None,
            status: None,
            evidence: None,
            note: None,
            priority: None,
            resume_when: None,
            blocks: Some(vec!["t3".into(), "t2".into()]),
            acceptance: None,
            owner: None,
            ts,
        })
        .unwrap();
    assert_eq!(blocks_of(&store, &gid, "t1"), Some("t3,t2".into()));
}

/// `todo update --blocks ""` clears the blocking set.
#[test]
fn update_blocks_empty_clears() {
    let (gid, mut store) = setup("clear", true); // t1 pre-blocked by t2
    assert_eq!(blocks_of(&store, &gid, "t1"), Some("t2".into()));
    let ts = future_loop::state::now_epoch();
    store
        .append(Event::TodoUpdated {
            goal_id: gid.clone(),
            todo_id: "t1".into(),
            text: None,
            status: None,
            evidence: None,
            note: None,
            priority: None,
            resume_when: None,
            blocks: Some(vec![]),
            acceptance: None,
            owner: None,
            ts,
        })
        .unwrap();
    assert_eq!(blocks_of(&store, &gid, "t1"), None);
}

/// A TodoUpdated without the `blocks` field (legacy event, or an update that
/// only touches text/priority) leaves the blocking set untouched.
#[test]
fn update_without_blocks_leaves_set_untouched() {
    let (gid, mut store) = setup("untouched", true); // t1 pre-blocked by t2
    let ts = future_loop::state::now_epoch();
    store
        .append(Event::TodoUpdated {
            goal_id: gid.clone(),
            todo_id: "t1".into(),
            text: Some("renamed".into()),
            status: None,
            evidence: None,
            note: None,
            priority: None,
            resume_when: None,
            blocks: None,
            acceptance: None,
            owner: None,
            ts,
        })
        .unwrap();
    let g = store.replay(&gid).unwrap().unwrap();
    assert_eq!(g.todo("t1").unwrap().text, "renamed");
    assert_eq!(g.todo("t1").unwrap().blocked_by_gate, Some("t2".into()));
}

/// Legacy TodoUpdated events written to the ledger BEFORE the `blocks` field
/// existed (JSON without `blocks`) still deserialize — `blocks` defaults to
/// None and the other fields apply.
#[test]
fn legacy_event_without_blocks_field_deserializes() {
    let json = serde_json::json!({
        "kind": "todo_updated",
        "goal_id": "g1",
        "todo_id": "t1",
        "text": "legacy rename",
        "status": null,
        "evidence": null,
        "note": null,
        "priority": null,
        "resume_when": null,
        "ts": 1786000000,
    });
    let ev: Event = serde_json::from_value(json).unwrap();
    match ev {
        Event::TodoUpdated { blocks, text, .. } => {
            assert_eq!(blocks, None, "missing blocks field must default to None");
            assert_eq!(text.as_deref(), Some("legacy rename"));
        }
        other => panic!("expected TodoUpdated, got {other:?}"),
    }
}

/// End-to-end: replay a ledger containing a blocks-replacing update rebuilds
/// the same projected state (events remain the source of truth).
#[test]
fn replay_rebuilds_updated_blocks() {
    let root = tmp_root("replay");
    let mut store = Store::open(&root).unwrap();
    let goal = Goal::new("g1", "objective", "/tmp");
    store.register(&goal).unwrap();
    let ts = goal.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "Dep work").blocking(&["t2"]),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t2", "Second"),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoUpdated {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            text: None,
            status: None,
            evidence: None,
            note: None,
            priority: None,
            resume_when: None,
            blocks: Some(vec!["t2".into(), "t3".into()]),
            acceptance: None,
            owner: None,
            ts,
        })
        .unwrap();
    drop(store);
    // Fresh store, same root → rebuild purely from the event ledger.
    let store2 = Store::open(&root).unwrap();
    let rebuilt = store2.replay("g1").unwrap().expect("goal exists");
    assert_eq!(
        rebuilt.todo("t1").unwrap().blocked_by_gate,
        Some("t2,t3".into())
    );
    assert_eq!(rebuilt.todos.len(), 2);
}

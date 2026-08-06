//! G-13 lease + replan-obligation contract tests: the task-lease state
//! machine (claim / renew / release / expiry / steal) through the event
//! store with exact replay, and the autonomous replan obligation
//! bookkeeping (record + query, ack-cleared).

use future_loop::state::{Goal, Todo};
use future_loop::store::{Event, Store};
use future_loop::work_items::replan_obligation;
use future_loop::work_items::task_lease::{self, LeaseOp, LeaseStatus};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "loopx-p2-lease-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn open_goal(store: &mut Store, goal_id: &str) -> u64 {
    let goal = Goal::new(goal_id, "objective", "/tmp");
    store.register(&goal).unwrap();
    let ts = goal.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: goal_id.into(),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: goal_id.into(),
            todo: Todo::advancement("t1", "shared work"),
            ts,
        })
        .unwrap();
    ts
}

/// ── Full lease lifecycle through the store, replayed exactly ─────────────
#[test]
fn lease_lifecycle_claim_renew_steal_release_replays() {
    let root = tmp_root("lifecycle");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");
    let now = 1_000_000u64;

    // alice claims (acquire).
    let mut goal = store.replay("g1").unwrap().unwrap();
    let op = task_lease::claim(goal.todo_mut("t1").unwrap(), "alice", 100, now).unwrap();
    assert_eq!(
        op,
        LeaseOp::Acquired {
            idempotent: false,
            steal: false
        }
    );
    store
        .append(Event::TodoClaimed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "alice".into(),
            lease_expires_at: now + 100,
            ts: now,
        })
        .unwrap();

    // bob cannot claim a live lease.
    let mut goal = store.replay("g1").unwrap().unwrap();
    assert!(task_lease::claim(goal.todo_mut("t1").unwrap(), "bob", 100, now + 10).is_err());

    // alice renews.
    let mut goal = store.replay("g1").unwrap().unwrap();
    let op = task_lease::renew(goal.todo_mut("t1").unwrap(), "alice", 100, now + 10).unwrap();
    assert_eq!(op, LeaseOp::Renewed);
    store
        .append(Event::TodoRenewed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "alice".into(),
            lease_expires_at: now + 110,
            ts: now + 10,
        })
        .unwrap();

    // After expiry, bob steals: TodoExpired + TodoClaimed.
    let mut goal = store.replay("g1").unwrap().unwrap();
    let op = task_lease::claim(goal.todo_mut("t1").unwrap(), "bob", 100, now + 500).unwrap();
    assert_eq!(
        op,
        LeaseOp::Acquired {
            idempotent: false,
            steal: true
        }
    );
    store
        .append(Event::TodoExpired {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            ts: now + 500,
        })
        .unwrap();
    store
        .append(Event::TodoClaimed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "bob".into(),
            lease_expires_at: now + 600,
            ts: now + 500,
        })
        .unwrap();

    // bob releases (while still active).
    let mut goal = store.replay("g1").unwrap().unwrap();
    let op = task_lease::release(goal.todo_mut("t1").unwrap(), "bob", now + 590).unwrap();
    assert_eq!(op, LeaseOp::Released { missing: false });
    store
        .append(Event::TodoReleased {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "bob".into(),
            ts: now + 590,
        })
        .unwrap();

    // Fresh store: replay reconstructs the lease history exactly.
    let store2 = Store::open(&root).unwrap();
    let goal = store2.replay("g1").unwrap().unwrap();
    let todo = goal.todo("t1").unwrap();
    assert_eq!(todo.claimed_by, None, "released lease is free");
    assert_eq!(todo.lease_expires_at, None);
    assert_eq!(task_lease::lease_status(todo, now + 700), LeaseStatus::Free);
    assert!(!task_lease::lease_is_active(todo, now + 700));

    // Verify the ledger is clean (5 lease events + started + added).
    let report = store2.verify("g1").unwrap();
    assert!(report.ok);
    assert_eq!(report.total_events, 7);
}

/// ── Steal-after-expiry replay: TodoExpired then TodoClaimed wins ─────────
#[test]
fn steal_after_expiry_replays_to_new_owner() {
    let root = tmp_root("steal");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");
    let now = 1_000_000u64;
    store
        .append(Event::TodoClaimed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "alice".into(),
            lease_expires_at: now + 100,
            ts: now,
        })
        .unwrap();
    // alice's lease lapses; bob steals.
    store
        .append(Event::TodoExpired {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            ts: now + 500,
        })
        .unwrap();
    store
        .append(Event::TodoClaimed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "bob".into(),
            lease_expires_at: now + 600,
            ts: now + 500,
        })
        .unwrap();

    let store2 = Store::open(&root).unwrap();
    let goal = store2.replay("g1").unwrap().unwrap();
    let todo = goal.todo("t1").unwrap();
    assert_eq!(todo.claimed_by.as_deref(), Some("bob"));
    assert_eq!(todo.lease_expires_at, Some(now + 600));
    assert_eq!(
        task_lease::lease_status(todo, now + 550),
        LeaseStatus::Active {
            owner: "bob".to_string(),
            expires_at: now + 600,
        }
    );
}

/// ── Replan obligations: raised by monitor stall, cleared by ack ───────────
#[test]
fn replan_obligation_bookkeeping_through_store() {
    let root = tmp_root("obligations");
    let mut store = Store::open(&root).unwrap();
    let goal = Goal::new("g2", "objective", "/tmp");
    store.register(&goal).unwrap();
    let ts = goal.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: "g2".into(),
            ts,
        })
        .unwrap();
    let mut monitor = Todo::monitor("m1", "watch target", std::time::Duration::from_secs(60));
    monitor.consecutive_no_change = 3;
    monitor.updated_at = 1_000;
    store
        .append(Event::TodoAdded {
            goal_id: "g2".into(),
            todo: monitor,
            ts,
        })
        .unwrap();

    let goal = store.replay("g2").unwrap().unwrap();
    let obligations = replan_obligation::unfulfilled_obligations(&goal);
    assert_eq!(obligations.len(), 1);
    assert_eq!(obligations[0].kind, "monitor_no_change_streak");
    assert_eq!(obligations[0].todo_id.as_deref(), Some("m1"));
    assert!(!obligations[0].cleared);

    // Ack AFTER the last poll with a frontier delta clears the obligation.
    store
        .append(Event::ReplanAcked {
            goal_id: "g2".into(),
            delta_kinds: vec!["vision_patch".to_string()],
            ts: 1_100,
        })
        .unwrap();
    let goal = store.replay("g2").unwrap().unwrap();
    let obligations = replan_obligation::detect_obligations(&goal);
    let monitor_ob = obligations
        .iter()
        .find(|o| o.kind == "monitor_no_change_streak")
        .unwrap();
    assert!(monitor_ob.cleared);
    assert_eq!(monitor_ob.cleared_reason.as_deref(), Some("replan_ack"));
    assert!(replan_obligation::unfulfilled_obligations(&goal).is_empty());

    // An ack WITHOUT a frontier delta does not clear.
    let mut goal = Goal::new("g3", "objective", "/tmp");
    let mut monitor = Todo::monitor("m1", "watch", std::time::Duration::from_secs(60));
    monitor.consecutive_no_change = 3;
    monitor.updated_at = 1_000;
    goal.add(monitor);
    goal.replan_ack = Some(future_loop::state::ReplanAck {
        recorded: true,
        delta_kinds: vec!["note_only".to_string()],
        at: 1_100,
    });
    assert!(replan_obligation::has_unfulfilled_obligation(&goal));
}

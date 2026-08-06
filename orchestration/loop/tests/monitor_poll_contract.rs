//! P1 contract tests — G-12 monitor metadata (target/policy/cadence) and
//! G-8 monitor_poll events: classification, durable event writeback, exact
//! replay, and no-spend semantics.

use std::time::{Duration, SystemTime};

use future_loop::decision::monitor_poll_classification;
use future_loop::state::{Goal, Todo, TodoStatus};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("future-loop-monitor-test-{tag}-{}", uuid_like()));
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

fn register_goal(store: &mut Store, goal_id: &str) -> u64 {
    let goal = Goal::new(goal_id, "objective", "/tmp");
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: goal_id.into(),
            ts: goal.created_at,
        })
        .unwrap();
    goal.created_at
}

// ── G-12: monitor metadata survives TodoAdded → replay ─────────────────────
#[test]
fn monitor_metadata_roundtrips_through_events() {
    let root = tmp_root("meta");
    let mut store = Store::open(&root).unwrap();
    let ts = register_goal(&mut store, "g1");
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::monitor_with(
                "M1",
                "Watch deployment",
                Some("https://example.com/deploy/status"),
                Some("material_transition_only"),
                Some("1h"),
                Duration::from_secs(3600),
            ),
            ts,
        })
        .unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    let m = goal.todo("M1").unwrap();
    assert_eq!(
        m.monitor_target.as_deref(),
        Some("https://example.com/deploy/status")
    );
    assert_eq!(
        m.monitor_policy.as_deref(),
        Some("material_transition_only")
    );
    assert_eq!(m.monitor_cadence.as_deref(), Some("1h"));
    assert_eq!(m.class, future_loop::state::TaskClass::Monitor);
}

// ── G-12: cadence drives the first due time (interval parsing) ─────────────
#[test]
fn cadence_derives_first_due_time() {
    let t0 = SystemTime::now();
    let m = Todo::monitor_with(
        "M1",
        "Watch",
        None,
        None,
        Some("2h"),
        Duration::from_secs(60),
    );
    let due = m.resume_when.expect("due time from cadence");
    let gap = due.duration_since(t0).unwrap().as_secs();
    assert!(
        (7100..=7300).contains(&gap),
        "2h cadence ⇒ ~7200s to first poll, got {gap}s"
    );
}

// ── G-8: MonitorPolled event lands on the decision path and replays exactly ─
#[test]
fn monitor_polled_event_replays_no_change_exactly() {
    let root = tmp_root("poll-event");
    let mut store = Store::open(&root).unwrap();
    let ts = register_goal(&mut store, "g1");
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::monitor("M1", "Watch", Duration::from_secs(3600)),
            ts,
        })
        .unwrap();

    // Two no-change polls: counter 1 then 2.
    let now = future_loop::state::now_epoch();
    store
        .append(Event::MonitorPolled {
            goal_id: "g1".into(),
            todo_id: "M1".into(),
            result: "no_change".into(),
            no_change_count: 1,
            ts: now,
        })
        .unwrap();
    store
        .append(Event::MonitorPolled {
            goal_id: "g1".into(),
            todo_id: "M1".into(),
            result: "no_change".into(),
            no_change_count: 2,
            ts: now + 1,
        })
        .unwrap();

    let goal = store.replay("g1").unwrap().unwrap();
    let m = goal.todo("M1").unwrap();
    assert_eq!(
        m.consecutive_no_change, 2,
        "replay restores the exact counter"
    );
    assert_eq!(
        m.status,
        TodoStatus::Open,
        "no-change poll keeps the monitor open"
    );
    assert!(
        m.resume_when.is_some_and(|d| d > SystemTime::now()),
        "replay sets the next due from the poll ts + backoff"
    );
}

#[test]
fn monitor_polled_event_closes_monitor_on_change() {
    let root = tmp_root("poll-change");
    let mut store = Store::open(&root).unwrap();
    let ts = register_goal(&mut store, "g1");
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::monitor("M1", "Watch", Duration::from_secs(3600)),
            ts,
        })
        .unwrap();
    store
        .append(Event::MonitorPolled {
            goal_id: "g1".into(),
            todo_id: "M1".into(),
            result: "changed".into(),
            no_change_count: 0,
            ts: 1_784_000_100,
        })
        .unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    let m = goal.todo("M1").unwrap();
    assert_eq!(
        m.status,
        TodoStatus::Done,
        "material transition closes the monitor"
    );
    assert_eq!(m.consecutive_no_change, 0);
}

// ── G-8: classification is exact (changed resets, no_change advances) ──────
#[test]
fn poll_classification_counts_are_exact() {
    let (result, count) = monitor_poll_classification(false, 0);
    assert_eq!(result, "no_change");
    assert_eq!(count, 1);
    let (_, count) = monitor_poll_classification(false, 2);
    assert_eq!(count, 3);
    let (result, count) = monitor_poll_classification(true, 3);
    assert_eq!(result, "changed");
    assert_eq!(count, 0, "changed resets the counter");
}

// ── G-8: no-change monitor polls never spend ───────────────────────────────
#[test]
fn no_change_polls_never_enter_the_spend_ledger() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::monitor("M1", "Watch A", Duration::from_millis(10)));
    std::thread::sleep(Duration::from_millis(30));
    let record = future_loop::state::RunRecord {
        turn: 1,
        todo_id: "M1".into(),
        run_id: "run-1".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec![],
        evidence: String::new(),
        recorded_at: future_loop::state::now_epoch(),
        spend_source: Some("heartbeat".into()),
        validation: None,
    };
    future_loop::executor::writeback(&mut goal, &record, Some(false), None);
    assert_eq!(goal.history.len(), 0, "no-change polls are quota-neutral");
    assert_eq!(goal.todo("M1").unwrap().consecutive_no_change, 1);
    // A changed poll closes the monitor and records (spendable).
    let record2 = future_loop::state::RunRecord {
        spend_source: Some("heartbeat".into()),
        validation: None,
        ..record
    };
    future_loop::executor::writeback(&mut goal, &record2, Some(true), None);
    assert_eq!(goal.history.len(), 1);
    assert_eq!(goal.todo("M1").unwrap().status, TodoStatus::Done);
}

// ── G-12: monitor metadata renders into the compat projection anchor ───────
#[test]
fn compat_projection_carries_monitor_metadata() {
    let dir = std::env::temp_dir().join(format!("future-loop-monitor-compat-{}", uuid_like()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut goal = Goal::new("g1", "objective", dir.to_str().unwrap());
    goal.add(Todo::monitor_with(
        "M1",
        "Watch",
        Some("http://target"),
        Some("read_only_observation_then_no_spend_if_unchanged"),
        Some("daily"),
        Duration::from_secs(86400),
    ));
    future_loop::compat::write_active_state(&dir.join(".future/loop/goals/g1"), &goal).unwrap();
    let md =
        std::fs::read_to_string(dir.join(".future/loop/goals/g1/ACTIVE_GOAL_STATE.md")).unwrap();
    assert!(
        md.contains("monitor_target=http://target"),
        "anchor target: {md}"
    );
    assert!(md.contains("monitor_policy=read_only_observation_then_no_spend_if_unchanged"));
    assert!(md.contains("cadence=daily"));
    assert!(md.contains("task_class=continuous_monitor"));
}

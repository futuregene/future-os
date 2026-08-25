//! State-substrate contract tests: registry + append-only event ledger,
//! replay-to-rebuild, projection-gap detection, and validated terminal
//! derivation. These mirror LoopX's `tests/control_plane/` substrate tests.

use std::time::{Duration, SystemTime};

use future_loop::decision::{decide, MONITOR_NO_CHANGE_REPLAN_THRESHOLD};
use future_loop::state::{Goal, Todo};
use future_loop::store::{projection_gap, Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("future-loop-store-test-{tag}-{}", uuid_like()));
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

/// ── Event ledger is the source of truth; active state is a read model ─────
#[test]
fn replay_rebuilds_active_state_from_events() {
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
            todo: Todo::advancement("t1", "Do work"),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t2", "More work"),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoCompleted {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            no_follow_up: true,
            successor_ids: vec![],
            evidence: None,
            ts,
        })
        .unwrap();

    // Rebuild from scratch by replay (fresh store, same root).
    let store2 = Store::open(&root).unwrap();
    let rebuilt = store2.replay("g1").unwrap().expect("goal exists");
    assert_eq!(rebuilt.todos.len(), 2);
    assert_eq!(
        rebuilt.todo("t1").unwrap().status,
        future_loop::state::TodoStatus::Done
    );
    assert!(rebuilt.todo("t1").unwrap().no_follow_up);
    assert_eq!(
        rebuilt.todo("t2").unwrap().status,
        future_loop::state::TodoStatus::Open
    );
}

/// ── Unregistered goals reject events (fail-closed) ────────────────────────
#[test]
fn append_requires_registered_goal() {
    let root = tmp_root("unregistered");
    let mut store = Store::open(&root).unwrap();
    let err = store.append(Event::GoalStarted {
        goal_id: "ghost".into(),
        ts: 0,
    });
    assert!(
        err.is_err(),
        "events for unregistered goals must fail closed"
    );
}

/// ── Projection gap: executable Next Action with no open todo ──────────────
#[test]
fn projection_gap_detects_next_action_without_todo() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.next_action = Some("Do the next thing".to_string());
    let gap = projection_gap(&goal);
    assert!(
        gap.is_some(),
        "executable next action with no open todo is a gap"
    );

    goal.add(Todo::advancement("t1", "Do the next thing"));
    let gap2 = projection_gap(&goal);
    assert!(gap2.is_none(), "matching open todo closes the gap");
}

#[test]
fn projection_gap_ignores_completion_statement() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.next_action = Some("all todos complete; no further action".to_string());
    assert!(projection_gap(&goal).is_none());
}

/// ── Validated terminal derivation ─────────────────────────────────────────
#[test]
fn terminal_derives_only_from_complete_closure_sources() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.add(Todo::advancement("t1", "Work"));
    // Open todo → not terminal.
    assert!(goal.terminal_closure().is_none());

    // Completed with no-follow-up → terminal.
    goal.todo_mut("t1").unwrap().complete(true, vec![]);
    assert!(goal.terminal_closure().is_some());

    // Open monitor → not terminal again.
    goal.add(Todo::monitor("m1", "watch", Duration::from_secs(60)));
    assert!(goal.terminal_closure().is_none());
}

/// ── Run history persists across replays ───────────────────────────────────
#[test]
fn run_history_persists() {
    let root = tmp_root("runs");
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

    let record = crate_helper::run_record(1, "t1", "completed");
    store.append_run("g2", &record).unwrap();
    store
        .append(Event::RunRecorded {
            goal_id: "g2".into(),
            record: record.clone(),
            ts,
        })
        .unwrap();

    let store2 = Store::open(&root).unwrap();
    let rebuilt = store2.replay("g2").unwrap().unwrap();
    assert_eq!(rebuilt.history.len(), 1);
    assert_eq!(rebuilt.history[0].run_id, record.run_id);
}

/// ── End-to-end substrate: gates resolve, monitors stall, closure validates ─
#[test]
fn full_goal_lifecycle_through_events() {
    let root = tmp_root("lifecycle");
    let mut store = Store::open(&root).unwrap();
    let goal = Goal::new("g3", "objective", "/tmp");
    store.register(&goal).unwrap();
    let ts = goal.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: "g3".into(),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g3".into(),
            todo: Todo::advancement("t1", "Work"),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g3".into(),
            todo: Todo::monitor("m1", "watch", Duration::from_millis(10)),
            ts,
        })
        .unwrap();

    // Complete t1 (advancement beats monitor in the decision pipeline), then
    // a due monitor allows one poll.
    store
        .append(Event::TodoCompleted {
            goal_id: "g3".into(),
            todo_id: "t1".into(),
            no_follow_up: true,
            successor_ids: vec![],
            evidence: None,
            ts,
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let g = store.replay("g3").unwrap().unwrap();
    assert_eq!(
        decide(&g, SystemTime::now()).interaction_contract.mode,
        future_loop::contract::TurnMode::MonitorPoll
    );

    // No-change stall → quiet wait + signal (ARCHITECTURE-SIMPLIFICATION: a
    // stalled monitor is an advisory, not a forced replan).
    let mut g = store.replay("g3").unwrap().unwrap();
    let rec = crate_helper::run_record(1, "m1", "completed");
    for _ in 0..MONITOR_NO_CHANGE_REPLAN_THRESHOLD {
        future_loop::executor::writeback(&mut g, &rec, Some(false), None);
    }
    let p = decide(&g, SystemTime::now());
    assert_eq!(
        p.interaction_contract.mode,
        future_loop::contract::TurnMode::WaitMonitor
    );
    assert!(p.reason.contains("stalled"), "{}", p.reason);
}

/// Helper module (integration tests cannot use `#[path]` into the bin).
mod crate_helper {
    pub fn run_record(turn: u32, todo_id: &str, state: &str) -> future_loop::state::RunRecord {
        future_loop::state::RunRecord {
            turn,
            todo_id: todo_id.to_string(),
            run_id: format!("run-{turn}"),
            terminal_state: state.to_string(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: String::new(),
            recorded_at: 0,
            spend_source: None,
            validation: None,
            failure_kind: None,
        }
    }
}

/// ── Replay assigns goal-relative indexes (LoopX: index for ordering) ─────
#[test]
fn replay_assigns_indexes() {
    let root = tmp_root("index");
    let mut store = Store::open(&root).unwrap();
    let g = Goal::new("gix", "objective", "/tmp");
    store.register(&g).unwrap();
    let ts = g.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: "gix".into(),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "gix".into(),
            todo: Todo::advancement("a", "first"),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "gix".into(),
            todo: Todo::advancement("b", "second"),
            ts,
        })
        .unwrap();
    let rebuilt = Store::open(&root).unwrap().replay("gix").unwrap().unwrap();
    assert_eq!(rebuilt.todo("a").unwrap().index, 1);
    assert_eq!(rebuilt.todo("b").unwrap().index, 2);
}

/// ── Bidirectional messaging: supervisor register + worker steer ────────────
#[test]
fn replay_folds_supervisor_registration_and_worker_steer() {
    let root = tmp_root("bidir");
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

    // Supervisor registers its session id (the up-channel target).
    store
        .append(Event::SupervisorRegistered {
            goal_id: "g1".into(),
            session_id: "sup-sess-1".into(),
            ts,
        })
        .unwrap();
    let rebuilt = store.replay("g1").unwrap().unwrap();
    assert_eq!(rebuilt.supervisor_session_id.as_deref(), Some("sup-sess-1"));

    // A steering instruction (broadcast) folds into pending_steer, latest wins.
    store
        .append(Event::WorkerSteered {
            goal_id: "g1".into(),
            agent_id: None,
            instruction: "stop and re-check".into(),
            ts: ts + 1,
        })
        .unwrap();
    store
        .append(Event::WorkerSteered {
            goal_id: "g1".into(),
            agent_id: Some("worker-a".into()),
            instruction: "targeted steer".into(),
            ts: ts + 2,
        })
        .unwrap();
    let rebuilt = store.replay("g1").unwrap().unwrap();
    let steer = rebuilt.pending_steer.as_ref().expect("steer folded");
    assert_eq!(steer.agent_id.as_deref(), Some("worker-a"));
    assert_eq!(steer.instruction, "targeted steer");
    assert_eq!(steer.ts, ts + 2);
}

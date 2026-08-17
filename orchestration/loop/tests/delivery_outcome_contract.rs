//! P0-2 contract tests: post-delivery outcome closure —
//!   ① `delivery_outcome` events: completing an advancement todo records a
//!      `delivered` signal; `delivery record` resolves it to
//!      verified/failed/rework with validated transitions.
//!   ② `outcome_followthrough`: a delivery left unverified for N turns
//!      auto-derives a follow-up todo (exactly once per delivery cycle).
//!
//! These exercise the real CLI entry (`console::run`) against an isolated
//! `FUTURE_LOOP_ROOT`, plus direct store replays for the turn counters.

use future_loop::console;
use future_loop::state::{Goal, RunRecord};
use future_loop::store::Store;
use future_loop::work_items::delivery_outcome as dov;

fn with_root<F: FnOnce(&str)>(tag: &str, f: F) {
    // FUTURE_LOOP_ROOT is process-global; tests run in parallel, so
    // serialize all CLI tests behind one mutex (each still gets its own
    // isolated root dir).
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("future-loop-delivery-{tag}-{}", uuid_like()));
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.join(".future/loop");
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("FUTURE_LOOP_ROOT", root.to_str().unwrap());
    f(root.to_str().unwrap());
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

fn cli(args: &[&str]) -> Result<(), String> {
    console::run(
        "future-loop",
        args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    )
    .map_err(|e| format!("{e:#}"))
}

/// Set up goal `g1` with one advancement todo; return its todo id.
fn setup_goal_with_todo() -> String {
    cli(&[
        "goal",
        "init",
        "--objective",
        "delivery closure test",
        "--cwd",
        "/tmp",
        "--goal-id",
        "g1",
    ])
    .unwrap();
    cli(&["todo", "add", "--goal", "g1", "--text", "ship the widget"]).unwrap();
    let store = Store::open(&std::env::var("FUTURE_LOOP_ROOT").unwrap()).unwrap();
    let g = store.replay("g1").unwrap().unwrap();
    g.todos
        .iter()
        .find(|t| t.text.contains("ship the widget"))
        .map(|t| t.id.clone())
        .unwrap()
}

fn replay(root: &str) -> Goal {
    Store::open(root).unwrap().replay("g1").unwrap().unwrap()
}

fn run_record(turn: u32) -> RunRecord {
    RunRecord {
        turn,
        todo_id: "t".into(),
        run_id: format!("r{turn}"),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec![],
        evidence: "ev".into(),
        recorded_at: 0,
        spend_source: None,
        validation: None,
    }
}

// ── ① todo complete records a `delivered` outcome; delivery record resolves
//    it with validated transitions. ─────────────────────────────────────────
#[test]
fn completion_records_delivered_and_record_resolves() {
    with_root("resolve", |root| {
        let todo_id = setup_goal_with_todo();
        cli(&[
            "todo",
            "complete",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--no-follow-up",
            "--evidence",
            "fixture evidence for completion contract",
        ])
        .unwrap();

        // The completion auto-recorded a pending delivery.
        let g = replay(root);
        let d = g
            .delivery_state(&todo_id)
            .expect("completion must record a delivery");
        assert_eq!(d.outcome, dov::OUTCOME_DELIVERED);
        assert_eq!(d.followthrough_todo_id, None);

        // Resolving without a pending delivery state machine violation:
        // verified → verified is rejected; bogus outcome is rejected.
        assert!(cli(&[
            "delivery",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--outcome",
            "verified",
            "--note",
            "confirmed in production",
        ])
        .is_ok());
        let g = replay(root);
        assert_eq!(
            g.delivery_state(&todo_id).unwrap().outcome,
            dov::OUTCOME_VERIFIED
        );
        assert_eq!(
            g.delivery_state(&todo_id).unwrap().note.as_deref(),
            Some("confirmed in production")
        );
        // A verified delivery is closed — no re-resolution, no re-delivery.
        assert!(cli(&[
            "delivery",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--outcome",
            "failed",
        ])
        .is_err());
        assert!(cli(&[
            "delivery",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--outcome",
            "delivered",
        ])
        .is_err());
        assert!(cli(&[
            "delivery",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--outcome",
            "bogus",
        ])
        .is_err());
        // Unknown todo id fails closed.
        assert!(cli(&[
            "delivery",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            "todo_nope",
            "--outcome",
            "verified",
        ])
        .is_err());
    });
}

// ── ① transitions: failed/rework allow re-delivery (a fresh cycle). ───────
#[test]
fn failed_delivery_allows_redelivery_cycle() {
    with_root("redelivery", |root| {
        let todo_id = setup_goal_with_todo();
        cli(&[
            "todo",
            "complete",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--no-follow-up",
            "--evidence",
            "fixture evidence for completion contract",
        ])
        .unwrap();
        cli(&[
            "delivery",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--outcome",
            "failed",
        ])
        .unwrap();
        // Re-delivery is legal after a failure and resets the cycle.
        cli(&[
            "delivery",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--outcome",
            "delivered",
        ])
        .unwrap();
        let g = replay(root);
        assert_eq!(
            g.delivery_state(&todo_id).unwrap().outcome,
            dov::OUTCOME_DELIVERED
        );
        // Double delivery while pending is rejected.
        assert!(cli(&[
            "delivery",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--outcome",
            "delivered",
        ])
        .is_err());
    });
}

// ── ② follow-through: overdue delivery auto-derives a follow-up todo,
//    exactly once; resolving the delivery stops the follow-through. ────────
#[test]
fn followthrough_fires_once_for_overdue_delivery() {
    with_root("followthrough", |root| {
        let todo_id = setup_goal_with_todo();
        cli(&[
            "todo",
            "complete",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--no-follow-up",
            "--evidence",
            "fixture evidence for completion contract",
        ])
        .unwrap();
        // Push the run-turn counter past the threshold (delivered at turn 0).
        let store = Store::open(root).unwrap();
        store.append_run("g1", &run_record(5)).unwrap();
        drop(store);

        // Manual scan: derives exactly one follow-up todo.
        cli(&["delivery", "followthrough", "--goal", "g1"]).unwrap();
        let g = replay(root);
        let d = g.delivery_state(&todo_id).unwrap();
        let followup_id = d
            .followthrough_todo_id
            .clone()
            .expect("overdue delivery must derive a follow-up todo");
        let followup = g.todo(&followup_id).expect("follow-up todo exists");
        assert!(followup.text.contains("Follow-through"));
        assert!(followup.text.contains(&todo_id));
        assert!(followup.text.contains("ship the widget"));
        let todo_count = g.todos.len();

        // Second scan: the stamp dedupes — no additional todo.
        cli(&["delivery", "followthrough", "--goal", "g1"]).unwrap();
        assert_eq!(replay(root).todos.len(), todo_count);

        // Resolving the source delivery closes the loop; further scans at a
        // much later turn still derive nothing for it.
        cli(&[
            "delivery",
            "record",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--outcome",
            "verified",
        ])
        .unwrap();
        let store = Store::open(root).unwrap();
        store.append_run("g1", &run_record(99)).unwrap();
        drop(store);
        cli(&["delivery", "followthrough", "--goal", "g1"]).unwrap();
        assert_eq!(replay(root).todos.len(), todo_count);
    });
}

// ── ② not-yet-overdue deliveries derive nothing. ──────────────────────────
#[test]
fn followthrough_respects_threshold() {
    with_root("threshold", |root| {
        let todo_id = setup_goal_with_todo();
        cli(&[
            "todo",
            "complete",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--no-follow-up",
            "--evidence",
            "fixture evidence for completion contract",
        ])
        .unwrap();
        // Turn 2 < default threshold 3 → nothing derived.
        let store = Store::open(root).unwrap();
        store.append_run("g1", &run_record(2)).unwrap();
        drop(store);
        cli(&["delivery", "followthrough", "--goal", "g1"]).unwrap();
        let g = replay(root);
        assert_eq!(
            g.delivery_state(&todo_id).unwrap().followthrough_todo_id,
            None
        );
        // --turns 1 overrides the threshold → fires.
        cli(&["delivery", "followthrough", "--goal", "g1", "--turns", "1"]).unwrap();
        assert!(replay(root)
            .delivery_state(&todo_id)
            .unwrap()
            .followthrough_todo_id
            .is_some());
    });
}

// ── read surface: `delivery status` text + JSON projections. ──────────────
#[test]
fn delivery_status_projects_read_model() {
    with_root("status", |root| {
        let todo_id = setup_goal_with_todo();
        cli(&[
            "todo",
            "complete",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--no-follow-up",
            "--evidence",
            "fixture evidence for completion contract",
        ])
        .unwrap();
        // JSON read model carries the delivery with its age + pending flag.
        cli(&["delivery", "status", "--goal", "g1", "--format", "json"]).unwrap();
        // Replay-level projection: the ledger round-trips the new events.
        let report = Store::open(root).unwrap().verify("g1").unwrap();
        assert!(report.ok, "ledger conflicts: {:?}", report.conflicts);
        let g = replay(root);
        assert_eq!(g.delivery_states.len(), 1);
        assert_eq!(g.delivery_states[0].todo_id, todo_id);
    });
}

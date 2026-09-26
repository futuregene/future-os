//! Guards and error arms named by llvm-cov's text report.
//!
//! Each test here was chosen from the *named* zero-count lines in
//! `cargo llvm-cov report --text`, not from a percentage:
//! - `store.rs:1719-1720` — the kanban-monotonicity guard (a superseded todo
//!   must not be resurrected to done by a late `TodoCompleted` writeback),
//! - `store.rs:1000-1002` — the relative-write-scope guard,
//! - `store.rs:1458` — a ledger read that fails for a reason other than NotFound,
//! - `run_index.rs:286` — a run file that cannot be read as UTF-8.

mod common;

use common::{cli_root, init_goal, open_store};
use future_loop::state::{Todo, TodoStatus};
use future_loop::store::{Event, Store};

/// The kanban's terminal states are monotonic: a todo superseded while a
/// detached run was in flight must NOT be resurrected to `done` by that run's
/// late `TodoCompleted` writeback. The fold must still record the event (the run
/// record is kept), so this asserts BOTH halves — the status stays superseded
/// and the event is not dropped.
#[test]
fn a_late_completion_does_not_resurrect_a_superseded_todo() {
    let cr = cli_root();
    let gid = init_goal(&cr, "monotonic terminal states");
    let mut store = open_store(&cr);
    let todo = store.replay(&gid).unwrap().unwrap().todos[0].id.clone();

    // Supersede it (cancel wins over an in-flight delivery).
    store
        .append(Event::TodoSuperseded {
            goal_id: gid.clone(),
            todo_id: todo.clone(),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    assert_eq!(
        store
            .replay(&gid)
            .unwrap()
            .unwrap()
            .todo(&todo)
            .unwrap()
            .status,
        TodoStatus::Superseded,
        "the fixture must start superseded"
    );

    // Now a late completion arrives for that same todo.
    store
        .append(Event::TodoCompleted {
            goal_id: gid.clone(),
            todo_id: todo.clone(),
            no_follow_up: true,
            successor_ids: vec![],
            evidence: Some("late delivery".into()),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();

    let goal = store.replay(&gid).unwrap().unwrap();
    assert_eq!(
        goal.todo(&todo).unwrap().status,
        TodoStatus::Superseded,
        "cancel/supersede must win over a late completion: {goal:?}"
    );
    assert!(
        goal.todo(&todo).unwrap().completed_at.is_none(),
        "the completion stamp must not be applied to a superseded todo"
    );
    // The event itself is kept (it is ledger history, not a status mutation).
    let completions = store
        .events(&gid)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(&e.event, Event::TodoCompleted { todo_id, .. } if todo_id == &todo))
        .count();
    assert_eq!(completions, 1, "the event must still be recorded");

    // A NON-superseded todo still completes normally: the guard must not block
    // legitimate completions.
    let other = "todo_plain".to_string();
    store
        .append(Event::TodoAdded {
            goal_id: gid.clone(),
            todo: Todo::advancement(&other, "ordinary work"),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    store
        .append(Event::TodoCompleted {
            goal_id: gid.clone(),
            todo_id: other.clone(),
            no_follow_up: true,
            successor_ids: vec![],
            evidence: Some("done".into()),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    let goal = store.replay(&gid).unwrap().unwrap();
    assert_eq!(goal.todo(&other).unwrap().status, TodoStatus::Done);
    assert!(
        goal.todo(&other).unwrap().completed_at.is_some(),
        "an ordinary completion must be stamped"
    );
}

/// A goal whose `cwd` is relative cannot resolve a task's relative write scope,
/// so the atomic claim must refuse with an actionable message instead of
/// comparing paths that cannot be interpreted. Claiming itself stays allowed
/// while the scope is absolute or empty.
#[test]
fn atomic_claim_refuses_a_relative_scope_under_a_relative_cwd() {
    let cr = cli_root();
    // A goal whose REGISTRY entry carries a RELATIVE cwd. `register()` normalizes
    // the cwd, so this legacy shape can only be simulated by writing the registry
    // file directly - which is exactly the imported-state case the guard is for.
    // (The state root is the harness's own tempdir.)
    let gid = "goal_relative_cwd".to_string();
    let mut store = Store::open(&cr.root).unwrap();
    let mut goal = future_loop::state::Goal::new(&gid, "legacy relative cwd", ".");
    goal.add(Todo::advancement("t1", "work").with_write_scopes(&["src/relative"]));
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: gid.clone(),
            ts: 1,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: gid.clone(),
            todo: Todo::advancement("t1", "work").with_write_scopes(&["src/relative"]),
            ts: 1,
        })
        .unwrap();
    // A SECOND todo, live-claimed by another agent, also carrying a relative
    // scope. The guard's `any(|t| (t.id == todo_id || (claimed && live)))`
    // short-circuits on the id match, so only a todo whose id differs reaches
    // the claimed-and-live half of the condition.
    store
        .append(Event::TodoAdded {
            goal_id: gid.clone(),
            todo: Todo::advancement("t2", "other work").with_write_scopes(&["src/other"]),
            ts: 1,
        })
        .unwrap();
    store
        .append(Event::TodoClaimed {
            goal_id: gid.clone(),
            todo_id: "t2".into(),
            agent_id: "peer".into(),
            lease_expires_at: future_loop::state::now_epoch() + 3600,
            holder_pid: Some(std::process::id()),
            ts: 1,
        })
        .unwrap();

    // Rewrite the registry with a relative cwd (the legacy/imported shape).
    let registry_path = std::path::Path::new(&cr.root).join("registry.json");
    let legacy = serde_json::json!([{
        "goal_id": gid,
        "objective": "legacy relative cwd",
        "cwd": ".",
        "status": "active",
        "created_at": 1,
    }]);
    std::fs::write(
        &registry_path,
        serde_json::to_string_pretty(&legacy).unwrap(),
    )
    .unwrap();
    // Re-open so the store loads the rewritten registry.
    let mut store = Store::open(&cr.root).unwrap();

    // The CLAIM TARGET is a third todo, declared after the claimed peer. `any`
    // iterates in order and short-circuits, so the loop must first pass a todo
    // that does NOT match the id — that is what makes the claimed-and-live half
    // of the condition evaluate instead of being short-circuited away.
    store
        .append(Event::TodoAdded {
            goal_id: gid.clone(),
            todo: Todo::advancement("t3", "claim target").with_write_scopes(&["src/target"]),
            ts: 1,
        })
        .unwrap();

    let err = store
        .try_claim_todo_with_workspace(&gid, "t3", "a1", 60, None, false)
        .map(|_| ())
        .expect_err("a relative scope under a relative cwd must be refused");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("cannot resolve relative task write scopes"),
        "the refusal must name the problem: {msg}"
    );
    assert!(
        msg.contains("absolute --cwd"),
        "the refusal must say how to fix it: {msg}"
    );
    // Nothing was claimed.
    assert!(
        store
            .replay(&gid)
            .unwrap()
            .unwrap()
            .todo("t3")
            .unwrap()
            .claimed_by
            .is_none(),
        "a refused claim must not write a claim"
    );
}

/// A ledger read that fails for a reason other than "removed between the
/// existence check and the read" must surface, not be swallowed as an empty
/// ledger: silently returning empty would make a permission problem look like a
/// fresh goal and the next append would then start a second history.
#[test]
fn an_unreadable_ledger_surfaces_instead_of_reading_as_empty() {
    let cr = cli_root();
    let gid = init_goal(&cr, "unreadable ledger");
    let mut store = open_store(&cr);
    store
        .append(Event::GoalStarted {
            goal_id: gid.clone(),
            ts: 1,
        })
        .unwrap();
    let ledger = std::path::Path::new(&cr.root)
        .join("goals")
        .join(&gid)
        .join("events.jsonl");
    assert!(ledger.is_file(), "the fixture must have a ledger");

    // Replace the ledger FILE with a directory of the same name: the path still
    // exists (so the NotFound arm cannot apply) but reading it fails.
    std::fs::remove_file(&ledger).unwrap();
    std::fs::create_dir(&ledger).unwrap();

    let err = store
        .events(&gid)
        .map(|_| ())
        .expect_err("an unreadable ledger must surface as an error");
    let msg = format!("{err:#}");
    assert!(msg.contains("read ledger"), "unexpected error: {msg}");
}

/// A run file the index scan cannot decode as UTF-8 is skipped rather than
/// aborting the whole index: one corrupt file must not hide every other run.
#[test]
fn the_index_scan_skips_a_run_file_that_is_not_utf8() {
    use future_loop::runtime::run_index::load_run_index;
    let cr = cli_root();
    let gid = init_goal(&cr, "non-utf8 run file");
    let mut store = open_store(&cr);
    store
        .append(Event::GoalStarted {
            goal_id: gid.clone(),
            ts: 1,
        })
        .unwrap();

    let runs = std::path::Path::new(&cr.root).join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    // A run file whose bytes are not valid UTF-8 (a lone 0xFF).
    std::fs::write(runs.join("corrupt.json"), [0xFFu8, 0xFE, 0x00, 0x41]).unwrap();
    // …plus a valid one that must still be indexed.
    let record = future_loop::state::RunRecord {
        agent_id: Some("w1".into()),
        turn: 1,
        todo_id: "t1".into(),
        run_id: "run-good".into(),
        terminal_state: "completed".into(),
        error: None,
        tokens_in_delta: 0,
        tokens_out_delta: 0,
        cost_delta: 0.0,
        tools: vec![],
        evidence: "artifact".into(),
        recorded_at: future_loop::state::now_epoch(),
        spend_source: None,
        validation: None,
        failure_kind: Some(future_loop::state::FailureKind::None),
        truncation: None,
    };
    future_loop::compat::write_run(
        &std::path::Path::new(&cr.root).join("goals").join(&gid),
        &gid,
        &record,
    )
    .unwrap();

    let rows = load_run_index(&cr.root, &gid).unwrap();
    assert!(
        rows.iter().any(|r| r.classification == "completed"),
        "the decodable run must still be indexed despite the corrupt one: {rows:?}"
    );
    assert!(
        !rows.iter().any(|r| r.path.contains("corrupt")),
        "the undecodable file must be skipped: {rows:?}"
    );
}

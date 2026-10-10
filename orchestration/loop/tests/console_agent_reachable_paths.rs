//! Two `console.rs` paths that need a *reachable* agent rather than an unreachable
//! one, which is why the earlier subprocess-driven tests could not reach them.
//!
//! Both are driven in-process so the mock agent's real port can be published to the
//! CLI (the subprocess helper deliberately pins an unreachable address so no test
//! can talk to a developer's live agent).

mod common;

use common::mock_agent::{completed_events, spawn_mock, MockState};
use common::{cli, cli_ok, cli_root, init_goal, open_store};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// `record_no_progress_if_idle` must persist a `TurnNoProgress` breach and say so.
///
/// The threshold is relaxed from the default (minutes) to 1s through
/// `FUTURE_LOOP_NO_PROGRESS_SECS`, which is the documented knob. The mock's stream is
/// delayed past that threshold so the TURN itself is long: `TurnProgressTracker` is
/// created per turn with `now_epoch()`, so the idle window is measured from turn start
/// and only real elapsed time can trip it (the validator runs later, in writeback).
///
/// This matters operationally: the event is what `status` renders and what a
/// supervisor polls to see a worker sitting idle mid-turn. If the condition were
/// evaluated but never recorded, a stuck turn would look identical to a busy one.
#[test]
fn a_turn_with_no_write_tool_records_a_no_progress_breach() {
    let cr = cli_root();
    let goal = init_goal(&cr, "no progress breach");
    cli_ok(&["agent", "register", "--goal", &goal, "--agent-id", "w1"]);

    #[cfg(windows)]
    const SLOW: &str = "ping -n 3 127.0.0.1 >NUL";
    #[cfg(not(windows))]
    const SLOW: &str = "sleep 2";
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "slow slice",
        "--verify",
        SLOW,
    ]);

    let rt = rt();
    let (addr, _state) = rt.block_on(spawn_mock(MockState {
        // One completed turn; no tool events at all, so nothing counts as a write.
        events: completed_events("mock-run-1"),
        // The turn must ITSELF exceed the 1s idle threshold: the tracker timestamps
        // the turn start, so only real elapsed time can trip the window (the validator
        // runs later, during writeback).
        stream_delay: Some(std::time::Duration::from_millis(1200)),
        ..Default::default()
    }));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    // Relax the idle threshold to 1s: the documented knob, and the only way to make
    // the breach deterministic without waiting out the default.
    std::env::set_var("FUTURE_LOOP_NO_PROGRESS_SECS", "1");

    let outcome = cli(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "w1",
        "--max-turns",
        "6",
    ]);
    std::env::remove_var("FUTURE_LOOP_NO_PROGRESS_SECS");
    let _ = outcome;

    let store = open_store(&cr);
    // The durable claim: the breach must be IN THE LEDGER, because that is what
    // `status` renders and what a supervisor polls. A printed warning alone would
    // leave a stuck turn indistinguishable from a busy one after the process exits.
    let breaches: Vec<(String, u64, u32)> = store
        .events(&goal)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.event {
            future_loop::store::Event::TurnNoProgress {
                todo_id,
                idle_secs,
                tool_calls_total,
                ..
            } => Some((todo_id, idle_secs, tool_calls_total)),
            _ => None,
        })
        .collect();
    assert!(
        !breaches.is_empty(),
        "a turn that ran past the idle threshold with no write tool must record a \
         TurnNoProgress breach; ledger held {:?}",
        store
            .events(&goal)
            .unwrap()
            .into_iter()
            .map(|e| format!("{:?}", std::mem::discriminant(&e.event)))
            .count()
    );
    let (todo_id, idle_secs, tool_calls) = &breaches[0];
    assert_eq!(
        todo_id,
        &store.replay(&goal).unwrap().unwrap().todos[0].id,
        "the breach must name the todo that was idle"
    );
    assert!(
        *idle_secs >= 1,
        "the recorded idle duration must be at least the configured threshold: {idle_secs}"
    );
    assert!(
        *tool_calls <= 1,
        "the recorded tool-call count must reflect the turn (the mock stream has no 
         write-class tool), not a default: {tool_calls}"
    );
}

/// `worker stop` against a REACHABLE agent: the abort succeeds, so the worker's
/// lease is released immediately (its worker was stopped) instead of being left to
/// expire. That is the whole point of the stop - a stopped worker can neither renew
/// nor complete its claim, so the claim would block peers until the TTL lapses.
///
/// Needs the mock agent because the abort has to actually succeed: with an
/// unreachable agent `abort_worker_sessions` returns early and keeps the leases on
/// purpose (a worker that may still be mid-turn must not lose its claim).
#[test]
fn worker_stop_through_a_reachable_agent_releases_the_claim() {
    let cr = cli_root();
    let goal = init_goal(&cr, "stop a reachable worker");
    cli_ok(&["agent", "register", "--goal", &goal, "--agent-id", "w9"]);
    let store = open_store(&cr);
    let todo = store.replay(&goal).unwrap().unwrap().todos[0].id.clone();
    drop(store);
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &goal,
        "--todo-id",
        &todo,
        "--agent-id",
        "w9",
        "--lease-secs",
        "600",
    ]);
    // The bound session is what `bound_worker_sessions` finds; `worker stop` also
    // needs it to know which session to abort.
    let runs = std::path::Path::new(&cr.root).join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    let header = serde_json::json!({
        "type": "run_header",
        "wall_ts": 1_700_000_000u64,
        "run_id": "run-w9",
        "session_id": "sess-w9",
        "agent_id": "w9",
        "todo_id": todo,
        "goal_id": goal,
    });
    std::fs::write(runs.join("run_w9.live.jsonl"), format!("{header}\n")).unwrap();

    let rt = rt();
    let (addr, _state) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    // The command prints its own confirmation; the durable proof is the ledger.
    cli_ok(&["worker", "stop", "--goal", &goal, "--agent-id", "w9"]);

    let store = open_store(&cr);
    let after = store.replay(&goal).unwrap().unwrap();
    assert!(
        after.todo(&todo).unwrap().claimed_by.is_none(),
        "a successfully aborted worker's claim must be released, not left to the \
         TTL: {:?}",
        after.todo(&todo)
    );
    let released = store
        .events(&goal)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.event, future_loop::store::Event::TodoReleased { .. }))
        .count();
    assert_eq!(
        released, 1,
        "exactly one release, and it must be the ledger record of the stop"
    );
    let stopped = store
        .events(&goal)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.event, future_loop::store::Event::WorkerStopped { .. }))
        .count();
    assert_eq!(stopped, 1, "the stop signal is written before the abort");
}

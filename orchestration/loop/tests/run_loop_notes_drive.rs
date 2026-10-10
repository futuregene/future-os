//! Turn-loop note assembly and the mid-run worker-stop break.
//!
//! `run_turns` builds each turn's note from three independent sources — a
//! down-channel `pending_steer`, durable `ControlIssued` instructions, and
//! recent `ProgressReported` milestones from the worker's own todo — and it
//! checks the ledger for an operator-issued `WorkerStopped` at every turn
//! boundary. These are the parts of the loop that a plain
//! `run --anonymous --max-turns N` drive never reaches.

mod common;

use common::mock_agent::{completed_events, ev, spawn_mock, MockState};
use common::{cli, cli_ok, cli_root, first_todo_id, init_goal, open_store};
use future_loop::state::now_epoch;
use future_loop::store::Event;

/// The mock's stream-event type, re-exported through the loop's own rpc dep
/// (`mock_agent` keeps `StreamEvent` private).
type StreamEvent = future_rpc::proto::StreamEvent;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Two completed turns, so the loop executes a second turn and can re-decide.
fn two_turns() -> Vec<StreamEvent> {
    let mut events = completed_events("mock-run-1");
    events.extend([
        ev("mock-run-2", 10, "agent_start", "{}"),
        ev("mock-run-2", 11, "text_chunk", "{\"text\":\"second turn\"}"),
        ev(
            "mock-run-2",
            12,
            "agent_end",
            "{\"state\":\"completed\",\"tokens_in\":1,\"tokens_out\":1}",
        ),
    ]);
    events
}

/// The note a turn received is visible in the mock's prompt log, so the
/// assertions are on real payloads rather than on `println!` output.
#[test]
fn turn_note_carries_steer_controls_and_reported_milestones() {
    let cr = cli_root();
    let rt = rt();
    let goal = init_goal(&cr, "turn notes");
    let todo = first_todo_id(&cr.root, &goal);

    // Seed all three note sources through the real CLI, before the run starts.
    let mut store = open_store(&cr);
    store
        .append(Event::WorkerSteered {
            goal_id: goal.clone(),
            agent_id: Some("w1".into()),
            instruction: "follow the ledger, not the plan".into(),
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::ControlIssued {
            goal_id: goal.clone(),
            instruction: future_loop::agents::control::Instruction {
                id: "ctrl-1".into(),
                agent_id: Some("w1".into()),
                text: "hold until the validator passes".into(),
                interrupt: false,
            },
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::ProgressReported {
            goal_id: goal.clone(),
            agent_id: "w1".into(),
            todo_id: todo.clone(),
            message: "halfway through the ledger work".into(),
            ts: now_epoch(),
        })
        .unwrap();
    // A progress note for a DIFFERENT todo must not leak into this turn.
    store
        .append(Event::ProgressReported {
            goal_id: goal.clone(),
            agent_id: "w1".into(),
            todo_id: "todo_other".into(),
            message: "LEAKED-OTHER-TODO-NOTE".into(),
            ts: now_epoch(),
        })
        .unwrap();
    drop(store);

    let (addr, shared) = rt.block_on(spawn_mock(MockState {
        events: two_turns(),
        ..Default::default()
    }));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    // `--agent-id w1` so the steer and the control instruction target this
    // worker (both are recipient-scoped).
    common::cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "w1",
        "--max-turns",
        "2",
    ]);
    let messages = shared.lock().unwrap().prompt_messages.join("\n");
    assert!(
        messages.contains("SUPERVISOR STEERING (new instructions"),
        "the pending steer must reach the turn: {messages}"
    );
    assert!(
        messages.contains("follow the ledger, not the plan"),
        "the steer text must be carried: {messages}"
    );
    assert!(
        messages.contains("SUPERVISOR STEERING [ctrl-1]"),
        "durable control instructions must reach the turn: {messages}"
    );
    assert!(
        messages.contains(
            "Reported milestone (claim, verify against artifacts): halfway through the ledger work"
        ),
        "the worker's own milestone must reach the turn: {messages}"
    );
    assert!(
        !messages.contains("LEAKED-OTHER-TODO-NOTE"),
        "another todo's milestone must not leak into this turn: {messages}"
    );
}

/// An operator `worker_stopped` event appended while the run is live must make
/// the run client exit at the next turn boundary, classified infra-recoverable
/// so the session stays resumable (an operator stop is not a science failure).
#[test]
fn a_mid_run_worker_stop_breaks_the_loop_as_infra_recoverable() {
    let cr = cli_root();
    let rt = rt();
    let goal = init_goal(&cr, "mid-run stop");
    // Register the worker first: the stop event is agent-scoped.
    cli_ok(&["agent", "register", "--goal", &goal, "--agent-id", "w1"]);
    // A second open todo keeps the goal non-terminal after turn 1, so the loop
    // returns to a turn boundary (where the stop is checked) instead of breaking
    // on terminal closure.
    let bootstrap = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "supersede",
        "--goal",
        &goal,
        "--todo-id",
        &bootstrap,
        "--reason",
        "fixture",
    ]);
    // Each todo carries a validator, which is a real subprocess on the
    // completion path. That gives the stopper a multi-second window to land the
    // event before the next boundary check; without it the race is
    // millisecond-scale and the fixture becomes a coin flip.
    #[cfg(windows)]
    const SLOW: &str = "ping -n 3 127.0.0.1 >NUL";
    #[cfg(not(windows))]
    const SLOW: &str = "sleep 2";
    for text in ["first slice", "second slice"] {
        cli_ok(&[
            "todo", "add", "--goal", &goal, "--text", text, "--verify", SLOW,
        ]);
    }

    // The agent streams one completed turn and then never yields.
    let (addr, shared) = rt.block_on(spawn_mock(MockState {
        events: completed_events("mock-run-1"),
        events_then_hang: true,
        ..Default::default()
    }));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    let root = cr.root.clone();
    let goal_for_stop = goal.clone();
    // Append the stop once the first turn has been prompted. Turn 1 then spends
    // seconds in its validator, so the event is in the ledger well before the
    // next boundary check.
    let stopper = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            {
                let st = shared.lock().unwrap();
                if !st.prompt_messages.is_empty() {
                    break;
                }
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let mut store = future_loop::store::Store::open(&root).unwrap();
        store
            .append(Event::WorkerStopped {
                goal_id: goal_for_stop,
                agent_id: Some("w1".into()),
                ts: now_epoch(),
            })
            .unwrap();
        true
    });

    // A generous turn budget: the loop must exit because of the stop, not
    // because it ran out of turns.
    let outcome = cli(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "w1",
        "--max-turns",
        "50",
    ]);
    let stopped = stopper.join().unwrap();
    assert!(
        stopped,
        "the fixture never got a prompt, so the stop could not be delivered"
    );
    let err = outcome.err().unwrap_or_default();
    assert!(
        !err.contains("max-turns"),
        "the loop must exit on the stop, not the budget: {err}"
    );
    // The second todo is untouched and the goal is therefore NOT terminally
    // closed, so the only way `run` can return success here is the stop break:
    // a terminal closure would have needed the second todo done too.
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    let second = g
        .todos
        .iter()
        .find(|t| t.text.contains("second slice"))
        .expect("the second todo must exist");
    assert_eq!(
        second.status,
        future_loop::state::TodoStatus::Open,
        "the stop must land before the second turn runs"
    );
    assert!(
        !g.is_terminal(),
        "the goal must not be terminally closed by the stopped run"
    );

    // FINDING (product, reported not fixed): the retention classification this
    // branch writes is not observable. `cmd_run` builds a `SessionRetention`
    // (`failure_kind: InfraRecoverable`, `resumable: true`) and assigns it to
    // the replayed `Goal`, then persists through `compat::write_active_state` —
    // but `compat::render_active_state` emits no retention field, no ledger
    // event carries it, and `store.replay` therefore never restores it. The
    // assertion below pins the current (lossy) behaviour so the gap is visible
    // in a test rather than only in prose; see docs/testing/module-loop.md §10.
    let state_path = std::path::Path::new(&cr.root)
        .join("goals")
        .join(&goal)
        .join("ACTIVE_GOAL_STATE.md");
    let state = std::fs::read_to_string(&state_path)
        .unwrap_or_else(|e| panic!("the run must write {}: {e}", state_path.display()));
    assert!(
        !state.contains("resumable"),
        "if this now fails, the retention gap was closed — delete this assertion \
         and assert the real classification instead: {state}"
    );
}

/// A turn whose todo was SUPERSEDED while it ran must still keep its spend/run
/// evidence, but must not reopen or re-complete the todo.
///
/// That is the `already_closed` arm of `run_turns`: an operator (or a supervisor)
/// can retire a todo mid-turn, and the writeback that lands afterwards is stale
/// work. Dropping it entirely would lose the tokens the run spent; applying it
/// would resurrect a todo the operator just retired - which the ledger already
/// refuses to do for `todo complete`, so the loop has to agree with it.
#[test]
fn a_turn_that_finishes_after_its_todo_was_superseded_keeps_evidence_only() {
    let cr = cli_root();
    let rt = rt();
    let goal = init_goal(&cr, "superseded mid-run");
    cli_ok(&["agent", "register", "--goal", &goal, "--agent-id", "w1"]);
    let bootstrap = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "supersede",
        "--goal",
        &goal,
        "--todo-id",
        &bootstrap,
        "--reason",
        "fixture",
    ]);
    // The validator is a real subprocess: it holds the turn open for seconds, which
    // is the window the supersede needs to land before the writeback.
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
        "work in flight",
        "--verify",
        SLOW,
    ]);

    let (addr, shared) = rt.block_on(spawn_mock(MockState {
        events: completed_events("mock-run-1"),
        events_then_hang: true,
        ..Default::default()
    }));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    let root = cr.root.clone();
    let goal_for_supersede = goal.clone();
    let superseder = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            {
                let st = shared.lock().unwrap();
                if !st.prompt_messages.is_empty() {
                    break;
                }
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Retire the todo the running turn is working on, through the same public
        // store API the CLI uses (the CLI shells out to itself for `supersede`).
        let mut store = future_loop::store::Store::open(&root).unwrap();
        let state = store.replay(&goal_for_supersede).unwrap().unwrap();
        let victim = state
            .todos
            .iter()
            .find(|t| t.text == "work in flight")
            .expect("the todo must exist")
            .id
            .clone();
        store
            .append(Event::TodoSuperseded {
                goal_id: goal_for_supersede.clone(),
                todo_id: victim.clone(),
                ts: now_epoch(),
            })
            .unwrap();
        Some(victim)
    });

    let outcome = cli(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "w1",
        "--max-turns",
        "20",
    ]);
    let victim = superseder
        .join()
        .unwrap()
        .expect("the fixture never got a prompt, so the supersede could not land");

    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    let todo = g.todo(&victim).expect("the todo still exists");
    assert_eq!(
        todo.status,
        future_loop::state::TodoStatus::Superseded,
        "the stale writeback must NOT reopen a superseded todo (outcome: {outcome:?})"
    );

    // The evidence is the part that MUST survive: the run really consumed tokens.
    let runs: Vec<_> = g.history.iter().filter(|r| r.todo_id == victim).collect();
    assert!(
        !runs.is_empty(),
        "the closed turn's spend/run evidence must be kept in the goal history, \
         not dropped: {:?}",
        g.history
            .iter()
            .map(|r| (&r.todo_id, &r.terminal_state))
            .collect::<Vec<_>>()
    );
    assert!(
        runs.iter().any(|r| r.spend_source.is_some()),
        "the kept run must still be classified for quota accounting: {runs:?}"
    );
    // And no RunRecorded-less silence: the run is also in the ledger per-goal index.
    let projected = store.replay(&goal).unwrap().unwrap();
    assert!(
        projected
            .history
            .iter()
            .any(|r| r.todo_id == victim && r.terminal_state == "completed"),
        "the completed turn must be visible with its terminal state"
    );
}

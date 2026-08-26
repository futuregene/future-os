//! Coverage drive for the gRPC-backed console commands: `run`, `models`,
//! and the `doctor --agent-addr` probe — all against the in-process mock
//! FutureAgent (FUTURE_LOOP_AGENT_ADDR points the CLI at the mock).
//!
//! Everything here is serialized through CLI_LOCK (process-global env), and
//! each test spawns its own mock server on an ephemeral port.

mod common;

use common::mock_agent::{completed_events, ev, spawn_mock, AttachPlan, MockState};
#[cfg(unix)]
use common::todo_id_by_text;
use common::{add_todo, cli, cli_err, cli_ok, cli_root, first_todo_id, init_goal, open_store};
use future_loop::state::{now_epoch, TodoStatus};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Spawn a mock agent and point the CLI at it. Returns the runtime guard
/// (keeps the server task alive) and the shared state.
fn mock_env(state: MockState) -> (tokio::runtime::Runtime, common::mock_agent::SharedState) {
    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(state));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    (rt, shared)
}

// ── run identity gate (no server needed) ───────────────────────────────────

#[test]
fn run_requires_agent_id_or_anonymous() {
    let cr = cli_root();
    let goal = init_goal(&cr, "identity gate");
    let err = cli_err(&["run", "--goal", &goal]);
    assert!(err.contains("requires --agent-id"), "{err}");
}

#[test]
fn run_missing_goal_fails() {
    let _cr = cli_root();
    let err = cli_err(&["run", "--goal", "goal_nope", "--agent-id", "a1"]);
    assert!(err.contains("not found"), "{err}");
}

// ── run turn loop against the mock ─────────────────────────────────────────

#[test]
fn run_anonymous_single_todo_to_terminal() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "single todo closure");
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    assert!(g.todos.iter().all(|t| t.status == TodoStatus::Done));
    assert!(g.terminal_closure().is_some(), "validated closure");
    assert_eq!(g.history.len(), 1);
    assert_eq!(shared.lock().unwrap().prompts, 1);
}

#[test]
fn run_with_agent_id_auto_registers_and_chains_successors() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: vec![
            ev("mock-run-1", 0, "tool_start", "{\"tool_name\":\"write\"}"),
            ev("mock-run-1", 1, "text_chunk", "{\"text\":\"one done\"}"),
            ev("mock-run-1", 2, "agent_end", "{\"state\":\"completed\"}"),
            ev("mock-run-2", 0, "tool_start", "{\"tool_name\":\"write\"}"),
            ev("mock-run-2", 1, "text_chunk", "{\"text\":\"two done\"}"),
            ev("mock-run-2", 2, "agent_end", "{\"state\":\"completed\"}"),
        ],
        ..Default::default()
    });
    let goal = init_goal(&cr, "two todos");
    add_todo(&cr, &goal, "second task");
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "worker-7",
        "--lease-secs",
        "120",
        "--max-turns",
        "5",
    ]);
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    assert!(g.todos.iter().all(|t| t.status == TodoStatus::Done));
    assert!(g.registered_agents.contains(&"worker-7".to_string()));
    assert_eq!(shared.lock().unwrap().prompts, 2);
    // The first completion names the second todo as successor.
    let first = g.todos.first().unwrap();
    assert_eq!(first.successor_ids.len(), 1, "successor chain: {first:?}");
}

#[test]
fn run_max_turns_bails() {
    let cr = cli_root();
    let (_rt, _shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "many todos");
    add_todo(&cr, &goal, "task two");
    add_todo(&cr, &goal, "task three");
    let err = cli_err(&["run", "--goal", &goal, "--anonymous", "--max-turns", "2"]);
    assert!(err.contains("max-turns"), "{err}");
}

#[test]
fn run_workspace_conflict_and_force_workspace() {
    let cr = cli_root();
    let goal = init_goal(&cr, "workspace conflict run");
    // Two agents declaring the same workspace.
    cli_ok(&[
        "agent",
        "onboard",
        "--goal",
        &goal,
        "--agent-id",
        "a1",
        "--workspace",
        "/ws1",
    ]);
    cli_ok(&[
        "agent",
        "onboard",
        "--goal",
        &goal,
        "--agent-id",
        "a2",
        "--workspace",
        "/ws1",
    ]);
    // a1 claims the onboarding todo (live lease in the shared workspace).
    let onboarding = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &goal,
        "--todo-id",
        &onboarding,
        "--agent-id",
        "a1",
    ]);
    // A second open todo for a2 to pick up after forcing.
    let second = add_todo(&cr, &goal, "second task");
    let (_rt, _shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    // a2 running in the same workspace is refused (degrade to serial).
    let err = cli_err(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "a2",
        "--max-turns",
        "2",
    ]);
    assert!(err.contains("workspace conflict"), "{err}");
    // --force-workspace lets a2 claim the second todo and complete it.
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "a2",
        "--force-workspace",
        "--max-turns",
        "3",
    ]);
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    assert_eq!(g.todo(&second).unwrap().status, TodoStatus::Done);
}

#[test]
fn run_failing_turns_exhaust_repair_budget() {
    let cr = cli_root();
    let events = vec![ev(
        "mock-run-1",
        0,
        "agent_end",
        "{\"state\":\"error\",\"error\":\"boom\"}",
    )];
    let (_rt, shared) = mock_env(MockState {
        events,
        ..Default::default()
    });
    let goal = init_goal(&cr, "failing turns");
    let onboarding = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &onboarding,
        "--no-follow-up",
        "--evidence",
        "operator closed the auto-created onboarding todo",
    ]);
    // Pre-seed a todo that already consumed one repair attempt (the ledger
    // does not persist failed_attempts from failed runs — see coverage
    // report), so this turn's failure crosses MAX_REPAIR_ATTEMPTS(=1).
    let mut seeded = future_loop::state::Todo::advancement("todo_seeded", "keeps failing");
    seeded.failed_attempts = 1;
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: goal.clone(),
            todo: seeded,
            ts: now_epoch(),
        })
        .unwrap();
    store.set_next_action(&goal, "keeps failing").unwrap();
    drop(store);
    // ARCHITECTURE-SIMPLIFICATION: the repair budget no longer stops the run
    // loop — a failed todo stays runnable and the failure count is surfaced
    // as an advisory (the agent decides to supersede / re-split). The run
    // loop keeps delivering until the max-turns bound fires (a budget bail,
    // not a repair-budget stop). Assert the loop behavior: it runs all the
    // way to max-turns with the signal in every delivery reason.
    let err = cli_err(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    assert!(
        err.contains("max-turns"),
        "a failed todo stays runnable — the run loop runs to max-turns (budget bail), got: {err}"
    );
    assert_eq!(
        shared.lock().unwrap().prompts,
        3,
        "a failed todo stays runnable — 3 prompts before the max-turns bail"
    );
}

#[test]
fn run_open_gate_breaks_with_ask_user() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "gate blocks the run");
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "plan approval gate",
        "--class",
        "user_gate",
        "--gate-question",
        "approve the plan?",
    ]);
    cli_ok(&["run", "--goal", &goal, "--anonymous"]);
    assert_eq!(
        shared.lock().unwrap().prompts,
        0,
        "no turn executed behind a gate"
    );
}

/// Append a hand-crafted monitor todo directly to the ledger (due_in=0 →
/// already due; the CLI cannot express sub-minute cadences).
fn append_monitor(cr: &common::CliRoot, goal: &str, id: &str, due_in_secs: u64) {
    let mut store = open_store(cr);
    let todo = future_loop::state::Todo::monitor(
        id,
        "watch the artifact",
        std::time::Duration::from_secs(due_in_secs),
    );
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: goal.to_string(),
            todo,
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    store.set_next_action(goal, "watch the artifact").unwrap();
}

#[test]
fn run_monitor_poll_changed_and_no_change() {
    let cr = cli_root();
    // Advancement outranks a due monitor, so turn 1 is the onboarding todo
    // and turn 2 is the poll; script the EXISTS evidence for the poll turn.
    let (_rt, _shared) = mock_env(MockState {
        events: vec![
            ev(
                "mock-run-1",
                0,
                "text_chunk",
                "{\"text\":\"onboarding done\"}",
            ),
            ev("mock-run-1", 1, "agent_end", "{\"state\":\"completed\"}"),
            ev(
                "mock-run-2",
                0,
                "text_chunk",
                "{\"text\":\"the file EXISTS now\"}",
            ),
            ev("mock-run-2", 1, "agent_end", "{\"state\":\"completed\"}"),
        ],
        ..Default::default()
    });
    let goal = init_goal(&cr, "monitor poll");
    append_monitor(&cr, &goal, "mon_due", 0);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "4"]);
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    let m = g.todos.iter().find(|t| t.id == "mon_due").unwrap();
    assert_eq!(
        m.status,
        TodoStatus::Done,
        "changed poll closes the monitor: {m:?}"
    );
}

#[test]
fn run_monitor_poll_no_change_never_spends() {
    let cr = cli_root();
    let (_rt, _shared) = mock_env(MockState {
        events: vec![
            ev(
                "mock-run-1",
                0,
                "text_chunk",
                "{\"text\":\"nothing new yet\"}",
            ),
            ev("mock-run-1", 1, "agent_end", "{\"state\":\"completed\"}"),
            ev(
                "mock-run-2",
                0,
                "text_chunk",
                "{\"text\":\"onboarding done\"}",
            ),
            ev("mock-run-2", 1, "agent_end", "{\"state\":\"completed\"}"),
        ],
        ..Default::default()
    });
    let goal = init_goal(&cr, "monitor no-change");
    append_monitor(&cr, &goal, "mon_due", 0);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "4"]);
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    let m = g.todos.iter().find(|t| t.id == "mon_due").unwrap();
    assert_eq!(m.consecutive_no_change, 1, "no-change counter: {m:?}");
    assert_ne!(m.status, TodoStatus::Done);
    // A no-change poll never spends quota (other turns may).
    let spent_on_monitor = store
        .events(&goal)
        .unwrap()
        .iter()
        .filter(|se| matches!(&se.event, future_loop::store::Event::QuotaSpent { todo_id, .. } if todo_id == "mon_due"))
        .count();
    assert_eq!(spent_on_monitor, 0);
}

#[test]
fn run_monitor_not_due_waits() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "monitor waiting");
    // Complete the only advancement todo so the (not-due) monitor is all that
    // remains → WaitMonitor → graceful "waiting…" stop.
    let onboarding = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &onboarding,
        "--no-follow-up",
        "--evidence",
        "operator closed the auto-created onboarding todo",
    ]);
    append_monitor(&cr, &goal, "mon_future", 3600);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    assert_eq!(shared.lock().unwrap().prompts, 0, "no turn while waiting");
}

#[cfg(unix)]
#[test]
fn run_validator_pass_completes_todo() {
    let cr = cli_root();
    let (_rt, _shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "validator pass");
    let onboarding = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &onboarding,
        "--no-follow-up",
        "--evidence",
        "operator closed the auto-created onboarding todo",
    ]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "validated real task",
        "--verify",
        "exit 0",
    ]);
    let vt = todo_id_by_text(&cr.root, &goal, "validated real task");
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    let todo = g.todos.iter().find(|t| t.id == vt).unwrap();
    assert_eq!(todo.status, TodoStatus::Done, "validator passed → done");
    let record = g.history.last().unwrap();
    assert!(record.validation.as_ref().unwrap().ok);
}

#[test]
fn run_validator_budget_exhaustion_stops_run() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "validator budget");
    let onboarding = first_todo_id(&cr.root, &goal);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &goal,
        "--todo-id",
        &onboarding,
        "--no-follow-up",
        "--evidence",
        "operator closed the auto-created onboarding todo",
    ]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "never validates",
        "--verify",
        "exit 1",
        "--max-validation-attempts",
        "1",
    ]);
    // Validation budget (1) hit after the first failed validation → stop, Ok.
    // Assert the loop stopped after exactly one turn (the budget branch
    // fired); the ledger keeps the todo open.
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "6"]);
    assert_eq!(
        shared.lock().unwrap().prompts,
        1,
        "validation budget break after one turn"
    );
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    let todo = g
        .todos
        .iter()
        .find(|t| t.text.contains("never validates"))
        .unwrap();
    assert_eq!(todo.status, TodoStatus::Open, "todo stays open: {todo:?}");
}

#[test]
fn run_with_model_and_thinking_flags() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "model flags");
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--model",
        "future/k3",
        "--thinking-level",
        "low",
    ]);
    let recorded = shared.lock().unwrap().recorded.clone();
    assert!(recorded.contains(&"set_model".to_string()));
    assert!(recorded.contains(&"set_thinking_level".to_string()));
}

#[test]
fn run_max_turn_secs_graceful_timeout() {
    let cr = cli_root();
    let st = MockState {
        hang_stream: true,
        ..Default::default()
    };
    let (_rt, _shared) = mock_env(st);
    let goal = init_goal(&cr, "hanging turn");
    // The turn stream never yields; the wall-clock budget stops the run gracefully.
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--max-turn-secs",
        "1",
        "--max-turns",
        "3",
    ]);
}

// ── bidirectional messaging: up-channel turn-boundary reports ────────────────

/// A registered supervisor receives an `enqueue_if_busy` report when a todo
/// completes (up-channel ②, exercised through the real `run` loop).
#[test]
fn run_notifies_supervisor_on_completion() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "notify on complete");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &goal,
        "--session-id",
        "sup-sess",
    ]);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    let calls = shared.lock().unwrap().prompt_calls.clone();
    // Exactly one report to the supervisor (the worker's own turn prompt goes
    // to the worker session, not sup-sess).
    let reports: Vec<_> = calls.iter().filter(|(sid, _)| sid == "sup-sess").collect();
    assert_eq!(reports.len(), 1, "one completion report: {calls:?}");
    assert_eq!(reports[0].1, "enqueue_if_busy");
}

/// A registered supervisor receives a report when a todo fails on a hard
/// error (up-channel ③; infra-recoverable 429/truncation is NOT reported).
#[test]
fn run_notifies_supervisor_on_hard_failure() {
    let cr = cli_root();
    let events = vec![ev(
        "mock-run-1",
        0,
        "agent_end",
        "{\"state\":\"error\",\"error\":\"boom\"}",
    )];
    let (_rt, shared) = mock_env(MockState {
        events,
        ..Default::default()
    });
    let goal = init_goal(&cr, "notify on failure");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &goal,
        "--session-id",
        "sup-sess",
    ]);
    // One failed turn, then max-turns bail (a failed todo stays runnable).
    let _ = cli_err(&["run", "--goal", &goal, "--anonymous", "--max-turns", "1"]);
    let calls = shared.lock().unwrap().prompt_calls.clone();
    let reports: Vec<_> = calls.iter().filter(|(sid, _)| sid == "sup-sess").collect();
    assert_eq!(reports.len(), 1, "one failure report: {calls:?}");
    assert_eq!(reports[0].1, "enqueue_if_busy");
}

/// A registered supervisor receives a report when a worker exhausts its
/// incomplete-retry budget: the stream closes before `agent_end` on every
/// turn (terminal_state=incomplete), and after N consecutive turns the run
/// `break`s — previously silently, leaving the supervisor blind to the stop.
///
/// The mock assigns run ids `mock-run-1`, `mock-run-2`, … on successive
/// prompts, and its replay loop sends every scripted event whose `run_id`
/// matches the current run; events that never match (and no `agent_end`)
/// produce the incomplete state on every turn.
#[test]
fn run_notifies_supervisor_on_incomplete_budget_exhausted() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        // One tool event bound to a run id the mock never serves: every turn
        // sees a stream with no matching events and no agent_end → incomplete.
        events: vec![ev(
            "mock-run-x",
            0,
            "tool_start",
            "{\"tool_name\":\"write\"}",
        )],
        ..Default::default()
    });
    let goal = init_goal(&cr, "notify on incomplete budget");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &goal,
        "--session-id",
        "sup-sess",
    ]);
    // One todo, --max-turns 5, budget 2: turn1 + turn2 incomplete → break at
    // the exhausted budget (turn 2), not at max-turns.
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--max-turns",
        "5",
        "--max-incomplete-retries",
        "2",
    ]);
    let st = shared.lock().unwrap();
    let reports: Vec<_> = st
        .prompt_calls
        .iter()
        .zip(st.prompt_messages.iter())
        .filter(|((sid, _), _)| sid == "sup-sess")
        .collect();
    assert_eq!(
        reports.len(),
        1,
        "one incomplete-budget report: {:?}",
        st.prompt_calls
    );
    assert_eq!(reports[0].0 .1, "enqueue_if_busy");
    assert!(
        reports[0]
            .1
            .contains("stopped before completion (incomplete_budget)"),
        "incomplete-budget stop reported: {}",
        reports[0].1
    );
}

/// A registered supervisor receives a report when the worker dies on a gRPC
/// transport failure (h2 reset / mid-stream non-gap error). This exit never
/// reaches a writeback, so without this report the supervisor would be left
/// polling a worker that already stopped.
#[test]
fn run_notifies_supervisor_on_transport_error() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        // A non-DataLoss mid-stream error: no reconnect, the turn fails.
        stream_attach_plan: vec![AttachPlan::HardErrorAfter(0)],
        ..Default::default()
    });
    let goal = init_goal(&cr, "notify on transport");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &goal,
        "--session-id",
        "sup-sess",
    ]);
    let err = cli_err(&["run", "--goal", &goal, "--anonymous", "--max-turns", "1"]);
    assert!(err.contains("stream error"), "{err}");
    let st = shared.lock().unwrap();
    let reports: Vec<_> = st
        .prompt_calls
        .iter()
        .zip(st.prompt_messages.iter())
        .filter(|((sid, _), _)| sid == "sup-sess")
        .collect();
    assert_eq!(
        reports.len(),
        1,
        "one transport report: {:?}",
        st.prompt_calls
    );
    assert_eq!(reports[0].0 .1, "enqueue_if_busy");
    assert!(
        reports[0]
            .1
            .contains("stopped before completion (transport)"),
        "transport stop reported: {}",
        reports[0].1
    );
}

/// A registered supervisor receives a report when a turn outlives its
/// wall-clock budget (--max-turn-secs). The early return previously skipped
/// the ②/③ reports, leaving the supervisor blind to the stop.
#[test]
fn run_notifies_supervisor_on_timeout() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        hang_stream: true,
        ..Default::default()
    });
    let goal = init_goal(&cr, "notify on timeout");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &goal,
        "--session-id",
        "sup-sess",
    ]);
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--max-turn-secs",
        "1",
        "--max-turns",
        "3",
    ]);
    let st = shared.lock().unwrap();
    let reports: Vec<_> = st
        .prompt_calls
        .iter()
        .zip(st.prompt_messages.iter())
        .filter(|((sid, _), _)| sid == "sup-sess")
        .collect();
    assert_eq!(
        reports.len(),
        1,
        "one timeout report: {:?}",
        st.prompt_calls
    );
    assert_eq!(reports[0].0 .1, "enqueue_if_busy");
    assert!(
        reports[0].1.contains("stopped before completion (timeout)"),
        "timeout stop reported: {}",
        reports[0].1
    );
}

/// A registered supervisor receives a report when a user gate opens
/// (up-channel ①, AskUser turn mode).
#[test]
fn run_notifies_supervisor_on_ask_user() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "notify on gate");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &goal,
        "--session-id",
        "sup-sess",
    ]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &goal,
        "--text",
        "plan gate",
        "--class",
        "user_gate",
        "--gate-question",
        "approve?",
    ]);
    cli_ok(&["run", "--goal", &goal, "--anonymous"]);
    let calls = shared.lock().unwrap().prompt_calls.clone();
    let reports: Vec<_> = calls.iter().filter(|(sid, _)| sid == "sup-sess").collect();
    assert_eq!(reports.len(), 1, "one gate report: {calls:?}");
    assert_eq!(reports[0].1, "enqueue_if_busy");
}

/// Without a registered supervisor, a completed turn produces NO report
/// (the durable user gate remains the authoritative channel).
#[test]
fn run_without_supervisor_sends_no_report() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "no supervisor");
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    let calls = shared.lock().unwrap().prompt_calls.clone();
    // Only the worker's own turn prompt; nothing to sup-sess (no registration).
    assert_eq!(calls.len(), 1, "one worker prompt, zero reports: {calls:?}");
    assert_ne!(calls[0].0, "sup-sess");
}

// ── bidirectional messaging: down-channel steering into the turn envelope ───

/// A `WorkerSteered` instruction folded into `pending_steer` is injected into
/// the next turn's envelope (down-channel, non-interrupt path — the instruction
/// is delivered without re-injecting across turns).
#[test]
fn run_injects_pending_steer_into_turn_envelope() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "steer envelope");
    // Pre-seed a steering instruction targeting this worker (broadcast).
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::WorkerSteered {
            goal_id: goal.clone(),
            agent_id: None,
            instruction: "switch to plan B".to_string(),
            ts: now_epoch(),
        })
        .unwrap();
    drop(store);
    cli_ok(&["run", "--goal", &goal, "--anonymous", "--max-turns", "3"]);
    let messages = shared.lock().unwrap().prompt_messages.clone();
    assert_eq!(messages.len(), 1, "one turn: {messages:?}");
    assert!(
        messages[0].contains("switch to plan B"),
        "steer instruction injected into the turn envelope: {}",
        messages[0]
    );
    assert!(
        messages[0].contains("SUPERVISOR STEERING"),
        "steer header present: {}",
        messages[0]
    );
}

/// A steer targeted at a DIFFERENT worker is NOT injected into this worker's
/// envelope (agent_id scoping).
#[test]
fn run_ignores_steer_targeting_another_worker() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "steer scoping");
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::WorkerSteered {
            goal_id: goal.clone(),
            agent_id: Some("other-worker".to_string()),
            instruction: "not for me".to_string(),
            ts: now_epoch(),
        })
        .unwrap();
    drop(store);
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "this-worker",
        "--max-turns",
        "3",
    ]);
    let messages = shared.lock().unwrap().prompt_messages.clone();
    assert!(
        !messages[0].contains("not for me"),
        "foreign steer must not leak into this worker: {}",
        messages[0]
    );
}

// ── models ─────────────────────────────────────────────────────────────────

#[test]
fn models_text_and_json() {
    let cr = cli_root();
    let (_rt, _shared) = mock_env(MockState::default());
    cli_ok(&["models"]);
    cli_ok(&["models", "--format", "json"]);
    cli_ok(&["models", "--json"]);
    let _ = cr;
}

#[test]
fn models_sparse_payload_defaults() {
    let _cr = cli_root();
    let st = MockState {
        models_payload: Some("{\"models\":[{\"id\":\"bare\"}]}".to_string()),
        ..Default::default()
    };
    let (_rt, _shared) = mock_env(st);
    cli_ok(&["models"]);
}

#[test]
fn models_unreachable_agent_errors() {
    let _cr = cli_root();
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", "127.0.0.1:1");
    let err = cli_err(&["models"]);
    assert!(err.contains("Failed to connect"), "{err}");
}

// ── doctor ─────────────────────────────────────────────────────────────────

#[test]
fn doctor_fresh_root_and_goal_filter() {
    let cr = cli_root();
    cli_ok(&["doctor"]);
    let goal = init_goal(&cr, "doctor goal");
    cli_ok(&["doctor", "--goal", &goal]);
    let err = cli_err(&["doctor", "--goal", "goal_nope"]);
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn doctor_agent_probe() {
    let _cr = cli_root();
    let (_rt, _shared) = mock_env(MockState::default());
    let addr = std::env::var("FUTURE_LOOP_AGENT_ADDR").unwrap();
    cli_ok(&["doctor", "--agent-addr", &addr]);
    let err = cli_err(&["doctor", "--agent-addr", "127.0.0.1:1"]);
    assert!(err.contains("gRPC"), "{err}");
}

// ── cli (error-path sanity through the mock-free surface) ──────────────────

#[test]
fn cli_result_helper_passthrough() {
    let cr = cli_root();
    // `cli` returns Ok for a trivial command; keep the helper itself honest.
    assert!(cli(&["version"]).is_ok());
    let _ = cr;
}

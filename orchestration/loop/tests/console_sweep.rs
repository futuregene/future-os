//! Coverage sweep for console.rs remainder: unknown-flag parse arms across
//! every command, the monitor claim-race re-decide loop, wall-clock-budget
//! success arm, session-cleanup warning, and assorted print arms unreachable
//! through the happy paths.

mod common;

use common::mock_agent::{completed_events, spawn_mock, MockState};
use common::{cli_err, cli_ok, cli_root, first_todo_id, init_goal, open_store};
use future_loop::state::{now_epoch, Todo, TodoStatus};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn mock_env(state: MockState) -> (tokio::runtime::Runtime, common::mock_agent::SharedState) {
    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(state));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    (rt, shared)
}

// ── unknown-flag parse arms (P0-3: strict rejection in every command) ──────

#[test]
fn unknown_flags_hard_error_everywhere() {
    let cr = cli_root();
    let gid = init_goal(&cr, "flag sweep");
    let first = first_todo_id(&cr.root, &gid);
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    // P0-3 strictness: an extra --zz 1 flag must hard-error on every command
    // (pre-P0-3 these were silently swallowed).
    let assert_unknown_flag = |args: &[&str]| {
        let err = cli_err(args);
        assert!(
            err.contains("unknown flag `--zz`"),
            "cli {args:?} should reject --zz, got: {err}"
        );
    };
    let cases: Vec<Vec<&str>> = vec![
        vec!["status", "--goal", &gid, "--zz", "1"],
        vec!["status", "--format", "json", "--zz", "1"],
        vec!["quota", "should-run", "--goal", &gid, "--zz", "1"],
        vec!["quota", "usage", "--goal", &gid, "--zz", "1"],
        vec!["quota", "usage", "--all", "--zz", "1"],
        vec!["quota", "spend", "--goal", &gid, "--zz", "1"],
        vec!["scheduler", "tick", "--goal", &gid, "--zz", "1"],
        vec!["scheduler", "show", "--goal", &gid, "--zz", "1"],
        vec!["store", "verify", "--goal", &gid, "--zz", "1"],
        vec!["store", "bridge", "--goal", &gid, "--zz", "1"],
        vec!["privacy", "--goal", &gid, "--zz", "1"],
        vec![
            "lease",
            "status",
            "--goal",
            &gid,
            "--todo-id",
            &first,
            "--zz",
            "1",
        ],
        vec!["runs", "history", "--goal", &gid, "--zz", "1"],
        vec!["runs", "index", "--goal", &gid, "--zz", "1"],
        vec!["runs", "retention", "--goal", &gid, "--zz", "1"],
        vec!["runs", "stale", "--goal", &gid, "--zz", "1"],
        vec!["heartbeat-prompt", "--goal", &gid, "--zz", "1"],
        vec!["scope", "--goal", &gid, "--agent-id", "w1", "--zz", "1"],
        vec!["lane", "--goal", &gid, "--agent-id", "w1", "--zz", "1"],
        vec!["supervisor", "events", "--goal", &gid, "--zz", "1"],
        vec!["task-graph", "--goal", &gid, "--zz", "1"],
        vec!["attention", "--goal", &gid, "--zz", "1"],
        vec!["inbox", "--project", &cr.cwd, "--zz", "1"],
        vec!["registry", "--zz", "1"],
        vec!["version", "--zz", "1"],
        vec!["canary", "smoke", "--zz", "1"],
        vec!["diagnose", "--goal", &gid, "--zz", "1"],
        vec!["doctor", "--goal", &gid, "--zz", "1"],
        vec!["history", "--goal", &gid, "--zz", "1"],
        vec!["turn", "--goal", &gid, "--todo-id", &first, "--zz", "1"],
        vec![
            "todo-event",
            "--goal",
            &gid,
            "--todo-id",
            &first,
            "--zz",
            "1",
        ],
        vec!["evidence-log", "--goal", &gid, "--zz", "1"],
        vec!["agent", "list", "--goal", &gid, "--zz", "1"],
        vec!["replan", "obligations", "--goal", &gid, "--zz", "1"],
    ];
    for args in &cases {
        assert_unknown_flag(args);
    }
    // Mutating commands with an unknown flag must also fail BEFORE mutating.
    assert_unknown_flag(&[
        "goal",
        "init",
        "--objective",
        "g2",
        "--goal-id",
        "goal_zz",
        "--cwd",
        &cr.cwd,
        "--zz",
        "1",
    ]);
    // Parse rejection precedes execution: goal_zz was never created, so a
    // well-formed cancel now fails with a goal lookup error instead.
    let err = cli_err(&["goal", "cancel", "--goal", "goal_zz"]);
    assert!(
        !err.contains("unknown flag"),
        "goal_zz should not exist after the rejected init, got: {err}"
    );
    assert_unknown_flag(&["goal", "cancel", "--goal", "goal_zz", "--zz", "1"]);
    assert_unknown_flag(&[
        "goal", "delete", "--goal", "goal_zz", "--force", "--zz", "1",
    ]);
    assert_unknown_flag(&[
        "todo", "add", "--goal", &gid, "--text", "zz flag", "--zz", "1",
    ]);
    assert_unknown_flag(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--agent-id",
        "w1",
        "--zz",
        "1",
    ]);
    assert_unknown_flag(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--no-follow-up",
        "--evidence",
        "fixture evidence for completion contract",
        "--zz",
        "1",
    ]);
    assert_unknown_flag(&[
        "todo",
        "archive",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--zz",
        "1",
    ]);
    assert_unknown_flag(&[
        "todo",
        "supersede",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--zz",
        "1",
    ]);
    assert_unknown_flag(&[
        "gate",
        "resolve",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--decision",
        "d",
        "--zz",
        "1",
    ]);
    assert_unknown_flag(&["backup", "--goal", &gid, "--zz", "1"]);
    assert_unknown_flag(&[
        "authority",
        "--goal",
        &gid,
        "--write-scope",
        "src",
        "--zz",
        "1",
    ]);
    assert_unknown_flag(&[
        "profile",
        "set",
        "--goal",
        &gid,
        "--outcome-floor",
        "2",
        "--zz",
        "1",
    ]);
    assert_unknown_flag(&[
        "replan",
        "ack",
        "--goal",
        &gid,
        "--delta-kind",
        "vision_patch",
        "--zz",
        "1",
    ]);
    assert_unknown_flag(&[
        "agent",
        "onboard",
        "--goal",
        &gid,
        "--agent-id",
        "w9",
        "--zz",
        "1",
    ]);
    assert_unknown_flag(&[
        "lease",
        "expire",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--zz",
        "1",
    ]);
    assert_unknown_flag(&[
        "supervisor",
        "propose",
        "--goal",
        &gid,
        "--agent-id",
        "s",
        "--decision-id",
        "dz",
        "--target-agent-id",
        "w1",
        "--kind",
        "execute",
        "--capabilities",
        "shell",
        "--zz",
        "1",
    ]);
    assert_unknown_flag(&[
        "supervisor",
        "receipt",
        "--goal",
        &gid,
        "--decision-id",
        "dz",
        "--receipt-id",
        "rz",
        "--adapter-id",
        "a",
        "--outcome",
        "executed",
        "--host-capabilities",
        "shell",
        "--authority-ref",
        "auth-1",
        "--zz",
        "1",
    ]);
    let md = std::path::Path::new(&cr.cwd).join("bf.md");
    std::fs::write(&md, "## Agent Todo\n\n- [ ] Task one\n").unwrap();
    assert_unknown_flag(&[
        "backfill",
        "--goal",
        &gid,
        "--from",
        md.to_str().unwrap(),
        "--dry-run",
        "--zz",
        "1",
    ]);
}

// ── monitor claim-race re-decide loop ──────────────────────────────────────

#[test]
fn run_monitor_claim_race_stops_without_selection() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "claim race");
    // A due monitor with a LIVE lease held by another agent — the lease must
    // exist IN THE LEDGER (try_claim_todo reconstructs from events, not from
    // the replayed struct).
    {
        let mut store = open_store(&cr);
        let mon = Todo::monitor(
            "mon_raced",
            "raced monitor",
            std::time::Duration::from_secs(0),
        );
        store
            .append(future_loop::store::Event::TodoAdded {
                goal_id: goal.clone(),
                todo: mon,
                ts: now_epoch(),
            })
            .unwrap();
        store
            .append(future_loop::store::Event::TodoClaimed {
                goal_id: goal.clone(),
                todo_id: "mon_raced".into(),
                agent_id: "other-agent".into(),
                holder_pid: None,
                lease_expires_at: now_epoch() + 3600,
                ts: now_epoch(),
            })
            .unwrap();
        store.set_next_action(&goal, "raced monitor").unwrap();
    }
    // Turn 1 completes the onboarding todo; turn 2 selects the due monitor,
    // loses the atomic claim 3×, and stops without executing it.
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--agent-id",
        "racer",
        "--max-turns",
        "5",
    ]);
    assert_eq!(
        shared.lock().unwrap().prompts,
        1,
        "only the onboarding turn ran"
    );
    let store = open_store(&cr);
    let g = store.replay(&goal).unwrap().unwrap();
    let m = g.todos.iter().find(|t| t.id == "mon_raced").unwrap();
    assert_ne!(m.status, TodoStatus::Done, "raced monitor never executed");
}

// ── run: wall-clock budget success arm + session cleanup warning ───────────

#[test]
fn run_with_time_budget_success_path() {
    let cr = cli_root();
    let (_rt, _shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "budget success");
    cli_ok(&[
        "run",
        "--goal",
        &goal,
        "--anonymous",
        "--max-turn-secs",
        "600",
        "--max-turns",
        "3",
    ]);
}

#[test]
fn run_retains_session() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let goal = init_goal(&cr, "retain session");
    cli_ok(&["run", "--goal", &goal, "--anonymous"]);
    // The agent session is NOT deleted when the run ends.
    assert!(
        shared
            .lock()
            .unwrap()
            .live_sessions
            .contains("mock-session-1"),
        "session must be retained after a completed run"
    );
}

// ── status print arms ──────────────────────────────────────────────────────

#[test]
fn status_closure_and_gap_arms() {
    let cr = cli_root();
    // All todos done with closure intent → closure_proof "valid".
    let gid = init_goal(&cr, "closure valid");
    let first = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--no-follow-up",
        "--evidence",
        "fixture evidence for completion contract",
    ]);
    cli_ok(&["status", "--goal", &gid]);
    // Projection gap → the ⚠ line.
    {
        let store = open_store(&cr);
        store.set_next_action(&gid, "phantom work").unwrap();
    }
    cli_ok(&["status", "--goal", &gid]);
    // quota usage --all with a registry that replays nothing (ghost entry).
    let cr2_dir;
    {
        // A registry entry pointing at a deleted goal dir: replay fails → skipped.
        let mut store = open_store(&cr);
        store
            .register(&future_loop::state::Goal::new("goal_ghost", "x", "/tmp"))
            .unwrap();
        cr2_dir = store.goal_dir("goal_ghost");
        let _ = cr2_dir;
    }
    cli_ok(&["quota", "usage", "--all"]);
}

// ── scheduler: no-progression (single execution) arm ───────────────────────

#[test]
fn scheduler_tick_single_execution_arm() {
    let cr = cli_root();
    let gid = init_goal(&cr, "sched single");
    {
        use future_loop::scheduler::state as st;
        let store = open_store(&cr);
        let state = st::build_scheduler_state(
            &gid,
            "codex-app",
            st::CODEX_APP_SURFACE,
            st::CODEX_APP_STATEFUL_BACKOFF_STATE_KEY,
            "tok",
            "id",
            0,
            vec![], // single execution: no progression
            "FREQ=MINUTELY;INTERVAL=15",
            now_epoch(),
            vec![],
        )
        .unwrap();
        st::write_scheduler_state(&store.goal_dir(&gid), &state).unwrap();
    }
    cli_ok(&["scheduler", "tick", "--goal", &gid]);
}

// ── runs index duplicate-group print arms ──────────────────────────────────

#[test]
fn runs_index_duplicate_report() {
    let cr = cli_root();
    let gid = init_goal(&cr, "dup index");
    {
        let store = open_store(&cr);
        let runs_dir = store.goal_dir(&gid).join("runs");
        std::fs::create_dir_all(&runs_dir).unwrap();
        std::fs::write(
            runs_dir.join("index.jsonl"),
            concat!(
                "{\"goal_id\":\"G\",\"timestamp\":\"2026-08-10T00:00:00+00:00\",\"path\":\"a.json\",\"classification\":\"work\"}\n",
                "{\"goal_id\":\"G\",\"timestamp\":\"2026-08-10T00:00:00+00:00\",\"path\":\"a.json\",\"classification\":\"work\"}\n",
            ),
        )
        .unwrap();
    }
    cli_ok(&["runs", "index", "--goal", &gid]);
}

// ── down-channel steering watcher seam ─────────────────────────────────────

#[test]
fn steer_worker_poll_once_aborts_on_targeted_and_broadcast_steer() {
    let cr = cli_root();
    let gid = init_goal(&cr, "steer worker");
    let events_path = open_store(&cr).goal_dir(&gid).join("events.jsonl");
    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    rt.block_on(async {
        // Missing file → offset unchanged, no client, no abort.
        let mut client = None;
        let off = future_loop::console::steer_worker_poll_once(
            std::path::Path::new("/nonexistent/events.jsonl"),
            0,
            Some("worker-a"),
            &mut client,
            "sess",
        )
        .await;
        assert_eq!(off, 0);
        assert!(client.is_none());

        // Broadcast steer (no agent_id) targets every worker.
        std::fs::write(
            &events_path,
            "{\"kind\":\"worker_steered\",\"instruction\":\"do X\",\"ts\":1}\n",
        )
        .unwrap();
        let mut client = None;
        let off = future_loop::console::steer_worker_poll_once(
            &events_path,
            0,
            Some("worker-a"),
            &mut client,
            "sess",
        )
        .await;
        assert!(off > 0);
        assert!(shared.lock().unwrap().recorded.contains(&"abort".to_string()));

        // A steer targeting a DIFFERENT worker does NOT abort this one.
        let n = shared.lock().unwrap().recorded.len();
        std::fs::write(
            &events_path,
            "{\"kind\":\"worker_steered\",\"agent_id\":\"worker-other\",\"instruction\":\"x\",\"ts\":2}\n",
        )
        .unwrap();
        let mut client = None;
        let _ = future_loop::console::steer_worker_poll_once(
            &events_path,
            0,
            Some("worker-a"),
            &mut client,
            "sess",
        )
        .await;
        assert_eq!(shared.lock().unwrap().recorded.len(), n, "foreign steer must not abort");

        // A targeted steer for THIS worker aborts.
        std::fs::write(
            &events_path,
            "{\"kind\":\"worker_steered\",\"agent_id\":\"worker-a\",\"instruction\":\"y\",\"ts\":3}\n",
        )
        .unwrap();
        let mut client = None;
        let _ = future_loop::console::steer_worker_poll_once(
            &events_path,
            0,
            Some("worker-a"),
            &mut client,
            "sess",
        )
        .await;
        assert!(shared.lock().unwrap().recorded.len() > n, "targeted steer must abort");

        // A non-steer event is ignored (no abort).
        let m = shared.lock().unwrap().recorded.len();
        std::fs::write(&events_path, "{\"kind\":\"todo_updated\",\"todo_id\":\"t\",\"ts\":4}\n").unwrap();
        let mut client = None;
        let _ = future_loop::console::steer_worker_poll_once(
            &events_path,
            0,
            Some("worker-a"),
            &mut client,
            "sess",
        )
        .await;
        assert_eq!(shared.lock().unwrap().recorded.len(), m, "non-steer must not abort");
    });
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

// ── up-channel notify seam ────────────────────────────────────────────────

#[test]
fn notify_supervisor_enqueues_to_registered_session_only() {
    // Hold the CLI lock so FUTURE_LOOP_AGENT_ADDR set/remove cannot race a
    // concurrent in-process run() driving the same global env.
    let _cr = cli_root();
    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    rt.block_on(async {
        let mut client = future_loop::agent_client::AgentClient::connect(
            &future_loop::agent_client::agent_addr(),
        )
        .await
        .unwrap();

        // No supervisor registered → dropped, no prompt on the wire.
        future_loop::console::notify_supervisor(&mut client, None, "hello", "k1").await;
        assert!(shared.lock().unwrap().prompt_calls.is_empty());

        // Registered → enqueued to that session with enqueue_if_busy.
        future_loop::console::notify_supervisor(&mut client, Some("sup-sess"), "hello", "k2").await;
        let calls = shared.lock().unwrap().prompt_calls.clone();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "sup-sess");
        assert_eq!(calls[0].1, "enqueue_if_busy");
    });
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

#[test]
fn steer_worker_poll_once_resets_client_on_abort_failure() {
    let cr = cli_root();
    let gid = init_goal(&cr, "steer abort fail");
    let events_path = open_store(&cr).goal_dir(&gid).join("events.jsonl");
    let rt = rt();
    // Abort answers success=false → the watcher resets its client and retries
    // on the next event.
    let st = MockState {
        fail_commands: ["abort".to_string()].into_iter().collect(),
        ..Default::default()
    };
    let (addr, _shared) = rt.block_on(spawn_mock(st));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    rt.block_on(async {
        std::fs::write(
            &events_path,
            "{\"kind\":\"worker_steered\",\"instruction\":\"x\",\"ts\":1}\n",
        )
        .unwrap();
        let mut client = None;
        let _ = future_loop::console::steer_worker_poll_once(
            &events_path,
            0,
            Some("worker-a"),
            &mut client,
            "sess",
        )
        .await;
        assert!(client.is_none(), "failed abort resets the client");
    });
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

#[test]
fn steer_worker_poll_once_read_errors_and_bad_lines() {
    let cr = cli_root();
    let gid = init_goal(&cr, "steer read errors");
    let events_path = open_store(&cr).goal_dir(&gid).join("events.jsonl");
    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    rt.block_on(async {
        // Non-UTF8 content → read_to_string fails → offset unchanged.
        std::fs::write(&events_path, [0xffu8, 0xfe, 0xfd]).unwrap();
        let mut client = None;
        let off = future_loop::console::steer_worker_poll_once(
            &events_path,
            0,
            Some("worker-a"),
            &mut client,
            "sess",
        )
        .await;
        assert_eq!(off, 0);

        // Bad JSON line is skipped (no abort); a valid steer after it fires.
        std::fs::write(
            &events_path,
            "{broken\n{\"kind\":\"worker_steered\",\"instruction\":\"x\",\"ts\":1}\n",
        )
        .unwrap();
        let mut client = None;
        let _ = future_loop::console::steer_worker_poll_once(
            &events_path,
            0,
            Some("worker-a"),
            &mut client,
            "sess",
        )
        .await;
        assert!(
            shared
                .lock()
                .unwrap()
                .recorded
                .contains(&"abort".to_string()),
            "the valid steer line must abort"
        );
    });
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

#[test]
fn notify_supervisor_survives_prompt_failure() {
    let _cr = cli_root();
    let rt = rt();
    let st = MockState {
        fail_commands: ["prompt".to_string()].into_iter().collect(),
        ..Default::default()
    };
    let (addr, _shared) = rt.block_on(spawn_mock(st));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    rt.block_on(async {
        let mut client = future_loop::agent_client::AgentClient::connect(
            &future_loop::agent_client::agent_addr(),
        )
        .await
        .unwrap();
        // A failing enqueue is best-effort: the report is dropped, no panic.
        future_loop::console::notify_supervisor(&mut client, Some("sup-sess"), "hi", "k").await;
    });
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

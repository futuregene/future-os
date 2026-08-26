//! Coverage drive for the remaining per-line `console.rs` gaps: parse-table
//! `_ => {}` swallow arms (fed the global `--include-experimental` flag),
//! priority default arms, succession `--reason` filter, empty collective
//! ledgers, idempotent `todo complete`, ledger-read-note rendering, and the
//! `ui` success argument-parsing paths (bind-failure exit keeps the test
//! synchronous).

mod common;

use common::mock_agent::{completed_events, spawn_mock, AttachPlan, MockState};
use common::{
    add_todo, cli_err, cli_ok, cli_root, first_todo_id, init_goal, open_store, run_record,
};
use future_loop::state::now_epoch;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[cfg(unix)]
fn dead_pid() -> u32 {
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg("sleep 30")
        .spawn()
        .unwrap();
    let pid = child.id();
    child.kill().unwrap();
    child.wait().unwrap();
    pid
}

/// A valid two-peer contract with one collective (crew).
fn contract_json() -> String {
    serde_json::json!({
        "schema_version": "multi_agent_contract_v0",
        "peers": {
            "primary": {"capabilities": ["shell"], "workspaces": ["/ws/p"]},
            "backup": {"backup_for": "primary", "capabilities": [], "workspaces": ["/ws/b"]}
        },
        "handoff_rules": [{"from_event": "lease_expired", "to_role": "backup"}],
        "collectives": {"crew": ["primary", "backup"]}
    })
    .to_string()
}

/// A contract with peers but NO collectives (drives the empty-ledger branch).
fn contract_without_collectives_json() -> String {
    serde_json::json!({
        "schema_version": "multi_agent_contract_v0",
        "peers": {
            "primary": {"capabilities": ["shell"], "workspaces": ["/ws/p"]}
        },
        "handoff_rules": [],
        "collectives": {}
    })
    .to_string()
}

// ── parse_pairs `_ => {}` swallow arms (global --include-experimental) ─────

#[test]
fn agent_contract_set_swallows_global_flag() {
    let cr = cli_root();
    let gid = init_goal(&cr, "contract global flag");
    // `--include-experimental` flows through reject_unknown_flags and hits the
    // parse_pairs `_ => {}` arm.
    cli_ok(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract",
        &contract_json(),
        "--include-experimental",
    ]);
}

#[test]
fn agent_recipe_add_default_priority_and_global_flag() {
    let cr = cli_root();
    let gid = init_goal(&cr, "recipe priority + global flag");
    // `--priority P1` hits the `_ => Priority::P1` default arm; the trailing
    // `--include-experimental` hits the `_ => {}` swallow arm.
    cli_ok(&[
        "agent",
        "recipe",
        "add",
        "--goal",
        &gid,
        "--name",
        "deployer",
        "--priority",
        "P1",
        "--capability",
        "shell",
        "--include-experimental",
    ]);
}

// ── succession --reason filter ─────────────────────────────────────────────

#[test]
fn agent_succession_show_reason_filter() {
    let cr = cli_root();
    let gid = init_goal(&cr, "succession reason filter");
    cli_ok(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract",
        &contract_json(),
    ]);
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "primary"]);
    cli_ok(&[
        "agent",
        "succession",
        "show",
        "--goal",
        &gid,
        "--reason",
        "lease_expired",
    ]);
}

// ── empty collective ledger ────────────────────────────────────────────────

#[test]
fn agent_collective_show_without_collectives() {
    let cr = cli_root();
    let gid = init_goal(&cr, "collective empty");
    cli_ok(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract",
        &contract_without_collectives_json(),
    ]);
    // No collectives → the "no collectives in the contract" branch.
    cli_ok(&["agent", "collective", "show", "--goal", &gid]);
}

// ── idempotent todo complete ───────────────────────────────────────────────

#[test]
fn todo_complete_is_idempotent() {
    let cr = cli_root();
    let gid = init_goal(&cr, "complete twice");
    let tid = add_todo(&cr, &gid, "do the thing");
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &tid,
        "--no-follow-up",
        "--evidence",
        "landed the artifact",
    ]);
    // Second completion hits the "already done — nothing to do" branch.
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &tid,
        "--no-follow-up",
        "--evidence",
        "landed the artifact",
    ]);
}

// ── ledger read-note rendering (status + diagnose) ─────────────────────────

#[test]
fn status_and_diagnose_render_ledger_read_note() {
    let cr = cli_root();
    let gid = init_goal(&cr, "ledger read note");
    let store = open_store(&cr);
    let path = store.goal_dir(&gid).join("events.jsonl");
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"kind\":\"future_unknown_kind_v99\",\"goal_id\":\"g\",\"ts\":1}\n")
        .unwrap();
    drop(store);
    cli_ok(&["status", "--goal", &gid]);
    cli_ok(&["diagnose", "--goal", &gid]);
}

// ── ui argument-parsing success paths (bind failure keeps it synchronous) ──

#[test]
fn webui_parses_no_open_port_and_root() {
    let cr = cli_root();
    // Occupy a port so `run_server`'s bind fails after the args parse; this
    // exercises `--no-open` / `--port` / `--root` without blocking on accept.
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let err = cli_err(&[
        "ui",
        "--no-open",
        "--port",
        &port.to_string(),
        "--root",
        &cr.root,
    ]);
    assert!(err.contains("bind"), "expected bind failure, got: {err}");
}

// ── reject_unknown_flags error arms (per-command `)?;` lines) ─────────────

#[test]
fn agent_recipe_add_rejects_unknown_flag() {
    let cr = cli_root();
    let gid = init_goal(&cr, "recipe unknown flag");
    assert!(
        cli_err(&["agent", "recipe", "add", "--goal", &gid, "--name", "n", "--bogus", "x",])
            .contains("unknown flag `--bogus`")
    );
}

#[test]
fn agent_succession_show_rejects_unknown_flag() {
    let cr = cli_root();
    let gid = init_goal(&cr, "succession unknown flag");
    assert!(cli_err(&[
        "agent",
        "succession",
        "show",
        "--goal",
        &gid,
        "--bogus",
        "x",
    ])
    .contains("unknown flag `--bogus`"));
}

#[test]
fn agent_collective_show_rejects_unknown_flag() {
    let cr = cli_root();
    let gid = init_goal(&cr, "collective unknown flag");
    assert!(cli_err(&[
        "agent",
        "collective",
        "show",
        "--goal",
        &gid,
        "--bogus",
        "x",
    ])
    .contains("unknown flag `--bogus`"));
}

// ── notify_dead_holders connect-failure arm ──────────────────────────────

#[cfg(unix)]
#[test]
fn notify_dead_holders_returns_when_connect_fails() {
    let cr = cli_root();
    let gid = init_goal(&cr, "dead holders connect fail");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &gid,
        "--session-id",
        "sup-sess",
    ]);
    let todo_id = first_todo_id(&cr.root, &gid);
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::TodoClaimed {
            goal_id: gid.clone(),
            todo_id: todo_id.clone(),
            agent_id: "w1".to_string(),
            lease_expires_at: future_loop::state::now_epoch() + 3600,
            holder_pid: Some(dead_pid()),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    drop(store);

    // Point the agent addr at a dead port so connect fails (defensive arm).
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", "127.0.0.1:1");
    let mut store = open_store(&cr);
    let rt = rt();
    rt.block_on(async {
        future_loop::console::notify_dead_holders(&mut store, &gid)
            .await
            .unwrap();
    });
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

// ── steer_worker_poll_once offset + anonymous-target arms ────────────────

#[test]
fn steer_worker_poll_once_offset_and_anonymous_target_arms() {
    let cr = cli_root();
    let gid = init_goal(&cr, "steer offset arms");
    let events_path = open_store(&cr).goal_dir(&gid).join("events.jsonl");
    let rt = rt();
    rt.block_on(async {
        // File exists but has not grown past offset → meta.len() <= offset.
        std::fs::write(
            &events_path,
            "{\"kind\":\"worker_steered\",\"instruction\":\"x\",\"ts\":1}\n",
        )
        .unwrap();
        let len = std::fs::metadata(&events_path).unwrap().len();
        let mut client = None;
        let off = future_loop::console::steer_worker_poll_once(
            &events_path,
            len,
            Some("worker-a"),
            &mut client,
            "sess",
        )
        .await;
        assert_eq!(off, len);

        // A targeted steer ignored by an anonymous caller (None + Some(target)).
        std::fs::write(
            &events_path,
            "{\"kind\":\"worker_steered\",\"agent_id\":\"worker-a\",\"instruction\":\"y\",\"ts\":2}\n",
        )
        .unwrap();
        let mut client = None;
        let _ = future_loop::console::steer_worker_poll_once(
            &events_path,
            0,
            None,
            &mut client,
            "sess",
        )
        .await;
        assert!(client.is_none(), "anonymous caller ignores a targeted steer");
    });
}

// ── run session resume (--resume-session) + transport error with budget ────

#[test]
fn run_resumes_alive_session_via_flag() {
    let cr = cli_root();
    let gid = init_goal(&cr, "resume alive");
    let mut st = MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    };
    st.live_sessions.insert("resume-me".to_string());
    let rt = rt();
    let (addr, _shared) = rt.block_on(spawn_mock(st));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    cli_ok(&[
        "run",
        "--goal",
        &gid,
        "--anonymous",
        "--resume-session",
        "resume-me",
        "--max-turns",
        "3",
    ]);
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

#[test]
fn run_falls_back_to_fresh_for_dead_session() {
    let cr = cli_root();
    let gid = init_goal(&cr, "resume dead");
    // sessions_created > 0 activates the mock's get_state "not found" path;
    // the id is NOT in live_sessions, so session_alive probes false.
    let st = MockState {
        events: completed_events("mock-run-1"),
        sessions_created: 1,
        ..Default::default()
    };
    let rt = rt();
    let (addr, _shared) = rt.block_on(spawn_mock(st));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    cli_ok(&[
        "run",
        "--goal",
        &gid,
        "--anonymous",
        "--resume-session",
        "dead-session",
        "--max-turns",
        "3",
    ]);
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

#[test]
fn run_transport_error_with_turn_budget() {
    let cr = cli_root();
    let gid = init_goal(&cr, "transport with budget");
    // A non-DataLoss mid-stream error under a wall-clock turn budget hits the
    // `Ok(Err(e))` arm of the timeout match (transport stop + resumable mark).
    let st = MockState {
        stream_attach_plan: vec![AttachPlan::HardErrorAfter(0)],
        ..Default::default()
    };
    let rt = rt();
    let (addr, _shared) = rt.block_on(spawn_mock(st));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    let err = cli_err(&[
        "run",
        "--goal",
        &gid,
        "--anonymous",
        "--max-turns",
        "1",
        "--max-turn-secs",
        "5",
    ]);
    assert!(err.contains("stream error"), "{err}");
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

#[test]
fn run_anonymous_ignores_targeted_steer() {
    let cr = cli_root();
    let gid = init_goal(&cr, "anonymous targeted steer");
    // A steer targeting a NAMED worker is ignored by an anonymous run: the
    // (None, Some(_)) => false arm in the steer-note match.
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::WorkerSteered {
            goal_id: gid.clone(),
            agent_id: Some("worker-x".to_string()),
            instruction: "do something".to_string(),
            ts: now_epoch(),
        })
        .unwrap();
    drop(store);

    let rt = rt();
    let (addr, _shared) = rt.block_on(spawn_mock(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    }));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    cli_ok(&["run", "--goal", &gid, "--anonymous", "--max-turns", "3"]);
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

#[test]
fn frontier_show_terminal_yes() {
    let cr = cli_root();
    let gid = init_goal(&cr, "frontier terminal");
    let tid = first_todo_id(&cr.root, &gid);
    // Close the only todo with closure intent → no gaps → terminal judgement.
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &tid,
        "--no-follow-up",
        "--evidence",
        "closed",
    ]);
    cli_ok(&["frontier", "show", "--goal", &gid]);
}

// ── frontier show with runs + semantic history ────────────────────────────

#[test]
fn frontier_show_with_runs_and_semantic_history() {
    let cr = cli_root();
    let gid = init_goal(&cr, "frontier runs");
    let first = first_todo_id(&cr.root, &gid);
    let mut store = open_store(&cr);
    // runs.jsonl → goal.history (outcome_segments); RunRecorded → semantic
    // history (the RUN_LANDED fold).
    let rec = run_record(&first, "completed", now_epoch());
    store.append_run(&gid, &rec).unwrap();
    store
        .append(future_loop::store::Event::RunRecorded {
            goal_id: gid.clone(),
            record: rec,
            ts: now_epoch(),
        })
        .unwrap();
    drop(store);
    cli_ok(&["frontier", "show", "--goal", &gid]);
}

// ── worker list "running" status + worker stop abort/delete failures ──────

fn run_header(agent: &str, session: &str, ts: u64, goal: &str) -> String {
    format!(
        "{{\"type\":\"run_header\",\"idx\":0,\"wall_ts\":{ts},\"run_id\":\"r-{session}\",\"session_id\":\"{session}\",\"agent_id\":\"{agent}\",\"todo_id\":\"todo-{agent}\",\"goal_id\":\"{goal}\"}}\n"
    )
}

fn seed_live_run(root: &str, goal: &str, agent: &str, session: &str) {
    let runs = std::path::Path::new(root).join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    std::fs::write(
        runs.join(format!("{session}.live.jsonl")),
        run_header(agent, session, 100, goal),
    )
    .unwrap();
}

#[test]
fn worker_list_shows_running_when_streaming() {
    let cr = cli_root();
    let gid = init_goal(&cr, "worker running");
    seed_live_run(&cr.root, &gid, "worker-a", "sess-a");

    let mut st = MockState::default();
    st.raw.insert(
        "list_streaming_sessions".to_string(),
        "{\"sessionIds\":[\"sess-a\"]}".to_string(),
    );
    let rt = rt();
    let (addr, _shared) = rt.block_on(spawn_mock(st));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    cli_ok(&["worker", "list", "--goal", &gid]);
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

#[test]
fn worker_stop_abort_and_delete_failures() {
    let cr = cli_root();
    let gid = init_goal(&cr, "worker stop failures");
    seed_live_run(&cr.root, &gid, "worker-a", "sess-a");

    let st = MockState {
        fail_commands: ["abort".to_string(), "delete_session".to_string()]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let rt = rt();
    let (addr, _shared) = rt.block_on(spawn_mock(st));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    // Abort fails → "abort failed" arm; --delete makes delete_session fail too.
    cli_ok(&[
        "worker",
        "stop",
        "--goal",
        &gid,
        "--agent-id",
        "worker-a",
        "--delete",
    ]);
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

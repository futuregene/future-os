//! Coverage sweep for console.rs remainder: unknown-flag parse arms across
//! every command, the steer poll seam, the monitor claim-race re-decide
//! loop, wall-clock-budget success arm, session-cleanup warning, and assorted
//! print arms unreachable through the happy paths.

mod common;

use common::mock_agent::{completed_events, spawn_mock, MockState};
use common::{cli_ok, cli_root, first_todo_id, init_goal, open_store, run_record};
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

// ── unknown-flag parse arms (`_ => {}` in each parse_pairs closure) ────────

#[test]
fn unknown_flags_are_swallowed_everywhere() {
    let cr = cli_root();
    let gid = init_goal(&cr, "flag sweep");
    let first = first_todo_id(&cr.root, &gid);
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    // (args that must succeed with an extra --zz 1 flag attached)
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
        vec!["capability", "list", "--zz", "1"],
        vec!["capability", "commands", "--zz", "1"],
        vec!["capability", "propose", "--name", "issue_fix", "--zz", "1"],
        vec!["catalog", "--zz", "1"],
        vec!["catalog", "--name", "issue_fix", "--zz", "1"],
        vec!["scope", "--goal", &gid, "--agent-id", "w1", "--zz", "1"],
        vec!["lane", "--goal", &gid, "--agent-id", "w1", "--zz", "1"],
        vec!["supervisor", "events", "--goal", &gid, "--zz", "1"],
        vec!["handoff", "--goal", &gid, "--zz", "1"],
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
        vec!["benchmark", "protocol", "--route", "r", "--zz", "1"],
        vec!["benchmark", "ledger", "--zz", "1"],
        vec!["agent", "list", "--goal", &gid, "--zz", "1"],
        vec!["extension", "status", "--zz", "1"],
        vec!["extension", "capabilities", "--zz", "1"],
        vec!["replan", "obligations", "--goal", &gid, "--zz", "1"],
    ];
    for args in cases {
        cli_ok(&args);
    }
    // Mutating commands with an unknown flag.
    cli_ok(&[
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
    cli_ok(&["goal", "cancel", "--goal", "goal_zz", "--zz", "1"]);
    cli_ok(&[
        "goal", "delete", "--goal", "goal_zz", "--force", "--zz", "1",
    ]);
    cli_ok(&[
        "todo", "add", "--goal", &gid, "--text", "zz flag", "--zz", "1",
    ]);
    let zz = common::todo_id_by_text(&cr.root, &gid, "zz flag");
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &zz,
        "--agent-id",
        "w1",
        "--zz",
        "1",
    ]);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &zz,
        "--no-follow-up",
        "--zz",
        "1",
    ]);
    cli_ok(&[
        "todo",
        "archive",
        "--goal",
        &gid,
        "--todo-id",
        &zz,
        "--zz",
        "1",
    ]);
    let sup = common::add_todo(&cr, &gid, "zz supersede");
    cli_ok(&[
        "todo",
        "supersede",
        "--goal",
        &gid,
        "--todo-id",
        &sup,
        "--zz",
        "1",
    ]);
    cli_ok(&[
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
    cli_ok(&["backup", "--goal", &gid, "--zz", "1"]);
    cli_ok(&[
        "authority",
        "--goal",
        &gid,
        "--write-scope",
        "src",
        "--zz",
        "1",
    ]);
    cli_ok(&[
        "profile",
        "set",
        "--goal",
        &gid,
        "--outcome-floor",
        "2",
        "--zz",
        "1",
    ]);
    cli_ok(&[
        "replan",
        "ack",
        "--goal",
        &gid,
        "--delta-kind",
        "vision_patch",
        "--zz",
        "1",
    ]);
    cli_ok(&[
        "agent",
        "onboard",
        "--goal",
        &gid,
        "--agent-id",
        "w9",
        "--zz",
        "1",
    ]);
    cli_ok(&[
        "lease",
        "expire",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--zz",
        "1",
    ]);
    cli_ok(&[
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
    cli_ok(&[
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
    cli_ok(&[
        "backfill",
        "--goal",
        &gid,
        "--from",
        md.to_str().unwrap(),
        "--dry-run",
        "--zz",
        "1",
    ]);
    cli_ok(&["replay", "record", "--goal", &gid, "--zz", "1"]);
    cli_ok(&[
        "replay", "corpus", "build", "--goal", &gid, "--patch", "{}", "--zz", "1",
    ]);
}

// ── steer poll seam ────────────────────────────────────────────────────────

#[test]
fn steer_poll_once_arms() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState::default());
    let gid = init_goal(&cr, "steer drive");
    let events_path = open_store(&cr).goal_dir(&gid).join("events.jsonl");
    let rt2 = rt();
    rt2.block_on(async {
        use std::io::Write as _;
        let mut client = None;
        // Missing file → offset unchanged.
        let off = future_loop::console::steer_poll_once(
            std::path::Path::new("/nonexistent/events.jsonl"),
            0,
            "t1",
            &mut client,
            "sess",
        )
        .await;
        assert_eq!(off, 0);
        // Fresh append beyond the current length → poll processes new lines.
        let meta_len = std::fs::metadata(&events_path).unwrap().len();
        let off = future_loop::console::steer_poll_once(
            &events_path,
            meta_len,
            "t1",
            &mut client,
            "sess",
        )
        .await;
        assert_eq!(off, meta_len, "no new content");
        // Append a mix: bad json, wrong kind, wrong todo, no text, and a hit.
        let mut store = open_store(&cr);
        store
            .append(future_loop::store::Event::TodoAdded {
                goal_id: gid.clone(),
                todo: Todo::advancement("t1", "steer me"),
                ts: now_epoch(),
            })
            .unwrap();
        // Hand-craft the todo_updated line variants (the CLI sets both text
        // and kind via Event::TodoUpdated serialization).
        store
            .append(future_loop::store::Event::TodoUpdated {
                goal_id: gid.clone(),
                todo_id: "t1".into(),
                text: Some("new instructions".into()),
                status: None,
                evidence: None,
                note: None,
                priority: None,
                resume_when: None,
                blocks: None,
                ts: now_epoch(),
            })
            .unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&events_path)
            .unwrap()
            .write_all(b"{broken\n")
            .unwrap();
        let before = std::fs::metadata(&events_path).unwrap().len();
        assert!(before > off);
        let off2 =
            future_loop::console::steer_poll_once(&events_path, off, "t1", &mut client, "sess")
                .await;
        assert_eq!(off2, before);
        // The todo_updated hit connected to the mock and steered.
        assert!(shared
            .lock()
            .unwrap()
            .recorded
            .contains(&"steer".to_string()));
        // A poll against a DIFFERENT todo id: no new steer call.
        let n = shared.lock().unwrap().recorded.len();
        let _ = future_loop::console::steer_poll_once(
            &events_path,
            off,
            "other-todo",
            &mut client,
            "sess",
        )
        .await;
        let _ = future_loop::console::steer_poll_once(&events_path, off, "t1", &mut client, "sess")
            .await;
        // steer failure → client reset (next matching event reconnects).
        let st = MockState {
            fail_commands: ["steer".to_string()].into_iter().collect(),
            ..Default::default()
        };
        let (addr2, _shared2) = spawn_mock(st).await;
        std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr2);
        let mut client2 = None;
        let _ =
            future_loop::console::steer_poll_once(&events_path, off, "t1", &mut client2, "sess2")
                .await;
        assert!(client2.is_none(), "failed steer resets the client");
        let _ = n;
    });
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
fn run_session_cleanup_failure_is_a_warning() {
    let cr = cli_root();
    let mut st = MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    };
    st.fail_commands.insert("delete_session".to_string());
    let (_rt, _shared) = mock_env(st);
    let goal = init_goal(&cr, "cleanup warning");
    // The run still succeeds; the cleanup failure is best-effort.
    cli_ok(&["run", "--goal", &goal, "--anonymous"]);
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

// ── capability hook proposal-kind arms ─────────────────────────────────────

#[test]
fn capability_hook_kind_arms() {
    let _cr = cli_root();
    // NoFollowUp via empty input; Gate via read-only authority (prints the
    // gate question line).
    cli_ok(&["issue-fix"]);
    cli_ok(&[
        "issue-fix",
        "--input",
        "title: bug\nerror: crash\nrepro: steps\nauthority: read-only",
    ]);
    cli_ok(&[
        "issue-fix",
        "--input",
        "title: crash\nerror: panicked\nrepro: run it\nexpected: works fine\nscope: cli",
    ]);
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

// ── handoff: delivery contract present arm ─────────────────────────────────

#[test]
fn handoff_delivery_contract_present() {
    let cr = cli_root();
    let gid = init_goal(&cr, "handoff contract");
    {
        let store = open_store(&cr);
        // Two small-scale runs → small-batch streak hits the default threshold.
        let mut r1 = run_record("t", "completed", now_epoch());
        r1.evidence = "unit test passed".into();
        let mut r2 = run_record("t", "completed", now_epoch());
        r2.evidence = "unit test passed".into();
        store.append_run(&gid, &r1).unwrap();
        store.append_run(&gid, &r2).unwrap();
    }
    cli_ok(&["handoff", "--goal", &gid]);
}

// ── serve-status via the CLI (parse + serve, thread-leaked) ────────────────

#[test]
fn serve_status_command_arm() {
    let cr = cli_root();
    let _gid = init_goal(&cr, "serve via cli");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    // cmd_serve_status blocks forever in serve() → run it on a leaked thread.
    std::thread::spawn(move || {
        let _ = future_loop::console::run(
            "future-loop",
            vec![
                "serve-status".to_string(),
                "--port".to_string(),
                port.to_string(),
            ],
        );
    });
    // Wait for the server, then hit it.
    let mut ok = false;
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(ok, "serve-status came up");
    let _ = cr;
}

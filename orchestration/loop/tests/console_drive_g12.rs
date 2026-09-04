//! Coverage drive for the G12 multi-agent CLI surface (`agent contract /
//! recipe / succession / collective`), `replan rules`, `frontier`, and the
//! `worker list/stop` + `scan_worker_sessions` error paths that per-line
//! coverage still shows as DA:0.

mod common;

use common::mock_agent::{spawn_mock, MockState};
use common::{add_todo, cli_err, cli_ok, cli_root, first_todo_id, init_goal, open_store};
use future_loop::state::now_epoch;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn replan_rules_set_and_show() {
    let cr = cli_root();
    let gid = init_goal(&cr, "replan rules surface");

    assert!(cli_err(&["replan", "rules", "bogus"]).contains("must be `show` or `set`"));
    assert!(cli_err(&["replan", "rules", "show"]).contains("--goal required"));
    assert!(cli_err(&["replan", "rules", "set", "--goal", "goal_nope"]).contains("not found"));

    // show text + json on a fresh goal.
    cli_ok(&["replan", "rules", "show", "--goal", &gid]);
    cli_ok(&[
        "replan", "rules", "show", "--goal", &gid, "--format", "json",
    ]);

    // set with ids (including an unknown id → warning arm).
    cli_ok(&[
        "replan",
        "rules",
        "set",
        "--goal",
        &gid,
        "--rule-ids",
        "totally_unknown_rule,replan_required",
    ]);
    // set reset (empty string → default set).
    cli_ok(&["replan", "rules", "set", "--goal", &gid, "--rule-ids", ""]);
}

// ── frontier show ──────────────────────────────────────────────────────────

#[test]
fn frontier_show_text_and_json() {
    let cr = cli_root();
    let gid = init_goal(&cr, "frontier surface");

    assert!(cli_err(&["frontier", "bogus", "--goal", &gid]).contains("must be `show`"));
    assert!(cli_err(&["frontier", "show"]).contains("--goal required"));
    assert!(cli_err(&["frontier", "show", "--goal", "goal_nope"]).contains("not found"));

    cli_ok(&["frontier", "show", "--goal", &gid]);
    cli_ok(&["frontier", "show", "--goal", &gid, "--format", "json"]);
}

// ── worker list/stop ───────────────────────────────────────────────────────

fn run_header(agent: &str, session: &str, ts: u64, goal: &str) -> String {
    format!(
        "{{\"type\":\"run_header\",\"idx\":0,\"wall_ts\":{ts},\"run_id\":\"r-{session}\",\"session_id\":\"{session}\",\"agent_id\":\"{agent}\",\"todo_id\":\"todo-{agent}\",\"goal_id\":\"{goal}\"}}\n"
    )
}

#[test]
fn worker_list_text_and_json_and_unknown_subcommand() {
    let cr = cli_root();
    let gid = init_goal(&cr, "worker surface");

    assert!(cli_err(&["worker", "bogus", "--goal", &gid]).contains("unknown worker subcommand"));
    assert!(cli_err(&["worker", "list"]).contains("--goal required"));

    // No run_header files, no agent reachable → idle registered agents only.
    cli_ok(&["worker", "list", "--goal", &gid]);
    cli_ok(&["worker", "list", "--goal", &gid, "--format", "json"]);
}

#[test]
fn worker_stop_target_all_and_no_sessions() {
    let cr = cli_root();
    let gid = init_goal(&cr, "worker stop surface");

    // Neither --agent-id nor --all → error.
    assert!(cli_err(&["worker", "stop", "--goal", &gid]).contains("--agent-id"));
    assert!(cli_err(&["worker", "stop"]).contains("--goal required"));

    // No sessions → early "nothing to stop" (before gRPC connect).
    cli_ok(&["worker", "stop", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&["worker", "stop", "--goal", &gid, "--all"]);
}

#[test]
fn worker_stop_aborts_live_sessions_via_grpc() {
    let cr = cli_root();
    let gid = init_goal(&cr, "worker stop abort");
    // Seed a run_header for worker-a under the goal's runs dir.
    let runs = std::path::Path::new(&cr.root).join("runs");
    std::fs::create_dir_all(&runs).unwrap();
    std::fs::write(
        runs.join("ra.live.jsonl"),
        run_header("worker-a", "sess-a", 100, &gid),
    )
    .unwrap();

    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    // Targeted stop with --delete reclaims the session.
    cli_ok(&[
        "worker",
        "stop",
        "--goal",
        &gid,
        "--agent-id",
        "worker-a",
        "--delete",
    ]);
    let recorded = shared.lock().unwrap().recorded.clone();
    assert!(recorded.contains(&"abort".to_string()), "{recorded:?}");
    assert!(
        recorded.contains(&"delete_session".to_string()),
        "{recorded:?}"
    );

    // Broadcast stop reaches the same session (no --delete → no delete call).
    let n = shared.lock().unwrap().recorded.len();
    cli_ok(&["worker", "stop", "--goal", &gid, "--all"]);
    assert!(shared.lock().unwrap().recorded.len() > n);

    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

// ── supervisor steer + attention succession hints ──────────────────────────

#[test]
fn supervisor_steer_records_and_renders() {
    let cr = cli_root();
    let gid = init_goal(&cr, "supervisor steer");

    assert!(cli_err(&["supervisor", "steer", "--goal", &gid]).contains("--instruction required"));
    cli_ok(&[
        "supervisor",
        "steer",
        "--goal",
        &gid,
        "--agent-id",
        "worker-a",
        "--instruction",
        "re-check",
    ]);
    let store = open_store(&cr);
    let g = store.replay(&gid).unwrap().unwrap();
    assert_eq!(g.pending_steer.as_ref().unwrap().instruction, "re-check");
}

// ── remaining error arms ───────────────────────────────────────────────────

#[test]
fn todo_add_acceptance_and_empty_text_and_bare_blocks() {
    let cr = cli_root();
    let gid = init_goal(&cr, "todo add arms");

    // Empty text.
    assert!(cli_err(&["todo", "add", "--goal", &gid, "--text", "  "]).contains("non-empty"));
    // Bare --blocks reads as "true".
    assert!(
        cli_err(&["todo", "add", "--goal", &gid, "--text", "x", "--blocks",])
            .contains("--blocks requires")
    );
    // --acceptance is a valid todo-add flag (the parse arm + assignment).
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "scored task",
        "--acceptance",
        "attempt,scored",
    ]);
    let store = open_store(&cr);
    let g = store.replay(&gid).unwrap().unwrap();
    let t = g
        .todos
        .iter()
        .find(|t| t.text.contains("scored task"))
        .unwrap();
    assert_eq!(t.acceptance.as_deref(), Some("attempt,scored"));
}

#[test]
fn todo_claim_not_open_arm() {
    let cr = cli_root();
    let gid = init_goal(&cr, "todo claim not open");
    let onboarding = first_todo_id(&cr.root, &gid);
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "a1"]);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &onboarding,
        "--no-follow-up",
        "--evidence",
        "closed",
    ]);
    let err = cli_err(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &onboarding,
        "--agent-id",
        "a1",
    ]);
    assert!(err.contains("not open"), "{err}");
}

#[test]
fn todo_complete_superseded_arm() {
    let cr = cli_root();
    let gid = init_goal(&cr, "complete superseded");
    let t = add_todo(&cr, &gid, "will be superseded");
    cli_ok(&["todo", "supersede", "--goal", &gid, "--todo-id", &t]);
    let err = cli_err(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--no-follow-up",
        "--evidence",
        "x",
    ]);
    assert!(err.contains("was superseded"), "{err}");
}

#[test]
fn lease_claim_requires_open_and_rejects_active_lease() {
    let cr = cli_root();
    let gid = init_goal(&cr, "lease claim arms");
    let onboarding = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &onboarding,
        "--no-follow-up",
        "--evidence",
        "closed",
    ]);
    assert!(cli_err(&[
        "lease",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &onboarding,
        "--agent-id",
        "a1",
    ])
    .contains("requires an open todo"));

    let t = add_todo(&cr, &gid, "lease contended");
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "a1",
    ]);
    assert!(cli_err(&[
        "lease",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--agent-id",
        "a2",
    ])
    .contains("active lease held by another agent"));
}

#[test]
fn todo_update_bare_blocks_and_bad_priority() {
    let cr = cli_root();
    let gid = init_goal(&cr, "todo update arms");
    let t = add_todo(&cr, &gid, "update me");
    assert!(cli_err(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--blocks",
    ])
    .contains("--blocks requires"));
    assert!(cli_err(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &t,
        "--priority",
        "P9",
    ])
    .contains("unknown --priority"));
}

#[test]
fn store_verify_reports_skipped_unknown_kinds() {
    let cr = cli_root();
    let gid = init_goal(&cr, "store verify unknown kind");
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
    cli_ok(&["store", "verify", "--goal", &gid]);
}

// ── webui argument parsing arms ────────────────────────────────────────────

#[test]
fn webui_argument_parsing_arms() {
    let _cr = cli_root();
    assert!(cli_err(&["ui", "--port"]).contains("--port requires a value"));
    assert!(cli_err(&["ui", "--port", "not-a-number"]).contains("--port must be 0-65535"));
    assert!(cli_err(&["ui", "--root"]).contains("--root requires a value"));
    // A positional (non-flag) argument hits the match's `other` arm.
    assert!(cli_err(&["ui", "positional"]).contains("unknown argument"));
}

// ── print_status_json turn_no_progress fields ──────────────────────────────

#[test]
fn status_json_renders_turn_no_progress_fields() {
    let cr = cli_root();
    let gid = init_goal(&cr, "status json no-progress");
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::TurnNoProgress {
            goal_id: gid.clone(),
            todo_id: "t1".to_string(),
            agent_id: Some("a1".to_string()),
            idle_secs: 42,
            tool_calls_total: 2,
            ts: now_epoch(),
        })
        .unwrap();
    drop(store);
    cli_ok(&["status", "--goal", &gid, "--format", "json"]);
}

// ── notify_dead_holders early-return arms ──────────────────────────────────

#[test]
fn notify_dead_holders_early_returns() {
    let cr = cli_root();
    let gid = init_goal(&cr, "dead holders early");
    let mut store = open_store(&cr);
    let rt = rt();
    let (addr, _shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    rt.block_on(async {
        // No goal → replay returns None → early return.
        future_loop::console::notify_dead_holders(&mut store, "goal_nope")
            .await
            .unwrap();
        // Goal exists but no supervisor → early return.
        future_loop::console::notify_dead_holders(&mut store, &gid)
            .await
            .unwrap();
        // Supervisor registered but no dead holders → early return.
        store
            .append(future_loop::store::Event::SupervisorRegistered {
                goal_id: gid.clone(),
                session_id: "sup".to_string(),
                ts: now_epoch(),
            })
            .unwrap();
        future_loop::console::notify_dead_holders(&mut store, &gid)
            .await
            .unwrap();
    });
    std::env::remove_var("FUTURE_LOOP_AGENT_ADDR");
}

// ── run session-policy removal + resume-session parsing ───────────────────

#[test]
fn run_rejects_removed_session_policy_flag() {
    let cr = cli_root();
    let gid = init_goal(&cr, "run session policy");
    let err = cli_err(&[
        "run",
        "--goal",
        &gid,
        "--agent-id",
        "a1",
        "--session-policy",
        "fresh",
    ]);
    assert!(err.contains("unknown flag `--session-policy`"), "{err}");
}

#[test]
fn run_rejects_resume_policy_value() {
    let cr = cli_root();
    let gid = init_goal(&cr, "run resume policy");
    let err = cli_err(&[
        "run",
        "--goal",
        &gid,
        "--agent-id",
        "a1",
        "--session-policy",
        "resume",
    ]);
    assert!(err.contains("unknown flag `--session-policy`"), "{err}");
}

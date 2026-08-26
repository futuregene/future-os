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

/// A valid two-peer contract with one backup chain + one collective.
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

// ── agent contract set/show ────────────────────────────────────────────────

#[test]
fn agent_contract_set_and_show_text_and_json() {
    let cr = cli_root();
    let gid = init_goal(&cr, "contract surface");

    // Missing --goal fails.
    assert!(cli_err(&["agent", "contract", "set"]).contains("--goal required"));
    // Neither --contract nor --contract-file fails.
    assert!(cli_err(&["agent", "contract", "set", "--goal", &gid]).contains("contract required"));
    // Malformed JSON fails.
    assert!(cli_err(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract",
        "{not-json",
    ])
    .contains("parse contract JSON"));
    // Missing goal on show fails.
    assert!(cli_err(&["agent", "contract", "show"]).contains("--goal required"));
    // Unknown subcommand fails.
    assert!(cli_err(&["agent", "contract", "bogus", "--goal", &gid])
        .contains("unknown agent contract subcommand"));

    // Set via inline JSON.
    cli_ok(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract",
        &contract_json(),
    ]);

    // show text (contract present, no issues).
    cli_ok(&["agent", "contract", "show", "--goal", &gid]);
    // show JSON.
    cli_ok(&[
        "agent", "contract", "show", "--goal", &gid, "--format", "json",
    ]);

    // Re-set via a contract file.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("contract.json");
    std::fs::write(&path, contract_json()).unwrap();
    cli_ok(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract-file",
        path.to_str().unwrap(),
    ]);
}

#[test]
fn agent_contract_show_without_contract_and_with_issues() {
    let cr = cli_root();
    let gid = init_goal(&cr, "contract no-set");
    // No contract set yet → the None arm.
    cli_ok(&["agent", "contract", "show", "--goal", &gid]);

    // Inject a drifted contract (schema mismatch) directly so `show` renders
    // validation issues.
    let mut store = open_store(&cr);
    let mut contract: future_loop::agents::multi_agent::MultiAgentContract =
        serde_json::from_str(&contract_json()).unwrap();
    contract.schema_version = "wrong_version".to_string();
    store
        .append(future_loop::store::Event::MultiAgentContractSet {
            goal_id: gid.clone(),
            contract,
            ts: now_epoch(),
        })
        .unwrap();
    drop(store);
    cli_ok(&["agent", "contract", "show", "--goal", &gid]);
}

// ── agent recipe add/show ──────────────────────────────────────────────────

#[test]
fn agent_recipe_add_and_show() {
    let cr = cli_root();
    let gid = init_goal(&cr, "recipe surface");

    assert!(cli_err(&["agent", "recipe", "add"]).contains("--goal required"));
    assert!(cli_err(&["agent", "recipe", "add", "--goal", &gid]).contains("--name required"));
    assert!(cli_err(&["agent", "recipe", "bogus"]).contains("unknown agent recipe subcommand"));
    // Goal must exist.
    assert!(cli_err(&[
        "agent",
        "recipe",
        "add",
        "--goal",
        "goal_nope",
        "--name",
        "r"
    ])
    .contains("not found"));

    cli_ok(&[
        "agent",
        "recipe",
        "add",
        "--goal",
        &gid,
        "--name",
        "r1",
        "--capabilities",
        "shell,github",
        "--workspace",
        "/ws/a,/ws/b",
        "--priority",
        "P0",
    ]);
    // Priority P2 path + default P1 path.
    cli_ok(&[
        "agent",
        "recipe",
        "add",
        "--goal",
        &gid,
        "--name",
        "r2",
        "--priority",
        "P2",
    ]);
    cli_ok(&["agent", "recipe", "add", "--goal", &gid, "--name", "r3"]);

    // show text (recipes present).
    cli_ok(&["agent", "recipe", "show", "--goal", &gid]);
    // show filtered by name.
    cli_ok(&["agent", "recipe", "show", "--goal", &gid, "--name", "r1"]);
    // show JSON.
    cli_ok(&[
        "agent", "recipe", "show", "--goal", &gid, "--format", "json",
    ]);
    // show with a name that matches nothing → empty arm.
    cli_ok(&["agent", "recipe", "show", "--goal", &gid, "--name", "ghost"]);
}

// ── agent succession show/apply ────────────────────────────────────────────

#[test]
fn agent_succession_show_and_apply() {
    let cr = cli_root();
    let gid = init_goal(&cr, "succession surface");

    // No contract → error (subcommand check happens after contract resolution).
    assert!(cli_err(&["agent", "succession", "show", "--goal", &gid])
        .contains("no multi-agent contract set"));

    cli_ok(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract",
        &contract_json(),
    ]);
    // Unknown subcommand (contract now present).
    assert!(cli_err(&["agent", "succession", "bogus", "--goal", &gid])
        .contains("unknown agent succession subcommand"));
    // Register the primary (the backup's `backup_for` target).
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "primary"]);

    // No succession triggers yet → empty show (text + json).
    cli_ok(&["agent", "succession", "show", "--goal", &gid]);
    cli_ok(&[
        "agent",
        "succession",
        "show",
        "--goal",
        &gid,
        "--format",
        "json",
    ]);

    // Seed an expired lease held by the primary → succession candidate.
    let onboarding = first_todo_id(&cr.root, &gid);
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::TodoClaimed {
            goal_id: gid.clone(),
            todo_id: onboarding.clone(),
            agent_id: "primary".to_string(),
            lease_expires_at: 1, // long past → expired
            holder_pid: None,
            ts: now_epoch(),
        })
        .unwrap();
    drop(store);

    // apply records the succession.
    cli_ok(&["agent", "succession", "apply", "--goal", &gid]);
    // show now surfaces the recorded succession.
    cli_ok(&["agent", "succession", "show", "--goal", &gid]);
    // apply again is idempotent (already recorded).
    cli_ok(&["agent", "succession", "apply", "--goal", &gid]);
    // apply with a non-matching --primary filter → no candidates.
    cli_ok(&[
        "agent",
        "succession",
        "apply",
        "--goal",
        &gid,
        "--primary",
        "ghost",
    ]);
}

// ── agent collective show ──────────────────────────────────────────────────

#[test]
fn agent_collective_show_text_and_json() {
    let cr = cli_root();
    let gid = init_goal(&cr, "collective surface");

    assert!(
        cli_err(&["agent", "collective", "bogus"]).contains("unknown agent collective subcommand")
    );
    // No contract → error.
    assert!(cli_err(&["agent", "collective", "show", "--goal", &gid])
        .contains("no multi-agent contract set"));

    cli_ok(&[
        "agent",
        "contract",
        "set",
        "--goal",
        &gid,
        "--contract",
        &contract_json(),
    ]);

    // Unknown collective name → error.
    assert!(cli_err(&[
        "agent",
        "collective",
        "show",
        "--goal",
        &gid,
        "--collective",
        "ghost",
    ])
    .contains("is not part of the contract"));

    // No turn ledger yet → empty text + json.
    cli_ok(&["agent", "collective", "show", "--goal", &gid]);
    cli_ok(&[
        "agent",
        "collective",
        "show",
        "--goal",
        &gid,
        "--format",
        "json",
    ]);
}

// ── replan rules set/show ──────────────────────────────────────────────────

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

#[test]
fn attention_with_succession_hints() {
    let cr = cli_root();
    let gid = init_goal(&cr, "attention succession");
    // attention requires --goal or --all.
    assert!(cli_err(&["attention"]).contains("--goal"));

    // A goal with a recorded succession exercises the succession-attention
    // join (the `?` on that extend line).
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::SuccessionOccurred {
            goal_id: gid.clone(),
            primary: "p".to_string(),
            backup: "b".to_string(),
            reason: "offline".to_string(),
            ts: now_epoch(),
        })
        .unwrap();
    drop(store);
    cli_ok(&["attention", "--goal", &gid]);
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
fn agent_onboard_recipe_conflict_and_missing() {
    let cr = cli_root();
    let gid = init_goal(&cr, "onboard recipe arms");
    // Recipe + explicit workspace conflict.
    assert!(cli_err(&[
        "agent",
        "onboard",
        "--goal",
        &gid,
        "--agent-id",
        "a1",
        "--recipe",
        "r1",
        "--workspace",
        "/ws",
    ])
    .contains("conflicts with an explicit --workspace"));
    // Recipe not found.
    assert!(cli_err(&[
        "agent",
        "onboard",
        "--goal",
        &gid,
        "--agent-id",
        "a1",
        "--recipe",
        "ghost",
    ])
    .contains("no agent recipe named"));
    // Happy path: recipe applies cleanly.
    cli_ok(&["agent", "recipe", "add", "--goal", &gid, "--name", "r1"]);
    cli_ok(&[
        "agent",
        "onboard",
        "--goal",
        &gid,
        "--agent-id",
        "a1",
        "--recipe",
        "r1",
    ]);
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

// ── run session-policy validation + resume-session parsing ─────────────────

#[test]
fn run_rejects_bad_session_policy() {
    let cr = cli_root();
    let gid = init_goal(&cr, "run session policy");
    let err = cli_err(&[
        "run",
        "--goal",
        &gid,
        "--agent-id",
        "a1",
        "--session-policy",
        "bogus",
    ]);
    assert!(
        err.contains("--session-policy must be auto | fresh | resume"),
        "{err}"
    );
}

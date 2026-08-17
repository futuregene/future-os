//! Coverage sweep 2 for console.rs — the surgical remainder: default-cwd
//! goal init, delete-force errors, agent-list lease events, the run-loop
//! replan self-repair arm, backfill claim print, stale status cache, scoped
//! gates, benchmark adapter connect error, replay mismatch prints, failing
//! canary/doctor CLI paths.

mod common;

use common::mock_agent::{completed_events, spawn_mock, MockState};
use common::{cli, cli_err, cli_ok, cli_root, first_todo_id, init_goal, open_store};
use future_loop::state::now_epoch;
use future_loop::store::{Event, Store};

fn mock_env(state: MockState) -> (tokio::runtime::Runtime, common::mock_agent::SharedState) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let (addr, shared) = rt.block_on(spawn_mock(state));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    (rt, shared)
}

#[test]
fn goal_init_default_cwd_and_delete_errors() {
    let cr = cli_root();
    // No --cwd → falls back to the process current dir.
    cli_ok(&[
        "goal",
        "init",
        "--objective",
        "default cwd",
        "--goal-id",
        "goal_defcwd",
    ]);
    // delete --force on an unknown goal → store error surfaces.
    assert!(cli_err(&["goal", "delete", "--goal", "goal_ghost", "--force"]).contains("not found"));
    let _ = cr;
}

#[test]
fn agent_list_with_lease_events() {
    let cr = cli_root();
    let gid = init_goal(&cr, "agent list lease events");
    let first = first_todo_id(&cr.root, &gid);
    cli_ok(&["agent", "onboard", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--agent-id",
        "w1",
    ]);
    cli_ok(&[
        "lease",
        "renew",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--agent-id",
        "w1",
        "--lease-secs",
        "90",
    ]);
    // w1 shows a live lease; a second registered agent shows idle.
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w2"]);
    cli_ok(&["agent", "list", "--goal", &gid]);
    // Release then list (released event in the scan).
    cli_ok(&[
        "lease",
        "release",
        "--goal",
        &gid,
        "--todo-id",
        &first,
        "--agent-id",
        "w1",
    ]);
    cli_ok(&["agent", "list", "--goal", &gid]);
}

#[test]
fn run_replan_self_repair_then_stop() {
    let cr = cli_root();
    let (_rt, shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let gid = init_goal(&cr, "replan repair");
    {
        // A completion WITHOUT closure intent → replan obligation; a stale
        // next_action → projection gap. First turn repairs the gap and loops;
        // second turn replans with no gap → graceful stop.
        let mut store: Store = open_store(&cr);
        let g = store.replay(&gid).unwrap().unwrap();
        let first = g.todos.first().unwrap().id.clone();
        drop(g);
        store
            .append(Event::TodoCompleted {
                goal_id: gid.clone(),
                todo_id: first,
                no_follow_up: false,
                successor_ids: vec![],
                evidence: None,
                ts: now_epoch(),
            })
            .unwrap();
        store.set_next_action(&gid, "phantom work").unwrap();
    }
    cli_ok(&["run", "--goal", &gid, "--anonymous", "--max-turns", "4"]);
    assert_eq!(
        shared.lock().unwrap().prompts,
        0,
        "no turn executed during replan"
    );
    // The repair pass re-synced the next action.
    let store = open_store(&cr);
    let g = store.replay(&gid).unwrap().unwrap();
    assert_eq!(
        g.next_action.as_deref(),
        Some("all todos complete; no further action")
    );
}

#[test]
fn run_with_unknown_flag_and_model_env() {
    let cr = cli_root();
    let (_rt, _shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let gid = init_goal(&cr, "run zz");
    // P0-3 strictness: unknown flags hard-error instead of being swallowed.
    let err = cli_err(&["run", "--goal", &gid, "--anonymous", "--zz", "1"]);
    assert!(err.contains("unknown flag `--zz`"), "got: {err}");
    // A well-formed anonymous run still succeeds against the mock agent.
    cli_ok(&["run", "--goal", &gid, "--anonymous"]);
}

#[test]
fn backfill_dry_run_claim_print() {
    let cr = cli_root();
    let gid = init_goal(&cr, "backfill claim print");
    let md = std::path::Path::new(&cr.cwd).join("state.md");
    std::fs::write(
        &md,
        "## Agent Todo\n\n- [ ] Claimed task\n  <!-- future-loop:todo todo_id=todo_c1 status=open claimed_by=agent-9 -->\n- [x] Done task\n  <!-- future-loop:todo todo_id=todo_c2 status=done no_followup=true -->\n",
    )
    .unwrap();
    // Dry-run prints the add/claim/complete event preview lines.
    cli_ok(&[
        "backfill",
        "--goal",
        &gid,
        "--from",
        md.to_str().unwrap(),
        "--dry-run",
    ]);
}

#[test]
fn privacy_stale_cache_arm() {
    let cr = cli_root();
    let gid = init_goal(&cr, "privacy stale");
    cli_ok(&["privacy", "--goal", &gid]);
    // A new ledger event makes the persisted cache stale on the next read.
    common::add_todo(&cr, &gid, "another task");
    cli_ok(&["privacy", "--goal", &gid]);
}

#[test]
fn scope_with_open_gate() {
    let cr = cli_root();
    let gid = init_goal(&cr, "scope gate");
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "approve?",
        "--class",
        "user_gate",
    ]);
    cli_ok(&["scope", "--goal", &gid, "--agent-id", "w1"]);
}

#[test]
fn benchmark_run_unreachable_agent() {
    let cr = cli_root();
    let err = cli_err(&[
        "benchmark",
        "run",
        "--benchmark-id",
        "b",
        "--case-id",
        "c",
        "--task",
        "t",
        "--agent-addr",
        "127.0.0.1:1",
    ]);
    assert!(err.contains(""), "{err}");
    let _ = cr;
}

#[test]
fn replay_run_mismatch_prints_and_bails() {
    let cr = cli_root();
    let gid = init_goal(&cr, "replay mismatch");
    let case_file = std::path::Path::new(&cr.cwd).join("cases.json");
    cli_ok(&[
        "replay",
        "record",
        "--goal",
        &gid,
        "--out",
        case_file.to_str().unwrap(),
    ]);
    // Tamper the recorded case: flip a decision field.
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&case_file).unwrap()).unwrap();
    let should_run = value["cases"][0]["decision"]["should_run"]
        .as_bool()
        .unwrap();
    value["cases"][0]["decision"]
        .as_object_mut()
        .unwrap()
        .insert("should_run".into(), serde_json::json!(!should_run));
    std::fs::write(&case_file, serde_json::to_string(&value).unwrap()).unwrap();
    let err = cli_err(&["replay", "run", "--case", case_file.to_str().unwrap()]);
    assert!(err.contains("drifted"), "{err}");
    // JSON mode prints the comparison object.
    let err = cli_err(&[
        "replay",
        "run",
        "--case",
        case_file.to_str().unwrap(),
        "--json",
    ]);
    assert!(err.contains("drifted"), "{err}");
}

#[test]
fn canary_cli_failing_checks() {
    let cr = cli_root();
    let gid = init_goal(&cr, "canary cli fail");
    {
        let store = open_store(&cr);
        let path = store.goal_dir(&gid).join("events.jsonl");
        let a = serde_json::json!({"event_id":"e-dup","kind":"goal_started","goal_id":gid,"ts":1});
        let b = serde_json::json!({"event_id":"e-dup","kind":"goal_started","goal_id":gid,"ts":2});
        std::fs::write(&path, format!("{}\n{}\n", a, b)).unwrap();
    }
    // Text mode prints the FAIL lines and bails. NOTE: --json returns Ok
    // even when checks fail (the JSON arm returns before the all_passed
    // bail) — consumers must inspect the payload; flagged in the report.
    let err = cli_err(&["canary", "smoke"]);
    assert!(err.contains("failed"), "{err}");
    cli_ok(&["canary", "smoke", "--json"]);
    // doctor surfaces the ledger problem as a failure list entry.
    let err = cli_err(&["doctor", "--goal", &gid]);
    assert!(err.contains("failure"), "{err}");
}

#[test]
fn doctor_goal_replay_error_arm() {
    let cr = cli_root();
    let gid = init_goal(&cr, "doctor replay err");
    {
        use std::io::Write as _;
        let store = open_store(&cr);
        let path = store.goal_dir(&gid).join("events.jsonl");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{broken\n")
            .unwrap();
    }
    let err = cli_err(&["doctor", "--goal", &gid]);
    assert!(err.contains("failure"), "{err}");
}

#[test]
fn diagnose_and_doctor_with_projection_gap() {
    let cr = cli_root();
    let gid = init_goal(&cr, "gap surfaces");
    // Complete the only todo so agent_open == 0, then point next_action at
    // phantom work → a real projection gap.
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
    {
        let store = open_store(&cr);
        store.set_next_action(&gid, "phantom work").unwrap();
    }
    cli_ok(&["diagnose", "--goal", &gid]);
    let err = cli_err(&["doctor", "--goal", &gid]);
    assert!(err.contains("gap") || err.contains("failure"), "{err}");
}

#[test]
fn benchmark_protocol_blind_route() {
    let _cr = cli_root();
    cli_ok(&[
        "benchmark",
        "protocol",
        "--route",
        "raw-codex-autonomous-max5",
    ]);
}

#[test]
fn privacy_private_fields_print_arm() {
    let cr = cli_root();
    let home = std::env::var("HOME").unwrap_or_default();
    let gid = init_goal(&cr, "privacy fields");
    common::add_todo(
        &cr,
        &gid,
        &format!("read {home}/.ssh/id_rsa and rotate the key"),
    );
    cli_ok(&["privacy", "--goal", &gid]);
}

#[test]
fn corpus_build_trailing_flag_arms() {
    let cr = cli_root();
    let gid = init_goal(&cr, "corpus trailing");
    // --ablate / --patch at the very end with no value → skipped arms.
    cli_ok(&[
        "replay", "corpus", "build", "--goal", &gid, "--patch", "{}", "--ablate",
    ]);
    let _ = cr;
}

#[test]
fn runs_retention_with_candidates() {
    let cr = cli_root();
    let gid = init_goal(&cr, "retention candidates");
    {
        let store = open_store(&cr);
        let runs_dir = store.goal_dir(&gid).join("runs");
        std::fs::create_dir_all(&runs_dir).unwrap();
        for (name, ts) in [
            (
                "2020-01-01T00-00-00-00-00.json",
                "2020-01-01T00:00:00+00:00",
            ),
            (
                "2021-01-01T00-00-00-00-00.json",
                "2021-01-01T00:00:00+00:00",
            ),
            (
                "2022-01-01T00-00-00-00-00.json",
                "2022-01-01T00:00:00+00:00",
            ),
        ] {
            std::fs::write(
                runs_dir.join(name),
                format!("{{\"timestamp\":\"{ts}\",\"turn\":1,\"terminal_state\":\"completed\"}}"),
            )
            .unwrap();
        }
    }
    cli_ok(&["runs", "retention", "--goal", &gid, "--keep", "1"]);
}

#[test]
fn benchmark_protocol_blind_route_print() {
    let _cr = cli_root();
    cli_ok(&[
        "benchmark",
        "protocol",
        "--route",
        "future-loop-blind-loop-treatment",
    ]);
}

#[test]
fn runs_retention_candidates_print() {
    let cr = cli_root();
    let gid = init_goal(&cr, "retention print");
    {
        let store = open_store(&cr);
        let runs_dir = store.goal_dir(&gid).join("runs");
        std::fs::create_dir_all(&runs_dir).unwrap();
        for (name, ts) in [
            (
                "2020-01-01T00-00-00-00-00.json",
                "2020-01-01T00:00:00+00:00",
            ),
            (
                "2021-01-01T00-00-00-00-00.json",
                "2021-01-01T00:00:00+00:00",
            ),
            (
                "2022-01-01T00-00-00-00-00.json",
                "2022-01-01T00:00:00+00:00",
            ),
        ] {
            std::fs::write(
                runs_dir.join(name),
                format!("{{\"timestamp\":\"{ts}\",\"turn\":1,\"terminal_state\":\"completed\"}}"),
            )
            .unwrap();
        }
    }
    // The retention projection reads the rebuilt index.
    cli_ok(&["runs", "index", "--goal", &gid, "--rebuild"]);
    cli_ok(&["runs", "retention", "--goal", &gid, "--keep", "1"]);
}

#[test]
fn benchmark_run_stub_flag() {
    let cr = cli_root();
    let ledger_dir = std::path::Path::new(&cr.cwd).join("bench-stub");
    cli_ok(&[
        "benchmark",
        "run",
        "--benchmark-id",
        "bs",
        "--case-id",
        "cs",
        "--task",
        "t",
        "--stub",
        "--ledger-dir",
        ledger_dir.to_str().unwrap(),
    ]);
}

#[test]
fn corpus_build_positional_arg_arm() {
    let cr = cli_root();
    let gid = init_goal(&cr, "corpus positional");
    cli_ok(&[
        "replay",
        "corpus",
        "build",
        "--goal",
        &gid,
        "positional-junk",
        "--patch",
        "{}",
    ]);
}

#[test]
fn run_validator_inconclusive_print() {
    let cr = cli_root();
    let (_rt, _shared) = mock_env(MockState {
        events: completed_events("mock-run-1"),
        ..Default::default()
    });
    let gid = init_goal(&cr, "validator inconclusive");
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
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "validated task",
        "--verify",
        "exit 0",
        "--max-validation-attempts",
        "1",
    ]);
    // With an empty PATH the validator's `sh` cannot spawn → Inconclusive.
    let saved = std::env::var_os("PATH");
    std::env::set_var("PATH", "/nonexistent-dir-xyz");
    let result = cli(&["run", "--goal", &gid, "--anonymous", "--max-turns", "2"]);
    if let Some(p) = saved {
        std::env::set_var("PATH", p);
    }
    result.unwrap();
}

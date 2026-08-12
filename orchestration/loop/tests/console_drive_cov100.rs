//! Coverage drive (per-line 100% push) for `console.rs` parse-table catch-alls
//! and error edges: every parse_pairs `_ => {}` arm is fed an unknown flag,
//! the error-propagating `?` lines are exercised with failing fixtures, and
//! the remaining if-let false edges get targeted scenarios.

mod common;

use common::{cli, cli_err, cli_ok, cli_root, init_goal, open_store, todo_id_by_text};

#[test]
fn parse_catch_all_arms_ignore_unknown_flags() {
    let cr = cli_root();
    let gid = init_goal(&cr, "parse catch-alls");

    // capability hook (issue-fix) unknown flag.
    cli_ok(&["issue-fix", "--input", "it broke", "--bogus", "x"]);
    // agent register unknown flag.
    cli_ok(&[
        "agent",
        "register",
        "--goal",
        &gid,
        "--agent-id",
        "a1",
        "--bogus",
        "x",
    ]);
    // authority with only --require-approval (no --write-scope) + unknown flag.
    cli_ok(&["authority", "--goal", &gid, "--require-approval", "publish"]);
    // profile set without --outcome-floor.
    cli_ok(&["profile", "set", "--goal", &gid]);
    // quota tools unknown flag.
    cli_ok(&["quota", "tools", "--goal", &gid, "--bogus", "x"]);
    // replay run / corpus run unknown flags (load fails after parsing).
    assert!(!cli_err(&["replay", "run", "--case", "/nonexistent.json", "--bogus", "y"]).is_empty());
    assert!(!cli_err(&[
        "replay",
        "corpus",
        "run",
        "--corpus",
        "/nonexistent.json",
        "--bogus",
        "y"
    ])
    .is_empty());
    // extension enable unknown flag (id missing fails after parsing).
    assert!(!cli_err(&["extension", "enable", "--id", "ghost", "--bogus", "y"]).is_empty());
}

#[test]
fn extension_install_unknown_flag_is_ignored() {
    let cr = cli_root();
    let manifest = std::path::Path::new(&cr.cwd).join("m.json");
    std::fs::write(
        &manifest,
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": future_loop::extensions::manifest::EXTENSION_MANIFEST_SCHEMA_VERSION,
            "id": "ext-z",
            "version": "1.0.0",
            "requires_future_loop_api": ">=1",
            "permissions": ["shell"],
            "runtime": {
                "protocol": "command_json_v0",
                "entrypoint": "sh",
                "args": [],
                "doctor_args": [],
                "required_permissions": ["shell"],
                "timeout_seconds": 30
            },
            "provides": [{"id": "ext_z_cap", "kind": "domain_rule", "visibility": "public"}],
            "implements": [{"capability_id": "ext_z_cap", "protocol": "command_json_v0"}]
        }))
        .unwrap(),
    )
    .unwrap();
    // Dry-run install with an unknown flag (parse catch-all, then success).
    cli_ok(&[
        "extension",
        "install",
        "--manifest",
        manifest.to_str().unwrap(),
        "--bogus",
        "x",
    ]);
}

#[test]
fn runs_compact_cutoff_without_index_errors() {
    let cr = cli_root();
    let gid = init_goal(&cr, "compact without index");
    let err = cli_err(&["runs", "compact", "--goal", &gid, "--cutoff", "0"]);
    assert!(err.contains("no run index"), "{err}");
}

#[test]
fn serve_status_on_a_busy_port_errors() {
    let cr = cli_root();
    let _hold = init_goal(&cr, "serve status busy port");
    // Hold the port so bind fails deterministically; --bogus feeds the
    // parse catch-all.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let err = cli_err(&["serve-status", "--port", &port, "--bogus", "x"]);
    assert!(!err.is_empty());
}

#[test]
fn benchmark_run_unknown_flag_and_adapter_failure() {
    let cr = cli_root();
    // Unknown flag is ignored; the run proceeds against the stub adapter.
    cli_ok(&[
        "benchmark",
        "run",
        "--benchmark-id",
        "b",
        "--case-id",
        "c",
        "--task",
        "t",
        "--bogus",
        "x",
    ]);
    // A dead agent address makes the gRPC adapter fail → the run errors.
    let err = cli_err(&[
        "benchmark",
        "run",
        "--benchmark-id",
        "b",
        "--case-id",
        "c2",
        "--task",
        "t",
        "--agent-addr",
        "127.0.0.1:1",
    ]);
    assert!(!err.is_empty());
}

#[test]
fn replay_corpus_build_with_trailing_patch_flag() {
    let cr = cli_root();
    let gid = init_goal(&cr, "corpus build trailing patch");
    // `--patch` with no following value is skipped (None arm) — no patches,
    // no ablations → the corpus build fails closed (empty corpus).
    let err = cli_err(&["replay", "corpus", "build", "--goal", &gid, "--patch"]);
    assert!(err.contains("at least one case"), "{err}");
}

#[test]
fn attention_all_tolerates_goals_without_ledgers() {
    let cr = cli_root();
    // Registry entry without any events → replay yields None → skipped.
    let mut store = open_store(&cr);
    store
        .register(&future_loop::state::Goal::new("ghost", "obj", "/tmp"))
        .unwrap();
    drop(store);
    cli_ok(&["attention", "--all"]);
}

#[test]
fn doctor_reports_conflicting_ledgers() {
    let cr = cli_root();
    let gid = init_goal(&cr, "doctor conflicts");
    // Append a conflicting duplicate of an existing event (same event_id,
    // different content).
    let store = open_store(&cr);
    let path = store.goal_dir(&gid).join("events.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    let first = text.lines().next().unwrap().to_string();
    let mut v: serde_json::Value = serde_json::from_str(&first).unwrap();
    v["objective"] = serde_json::json!("tampered objective");
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(format!("{v}\n").as_bytes())
        .unwrap();
    drop(store);
    let err = cli_err(&["doctor", "--goal", &gid]);
    assert!(err.contains("ledger conflicts"), "{err}");
}

#[test]
fn todo_add_priority_tag_is_not_doubled() {
    let cr = cli_root();
    let gid = init_goal(&cr, "priority already tagged");
    // Text already carries the tag → the retag block is skipped entirely.
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "[P1] already tagged",
        "--priority",
        "P1",
    ]);
    let tid = todo_id_by_text(&cr.root, &gid, "already tagged");
    let store = open_store(&cr);
    let g = store.replay(&gid).unwrap().unwrap();
    assert_eq!(g.todo(&tid).unwrap().text, "[P1] already tagged");
}

#[cfg(unix)]
#[test]
fn backfill_append_fails_on_read_only_goal_dir() {
    use std::os::unix::fs::PermissionsExt;
    let cr = cli_root();
    let gid = init_goal(&cr, "backfill read-only goal dir");
    let md = std::path::Path::new(&cr.cwd).join("active.md");
    std::fs::write(
        &md,
        "## Agent Todo\n\n- [ ] work\n  <!-- future-loop:todo todo_id=todo_1 status=open -->\n",
    )
    .unwrap();
    let events = open_store(&cr).goal_dir(&gid).join("events.jsonl");
    let mut perms = std::fs::metadata(&events).unwrap().permissions();
    perms.set_mode(0o444);
    std::fs::set_permissions(&events, perms).unwrap();
    let err = cli_err(&["backfill", "--goal", &gid, "--from", md.to_str().unwrap()]);
    assert!(!err.is_empty());
    let mut perms = std::fs::metadata(&events).unwrap().permissions();
    perms.set_mode(0o644);
    std::fs::set_permissions(&events, perms).unwrap();
}

#[cfg(unix)]
#[test]
fn supervisor_propose_append_fails_on_read_only_goal_dir() {
    use std::os::unix::fs::PermissionsExt;
    let cr = cli_root();
    let gid = init_goal(&cr, "supervisor read-only goal dir");
    let events = open_store(&cr).goal_dir(&gid).join("events.jsonl");
    let mut perms = std::fs::metadata(&events).unwrap().permissions();
    perms.set_mode(0o444);
    std::fs::set_permissions(&events, perms).unwrap();
    let err = cli_err(&[
        "supervisor",
        "propose",
        "--goal",
        &gid,
        "--agent-id",
        "sup",
        "--decision-id",
        "d1",
        "--target-agent-id",
        "worker",
    ]);
    assert!(!err.is_empty());
    let mut perms = std::fs::metadata(&events).unwrap().permissions();
    perms.set_mode(0o644);
    std::fs::set_permissions(&events, perms).unwrap();
}

#[test]
fn benchmark_run_qualification_error_propagates() {
    let cr = cli_root();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    // The session state call fails (preflight errors INSIDE
    // run_qualification_case — unlike a dead --agent-addr, which fails at
    // adapter construction).
    let (addr, _shared) = rt.block_on(common::mock_agent::spawn_mock(
        common::mock_agent::MockState::fail("get_state"),
    ));
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
        &addr,
    ]);
    assert!(!err.is_empty());
}

#[test]
fn todo_add_priority_tag_skips_mismatched_custom_title() {
    let cr = cli_root();
    let gid = init_goal(&cr, "priority tag title mismatch");
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        &gid,
        "--text",
        "body text",
        "--title",
        "Custom title",
        "--priority",
        "P0",
    ]);
    let tid = todo_id_by_text(&cr.root, &gid, "body text");
    let store = open_store(&cr);
    let g = store.replay(&gid).unwrap().unwrap();
    let t = g.todo(&tid).unwrap();
    assert_eq!(t.text, "[P0] body text");
    // The custom title did not match the tagged text → left untouched.
    assert_eq!(t.title, "Custom title");
}

#[test]
fn lease_reclaim_same_agent_is_idempotent() {
    let cr = cli_root();
    let gid = init_goal(&cr, "lease idempotent");
    let tid = todo_id_by_text(&cr.root, &gid, "status");
    for _ in 0..2 {
        cli_ok(&[
            "lease",
            "claim",
            "--goal",
            &gid,
            "--todo-id",
            &tid,
            "--agent-id",
            "a1",
        ]);
    }
}

#[test]
fn replan_obligations_prints_todo_bound_rows() {
    let cr = cli_root();
    let gid = init_goal(&cr, "replan obligations print");
    // Sanity: the obligations command runs (the todo_id-less rendering arm
    // is unit-tested via print_obligation in console.rs).
    let out = cli(&["replan", "obligations", "--goal", &gid]);
    assert!(out.is_ok(), "{out:?}");
}

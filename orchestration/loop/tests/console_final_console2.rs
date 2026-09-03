//! Coverage drive (per-line 100% push) — console.rs residual branches round 2:
//! reject_unknown_flags error edges, if-let None paths, read-only append
//! failure injection.

mod common;

use common::{cli_err, cli_ok, cli_root, init_goal, open_store};

// ── reject_unknown_flags `?` error edges (6 residual commands) ────────────

#[test]
fn residual_unknown_flag_error_edges() {
    let cr = cli_root();
    let gid = init_goal(&cr, "residual unknown flags");
    for args in [
        vec!["scheduler", "liveness", "--goal", &gid, "--bogus", "x"],
        vec![
            "scheduler",
            "record-host-failure",
            "--goal",
            &gid,
            "--bogus",
            "x",
        ],
    ] {
        let err = cli_err(&args);
        assert!(err.contains("unknown flag `--bogus`"), "{args:?}: {err}");
    }
}

// ── if-let None paths ─────────────────────────────────────────────────────

#[test]
fn authority_without_optional_flags() {
    let cr = cli_root();
    let gid = init_goal(&cr, "authority minimal");
    // Neither --write-scope nor --require-approval → both if-let None paths.
    cli_ok(&["authority", "--goal", &gid]);
}

#[test]
fn scheduler_liveness_threshold_parse_edges() {
    let cr = cli_root();
    let gid = init_goal(&cr, "scheduler liveness threshold parse");
    // Non-numeric threshold → parse Err arm.
    cli_ok(&[
        "scheduler",
        "liveness",
        "--goal",
        &gid,
        "--threshold-secs",
        "not-a-number",
    ]);
    // Zero threshold → `n > 0` false arm.
    cli_ok(&[
        "scheduler",
        "liveness",
        "--goal",
        &gid,
        "--threshold-secs",
        "0",
    ]);
}

// ── read-only append failure injection (store.append `?` error edges) ─────

#[cfg(unix)]
#[test]
fn scheduler_tick_heartbeat_append_fails_on_read_only_ledger() {
    use std::os::unix::fs::PermissionsExt;
    let cr = cli_root();
    let gid = init_goal(&cr, "scheduler tick read-only ledger");
    let events = open_store(&cr).goal_dir(&gid).join("events.jsonl");
    let mut perms = std::fs::metadata(&events).unwrap().permissions();
    perms.set_mode(0o444);
    std::fs::set_permissions(&events, perms).unwrap();
    // record_tick_heartbeat appends to the read-only ledger → `?` error edge.
    let err = cli_err(&["scheduler", "tick", "--goal", &gid]);
    assert!(!err.is_empty());
    let mut perms = std::fs::metadata(&events).unwrap().permissions();
    perms.set_mode(0o644);
    std::fs::set_permissions(&events, perms).unwrap();
}

// ── `--include-experimental` is a global pass-through flag: it reaches the
// parse_pairs closure as an unmatched key, which exercises the trailing
// else-if / single-flag "false" edges (the `}` ghost lines). ───────────────

#[test]
fn include_experimental_pass_through_covers_unmatched_key_edges() {
    let cr = cli_root();
    let gid = init_goal(&cr, "include experimental pass-through");
    let tid = common::first_todo_id(&cr.root, &gid);
    // Single-flag goal parsers (quota spend, store bridge).
    cli_ok(&["quota", "spend", "--goal", &gid, "--include-experimental"]);
    cli_ok(&["store", "bridge", "--goal", &gid, "--include-experimental"]);
    // Trailing else-if in cmd_scope.
    cli_ok(&[
        "scope",
        "--goal",
        &gid,
        "--agent-id",
        "a1",
        "--include-experimental",
    ]);
    // Trailing else-if in todo update (--blocks last).
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &tid,
        "--text",
        "updated",
        "--include-experimental",
    ]);
}

// ── monitor poll plan: no-spend=false due monitor + stalled monitor ───────

#[test]
fn monitor_poll_plan_no_spend_false_and_stalled() {
    let cr = cli_root();
    let gid = init_goal(&cr, "monitor poll plan variants");
    let now = future_loop::state::now_epoch();
    let mut store = open_store(&cr);
    // Due monitor with a non-external policy → no_spend_if_unchanged=false.
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: gid.clone(),
            todo: future_loop::state::Todo::monitor_with(
                "mon_custom",
                "watch custom",
                None,
                Some("custom_policy"),
                None,
                std::time::Duration::from_secs(0),
            ),
            ts: now,
        })
        .unwrap();
    // Stalled monitor: consecutive_no_change >= replan threshold.
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: gid.clone(),
            todo: future_loop::state::Todo::monitor(
                "mon_stalled",
                "watch stalled",
                std::time::Duration::from_secs(3600),
            ),
            ts: now,
        })
        .unwrap();
    store
        .append(future_loop::store::Event::MonitorPolled {
            goal_id: gid.clone(),
            todo_id: "mon_stalled".to_string(),
            result: "no_change".to_string(),
            no_change_count: 3,
            ts: now,
        })
        .unwrap();
    drop(store);
    cli_ok(&["scheduler", "tick", "--goal", &gid]);
}

// ── liveness breach with a non-alphanumeric agent id → clean() else arm ────

#[test]
fn liveness_breach_special_char_agent_cleans_inbox_name() {
    let cr = cli_root();
    let gid = init_goal(&cr, "liveness breach special agent");
    let now = future_loop::state::now_epoch();
    // Fabricate a stale heartbeat for an agent whose id has a `.` (not
    // alphanumeric/`-`/`_`) → the clean() else arm maps it to `-`.
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::SchedulerTicked {
            goal_id: gid.clone(),
            agent_id: "codex.app".to_string(),
            action: "tick_next".to_string(),
            rrule: None,
            ts: now.saturating_sub(3 * 3600),
        })
        .unwrap();
    drop(store);
    cli_ok(&[
        "scheduler",
        "liveness",
        "--goal",
        &gid,
        "--agent-id",
        "codex.app",
        "--threshold-secs",
        "60",
    ]);
}

// ── store verify --repair on a drifted index → non-empty backup path ──────

#[test]
fn store_verify_repair_backs_up_a_drifted_index() {
    let cr = cli_root();
    let gid = init_goal(&cr, "store verify repair drift");
    let runs = format!("{}/goals/{gid}/runs", cr.root);
    std::fs::create_dir_all(&runs).unwrap();
    // One run file on disk (rebuild source of truth) + a stale index.
    std::fs::write(
        format!("{runs}/a.json"),
        r#"{"timestamp":"123","turn":1,"terminal_state":"completed"}"#,
    )
    .unwrap();
    std::fs::write(format!("{runs}/index.jsonl"), "").unwrap();
    cli_ok(&["store", "verify", "--goal", &gid, "--repair"]);
}

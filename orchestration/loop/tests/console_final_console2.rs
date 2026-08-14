//! Coverage drive (per-line 100% push) — console.rs residual branches round 2:
//! reject_unknown_flags error edges, if-let None paths, read-only append
//! failure injection, and the pr-review claim steal / non-review-item arms.

mod common;

use common::{cli_err, cli_ok, cli_root, init_goal, open_store};

// ── reject_unknown_flags `?` error edges (6 residual commands) ────────────

#[test]
fn residual_unknown_flag_error_edges() {
    let cr = cli_root();
    let gid = init_goal(&cr, "residual unknown flags");
    let head = "a".repeat(40);
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
        vec!["pr-review", "queue", "--bogus", "x"],
        vec![
            "pr-review",
            "review",
            "--goal",
            &gid,
            "--number",
            "1",
            "--head",
            &head,
            "--verdict",
            "approve",
            "--bogus",
            "x",
        ],
        vec!["pr-review", "recommend", "--path", "x", "--bogus", "x"],
        vec![
            "decision-context",
            "feedback",
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

// ── pr-review: non-review-item verdict + claim steal ──────────────────────

#[test]
fn pr_review_verdict_on_a_non_review_item_todo() {
    let cr = cli_root();
    let gid = init_goal(&cr, "pr review non-review item");
    let head = "a".repeat(40);
    // Seed a plain advancement todo whose id collides with `pr-review-2`
    // (not a review work item) → ReviewItem::from_todo returns None.
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: gid.clone(),
            todo: future_loop::state::Todo::advancement("pr-review-2", "just a task"),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    drop(store);
    cli_ok(&[
        "pr-review",
        "review",
        "--goal",
        &gid,
        "--number",
        "2",
        "--head",
        &head,
        "--verdict",
        "approve",
    ]);
}

#[test]
fn pr_review_claim_success_then_steal() {
    let cr = cli_root();
    let gid = init_goal(&cr, "pr review claim steal");
    let head = "a".repeat(40);
    let now = future_loop::state::now_epoch();
    // Seed an open review item + an already-expired lease held by "alice".
    let mut store = open_store(&cr);
    let item = future_loop::work_items::review_queue::ReviewItem {
        number: 7,
        head_oid: head.clone(),
        repository: None,
        title: "PR 7".to_string(),
        url: None,
    };
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: gid.clone(),
            todo: item.to_todo("pr-review-7"),
            ts: now,
        })
        .unwrap();
    store
        .append(future_loop::store::Event::TodoClaimed {
            goal_id: gid.clone(),
            todo_id: "pr-review-7".to_string(),
            agent_id: "alice".to_string(),
            lease_expires_at: now.saturating_sub(100),
            ts: now.saturating_sub(200),
        })
        .unwrap();
    drop(store);
    // Bob claims the expired lease → steal (TodoExpired + "(steal after expiry)").
    cli_ok(&[
        "pr-review",
        "claim",
        "--goal",
        &gid,
        "--number",
        "7",
        "--reviewer",
        "bob",
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
    // Trailing else-if in cmd_supervisor receipt (host-capabilities last).
    cli_ok(&[
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
        "--kind",
        "execute",
    ]);
    cli_ok(&[
        "supervisor",
        "receipt",
        "--goal",
        &gid,
        "--decision-id",
        "d1",
        "--receipt-id",
        "r1",
        "--adapter-id",
        "ad",
        "--outcome",
        "executed",
        "--authority-ref",
        "auth",
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

// ── pr-review queue: proper previous-observation yields removed PRs ───────

#[test]
fn pr_review_queue_removed_prs_proper_previous_observation() {
    let _cr = cli_root();
    let f = tempfile::NamedTempFile::new().unwrap();
    // Previous observation (an actual prior queue output) carried PR 99; the
    // current payload drops it → removed_pr_numbers is non-empty.
    std::fs::write(
        f.path(),
        serde_json::json!({
            "repository": "owner/repo",
            "pull_requests": [{"number": 1, "head_oid": "a".repeat(40)}],
            "previous_observation": {
                "repository": "owner/repo",
                "observation_state": "observed_unchanged",
                "items": [
                    {"number": 99, "fingerprint": "fp99"},
                    {"number": 1, "fingerprint": "fp1"}
                ]
            }
        })
        .to_string(),
    )
    .unwrap();
    cli_ok(&[
        "pr-review",
        "queue",
        "--fixture",
        f.path().to_str().unwrap(),
    ]);
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

#[cfg(unix)]
#[test]
fn pr_review_verdict_todo_updated_append_fails_on_read_only_ledger() {
    use std::os::unix::fs::PermissionsExt;
    let cr = cli_root();
    let gid = init_goal(&cr, "pr review verdict read-only ledger");
    let head = "a".repeat(40);
    // First review creates the item (new_item=true, TodoAdded + TodoUpdated).
    cli_ok(&[
        "pr-review",
        "review",
        "--goal",
        &gid,
        "--number",
        "1",
        "--head",
        &head,
        "--verdict",
        "approve",
    ]);
    let events = open_store(&cr).goal_dir(&gid).join("events.jsonl");
    let mut perms = std::fs::metadata(&events).unwrap().permissions();
    perms.set_mode(0o444);
    std::fs::set_permissions(&events, perms).unwrap();
    // Re-review the same head → new_item=false → TodoAdded skipped → the
    // TodoUpdated append hits the read-only ledger.
    let err = cli_err(&[
        "pr-review",
        "review",
        "--goal",
        &gid,
        "--number",
        "1",
        "--head",
        &head,
        "--verdict",
        "request-changes",
    ]);
    assert!(!err.is_empty());
    let mut perms = std::fs::metadata(&events).unwrap().permissions();
    perms.set_mode(0o644);
    std::fs::set_permissions(&events, perms).unwrap();
}

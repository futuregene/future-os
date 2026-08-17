//! Coverage drive (per-line 100% push) for the remaining `console.rs`
//! branches after the parse_pairs catch-all refactor: JSON output arms,
//! optional-flag branches, error edges, and subcommand dispatch catch-alls
//! that the happy-path drives never reach.

mod common;

use common::{cli_err, cli_ok, cli_root, first_todo_id, init_goal, open_store};

// ── agent list: JSON / declared workspaces / live workspace conflict ──────

#[test]
fn agent_list_json_and_workspace_conflict() {
    let cr = cli_root();
    let gid = init_goal(&cr, "agent list surface");
    // Two agents declaring the same workspace.
    cli_ok(&[
        "agent",
        "onboard",
        "--goal",
        &gid,
        "--agent-id",
        "a1",
        "--workspace",
        "/definitely/not/here/wt1",
    ]);
    cli_ok(&[
        "agent",
        "onboard",
        "--goal",
        &gid,
        "--agent-id",
        "a2",
        "--workspace",
        "/definitely/not/here/wt1",
    ]);
    // a1 claims the onboarding todo (live lease) → a2's workspace overlaps.
    let tid = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &tid,
        "--agent-id",
        "a1",
    ]);
    // Text mode renders the workspace column + the live conflict.
    cli_ok(&["agent", "list", "--goal", &gid]);
    // JSON projection.
    cli_ok(&["agent", "list", "--goal", &gid, "--format", "json"]);
    cli_ok(&["agent", "list", "--goal", &gid, "--json"]);
}

// ── authority: write-scope + require-approval branches ────────────────────

#[test]
fn authority_sets_write_scope_and_approval_gates() {
    let cr = cli_root();
    let gid = init_goal(&cr, "authority");
    cli_ok(&[
        "authority",
        "--goal",
        &gid,
        "--write-scope",
        "src,doc",
        "--require-approval",
        "publish,deploy",
    ]);
}

// ── scheduler: ack full flags / tick / show json / liveness / failure ─────

#[test]
fn scheduler_ack_with_every_flag() {
    let cr = cli_root();
    let gid = init_goal(&cr, "scheduler ack");
    cli_ok(&[
        "scheduler",
        "ack",
        "--goal",
        &gid,
        "--agent-id",
        "codex-app",
        "--action",
        "tick_next",
        "--cadence-class",
        "monitor_backoff",
        "--rrule",
        "FREQ=MINUTELY;INTERVAL=15",
        "--source",
        "scheduler_cli",
    ]);
}

#[test]
fn scheduler_tick_show_and_liveness() {
    let cr = cli_root();
    let gid = init_goal(&cr, "scheduler tick");
    // Bootstrap tick (installs state + heartbeat + monitor poll plan).
    cli_ok(&[
        "scheduler",
        "tick",
        "--goal",
        &gid,
        "--agent-id",
        "codex-app",
    ]);
    // Second tick advances progression (Some rrule).
    cli_ok(&[
        "scheduler",
        "tick",
        "--goal",
        &gid,
        "--agent-id",
        "codex-app",
    ]);
    // show text + json.
    cli_ok(&[
        "scheduler",
        "show",
        "--goal",
        &gid,
        "--agent-id",
        "codex-app",
    ]);
    cli_ok(&[
        "scheduler",
        "show",
        "--goal",
        &gid,
        "--agent-id",
        "codex-app",
        "--format",
        "json",
    ]);
    // liveness: fresh heartbeat → alive (text + json).
    cli_ok(&[
        "scheduler",
        "liveness",
        "--goal",
        &gid,
        "--agent-id",
        "codex-app",
        "--threshold-secs",
        "3600",
    ]);
    cli_ok(&[
        "scheduler",
        "liveness",
        "--goal",
        &gid,
        "--agent-id",
        "codex-app",
        "--format",
        "json",
    ]);
    // A goal with no heartbeat → no-heartbeat projection.
    let gid2 = init_goal(&cr, "scheduler liveness fresh");
    cli_ok(&[
        "scheduler",
        "liveness",
        "--goal",
        &gid2,
        "--agent-id",
        "codex-app",
    ]);
}

#[test]
fn scheduler_record_host_failure_bootstraps_state() {
    let cr = cli_root();
    let gid = init_goal(&cr, "scheduler failure");
    cli_ok(&[
        "scheduler",
        "record-host-failure",
        "--goal",
        &gid,
        "--agent-id",
        "codex-app",
        "--target-rrule",
        "FREQ=MINUTELY;INTERVAL=15",
        "--observed-rrule",
        "FREQ=HOURLY",
        "--failure-kind",
        "host_stale_rrule",
        "--failure-count",
        "2",
    ]);
}

// ── heartbeat / attention / inbox ────────────────────────────

#[test]
fn heartbeat_with_agent_id() {
    let cr = cli_root();
    let gid = init_goal(&cr, "heartbeat");
    cli_ok(&["heartbeat-prompt", "--goal", &gid, "--agent-id", "a1"]);
}

#[test]
fn attention_and_inbox_json() {
    let cr = cli_root();
    let gid = init_goal(&cr, "attention inbox");
    cli_ok(&["attention", "--goal", &gid]);
    cli_ok(&["attention", "--goal", &gid, "--format", "json"]);
    cli_ok(&["attention", "--all"]);
    cli_ok(&["inbox", "--project", &cr.cwd]);
    cli_ok(&["inbox", "--project", &cr.cwd, "--format", "json"]);
    cli_ok(&[
        "inbox",
        "--project",
        &cr.cwd,
        "--scope",
        "direct_only",
        "--name",
        "op",
    ]);
}

// ── delivery / commands / canary ───────

#[test]
fn delivery_status_display_and_subcommands() {
    let cr = cli_root();
    let gid = init_goal(&cr, "delivery status");
    // Complete the onboarding advancement todo → records a delivery outcome.
    let tid = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &tid,
        "--no-follow-up",
        "--evidence",
        "delivered: fixture output files written and verified",
    ]);
    cli_ok(&["delivery", "status", "--goal", &gid]);
    cli_ok(&["delivery", "status", "--goal", &gid, "--format", "json"]);
    // Followthrough scan (no overdue deliveries).
    cli_ok(&["delivery", "followthrough", "--goal", &gid, "--turns", "1"]);
    // Unknown subcommand.
    let err = cli_err(&["delivery", "bogus"]);
    assert!(err.contains("delivery subcommand"), "{err}");
}

#[test]
fn commands_json_and_canary_bare_smoke() {
    let _cr = cli_root();
    cli_ok(&["commands", "--format", "json"]);
    cli_ok(&["registry", "--format", "json"]);
    // Legacy bare `canary` keeps the smoke default.
    cli_ok(&["canary"]);
    cli_ok(&["canary", "smoke", "--json"]);
    cli_ok(&["canary", "smoke", "--profile", "release-gate"]);
    // premerge gate (isolated root) passes.
    cli_ok(&["canary", "premerge"]);
    cli_ok(&["canary", "premerge", "--json"]);
}

// ── run identity / quota usage --all ──────────────────────────────────────

#[test]
fn run_identity_and_quota_usage_all() {
    let cr = cli_root();
    let gid = init_goal(&cr, "run identity");
    // `run` without --agent-id or --anonymous fails with the hint.
    let err = cli_err(&["run", "--goal", &gid]);
    assert!(err.contains("--agent-id"), "{err}");
    // quota usage --all (aggregate over registered goals).
    cli_ok(&["quota", "usage", "--all"]);
    cli_ok(&["quota", "usage", "--all", "--format", "json"]);
    // quota usage without --goal or --all fails.
    let err = cli_err(&["quota", "usage"]);
    assert!(err.contains("--all"), "{err}");
}

// ── remaining subcommand flag branches + display arms ───────────────────── (scope/supervisor)

#[test]
fn scope_and_supervisor_flag_branches() {
    let cr = cli_root();
    let gid = init_goal(&cr, "remaining branches");
    // scope with an exclusion list.
    cli_ok(&[
        "scope",
        "--goal",
        &gid,
        "--agent-id",
        "a1",
        "--exclude",
        "a2,a3",
    ]);
    // supervisor propose (anchors the decision the receipt references).
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
        "--capabilities",
        "shell",
        "--summary",
        "do it",
    ]);
    // supervisor receipt with host-capabilities.
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
        "--host-capabilities",
        "shell,github",
    ]);
    // supervisor events projection.
    cli_ok(&["supervisor", "events", "--goal", &gid]);
}

#[test]
fn delivery_record_verified_and_empty_projection() {
    let cr = cli_root();
    let gid = init_goal(&cr, "delivery verified");
    // No deliveries yet → empty projection.
    cli_ok(&["delivery", "status", "--goal", &gid]);
    let tid = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &tid,
        "--no-follow-up",
        "--evidence",
        "delivered: fixture output files written and verified",
    ]);
    // Resolve the delivered signal → verified (non-pending age rendering).
    cli_ok(&[
        "delivery",
        "record",
        "--goal",
        &gid,
        "--todo-id",
        &tid,
        "--outcome",
        "verified",
        "--note",
        "looks good",
    ]);
    cli_ok(&["delivery", "status", "--goal", &gid]);
}

#[test]
fn commands_text_mode() {
    let _cr = cli_root();
    // `commands` text mode (journey rendering).
    cli_ok(&["commands"]);
    let incomplete = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        incomplete.path(),
        serde_json::json!({ "pull_requests": [], "result_completeness": { "complete": false } })
            .to_string(),
    )
    .unwrap();
    // No pull requests → candidate none.
    let empty = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        empty.path(),
        serde_json::json!({ "pull_requests": [] }).to_string(),
    )
    .unwrap();
}

#[test]
fn scheduler_tick_projects_due_and_future_monitors() {
    let cr = cli_root();
    let gid = init_goal(&cr, "monitor poll plan");
    let now = future_loop::state::now_epoch();
    // A future monitor → "none due (next poll in …)".
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: gid.clone(),
            todo: future_loop::state::Todo::monitor(
                "mon_future",
                "watch later",
                std::time::Duration::from_secs(3600),
            ),
            ts: now,
        })
        .unwrap();
    drop(store);
    cli_ok(&[
        "scheduler",
        "tick",
        "--goal",
        &gid,
        "--agent-id",
        "codex-app",
    ]);
    // A due monitor → the "N due" poll-plan display.
    let mut store = open_store(&cr);
    store
        .append(future_loop::store::Event::TodoAdded {
            goal_id: gid.clone(),
            todo: future_loop::state::Todo::monitor(
                "mon_due",
                "watch now",
                std::time::Duration::from_secs(0),
            ),
            ts: now,
        })
        .unwrap();
    drop(store);
    cli_ok(&[
        "scheduler",
        "tick",
        "--goal",
        &gid,
        "--agent-id",
        "codex-app",
    ]);
}

#[test]
fn store_verify_repair_and_bridge() {
    let cr = cli_root();
    let gid = init_goal(&cr, "store verify repair");
    cli_ok(&["store", "verify", "--goal", &gid, "--repair"]);
    cli_ok(&["store", "verify", "--goal", &gid, "--format", "json"]);
    cli_ok(&["store", "bridge", "--goal", &gid]);
}

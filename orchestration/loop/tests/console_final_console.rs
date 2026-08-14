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
        "--capability",
        "shell",
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

// ── pr-review: dispatch catch-all + queue text + verdict json + claim + recommend ──

fn pr_fixture_file(number: u64, head: &str, with_previous: bool) -> tempfile::NamedTempFile {
    let f = tempfile::NamedTempFile::new().unwrap();
    let mut payload = serde_json::json!({
        "repository": "owner/repo",
        "pull_requests": [
            {
                "number": number,
                "title": format!("PR {number}"),
                "url": format!("https://github.com/owner/repo/pull/{number}"),
                "state": "OPEN",
                "head_oid": head,
                "review_decision": "REVIEW_REQUIRED",
                "is_draft": false,
                "merge_state": "CLEAN",
                "checks": {"counts": {"success": 1, "failure": 0, "pending": 0, "unknown": 0}, "failures": [], "pending": []}
            }
        ]
    });
    if with_previous {
        payload["previous_observation"] = serde_json::json!({
            "pull_requests": [
                {"number": number, "head_oid": "0".repeat(40)}
            ]
        });
    }
    std::fs::write(f.path(), payload.to_string()).unwrap();
    f
}

#[test]
fn pr_review_unknown_subcommand() {
    let _cr = cli_root();
    let err = cli_err(&["pr-review", "bogus"]);
    assert!(err.contains("unknown pr-review subcommand"), "{err}");
    let err = cli_err(&["pr-review"]);
    assert!(err.contains("pr-review requires a subcommand"), "{err}");
}

#[test]
fn pr_review_queue_text_and_input_branches() {
    let cr = cli_root();
    let gid = init_goal(&cr, "pr review queue");
    let f = pr_fixture_file(1, &"a".repeat(40), true);
    // Text mode with a previous observation → candidate + changed/removed.
    cli_ok(&[
        "pr-review",
        "queue",
        "--fixture",
        f.path().to_str().unwrap(),
        "--repo",
        "owner/repo",
        "--goal",
        &gid,
    ]);
    // Inline --input payload.
    let input = serde_json::json!({"pull_requests": [{"number": 2, "head_oid": "b".repeat(40)}]})
        .to_string();
    cli_ok(&["pr-review", "queue", "--input", &input]);
    // A handled cursor is validated against the observed candidate; a bogus
    // one is rejected (the parse_pairs push still runs).
    let handled = format!("1@{}", "a".repeat(40));
    cli_err(&[
        "pr-review",
        "queue",
        "--input",
        &input,
        "--handled-exact-head",
        &handled,
    ]);
    // JSON projection.
    cli_ok(&[
        "pr-review",
        "queue",
        "--fixture",
        f.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    // previous-observation from a separate file.
    let prev = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        prev.path(),
        serde_json::json!({"pull_requests": [{"number": 1, "head_oid": "c".repeat(40)}]})
            .to_string(),
    )
    .unwrap();
    cli_ok(&[
        "pr-review",
        "queue",
        "--fixture",
        f.path().to_str().unwrap(),
        "--previous-observation-json",
        prev.path().to_str().unwrap(),
    ]);
    // No payload source → error.
    let err = cli_err(&["pr-review", "queue"]);
    assert!(err.contains("--fixture"), "{err}");
}

#[test]
fn pr_review_verdict_json_and_claim_errors() {
    let cr = cli_root();
    let gid = init_goal(&cr, "pr review verdict");
    let head = "d".repeat(40);
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
        "--format",
        "json",
    ]);
    // Claiming a missing work item fails (no open review item exists).
    let err = cli_err(&["pr-review", "claim", "--goal", &gid, "--number", "99"]);
    assert!(err.contains("not found"), "{err}");
    // Re-verdict the same head (supersede path stays false).
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
        "request-changes",
    ]);
    // A new exact head supersedes.
    let head2 = "e".repeat(40);
    cli_ok(&[
        "pr-review",
        "review",
        "--goal",
        &gid,
        "--number",
        "1",
        "--head",
        &head2,
        "--verdict",
        "rework",
        "--reviewer",
        "alice",
        "--comment",
        "fix it",
        "--repo",
        "owner/repo",
    ]);
}

#[test]
fn pr_review_recommend_branches() {
    let _cr = cli_root();
    // Empty candidate set (no owner mapping / commits).
    cli_ok(&["pr-review", "recommend", "--path", "src/lib.rs"]);
    // JSON projection.
    cli_ok(&[
        "pr-review",
        "recommend",
        "--path",
        "src/lib.rs",
        "--format",
        "json",
    ]);
    // With a CODEOWNERS file → owner mapping candidates.
    let co = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(co.path(), "src/*.rs @alice\n").unwrap();
    cli_ok(&[
        "pr-review",
        "recommend",
        "--path",
        "src/lib.rs",
        "--codeowners",
        co.path().to_str().unwrap(),
        "--since-days",
        "7",
    ]);
    // With an owners root (empty dir → no OWNERS files).
    let dir = tempfile::tempdir().unwrap();
    cli_ok(&[
        "pr-review",
        "recommend",
        "--path",
        "src/lib.rs",
        "--owners-root",
        dir.path().to_str().unwrap(),
    ]);
    // With a repo dir (empty → no commits).
    let repo = tempfile::tempdir().unwrap();
    cli_ok(&[
        "pr-review",
        "recommend",
        "--path",
        "src/lib.rs",
        "--repo-dir",
        repo.path().to_str().unwrap(),
    ]);
    // No paths → error.
    let err = cli_err(&["pr-review", "recommend"]);
    assert!(err.contains("--path"), "{err}");
}

// ── heartbeat / capability / attention / inbox ────────────────────────────

#[test]
fn heartbeat_with_agent_id() {
    let cr = cli_root();
    let gid = init_goal(&cr, "heartbeat");
    cli_ok(&["heartbeat-prompt", "--goal", &gid, "--agent-id", "a1"]);
}

#[test]
fn capability_commands_experimental_hint() {
    let cr = cli_root();
    // An experimental capability's commands are hidden without the flag.
    cli_ok(&["capability", "commands", "--name", "auto_research"]);
    // With the flag they show.
    cli_ok(&[
        "capability",
        "commands",
        "--name",
        "auto_research",
        "--include-experimental",
    ]);
    // A capability hook with a goal context (quota ledger).
    let gid = init_goal(&cr, "capability quota");
    cli_ok(&["issue-fix", "--input", "it broke", "--goal", &gid]);
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

// ── delivery / reward-memory / decision-context / commands / canary ───────

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
fn reward_memory_query_and_record_branches() {
    let cr = cli_root();
    let gid = init_goal(&cr, "reward memory");
    // Empty projection + JSON.
    cli_ok(&["reward-memory", "query", "--goal", &gid]);
    cli_ok(&["reward-memory", "query", "--goal", &gid, "--format", "json"]);
    // Invalid source.
    let err = cli_err(&[
        "reward-memory",
        "query",
        "--goal",
        &gid,
        "--source",
        "bogus",
    ]);
    assert!(err.contains("--source must be one of"), "{err}");
    // Record a signal, then query with a scope filter.
    let tid = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "reward-memory",
        "record",
        "--goal",
        &gid,
        "--todo-id",
        &tid,
        "--score",
        "0.5",
        "--source",
        "evidence",
        "--note",
        "n",
        "--agent-id",
        "a1",
    ]);
    cli_ok(&[
        "reward-memory",
        "query",
        "--goal",
        &gid,
        "--source",
        "evidence",
    ]);
    cli_ok(&[
        "reward-memory",
        "query",
        "--goal",
        &gid,
        "--agent-id",
        "a1",
        "--todo-id",
        &tid,
    ]);
    // Unknown subcommand.
    let err = cli_err(&["reward-memory", "bogus"]);
    assert!(err.contains("reward-memory subcommand"), "{err}");
}

#[test]
fn decision_context_branches() {
    let cr = cli_root();
    let gid = init_goal(&cr, "decision context");
    // Unknown subcommand.
    let err = cli_err(&["decision-context", "bogus"]);
    assert!(err.contains("decision-context subcommand"), "{err}");
    // assemble text + json.
    cli_ok(&["decision-context", "assemble", "--goal", &gid]);
    cli_ok(&[
        "decision-context",
        "assemble",
        "--goal",
        &gid,
        "--format",
        "json",
    ]);
    // outcomes (empty read model).
    cli_ok(&["decision-context", "outcomes", "--goal", &gid]);
    cli_ok(&[
        "decision-context",
        "outcomes",
        "--goal",
        &gid,
        "--format",
        "json",
    ]);
    // feedback fails closed (no decision summary anchors the turn).
    let err = cli_err(&[
        "decision-context",
        "feedback",
        "--goal",
        &gid,
        "--turn",
        "1",
        "--status",
        "verified",
        "--agent-id",
        "a1",
        "--context-digest",
        "d1",
    ]);
    assert!(!err.is_empty());
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

// ── remaining subcommand flag branches + display arms ─────────────────────

#[test]
fn capability_propose_and_scope_supervisor_branches() {
    let cr = cli_root();
    let gid = init_goal(&cr, "remaining branches");
    // capability propose with a goal context (quota ledger at the boundary).
    cli_ok(&[
        "capability",
        "propose",
        "--name",
        "issue_fix",
        "--input",
        "it broke",
        "--goal",
        &gid,
    ]);
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
fn commands_text_mode_and_incomplete_pr_queue() {
    let _cr = cli_root();
    // `commands` text mode (journey rendering).
    cli_ok(&["commands"]);
    // pr-review queue with an incomplete read → "NOT observed".
    let incomplete = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        incomplete.path(),
        serde_json::json!({ "pull_requests": [], "result_completeness": { "complete": false } })
            .to_string(),
    )
    .unwrap();
    cli_ok(&[
        "pr-review",
        "queue",
        "--fixture",
        incomplete.path().to_str().unwrap(),
    ]);
    // No pull requests → candidate none.
    let empty = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        empty.path(),
        serde_json::json!({ "pull_requests": [] }).to_string(),
    )
    .unwrap();
    cli_ok(&[
        "pr-review",
        "queue",
        "--fixture",
        empty.path().to_str().unwrap(),
    ]);
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
fn decision_context_assemble_with_open_acceptance_gaps() {
    let cr = cli_root();
    let gid = format!(
        "goal_gaps_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let mut store = open_store(&cr);
    let goal = future_loop::state::Goal::new(&gid, "objective", &cr.cwd)
        .with_acceptance(vec![("A1", "result matches tolerance")]);
    store.register(&goal).unwrap();
    store
        .append(future_loop::store::Event::GoalStarted {
            goal_id: gid.clone(),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    drop(store);
    // assemble renders the open acceptance gaps.
    cli_ok(&["decision-context", "assemble", "--goal", &gid]);
}

#[test]
fn pr_review_queue_removed_prs() {
    let _cr = cli_root();
    let f = tempfile::NamedTempFile::new().unwrap();
    // Previous observation had PR 99; the current payload drops it → removed.
    std::fs::write(
        f.path(),
        serde_json::json!({
            "repository": "owner/repo",
            "pull_requests": [{"number": 1, "head_oid": "a".repeat(40)}],
            "previous_observation": {"pull_requests": [
                {"number": 99, "head_oid": "9".repeat(40)},
                {"number": 1, "head_oid": "0".repeat(40)}
            ]}
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

#[test]
fn store_verify_repair_and_bridge() {
    let cr = cli_root();
    let gid = init_goal(&cr, "store verify repair");
    cli_ok(&["store", "verify", "--goal", &gid, "--repair"]);
    cli_ok(&["store", "verify", "--goal", &gid, "--format", "json"]);
    cli_ok(&["store", "bridge", "--goal", &gid]);
}

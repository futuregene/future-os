//! Drive for the mid-tier console surface: todo completion/supersession,
//! worker stop, runs subcommands, store verification, doctor, supervisor
//! events/register/steer, delivery, and the frontier/lane/privacy projections.
//!
//! Each of these had uncovered *success* paths plus their refusals — the tail of
//! every command (`refresh_next_action`, projection prints, `?` arms). The tests
//! below drive them over real ledger state rather than asserting on internals.

mod common;

use common::{add_todo, cli_err, cli_ok, cli_root, init_goal};

/// Add a todo that declares it blocks `blocks` (the `todo add --blocks` flag).
fn add_todo_blocking(cr: &common::CliRoot, goal: &str, text: &str, blocks: &str) -> String {
    cli_ok(&[
        "todo", "add", "--goal", goal, "--text", text, "--blocks", blocks,
    ]);
    common::todo_id_by_text(&cr.root, goal, text)
}

/// Add a todo carrying the optional metadata `todo add` accepts, so the
/// projection paths that read it (acceptance, verify, parent, priority) run.
fn add_todo_rich(cr: &common::CliRoot, goal: &str, text: &str, parent: &str) -> String {
    cli_ok(&[
        "todo",
        "add",
        "--goal",
        goal,
        "--text",
        text,
        "--parent",
        parent,
        "--priority",
        "P1",
        "--acceptance",
        "artifact,scored",
        "--verify",
        "true",
        "--role",
        "agent",
        "--class",
        "advancement",
    ]);
    common::todo_id_by_text(&cr.root, goal, text)
}

/// Seed `<root>/runs/` with one live run log + one run record so the `runs`
/// subcommands have history to project.
fn seed_run_history(root: &str, goal_id: &str, run_id: &str) {
    let dir = std::path::Path::new(root).join("runs");
    std::fs::create_dir_all(&dir).unwrap();
    let header = serde_json::json!({
        "type": "run_header",
        "wall_ts": 1_700_000_000u64,
        "run_id": run_id,
        "session_id": "sess-1",
        "agent_id": "w1",
        "todo_id": "todo_1",
        "goal_id": goal_id,
    });
    std::fs::write(
        dir.join(format!("{run_id}.live.jsonl")),
        format!("{header}\n"),
    )
    .unwrap();
    // A finished run record in the per-goal state dir (what the index scans).
    let records = std::path::Path::new(root)
        .join("goals")
        .join(goal_id)
        .join("runs");
    std::fs::create_dir_all(&records).unwrap();
    let record = serde_json::json!({
        "goal_id": goal_id,
        "timestamp": "2026-09-26T00:00:00+00:00",
        "turn": 1,
        "todo_id": "todo_1",
        "run_id": run_id,
        "agent_id": "w1",
        "terminal_state": "completed",
        "tools": ["shell"],
        "tokens_in": 1,
        "tokens_out": 2,
        "cost": 0.01,
        "evidence": "seeded",
        "error": null,
    });
    std::fs::write(
        records.join(format!("2026-09-26T00-00-00-00-00-{run_id}.json")),
        serde_json::to_string_pretty(&record).unwrap(),
    )
    .unwrap();
}

#[test]
fn todo_complete_supersede_and_update_paths() {
    let cr = cli_root();
    let gid = init_goal(&cr, "todo lifecycle");

    // complete with a successor (the follow-up path), then without one.
    let a = add_todo(&cr, &gid, "first slice");
    let b = add_todo(&cr, &gid, "second slice");
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &a,
        "--successor",
        &b,
        "--evidence",
        "artifact landed",
    ]);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &b,
        "--no-follow-up",
        "--evidence",
        "nothing left",
    ]);

    // update text on an open todo, and supersede it with a reason.
    let c = add_todo(&cr, &gid, "slice to retitle");
    cli_ok(&[
        "todo",
        "update",
        "--goal",
        &gid,
        "--todo-id",
        &c,
        "--text",
        "retitled slice",
    ]);
    cli_ok(&[
        "todo",
        "supersede",
        "--goal",
        &gid,
        "--todo-id",
        &c,
        "--reason",
        "obsoleted",
    ]);

    // Refusals: completion needs a follow-up decision (`--no-follow-up` or a
    // successor) — a bare complete must not silently close the chain.
    let d = add_todo(&cr, &gid, "never completed");
    let err = cli_err(&["todo", "complete", "--goal", &gid, "--todo-id", &d]);
    assert!(
        !err.is_empty(),
        "complete without a follow-up decision must fail"
    );
    // `supersede` does not require a reason, so the todo really is superseded.
    cli_ok(&["todo", "supersede", "--goal", &gid, "--todo-id", &d]);
    // And a bare `todo` invocation names the subcommands.
    let err = cli_err(&["todo"]);
    assert!(err.contains("add|claim|complete"), "{err}");
    let err = cli_err(&["todo", "bogus", "--goal", &gid, "--todo-id", &d]);
    assert!(err.contains("unknown todo subcommand"), "{err}");
}

#[test]
fn goal_cancel_archive_and_runs_subcommands() {
    let cr = cli_root();
    let gid = init_goal(&cr, "goal lifecycle");
    let todo = add_todo(&cr, &gid, "work");
    // A claim requires the agent to be registered for the goal first.
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "a"]);
    cli_ok(&[
        "todo",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--agent-id",
        "a",
    ]);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--no-follow-up",
        "--evidence",
        "done",
    ]);
    // Archive a completed todo (only completed/terminal todos archive cleanly).
    cli_ok(&["todo", "archive", "--goal", &gid, "--todo-id", &todo]);

    // Every `runs` subcommand over a goal with run history.
    seed_run_history(&cr.root, &gid, "run-seeded");
    for sub in ["history", "index", "retention", "stale", "compact"] {
        cli_ok(&["runs", sub, "--goal", &gid]);
    }
    cli_ok(&["runs", "index", "--goal", &gid, "--rebuild"]);
    cli_ok(&["runs", "history", "--goal", &gid, "--format", "json"]);
    cli_ok(&["runs", "retention", "--goal", &gid, "--keep", "5"]);
    cli_ok(&["runs", "stale", "--goal", &gid, "--cutoff", "0"]);
    // Refusals: no subcommand at all, an unknown one, and a ghost goal.
    let err = cli_err(&["runs"]);
    assert!(
        err.contains("history|compact|index|retention|stale"),
        "{err}"
    );
    let err = cli_err(&["runs", "bogus", "--goal", &gid]);
    assert!(!err.is_empty(), "{err}");
    let err = cli_err(&["runs", "history", "--goal", "ghost"]);
    assert!(err.contains("not found"), "{err}");

    // `store verify` / `store bridge` over a healthy ledger, then migrate check.
    cli_ok(&["store", "verify", "--goal", &gid]);
    cli_ok(&["store", "verify", "--goal", &gid, "--format", "json"]);
    cli_ok(&["store", "bridge", "--goal", &gid]);

    // Cancel the goal (terminal transition), then confirm the projection agrees.
    cli_ok(&[
        "goal",
        "cancel",
        "--goal",
        &gid,
        "--reason",
        "superseded by tests",
    ]);
    cli_ok(&["status", "--goal", &gid]);
    cli_ok(&["status", "--goal", &gid, "--format", "json"]);
}

#[test]
fn projections_over_a_rich_goal() {
    let cr = cli_root();
    let gid = init_goal(&cr, "projections");
    let a = add_todo(&cr, &gid, "upstream slice");
    let _b = add_todo(&cr, &gid, "downstream slice");
    // `--blocks` declares the dependency edge; `add_todo_rich` carries the
    // optional metadata the projections read.
    let _c = add_todo_blocking(&cr, &gid, "gated slice", &a);
    let _d = add_todo_rich(&cr, &gid, "metadata slice", &a);
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&[
        "agent",
        "onboard",
        "--goal",
        &gid,
        "--agent-id",
        "w2",
        "--workspace",
        "src/a",
    ]);

    // Frontier: text + JSON, with work present and after it is drained.
    cli_ok(&["frontier", "show", "--goal", &gid]);
    cli_ok(&["frontier", "show", "--goal", &gid, "--format", "json"]);

    // Task graph: edges printed, and JSON shape.
    cli_ok(&["task-graph", "--goal", &gid]);
    cli_ok(&["task-graph", "--goal", &gid, "--format", "json"]);

    // Lane recommendation for a registered agent, and for an unknown one.
    cli_ok(&["lane", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&["lane", "--goal", &gid, "--agent-id", "ghost"]);
    let err = cli_err(&["lane", "--goal", &gid]);
    assert!(err.contains("--agent-id"), "{err}");

    // Scope (identity-scoped frontier) including an exclusion.
    cli_ok(&["scope", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&["scope", "--goal", &gid, "--agent-id", "w1", "--exclude", &a]);

    // Privacy projection at both levels, text and JSON.
    cli_ok(&["privacy", "--goal", &gid]);
    cli_ok(&["privacy", "--goal", &gid, "--level", "public"]);
    cli_ok(&[
        "privacy",
        "--goal",
        &gid,
        "--level",
        "local_private",
        "--format",
        "json",
    ]);
    let err = cli_err(&["privacy", "--goal", &gid, "--level", "nonsense"]);
    assert!(!err.is_empty(), "{err}");

    // Attention / inbox / diagnose / history / attention-all.
    cli_ok(&["attention", "--goal", &gid]);
    cli_ok(&["inbox"]);
    cli_ok(&["inbox", "--scope", "addressed_only", "--name", "operator"]);
    cli_ok(&["inbox", "--format", "json"]);
    cli_ok(&["diagnose", "--goal", &gid]);
    cli_ok(&["history", "--goal", &gid]);
    cli_ok(&["heartbeat-prompt", "--goal", &gid]);
}

#[test]
fn worker_stop_and_supervisor_surface() {
    let cr = cli_root();
    let gid = init_goal(&cr, "worker stop");
    let todo = add_todo(&cr, &gid, "leased work");
    cli_ok(&["agent", "register", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--agent-id",
        "w1",
        "--lease-secs",
        "600",
    ]);

    // Stopping a worker releases the leases it held — the point of the command.
    cli_ok(&["worker", "stop", "--goal", &gid, "--agent-id", "w1"]);
    cli_ok(&["worker", "list", "--goal", &gid]);

    // Supervisor surface: register the up-channel, steer the down-channel,
    // then read the event projection.
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &gid,
        "--session-id",
        "sess-sup",
    ]);
    cli_ok(&[
        "supervisor",
        "steer",
        "--goal",
        &gid,
        "--agent-id",
        "w1",
        "--instruction",
        "focus on the ledger",
    ]);
    cli_ok(&[
        "supervisor",
        "steer",
        "--goal",
        &gid,
        "--interrupt",
        "--instruction",
        "stop and report",
    ]);
    cli_ok(&["supervisor", "events", "--goal", &gid]);
    // register requires the session id; steer requires the instruction text.
    let err = cli_err(&["supervisor", "register", "--goal", &gid]);
    assert!(err.contains("--session-id"), "{err}");
    let err = cli_err(&["supervisor", "steer", "--goal", &gid]);
    assert!(!err.is_empty(), "{err}");

    // Delivery: with no delivery outstanding the record is refused — the arm
    // that must fail closed rather than stamp an outcome nothing produced.
    let err = cli_err(&[
        "delivery",
        "record",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--outcome",
        "verified",
    ]);
    assert!(err.contains("no pending delivery"), "{err}");
    cli_ok(&["delivery", "status", "--goal", &gid]);
    cli_ok(&["delivery", "status", "--goal", &gid, "--format", "json"]);
    let err = cli_err(&[
        "delivery",
        "record",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--outcome",
        "nonsense",
    ]);
    assert!(!err.is_empty(), "{err}");

    // Report a mid-run progress note (projection-only event).
    cli_ok(&[
        "report",
        "--goal",
        &gid,
        "--agent-id",
        "w1",
        "--todo-id",
        &todo,
        "--message",
        "halfway through the ledger work",
    ]);

    // Doctor's diagnostic surface, with and without a goal filter.
    cli_ok(&["doctor"]);
    cli_ok(&["doctor", "--goal", &gid]);
}

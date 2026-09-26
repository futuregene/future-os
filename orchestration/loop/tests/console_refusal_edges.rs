//! Refusal edges of the `--goal`-scoped console commands.
//!
//! Every command that resolves a goal has the same two guards: a missing
//! `--goal` and a goal the ledger does not know. They are one line each, which
//! is exactly the kind of arm that never gets a test — and exactly the arm that
//! decides whether a typo'd goal id fails with an actionable message or panics.
//!
//! The table is explicit rather than generated: each entry names the command,
//! its arguments, and the message the operator must see, so a regression in any
//! one command fails on that entry.

mod common;

use common::{cli_err, cli_root, init_goal};

/// `(args, expected error substring)`.
const NO_GOAL: &[(&[&str], &str)] = &[
    (&["history"], "--goal required"),
    (&["todo-event"], "--goal required"),
    (&["evidence-log"], "--goal required"),
    (&["turn"], "--goal required"),
    (&["heartbeat-prompt"], "--goal required"),
    (&["scope"], "--goal required"),
    (&["lane"], "--goal required"),
    (&["task-graph"], "--goal required"),
    (&["delivery", "status"], "--goal required"),
    (&["diagnose"], "--goal required"),
    (&["quota", "usage"], "requires --goal"),
    (&["frontier", "show"], "--goal required"),
    (&["replan", "show"], "--goal required"),
    (&["gate", "list"], "--goal required"),
    (&["report"], "--goal required"),
    (&["run"], "--goal required"),
];

#[test]
fn goal_scoped_commands_refuse_a_missing_goal() {
    let cr = cli_root();
    let _gid = init_goal(&cr, "refusal edges");
    for (args, expected) in NO_GOAL {
        let err = cli_err(args);
        assert!(
            err.contains(expected),
            "{args:?} should report {expected:?}, got: {err}"
        );
    }
}

#[test]
fn goal_scoped_commands_refuse_an_unknown_goal() {
    let cr = cli_root();
    let _gid = init_goal(&cr, "refusal edges");
    let ghost = "goal_does_not_exist";
    // Collect every offender before failing, so one run reports the whole set
    // rather than stopping at the first command that accepts a bogus goal.
    let mut accepted = Vec::new();
    for (args, _) in NO_GOAL {
        // `evidence-log` is deliberately not in this set: it is a projection over
        // the ledger, so an unknown goal yields an empty trail rather than an
        // error (asserted in `evidence_log_projects_an_unknown_goal_as_empty`).
        if args.first() == Some(&"evidence-log") {
            continue;
        }
        let mut with_goal: Vec<&str> = args.to_vec();
        with_goal.push("--goal");
        with_goal.push(ghost);
        if let Ok(()) = common::cli(&with_goal) {
            accepted.push(with_goal.join(" "));
        }
    }
    assert!(
        accepted.is_empty(),
        "these commands accepted a goal that does not exist: {accepted:#?}"
    );
}

/// The exception above, pinned as behaviour: `evidence-log` must still demand
/// `--goal` (it is unscoped otherwise) but must not fail for a goal that has no
/// evidence — an operator reading an empty trail is the correct outcome.
#[test]
fn evidence_log_projects_an_unknown_goal_as_empty() {
    let cr = cli_root();
    let _gid = init_goal(&cr, "evidence-log edges");
    let err = cli_err(&["evidence-log"]);
    assert!(err.contains("--goal required"), "{err}");
    common::cli_ok(&["evidence-log", "--goal", "goal_does_not_exist"]);
    common::cli_ok(&[
        "evidence-log",
        "--goal",
        "goal_does_not_exist",
        "--format",
        "json",
    ]);
}

#[test]
fn lease_and_todo_commands_refuse_an_unknown_todo() {
    let cr = cli_root();
    let gid = init_goal(&cr, "todo refusal edges");
    // These all resolve the todo through the ledger and must name the miss
    // instead of panicking or silently claiming a non-existent todo.
    for sub in ["status", "claim", "renew", "release", "expire"] {
        let err = cli_err(&[
            "lease",
            sub,
            "--goal",
            &gid,
            "--todo-id",
            "todo_ghost",
            "--agent-id",
            "a",
        ]);
        assert!(
            err.contains("todo_ghost"),
            "lease {sub} should name the unknown todo, got: {err}"
        );
    }
    let err = cli_err(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        "todo_ghost",
    ]);
    assert!(
        err.contains("todo_ghost") || err.contains("not found"),
        "todo complete should refuse an unknown todo, got: {err}"
    );
}

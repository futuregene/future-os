//! Flag-validation table for the whole console surface.
//!
//! Every subcommand starts by rejecting flags it does not understand, and that
//! rejection is a single `?`-arm per command — the kind of arm that never gets a
//! test, and the one that decides whether a typo (`--gaol` for `--goal`) fails
//! with an actionable message or is silently ignored and the run proceeds with
//! defaults.
//!
//! The table is explicit so a regression in one command fails on that entry, and
//! it collects *all* offenders before failing so one run reports the whole set.

mod common;

use common::{cli, cli_err, cli_root, init_goal};

/// Every flag-taking entry point, with an unknown flag appended. `reject_unknown_flags`
/// runs before any other parsing, so no other argument is needed for the command to
/// reach — and fail at — its flag check.
const UNKNOWN_FLAG_INVOCATIONS: &[&[&str]] = &[
    &["goal", "init"],
    &["goal", "cancel"],
    &["goal", "delete"],
    &["todo", "add"],
    &["todo", "claim"],
    &["todo", "complete"],
    &["todo", "archive"],
    &["todo", "supersede"],
    &["todo", "update"],
    &["agent", "register"],
    &["agent", "onboard"],
    &["agent", "list"],
    &["gate", "resolve"],
    &["backup"],
    &["authority", "set"],
    &["replan", "show"],
    &["replan", "ack"],
    &["replan", "rules", "show"],
    &["frontier", "show"],
    &["profile", "set"],
    &["status"],
    &["quota", "decisions"],
    &["quota", "should-run"],
    &["quota", "usage"],
    &["quota", "spend"],
    &["scheduler", "ack"],
    &["scheduler", "tick"],
    &["scheduler", "liveness"],
    &["scheduler", "show"],
    &["scheduler", "record-host-failure"],
    &["models"],
    &["store", "verify"],
    &["backfill"],
    &["privacy"],
    &["lease", "status"],
    &["runs", "history"],
    &["heartbeat-prompt"],
    &["worker", "tail"],
    &["worker", "list"],
    &["worker", "stop"],
    &["scope"],
    &["lane"],
    &["supervisor", "watch"],
    &["supervisor", "events"],
    &["supervisor", "register"],
    &["supervisor", "steer"],
    &["report"],
    &["task-graph"],
    &["attention"],
    &["inbox"],
    &["delivery", "status"],
    &["delivery", "record"],
    &["delivery", "followthrough"],
    &["registry"],
    &["commands"],
    &["canary", "smoke"],
    &["canary", "premerge"],
    &["diagnose"],
    &["doctor"],
    &["history"],
    &["turn"],
    &["todo-event"],
    &["evidence-log"],
];

#[test]
fn every_subcommand_rejects_an_unknown_flag() {
    let cr = cli_root();
    let gid = init_goal(&cr, "unknown flag matrix");
    let mut accepted: Vec<String> = Vec::new();
    let mut wrong_message: Vec<String> = Vec::new();
    for path in UNKNOWN_FLAG_INVOCATIONS {
        let mut args: Vec<&str> = path.to_vec();
        args.push("--zzz-not-a-flag");
        match cli(&args) {
            Ok(()) => accepted.push(args.join(" ")),
            Err(err) => {
                if !err.contains("unknown flag") {
                    wrong_message.push(format!("{} -> {err}", args.join(" ")));
                }
            }
        }
    }
    assert!(
        accepted.is_empty(),
        "these entry points silently accepted a flag they do not implement: {accepted:#?}"
    );
    assert!(
        wrong_message.is_empty(),
        "these rejected the flag without naming it: {wrong_message:#?}"
    );

    // A typo'd `--goal` is the field failure this guard exists for: the command
    // must refuse it rather than defaulting the goal and mutating the wrong one.
    for typo in ["--gaol", "--Goal", "--goal_"] {
        let err = cli_err(&["status", typo, &gid]);
        assert!(
            err.contains("unknown flag"),
            "{typo} must be refused, got: {err}"
        );
    }
}

/// The refusal must name the offending flag, not just fail, and `--help` must
/// stay usable even when other arguments are unknown.
#[test]
fn unknown_flag_error_names_the_flag_and_help_still_works() {
    let cr = cli_root();
    let _gid = init_goal(&cr, "flag messages");
    let err = cli_err(&["status", "--zzz"]);
    assert!(err.contains("--zzz"), "the flag must be named: {err}");
    assert!(
        err.contains("--help"),
        "the message should point at `--help`: {err}"
    );

    // `--help` is a global escape hatch: it is accepted (and prints usage)
    // rather than being reported as an unknown flag.
    for args in [
        vec!["--help"],
        vec!["status", "--help"],
        vec!["todo", "add", "--help"],
    ] {
        let outcome = cli(&args);
        assert!(
            outcome.is_ok(),
            "{args:?} must be accepted: {:?}",
            outcome.err()
        );
    }
}

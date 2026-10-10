//! `lease claim|status|renew|release|expire` lifecycle over the real CLI.
//!
//! The unit tests in `work_items::task_lease` pin the state machine; these drive
//! it the way an operator does — through argument parsing, the ledger, and the
//! projection sync — so the success paths (including the events each subcommand
//! appends) are exercised, not just the refusals.

mod common;

use common::{add_todo, cli_err, cli_ok, cli_root, init_goal};

#[test]
fn claim_status_renew_release_lifecycle() {
    let cr = cli_root();
    let gid = init_goal(&cr, "lease lifecycle");
    let todo = add_todo(&cr, &gid, "work worth leasing");

    // Free before anyone claims it.
    cli_ok(&["lease", "status", "--goal", &gid, "--todo-id", &todo]);
    cli_ok(&[
        "lease",
        "status",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--format",
        "json",
    ]);

    // Claim, then the same owner may re-claim idempotently…
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--agent-id",
        "a",
        "--lease-secs",
        "60",
    ]);
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--agent-id",
        "a",
        "--lease-secs",
        "60",
    ]);

    // …while a different owner is refused, and `--force` does NOT change that:
    // per the flag's own help it overrides the *workspace* conflict guard, not a
    // live lease (only `expire` clears one). Asserting the message keeps the two
    // guards from being conflated again.
    for extra in [None, Some("--force")] {
        let mut args = vec![
            "lease",
            "claim",
            "--goal",
            &gid,
            "--todo-id",
            &todo,
            "--agent-id",
            "b",
            "--lease-secs",
            "60",
        ];
        if let Some(flag) = extra {
            args.push(flag);
        }
        let err = cli_err(&args);
        assert!(
            err.contains("active lease held by another agent"),
            "{args:?} should refuse a live lease held by `a`, got: {err}"
        );
    }

    // A non-owner cannot renew or release either — those are owner-scoped.
    for sub in ["renew", "release"] {
        let err = cli_err(&[
            "lease",
            sub,
            "--goal",
            &gid,
            "--todo-id",
            &todo,
            "--agent-id",
            "b",
            "--lease-secs",
            "120",
        ]);
        assert!(!err.is_empty(), "lease {sub} by a non-owner must fail");
    }

    // Renew as the owner (success path: appends TodoRenewed).
    cli_ok(&[
        "lease",
        "renew",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--agent-id",
        "a",
        "--lease-secs",
        "120",
    ]);

    // `expire` is fail-closed: it must NOT clear a lease that is still live
    // (that would let a third party steal work mid-turn).
    let err = cli_err(&["lease", "expire", "--goal", &gid, "--todo-id", &todo]);
    assert!(err.contains("still active"), "{err}");

    // Owner releases voluntarily.
    cli_ok(&[
        "lease",
        "release",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--agent-id",
        "a",
    ]);

    // The recovery path: a short lease lapses, `status` reports EXPIRED, and the
    // slice can then be expired/claimed by someone else.
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--agent-id",
        "c",
        "--lease-secs",
        "1",
    ]);
    std::thread::sleep(std::time::Duration::from_secs(2));
    cli_ok(&["lease", "status", "--goal", &gid, "--todo-id", &todo]);
    cli_ok(&[
        "lease",
        "status",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--format",
        "json",
    ]);
    cli_ok(&["lease", "expire", "--goal", &gid, "--todo-id", &todo]);
    cli_ok(&[
        "lease",
        "claim",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--agent-id",
        "d",
        "--lease-secs",
        "60",
    ]);

    // An unknown subcommand is refused with the allowed set.
    let err = cli_err(&["lease", "bogus", "--goal", &gid, "--todo-id", &todo]);
    assert!(err.contains("claim|renew|release|expire|status"), "{err}");

    // The ledger really carries the lifecycle: claim/renew/release/expire events.
    let store = common::open_store(&cr);
    let kinds: Vec<String> = store
        .events(&gid)
        .unwrap()
        .into_iter()
        .map(|e| {
            format!("{:?}", e.event)
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string()
        })
        .collect();
    for expected in ["TodoClaimed", "TodoRenewed", "TodoReleased", "TodoExpired"] {
        assert!(
            kinds.iter().any(|k| k.contains(expected)),
            "{expected} missing from the ledger: {kinds:?}"
        );
    }
}

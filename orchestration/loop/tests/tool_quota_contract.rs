//! Contract tests: per-tool quota at the capability boundary (LoopX 对比改
//! 进项 ②). Accepted invocations are counted into the goal ledger
//! (`CapabilityInvoked` outcome=accepted), over-limit invocations are refused
//! and the refusal ledgered (outcome=rejected); the packet's
//! `capability_repair_allowed` predicate (one of the seven LoopX
//! allowed-predicates) tracks tool saturation.

mod common;

use common::{cli_err, cli_ok, cli_root, init_goal, open_store};
use future_loop::decision::decide_for;
use future_loop::quota::tool_quota::{
    DEFAULT_TOOL_QUOTA_LIMIT, OUTCOME_ACCEPTED, OUTCOME_REJECTED,
};
use future_loop::state::{now_epoch, CAPABILITY_INVOCATION_PROJECTION_CAP};
use future_loop::store::Event;

/// Seed `n` accepted capability invocations at timestamp `ts`. Each event
/// carries a unique invocation id — the ledger's content-derived-id
/// idempotency would otherwise collapse identical events into one.
fn seed_invocations(cr: &common::CliRoot, goal: &str, capability: &str, n: u64, ts: u64) {
    let mut store = open_store(cr);
    for i in 0..n {
        store
            .append(Event::CapabilityInvoked {
                goal_id: goal.to_string(),
                capability: capability.to_string(),
                command: "propose".to_string(),
                outcome: OUTCOME_ACCEPTED.to_string(),
                invocation_id: format!("seed-{ts}-{i}"),
                ts,
            })
            .unwrap();
    }
}

/// Count CapabilityInvoked events in the goal ledger by outcome.
fn ledger_outcomes(cr: &common::CliRoot, goal: &str) -> (usize, usize) {
    let store = open_store(cr);
    let events = store.events(goal).unwrap();
    let mut accepted = 0;
    let mut rejected = 0;
    for se in events {
        if let Event::CapabilityInvoked { outcome, .. } = se.event {
            match outcome.as_str() {
                OUTCOME_ACCEPTED => accepted += 1,
                OUTCOME_REJECTED => rejected += 1,
                other => panic!("unknown outcome {other}"),
            }
        }
    }
    (accepted, rejected)
}

#[test]
fn propose_with_goal_is_counted_into_the_ledger() {
    let cr = cli_root();
    let gid = init_goal(&cr, "quota-counted goal");
    cli_ok(&[
        "capability",
        "propose",
        "--name",
        "issue_fix",
        "--input",
        "crash on empty input; repro: run it",
        "--goal",
        &gid,
    ]);
    let (accepted, rejected) = ledger_outcomes(&cr, &gid);
    assert_eq!((accepted, rejected), (1, 0));
    // The replay projection folds the accepted invocation.
    let store = open_store(&cr);
    let goal = store.replay(&gid).unwrap().unwrap();
    assert_eq!(goal.capability_invocations.len(), 1);
    assert_eq!(goal.capability_invocations[0].1, "issue_fix");
}

#[test]
fn propose_without_goal_stays_uncounted() {
    let cr = cli_root();
    let gid = init_goal(&cr, "goal-less propose goal");
    cli_ok(&[
        "capability",
        "propose",
        "--name",
        "issue_fix",
        "--input",
        "crash on empty input; repro: run it",
    ]);
    assert_eq!(ledger_outcomes(&cr, &gid), (0, 0));
}

#[test]
fn over_limit_invocation_is_refused_and_the_refusal_is_ledgered() {
    let cr = cli_root();
    let gid = init_goal(&cr, "saturated tool goal");
    let now = now_epoch();
    seed_invocations(&cr, &gid, "issue_fix", DEFAULT_TOOL_QUOTA_LIMIT, now);
    let err = cli_err(&[
        "capability",
        "propose",
        "--name",
        "issue_fix",
        "--input",
        "crash; repro: run it",
        "--goal",
        &gid,
    ]);
    assert!(err.contains("per-tool quota exceeded"), "{err}");
    assert!(err.contains("issue_fix"), "{err}");
    // The refusal was ledgered; the accepted count did not grow.
    let (accepted, rejected) = ledger_outcomes(&cr, &gid);
    assert_eq!(accepted, DEFAULT_TOOL_QUOTA_LIMIT as usize);
    assert_eq!(rejected, 1);
    // A different tool on the same goal is unaffected.
    cli_ok(&[
        "capability",
        "propose",
        "--name",
        "explore",
        "--input",
        "probe the repo",
        "--goal",
        &gid,
    ]);
    let (accepted, _) = ledger_outcomes(&cr, &gid);
    assert_eq!(accepted, DEFAULT_TOOL_QUOTA_LIMIT as usize + 1);
}

#[test]
fn rejected_invocations_never_consume_quota() {
    let cr = cli_root();
    let gid = init_goal(&cr, "rejected-only goal");
    let mut store = open_store(&cr);
    for i in 0..DEFAULT_TOOL_QUOTA_LIMIT + 5 {
        store
            .append(Event::CapabilityInvoked {
                goal_id: gid.clone(),
                capability: "issue_fix".to_string(),
                command: "propose".to_string(),
                outcome: OUTCOME_REJECTED.to_string(),
                invocation_id: format!("rejected-seed-{i}"),
                ts: now_epoch(),
            })
            .unwrap();
    }
    drop(store);
    // Rejected events are audit-only: the tool still has its full quota.
    cli_ok(&[
        "capability",
        "propose",
        "--name",
        "issue_fix",
        "--input",
        "crash; repro: run it",
        "--goal",
        &gid,
    ]);
    let store = open_store(&cr);
    let goal = store.replay(&gid).unwrap().unwrap();
    assert_eq!(
        goal.capability_invocations.len(),
        1,
        "only the accepted invocation folds into the projection"
    );
}

#[test]
fn invocations_outside_the_window_free_quota() {
    let cr = cli_root();
    let gid = init_goal(&cr, "expired window goal");
    // Saturate the tool, but far outside the trailing window.
    let old = now_epoch().saturating_sub(2 * 3600 + 60);
    seed_invocations(&cr, &gid, "issue_fix", DEFAULT_TOOL_QUOTA_LIMIT, old);
    cli_ok(&[
        "capability",
        "propose",
        "--name",
        "issue_fix",
        "--input",
        "crash; repro: run it",
        "--goal",
        &gid,
    ]);
}

#[test]
fn capability_command_hook_enforces_quota_with_goal() {
    let cr = cli_root();
    let gid = init_goal(&cr, "hook quota goal");
    // G-24 hook form: `loopx issue-fix --input ... --goal G`.
    cli_ok(&[
        "issue-fix",
        "--input",
        "panic on empty input; repro: run it",
        "--goal",
        &gid,
    ]);
    let (accepted, _) = ledger_outcomes(&cr, &gid);
    assert_eq!(accepted, 1);
    // Saturate, then the hook refuses too.
    seed_invocations(
        &cr,
        &gid,
        "issue_fix",
        DEFAULT_TOOL_QUOTA_LIMIT - 1,
        now_epoch(),
    );
    let err = cli_err(&["issue-fix", "--input", "x", "--goal", &gid]);
    assert!(err.contains("per-tool quota exceeded"), "{err}");
    // Hooks without a goal stay uncounted (back-compat).
    cli_ok(&[
        "issue-fix",
        "--input",
        "panic on empty input; repro: run it",
    ]);
    let (accepted, _) = ledger_outcomes(&cr, &gid);
    assert_eq!(accepted, DEFAULT_TOOL_QUOTA_LIMIT as usize);
}

#[test]
fn quota_tools_renders_usage_and_the_predicate() {
    let cr = cli_root();
    let gid = init_goal(&cr, "quota tools goal");
    // Empty: explicit empty state + the lane is open.
    cli_ok(&["quota", "tools", "--goal", &gid]);
    seed_invocations(&cr, &gid, "issue_fix", 3, now_epoch());
    cli_ok(&["quota", "tools", "--goal", &gid]);
    cli_ok(&["quota", "tools", "--goal", &gid, "--format", "json"]);
    // Missing args / unknown goal errors.
    assert!(cli_err(&["quota", "tools"]).contains("--goal required"));
    assert!(cli_err(&["quota", "tools", "--goal", "goal_nope"]).contains("not found"));
}

#[test]
fn packet_predicate_tracks_tool_saturation() {
    let cr = cli_root();
    let gid = init_goal(&cr, "predicate goal");
    let store = open_store(&cr);
    let goal = store.replay(&gid).unwrap().unwrap();
    let packet = decide_for(&goal, std::time::SystemTime::now(), None);
    assert!(
        packet.capability_repair_allowed,
        "no invocations → the capability-repair lane is open"
    );
    drop(store);
    seed_invocations(
        &cr,
        &gid,
        "issue_fix",
        DEFAULT_TOOL_QUOTA_LIMIT,
        now_epoch(),
    );
    let store = open_store(&cr);
    let goal = store.replay(&gid).unwrap().unwrap();
    let packet = decide_for(&goal, std::time::SystemTime::now(), None);
    assert!(
        !packet.capability_repair_allowed,
        "a saturated tool closes the capability-repair lane"
    );
}

#[test]
fn invocation_projection_is_bounded() {
    let cr = cli_root();
    let gid = init_goal(&cr, "projection cap goal");
    let mut store = open_store(&cr);
    let total = CAPABILITY_INVOCATION_PROJECTION_CAP + 8;
    for i in 0..total {
        store
            .append(Event::CapabilityInvoked {
                goal_id: gid.clone(),
                capability: "issue_fix".to_string(),
                command: "propose".to_string(),
                outcome: OUTCOME_ACCEPTED.to_string(),
                invocation_id: format!("cap-seed-{i}"),
                ts: i as u64,
            })
            .unwrap();
    }
    let goal = store.replay(&gid).unwrap().unwrap();
    assert_eq!(
        goal.capability_invocations.len(),
        CAPABILITY_INVOCATION_PROJECTION_CAP,
        "the projection drops the oldest entries beyond the cap"
    );
    // The oldest 8 entries (ts 0..8) were dropped; the kept tail starts at 8.
    assert_eq!(goal.capability_invocations[0].0, 8);
}

#[test]
fn capability_invoked_roundtrips_the_ledger() {
    let cr = cli_root();
    let gid = init_goal(&cr, "event roundtrip goal");
    let mut store = open_store(&cr);
    store
        .append(Event::CapabilityInvoked {
            goal_id: gid.clone(),
            capability: "explore".to_string(),
            command: "explore".to_string(),
            outcome: OUTCOME_ACCEPTED.to_string(),
            invocation_id: "inv-42".to_string(),
            ts: 42,
        })
        .unwrap();
    let events = store.events(&gid).unwrap();
    let found = events.iter().any(|se| {
        matches!(
            &se.event,
            Event::CapabilityInvoked { capability, command, outcome, invocation_id, ts, .. }
            if capability == "explore" && command == "explore" && outcome == OUTCOME_ACCEPTED
                && invocation_id == "inv-42" && *ts == 42
        )
    });
    assert!(found, "the event survives append → read with every field");
    // Ledger id/conflict verification accepts the new variant.
    store.verify(&gid).unwrap();
}

#[test]
fn quota_tools_json_shape() {
    let cr = cli_root();
    let gid = init_goal(&cr, "json shape goal");
    seed_invocations(&cr, &gid, "issue_fix", 2, now_epoch());
    // Drive the JSON path and validate the payload parses with the contract
    // fields (the CLI prints to stdout; re-run through `cli` for the side
    // effect and check the read model directly for the shape).
    cli_ok(&["quota", "tools", "--goal", &gid, "--format", "json"]);
    let store = open_store(&cr);
    let goal = store.replay(&gid).unwrap().unwrap();
    let rows =
        future_loop::quota::tool_quota::usage_rows(&goal.capability_invocations, now_epoch());
    let value = serde_json::to_value(&rows).unwrap();
    assert_eq!(value[0]["tool"], "issue_fix");
    assert_eq!(value[0]["used"], 2);
    assert_eq!(value[0]["limit"], DEFAULT_TOOL_QUOTA_LIMIT);
    assert_eq!(value[0]["window_secs"], 3600);
    assert_eq!(value[0]["allowed"], true);
}

#[test]
fn propose_unknown_goal_fails_before_proposing() {
    let cr = cli_root();
    let _gid = init_goal(&cr, "ghost goal context");
    let err = cli_err(&[
        "capability",
        "propose",
        "--name",
        "issue_fix",
        "--input",
        "x",
        "--goal",
        "goal_nope",
    ]);
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn quota_subcommand_error_lists_tools() {
    let _cr = cli_root();
    let err = cli_err(&["quota", "bogus"]);
    assert!(err.contains("tools"), "{err}");
}

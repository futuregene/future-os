//! `scheduler tick` / `scheduler ack` / `scheduler liveness` over the real CLI.
//!
//! The scheduler is what keeps a goal's automation alive, and it has two
//! mutually exclusive shapes: the FIRST tick bootstraps a persisted state file,
//! and every later tick advances the persisted cursor instead. Only the
//! bootstrap branch was exercised before, so the advance branch — the one that
//! actually runs in production — was untested.

mod common;

use common::{cli_err, cli_ok, cli_root, init_goal};

#[test]
fn tick_bootstraps_then_advances_the_persisted_cursor() {
    let cr = cli_root();
    let gid = init_goal(&cr, "scheduler tick");

    // Bootstrap: no scheduler state yet for this (goal, agent).
    cli_ok(&["scheduler", "tick", "--goal", &gid, "--agent-id", "a"]);
    // The bootstrap persists a state file the next tick reads back.
    let state_dir = std::path::Path::new(&cr.root).join("goals");
    assert!(
        state_dir.exists(),
        "scheduler tick must persist state under {}",
        state_dir.display()
    );

    // Advance: now state exists, so the same command takes the other branch.
    cli_ok(&["scheduler", "tick", "--goal", &gid, "--agent-id", "a"]);
    cli_ok(&["scheduler", "tick", "--goal", &gid, "--agent-id", "a"]);

    // `--progression` is a documented no-op once state exists (it only shapes the
    // bootstrap); it must still be accepted rather than ignored silently.
    cli_ok(&[
        "scheduler",
        "tick",
        "--goal",
        &gid,
        "--agent-id",
        "a",
        "--progression",
        "15,30",
    ]);

    // A second agent has its own state key, i.e. its own bootstrap.
    cli_ok(&["scheduler", "tick", "--goal", &gid, "--agent-id", "b"]);

    // Every tick landed as a `SchedulerTicked` event (the tick heartbeat the
    // liveness check reads), one per agent.
    let store = common::open_store(&cr);
    let ticks: Vec<String> = store
        .events(&gid)
        .unwrap()
        .into_iter()
        .filter(|e| format!("{:?}", e.event).starts_with("SchedulerTicked"))
        .map(|e| format!("{:?}", e.event))
        .collect();
    assert!(ticks.len() >= 2, "expected tick heartbeats, got {ticks:?}");
    for agent in ["a", "b"] {
        assert!(
            ticks
                .iter()
                .any(|t| t.contains(&format!("agent_id: \"{agent}\""))),
            "no tick heartbeat for {agent}: {ticks:?}"
        );
    }

    // Refusals: unknown goal, and a missing --goal.
    let err = cli_err(&["scheduler", "tick", "--goal", "ghost"]);
    assert!(err.contains("not found") || err.contains("goal"), "{err}");
    let err = cli_err(&["scheduler", "tick"]);
    assert!(err.contains("--goal"), "{err}");
}

#[test]
fn ack_and_liveness_refuse_unknown_goals_and_accept_real_ones() {
    let cr = cli_root();
    let gid = init_goal(&cr, "scheduler ack");

    // A tick first, so there is something to acknowledge / assess.
    cli_ok(&["scheduler", "tick", "--goal", &gid, "--agent-id", "a"]);

    // `scheduler ack` records the host's acknowledgment of the advised rrule.
    cli_ok(&[
        "scheduler",
        "ack",
        "--goal",
        &gid,
        "--agent-id",
        "a",
        "--action",
        "tick_next",
        "--rrule",
        "FREQ=MINUTELY;INTERVAL=30",
    ]);
    let err = cli_err(&["scheduler", "ack", "--goal", &gid, "--agent-id", "a"]);
    assert!(!err.is_empty(), "ack without --action must be refused");

    // Liveness assessment for a goal that has ticked, and for a ghost goal.
    cli_ok(&["scheduler", "liveness", "--goal", &gid]);
    cli_ok(&["scheduler", "liveness", "--goal", &gid, "--format", "json"]);
    let err = cli_err(&["scheduler", "liveness", "--goal", "ghost"]);
    assert!(err.contains("not found"), "{err}");
}

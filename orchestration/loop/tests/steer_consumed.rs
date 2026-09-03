//! Steer consumption: once a run drains a `WorkerSteered` instruction into a
//! turn envelope, the appended `SteerConsumed` event clears `pending_steer`
//! on replay — so a NEW run client (whose in-memory cursor starts at zero)
//! never re-injects the stale instruction into its first turn.

use future_loop::state::now_epoch;
use future_loop::store::{Event, Store};

#[test]
fn steer_consumed_clears_pending_steer_and_matches_episode() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("loop-root");
    let mut store = Store::open(root.to_str().unwrap()).unwrap();
    let goal = future_loop::state::Goal::new("g", "steer", "/tmp");
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: "g".into(),
            ts: 1,
        })
        .unwrap();

    let steer_ts = now_epoch() - 10;
    store
        .append(Event::WorkerSteered {
            goal_id: "g".into(),
            agent_id: Some("w".into()),
            instruction: "redirect".into(),
            ts: steer_ts,
        })
        .unwrap();
    let g = store.replay("g").unwrap().unwrap();
    assert!(
        g.pending_steer.is_some(),
        "steer pending before consumption"
    );

    // The run drains THE SAME episode → cleared.
    store
        .append(Event::SteerConsumed {
            goal_id: "g".into(),
            agent_id: Some("w".into()),
            steer_ts,
            ts: now_epoch(),
        })
        .unwrap();
    let g = store.replay("g").unwrap().unwrap();
    assert!(
        g.pending_steer.is_none(),
        "consumed steer must not re-inject"
    );

    // A newer steer with a different ts is NOT cleared by a stale consumption.
    let newer_ts = now_epoch();
    store
        .append(Event::WorkerSteered {
            goal_id: "g".into(),
            agent_id: Some("w".into()),
            instruction: "newer redirect".into(),
            ts: newer_ts,
        })
        .unwrap();
    store
        .append(Event::SteerConsumed {
            goal_id: "g".into(),
            agent_id: Some("w".into()),
            steer_ts, // stale consumption of the OLD episode
            ts: now_epoch(),
        })
        .unwrap();
    let g = store.replay("g").unwrap().unwrap();
    assert!(
        g.pending_steer.is_some(),
        "latest-wins: a stale consumption must not clear a newer steer"
    );
}

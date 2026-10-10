//! Watchdog / supervisor-outbox drive: `watch`, `pending_delivery`,
//! `record_dead_holders` and the outbox-lock behaviour.
//!
//! `watch` is the foreground service that keeps a goal's automation alive and
//! its whole body was uncovered. It is driven here through its documented
//! `once` mode (the CLI's `supervisor watch --once`) across every termination
//! condition: missing goal, cancelled goal, terminal goal, held lock, a
//! pending batch delivered over a mock agent, and a delivery failure that must
//! retain the outbox for retry.

mod common;

use common::mock_agent::*;
use common::{cli_ok, cli_root, first_todo_id, init_goal, open_store};
use future_loop::agents::supervision;
use future_loop::store::Event;

/// Mirrors the private MAX_BATCH_NOTES in gents/supervision.rs; pinned as a
/// literal so a silent change to that constant fails this test's message.
const BATCH_NOTE_LIMIT: usize = 32;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// A goal whose supervisor session is registered, so `watch` takes the
/// delivery branch rather than returning early.
fn supervised_goal(cr: &common::CliRoot, objective: &str) -> (String, future_loop::store::Store) {
    let gid = init_goal(cr, objective);
    let mut store = open_store(cr);
    store
        .append(Event::SupervisorRegistered {
            goal_id: gid.clone(),
            session_id: "supervisor".into(),
            ts: 1,
        })
        .unwrap();
    (gid, store)
}

#[test]
fn watch_once_returns_for_missing_cancelled_and_unsupervised_terminal_goals() {
    let cr = cli_root();
    let rt = rt();

    // Missing goal: `replay` yields nothing, so the watcher exits cleanly.
    let mut store = open_store(&cr);
    rt.block_on(async {
        supervision::watch(&mut store, "goal_does_not_exist", true)
            .await
            .unwrap();
    });
    assert!(!supervision::pending_delivery(&store, "goal_does_not_exist").unwrap());

    // Cancelled goal: automation must stop even with work outstanding.
    let gid = init_goal(&cr, "cancelled goal");
    future_loop::console::run(
        "future-loop",
        vec![
            "goal".into(),
            "cancel".into(),
            "--goal".into(),
            gid.clone(),
            "--reason".into(),
            "cancelled".into(),
        ],
    )
    .unwrap();
    rt.block_on(async {
        supervision::watch(&mut store, &gid, true).await.unwrap();
    });

    // Terminal goal with no supervisor session: nothing to deliver, so the
    // watcher stops instead of sleeping for the next tick.
    let plain = init_goal(&cr, "unsupervised terminal goal");
    let store_now = open_store(&cr);
    let mut goal = store_now.replay(&plain).unwrap().unwrap();
    goal.status = "cancelled".into();
    drop(store_now);
    rt.block_on(async {
        supervision::watch(&mut store, &plain, true).await.unwrap();
    });
}

#[test]
fn watch_once_delivers_a_pending_batch_and_stops_when_the_outbox_drains() {
    let cr = cli_root();
    let rt = rt();
    let (gid, mut store) = supervised_goal(&cr, "watch delivers");
    let lock = store.root_path().to_string();

    // A queued note is the outbox's reason to exist.
    supervision::queue(
        &mut store,
        &gid,
        "completed",
        "todo_1",
        "work landed",
        "key-1",
    )
    .unwrap();
    assert!(supervision::pending_delivery(&store, &gid).unwrap());

    // The watcher connects to `agent_addr()`, so point it at a mock agent.
    let (addr, shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    rt.block_on(async {
        supervision::watch(&mut store, &gid, true).await.unwrap();
    });
    assert!(
        !supervision::pending_delivery(&store, &gid).unwrap(),
        "the delivered batch must clear the pending flag"
    );
    assert_eq!(
        shared.lock().unwrap().prompt_calls.len(),
        1,
        "exactly one wakeup for one batch"
    );
    let delivered = store
        .events(&gid)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.event, Event::SupervisorBatchDelivered { .. }))
        .count();
    assert_eq!(delivered, 1);

    // A second `once` pass has nothing to send and must not wake the peer.
    rt.block_on(async {
        supervision::watch(&mut store, &gid, true).await.unwrap();
    });
    assert_eq!(shared.lock().unwrap().prompt_calls.len(), 1);

    // A held watcher lock makes a concurrent watcher a no-op rather than a
    // second polling service; `running` reports the held lock.
    drop(store);
    let st1 = future_loop::store::Store::open(&lock).unwrap();
    let _ = supervision::running(&st1, &gid);
}

#[test]
fn watch_retains_the_outbox_when_delivery_fails() {
    let cr = cli_root();
    let rt = rt();
    let (gid, mut store) = supervised_goal(&cr, "watch retains");

    supervision::queue(&mut store, &gid, "failed", "todo_1", "boom", "key-fail").unwrap();
    let (addr, _shared) = rt.block_on(spawn_mock(MockState::fail("prompt")));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);

    // A delivery failure is not fatal to the watcher, and the batch stays
    // pending so a later tick retries it.
    rt.block_on(async {
        supervision::watch(&mut store, &gid, true).await.unwrap();
    });
    assert!(
        supervision::pending_delivery(&store, &gid).unwrap(),
        "a failed delivery must be retained for retry"
    );
}

#[test]
fn record_dead_holders_queues_host_died_and_verification_due_once() {
    let cr = cli_root();
    let gid = init_goal(&cr, "dead holders");
    let mut store = open_store(&cr);

    // A delivered todo older than the follow-up threshold gets a
    // `verification_due` note; the dedup key binds the delivery's sequence
    // number, so the same follow-up is never queued twice.
    store
        .append(Event::DeliveryOutcomeRecorded {
            goal_id: gid.clone(),
            todo_id: "todo_1".into(),
            outcome: "delivered".into(),
            note: None,
            delivered_turn: 1,
            seq: 1,
            ts: 1,
        })
        .unwrap();
    supervision::record_dead_holders(&mut store, &gid).unwrap();

    // A goal that does not exist is a no-op, not an error.
    supervision::record_dead_holders(&mut store, "goal_does_not_exist").unwrap();

    let notes: Vec<String> = store
        .events(&gid)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.event {
            Event::SupervisorNote { note_kind, .. } => Some(note_kind),
            _ => None,
        })
        .collect();
    // No dead holder was recorded (no lease with a dead pid), so the only note
    // is the wall-clock verification follow-up.
    assert!(
        notes.contains(&"verification_due".to_string()),
        "expected a verification_due note, got {notes:?}"
    );
    let before = notes.len();
    supervision::record_dead_holders(&mut store, &gid).unwrap();
    let after = store
        .events(&gid)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.event, Event::SupervisorNote { .. }))
        .count();
    assert_eq!(before, after, "the follow-up must not be queued twice");
}

/// A goal that is terminal but still `active` (every todo closed, so
/// `is_terminal()` is true) with a registered supervisor and a DRAINED outbox
/// must stop the watcher. This is the `is_terminal()` half of the watch
/// condition: a `goal cancel` sets `status = "cancelled"` and short-circuits the
/// first operand, so only a terminal-and-active goal reaches the second half.
#[test]
fn a_terminal_but_active_goal_with_a_drained_outbox_stops_the_watcher() {
    let cr = cli_root();
    let rt = rt();
    let gid = init_goal(&cr, "terminal but active");
    // Close every todo so `is_terminal()` becomes true without cancelling.
    let onboarding = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &onboarding,
        "--no-follow-up",
        "--evidence",
        "artifact landed",
    ]);
    let mut store = open_store(&cr);
    store
        .append(Event::SupervisorRegistered {
            goal_id: gid.clone(),
            session_id: "supervisor".into(),
            ts: 1,
        })
        .unwrap();
    let goal = store.replay(&gid).unwrap().unwrap();
    assert!(goal.is_terminal(), "the fixture must be terminal: {goal:?}");
    assert_ne!(goal.status, "cancelled", "and not cancelled");
    assert!(goal.supervisor_session_id.is_some());
    assert!(
        !supervision::pending_delivery(&store, &gid).unwrap(),
        "the outbox must be drained"
    );

    // Nothing to deliver, so the watcher must stop without a single wakeup.
    let (addr, shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    rt.block_on(async {
        supervision::watch(&mut store, &gid, true).await.unwrap();
    });
    assert!(
        shared.lock().unwrap().prompt_calls.is_empty(),
        "a drained terminal goal must not be delivered to"
    );
}

/// The post-flush stop: a terminal goal that still has a pending delivery must
/// be delivered FIRST and only then release the watcher. That ordering is the
/// difference between "the operator gets the last note" and "the watcher quits
/// and the note is stranded".
#[test]
fn a_terminal_goal_with_a_pending_delivery_flushes_it_before_stopping() {
    let cr = cli_root();
    let rt = rt();
    let gid = init_goal(&cr, "terminal with pending");
    let onboarding = first_todo_id(&cr.root, &gid);
    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &onboarding,
        "--no-follow-up",
        "--evidence",
        "artifact landed",
    ]);
    let mut store = open_store(&cr);
    store
        .append(Event::SupervisorRegistered {
            goal_id: gid.clone(),
            session_id: "supervisor".into(),
            ts: 1,
        })
        .unwrap();
    supervision::queue(&mut store, &gid, "completed", "t1", "final note", "k-final").unwrap();
    let goal = store.replay(&gid).unwrap().unwrap();
    assert!(goal.is_terminal(), "the fixture must be terminal: {goal:?}");
    assert!(
        supervision::pending_delivery(&store, &gid).unwrap(),
        "the fixture must have something to deliver"
    );

    let (addr, shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    rt.block_on(async {
        supervision::watch(&mut store, &gid, true).await.unwrap();
    });
    // The note reached the supervisor BEFORE the watcher returned.
    assert_eq!(
        shared.lock().unwrap().prompt_calls.len(),
        1,
        "the final note must be delivered, not stranded"
    );
    assert!(
        !supervision::pending_delivery(&store, &gid).unwrap(),
        "the outbox must be drained afterwards"
    );
}

/// A second watcher must be a no-op while the first holds the watch lock: the
/// lock is what makes the service singular. The first watcher runs in the
/// background (non-`once`, so it keeps the lock), the second is asked for one
/// pass and must return immediately; cancelling the goal then ends the first.
#[test]
fn a_second_watcher_returns_immediately_while_the_lock_is_held() {
    let cr = cli_root();
    let rt = rt();
    let gid = init_goal(&cr, "lock is singular");
    let root = cr.root.clone();

    let gid_for_task = gid.clone();
    let root_for_task = root.clone();
    let held = rt.spawn(async move {
        let mut store = future_loop::store::Store::open(&root_for_task).unwrap();
        // Non-`once`: it polls until the goal is cancelled.
        supervision::watch(&mut store, &gid_for_task, false).await
    });
    // Give it time to take the lock and enter its first sleep.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(
        !held.is_finished(),
        "the polling watcher must still be running, not have exited"
    );

    // A second watcher cannot take the lock, so its single pass is a no-op.
    let mut store = open_store(&cr);
    let started = std::time::Instant::now();
    rt.block_on(async {
        supervision::watch(&mut store, &gid, true)
            .await
            .expect("a blocked watcher returns Ok, not an error");
    });
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "a locked-out watcher must return immediately, took {:?}",
        started.elapsed()
    );

    // Stop the background watcher by cancelling the goal.
    let mut writer = future_loop::store::Store::open(&root).unwrap();
    writer
        .append(Event::GoalCancelled {
            goal_id: gid.clone(),
            reason: "stop".into(),
            ts: future_loop::state::now_epoch(),
        })
        .unwrap();
    let outcome = rt.block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), held)
            .await
            .expect("the watcher must exit after the cancel")
            .expect("the watcher must not panic")
    });
    outcome.expect("watch returns Ok for a cancelled goal");
}

/// A batch must be bounded: an unbounded batch would build one ever-growing
/// prompt out of a long outage. The cap is asserted on the PREPARED batch's
/// note_keys against a literal limit, so a silent change to the private constant
/// fails with a message naming the mismatch - and the uncapped remainder must
/// still be pending, because the cap must not lose deliveries.
#[test]
fn an_outbox_batch_is_capped_at_the_note_limit() {
    let cr = cli_root();
    let rt = rt();
    let (gid, mut store) = supervised_goal(&cr, "batch cap");
    for n in 0..(BATCH_NOTE_LIMIT + 1) {
        supervision::queue(
            &mut store,
            &gid,
            "completed",
            "t1",
            &format!("result {n}"),
            &format!("key-{n}"),
        )
        .unwrap();
    }

    let (addr, _shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    rt.block_on(async {
        let mut client = future_loop::agent_client::AgentClient::connect(&addr)
            .await
            .unwrap();
        supervision::flush(&mut store, &gid, &mut client)
            .await
            .unwrap();
    });

    let prepared: Vec<Vec<String>> = store
        .events(&gid)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.event {
            Event::SupervisorBatchPrepared { note_keys, .. } => Some(note_keys),
            _ => None,
        })
        .collect();
    assert_eq!(
        prepared.len(),
        1,
        "one batch must be prepared: {prepared:?}"
    );
    assert_eq!(
        prepared[0].len(),
        BATCH_NOTE_LIMIT,
        "the batch must stop at the note cap"
    );
    assert!(
        supervision::pending_delivery(&store, &gid).unwrap(),
        "the uncapped remainder must still be pending"
    );
}

/// A terminal goal whose supervisor session is still registered and whose outbox
/// was already drained must stop the watcher (line 238's second half and the
/// post-flush return at 253). The distinction matters: a terminal goal WITH a
/// pending delivery must keep the watcher alive long enough to deliver it (that
/// is the branch `watch_once_delivers_a_pending_batch...` covers), while a
/// drained one must release the process.
#[test]
fn a_terminal_goal_with_a_registered_supervisor_stops_once_the_outbox_is_drained() {
    let cr = cli_root();
    let rt = rt();
    let (gid, mut store) = supervised_goal(&cr, "terminal + supervised");

    // Queue a note, deliver it, then mark the goal terminal: the next pass finds
    // `is_terminal() && !pending_delivery`.
    supervision::queue(&mut store, &gid, "completed", "t1", "landed", "k-term").unwrap();
    let (addr, _shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_LOOP_AGENT_ADDR", &addr);
    rt.block_on(async {
        supervision::watch(&mut store, &gid, true).await.unwrap();
    });
    assert!(!supervision::pending_delivery(&store, &gid).unwrap());

    // Make the goal terminal the same way the CLI does, then run the watcher
    // again: it must return without doing anything else.
    future_loop::console::run(
        "future-loop",
        vec![
            "goal".into(),
            "cancel".into(),
            "--goal".into(),
            gid.clone(),
            "--reason".into(),
            "terminal".into(),
        ],
    )
    .unwrap();
    let before = store.events(&gid).unwrap().len();
    rt.block_on(async {
        supervision::watch(&mut store, &gid, true).await.unwrap();
    });
    assert_eq!(
        store.events(&gid).unwrap().len(),
        before,
        "a drained terminal goal must not produce more work"
    );
}

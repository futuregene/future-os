//! Fault-point drives for the supervisor sidecar's IO-arm behaviour.
//!
//! `agents/supervision.rs` takes an exclusive lock file per goal under
//! `<root>/supervision/`. Every `?` on that path (create the dir, open the file)
//! was uncovered, because the happy path never fails. Making the sidecar path
//! unusable is the deterministic way to reach them, and it is a real operational
//! state: a stray file where the directory belongs, or a read-only state root.
//!
//! The refusal matters: the supervisor outbox must *report* that it could not
//! persist a note rather than silently dropping a delivery the operator is
//! waiting for.

mod common;

use common::{init_goal, open_store};
use future_loop::agents::supervision;

/// Plant a regular file where the sidecar directory belongs. Every `lock_file`
/// call then fails at `create_dir_all`, which is what that `?` arm exists for.
fn block_the_sidecar_dir(root: &str) {
    std::fs::write(std::path::Path::new(root).join("supervision"), b"not a dir").unwrap();
}

/// The next failure mode up: the sidecar directory exists, but the lock FILE
/// name is occupied by a directory, so `OpenOptions::open` fails. `create_dir_all`
/// succeeds here, which is what distinguishes this arm from the one above.
fn block_one_lock_file(root: &str, goal: &str, suffix: &str) {
    use sha2::{Digest, Sha256};
    let dir = std::path::Path::new(root).join("supervision");
    std::fs::create_dir_all(&dir).unwrap();
    let name = format!("{:x}.{suffix}", Sha256::digest(goal.as_bytes()));
    std::fs::create_dir_all(dir.join(name)).unwrap();
}

#[test]
fn queue_and_flush_report_an_unusable_sidecar_dir_instead_of_dropping_the_note() {
    let cr = common::cli_root();
    let gid = init_goal(&cr, "sidecar unusable");
    let mut store = open_store(&cr);
    block_the_sidecar_dir(&cr.root);

    // queue() must surface the failure — a silently dropped note is a delivery
    // the operator never sees.
    let err = supervision::queue(&mut store, &gid, "completed", "t1", "landed", "key-1")
        .expect_err("queue must not pretend it persisted a note");
    let _ = format!("{err:#}");

    // Nothing was written: the note is not in the ledger.
    let notes = store
        .events(&gid)
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.event, future_loop::store::Event::SupervisorNote { .. }))
        .count();
    assert_eq!(notes, 0, "a refused queue must not write a ledger note");

    // `running()` reports "no watcher" rather than erroring, because it is a
    // liveness probe: an unusable sidecar means no watcher holds the lock.
    assert!(
        !supervision::running(&store, &gid),
        "an unusable sidecar dir means no watcher is running"
    );
}

#[test]
fn watches_and_outbox_delivery_degrade_cleanly_when_the_sidecar_is_unusable() {
    let cr = common::cli_root();
    let gid = init_goal(&cr, "sidecar unusable watch");
    let mut store = open_store(&cr);
    block_the_sidecar_dir(&cr.root);

    // Nothing to report yet: `record_dead_holders` is a no-op, not an error.
    supervision::record_dead_holders(&mut store, &gid)
        .expect("no dead holders and no overdue delivery is a clean no-op");

    // Now make a delivery overdue (the wall-clock follow-up path), so the call
    // really tries to queue a note — which cannot be persisted here.
    store
        .append(future_loop::store::Event::DeliveryOutcomeRecorded {
            goal_id: gid.clone(),
            todo_id: "t1".into(),
            outcome: "delivered".into(),
            note: None,
            delivered_turn: 1,
            seq: 1,
            ts: 1,
        })
        .unwrap();
    let err = supervision::record_dead_holders(&mut store, &gid);
    assert!(
        err.is_err(),
        "an unpersistable overdue follow-up must be reported, not swallowed"
    );

    // `pending_delivery` reads the ledger only, so it stays usable (and must not
    // invent a delivery that was never queued).
    assert!(!supervision::pending_delivery(&store, &gid).unwrap());
    // An unknown goal is false, not an error.
    assert!(!supervision::pending_delivery(&store, "goal_missing").unwrap());

    // A foreground `watch --once` pass must not panic when the lock cannot be
    // taken; it returns and lets the operator fix the state root.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let outcome = supervision::watch(&mut store, &gid, true).await;
        assert!(
            outcome.is_err(),
            "watch must report that it could not take its lock"
        );
    });
}

#[test]
fn an_occupied_lock_file_name_is_reported() {
    let cr = common::cli_root();
    let gid = init_goal(&cr, "lock name occupied");
    // The directory exists, so only the per-goal file open can fail.
    block_one_lock_file(&cr.root, &gid, "notes.lock");
    let mut store = open_store(&cr);
    let err = supervision::queue(&mut store, &gid, "completed", "t1", "landed", "key-2")
        .expect_err("an unopenable lock file must be reported");
    let _ = format!("{err:#}");
    // `running()` probes the watch lock, which is a different file and stays
    // usable: it reports "no watcher", not an error.
    let _ = supervision::running(&store, &gid);
}

#[test]
fn ensure_watchdog_is_a_noop_when_detach_is_opted_out() {
    let cr = common::cli_root();
    let gid = init_goal(&cr, "watchdog opt-out");
    let store = open_store(&cr);
    // The harness sets this for its CLI invocations; assert the contract
    // directly so the opt-out cannot regress into spawning a process.
    std::env::set_var("FUTURE_LOOP_NO_DETACH", "1");
    supervision::ensure_watchdog(&store, &gid).expect("the opt-out must succeed");
    // No watcher lock was taken as a side effect.
    assert!(
        !supervision::running(&store, &gid),
        "opting out must not start a watcher"
    );
}

/// `notify_supervisor`'s two failure arms. It is advisory, and the two arms are
/// deliberately asymmetric:
///
/// * a note that cannot be PERSISTED is reported and abandoned — there is nothing
///   to retry from, and inventing a delivery would tell the operator about
///   something the ledger never recorded;
/// * a note that persisted but could not be DELIVERED is left in the outbox,
///   because the outbox exists precisely so a later flush retries it. Dropping it
///   there would lose a message the operator is waiting for.
#[test]
fn notify_supervisor_reports_an_unpersisted_note_and_retains_an_undelivered_one() {
    use common::mock_agent::{spawn_mock, MockState};
    use future_loop::agent_client::AgentClient;

    fn notes(store: &future_loop::store::Store, goal: &str) -> usize {
        store
            .events(goal)
            .unwrap()
            .into_iter()
            .filter(|e| matches!(e.event, future_loop::store::Event::SupervisorNote { .. }))
            .count()
    }

    let cr = common::cli_root();
    let gid = init_goal(&cr, "notify arms");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    // ── Arm 1: the note cannot be persisted (sidecar path unusable). ─────────
    let mut store = open_store(&cr);
    block_the_sidecar_dir(&cr.root);
    let (ok_addr, _ok_state) = rt.block_on(spawn_mock(MockState::default()));
    rt.block_on(async {
        let mut client = AgentClient::connect(&ok_addr)
            .await
            .expect("the mock agent must be reachable");
        future_loop::console::notify_supervisor(
            &mut store,
            &mut client,
            &gid,
            Some("sess-supervisor"),
            "completed",
            "todo_1",
            "landed",
            "dedup-unpersisted",
        )
        .await;
    });
    assert_eq!(
        notes(&store, &gid),
        0,
        "a note that could not be persisted must not appear in the ledger"
    );
    assert!(
        !supervision::pending_delivery(&store, &gid).unwrap(),
        "nothing pending: the note never got as far as the outbox"
    );

    // ── Arm 2: the note persists, but its delivery fails. ───────────────────
    let sidecar = std::path::Path::new(&cr.root).join("supervision");
    std::fs::remove_file(&sidecar).expect("the blocker is a file this test planted");
    common::cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &gid,
        "--session-id",
        "sess-supervisor",
    ]);
    let mut refusing = MockState::default();
    refusing.fail_commands.insert("prompt".to_string());
    let (refusing_addr, _refusing_state) = rt.block_on(spawn_mock(refusing));
    rt.block_on(async {
        let mut client = AgentClient::connect(&refusing_addr)
            .await
            .expect("the mock agent must be reachable");
        future_loop::console::notify_supervisor(
            &mut store,
            &mut client,
            &gid,
            Some("sess-supervisor"),
            "completed",
            "todo_2",
            "landed",
            "dedup-undelivered",
        )
        .await;
        // The mock answers a refused `prompt` BEFORE it records anything, so the
        // attempt cannot be observed from its state. Prove the delivery failure
        // directly instead: the same goal, the same refusing client, and
        // `notify_supervisor`'s guard (`running()` is false here - no watcher holds
        // watch.lock) means this is exactly the call it made. If flush returned Ok the
        // assertion below fails, and with it the claim that the note was left pending.
        let flushed = supervision::flush(&mut store, &gid, &mut client).await;
        assert!(
            flushed.is_err(),
            "a refused delivery must surface as an error, not as a silent success"
        );
    });
    assert_eq!(
        notes(&store, &gid),
        1,
        "the note persisted, so the ledger must hold it"
    );
    assert!(
        supervision::pending_delivery(&store, &gid).unwrap(),
        "an undelivered note must stay in the outbox so a retry can send it, \
         not be dropped by the failed flush"
    );
}

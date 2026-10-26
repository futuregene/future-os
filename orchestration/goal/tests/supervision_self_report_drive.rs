//! The supervisor's self-report: the two directions that were missing.
//!
//! The evidence this drive pins is a goal that ran for twelve hours while its
//! supervisor was told nothing:
//!
//! * a worker finished cleanly and the supervisor only learned about it because
//!   a human ran a command (`record_dead_holders` reported failures —
//!   `host_died`, `verification_due` — and nothing else);
//! * the watcher — the only thing that delivers the outbox on a tick — died, and
//!   nothing ever started another one, so a note that WAS queued sat there.
//!
//! Both halves are asserted here through the commands an operator actually runs,
//! not by calling the helper directly: a `todo complete` must leave a note, a
//! `status` poll on an unwatched goal must deliver what is waiting inline, and
//! neither may cost anything while a watcher is live.

mod common;

use common::mock_agent::{spawn_mock, MockState};
use common::{add_todo, cli_ok, cli_root, init_goal, open_store};
use future_goal::agents::supervision;
use future_goal::state::now_epoch;
use future_goal::store::Event;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// A pid that is definitely gone: spawn a process, reap it, keep its number.
fn dead_pid() -> u32 {
    #[cfg(windows)]
    let mut child = std::process::Command::new("cmd")
        .args(["/c", "exit"])
        .spawn()
        .expect("spawn a throwaway process");
    #[cfg(not(windows))]
    let mut child = std::process::Command::new("true")
        .spawn()
        .expect("spawn a throwaway process");
    let pid = child.id();
    child.wait().expect("reap the throwaway process");
    pid
}

/// Every supervisor note kind in the ledger, in order.
fn note_kinds(store: &future_goal::store::Store, goal: &str) -> Vec<String> {
    store
        .events(goal)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e.event {
            Event::SupervisorNote { note_kind, .. } => Some(note_kind),
            _ => None,
        })
        .collect()
}

/// An unwatched goal whose only worker is a dead holder: the exact state a
/// crashed `future goal run` leaves behind, and the one nothing reported before
/// this change unless an operator happened to run the sweep.
fn goal_with_a_dead_holder(cr: &common::CliRoot, objective: &str) -> (String, String) {
    let gid = init_goal(cr, objective);
    let todo = add_todo(cr, &gid, "work that dies mid-slice");
    cli_ok(&[
        "supervisor",
        "register",
        "--goal",
        &gid,
        "--session-id",
        "sup-self-report",
    ]);
    let dead = dead_pid();
    let mut store = open_store(cr);
    store
        .append(Event::TodoClaimed {
            goal_id: gid.clone(),
            todo_id: todo.clone(),
            agent_id: "w1".into(),
            lease_expires_at: now_epoch() + 3600,
            holder_pid: Some(dead),
            ts: now_epoch(),
        })
        .unwrap();
    // Deliberately no sweep here: the caller is the test of the path that is
    // supposed to notice the dead holder.
    (gid, todo)
}

/// R3: a clean completion is reported. The counterpart is the delivery verdict:
/// recording `verified` afterwards must not manufacture a second note, and must
/// not retract the one that reported the completion — the note is a report, the
/// verdict is a separate decision.
#[test]
fn a_clean_completion_is_reported_and_a_verdict_neither_repeats_nor_retracts_it() {
    let cr = cli_root();
    let gid = init_goal(&cr, "completion is reported");
    let todo = add_todo(&cr, &gid, "ship the acceptance surface");
    // No agent: the note has to be durable before any delivery is attempted.
    std::env::set_var("FUTURE_GOAL_AGENT_ADDR", "http://127.0.0.1:1");

    cli_ok(&[
        "todo",
        "complete",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--no-follow-up",
        "--evidence",
        "artifact landed at /tmp/out",
    ]);

    let store = open_store(&cr);
    let kinds = note_kinds(&store, &gid);
    assert_eq!(
        kinds,
        vec![supervision::TODO_DONE_NOTE_KIND.to_string()],
        "a clean completion must produce exactly one acceptance reminder: {kinds:?}"
    );
    assert!(
        supervision::pending_delivery(&store, &gid).unwrap(),
        "with no agent reachable the note stays in the outbox for a later flush"
    );
    let notes_for_todo = store
        .events(&gid)
        .unwrap()
        .into_iter()
        .filter(|e| {
            matches!(
                &e.event,
                Event::SupervisorNote { todo_id, note_kind, .. }
                    if todo_id == &todo && note_kind == supervision::TODO_DONE_NOTE_KIND
            )
        })
        .count();
    assert_eq!(notes_for_todo, 1);

    // The verdict, then every path that could re-derive the note.
    cli_ok(&[
        "delivery",
        "record",
        "--goal",
        &gid,
        "--todo-id",
        &todo,
        "--outcome",
        "verified",
    ]);
    let mut store = open_store(&cr);
    supervision::record_completions(&mut store, &gid).unwrap();
    supervision::sweep(&mut store, &gid).unwrap();

    let after = note_kinds(&store, &gid);
    assert_eq!(
        after, kinds,
        "a recorded verdict must not retract or repeat the completion note: {after:?}"
    );
    let delivery = store
        .replay(&gid)
        .unwrap()
        .unwrap()
        .delivery_state(&todo)
        .cloned()
        .expect("the completion recorded a delivery");
    assert_eq!(
        delivery.outcome, "verified",
        "the note is a report; the verdict is still the operator's, recorded separately"
    );
}

/// R2 + R4 (first half): the dead-holder chain end to end. A worker is gone, no
/// watcher was ever started, and the operator runs the ordinary `status` poll —
/// the note must be queued AND pushed to the supervisor session inline, because
/// "no watcher" must not mean "never reported".
#[test]
fn a_dead_worker_reaches_the_supervisor_through_a_status_poll_with_no_watcher() {
    let cr = cli_root();
    let (gid, todo) = goal_with_a_dead_holder(&cr, "dead worker self report");
    {
        let store = open_store(&cr);
        assert!(
            !supervision::running(&store, &gid),
            "the fixture must start unwatched, or the inline path is not what runs"
        );
    }

    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_GOAL_AGENT_ADDR", &addr);

    cli_ok(&["status", "--goal", &gid]);

    let store = open_store(&cr);
    let kinds = note_kinds(&store, &gid);
    assert!(
        kinds.contains(&"host_died".to_string()),
        "the poll must report the dead holder: {kinds:?}"
    );
    let held = shared.lock().unwrap();
    assert_eq!(
        held.prompt_calls.len(),
        1,
        "an unwatched goal must deliver inline, not wait for a watcher that does not exist"
    );
    assert!(
        held.prompt_messages
            .first()
            .is_some_and(|m| m.contains(&todo)),
        "the delivered batch must name the todo whose holder died: {:?}",
        held.prompt_messages
    );
    drop(held);
    assert!(
        !supervision::pending_delivery(&store, &gid).unwrap(),
        "the inline delivery must drain the outbox"
    );
}

/// R2 + R3 + R4 (second half): one watcher tick — the tick alone, with no
/// supervisor command run by hand — reports a dead worker and pushes it. This is
/// the path that was expected to work all along and did not: when the holder dies
/// the worker executes nothing, so nothing but the tick can notice.
#[test]
fn a_watcher_tick_reports_a_dead_worker_by_itself() {
    let cr = cli_root();
    let (gid, todo) = goal_with_a_dead_holder(&cr, "watch tick self report");
    let mut store = open_store(&cr);
    assert!(
        !supervision::running(&store, &gid),
        "the fixture must start with no watcher"
    );
    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_GOAL_AGENT_ADDR", &addr);

    // One pass of the real watch loop: the `--once` mode the CLI exposes.
    rt.block_on(async { supervision::watch(&mut store, &gid, true).await.unwrap() });

    let kinds = note_kinds(&store, &gid);
    assert!(
        kinds.contains(&"host_died".to_string()),
        "the tick must record the dead holder: {kinds:?}"
    );
    let delivered = shared.lock().unwrap().prompt_messages.clone();
    assert_eq!(delivered.len(), 1, "the tick must deliver the batch once");
    assert!(
        delivered[0].contains(&todo),
        "the batch must name the todo whose holder died: {:?}",
        delivered[0]
    );
    assert!(
        !supervision::pending_delivery(&store, &gid).unwrap(),
        "the outbox must be drained by the tick"
    );
}

/// R2 (the cheap half): while a watcher holds the lock, the same poll must not
/// replay the goal, must not sweep, and must not deliver — the watcher's own
/// tick is what reports, two seconds later. This is what keeps `status` free
/// enough to poll in a loop.
#[test]
fn a_live_watcher_makes_the_status_poll_free() {
    use fs2::FileExt;
    use sha2::{Digest, Sha256};

    let cr = cli_root();
    let (gid, _todo) = goal_with_a_dead_holder(&cr, "poll is free while watched");
    // Take the watcher lock the way the real watchdog holds it.
    let lock_path = std::path::PathBuf::from(cr.root.clone())
        .join("supervision")
        .join(format!("{:x}.watch.lock", Sha256::digest(gid.as_bytes())));
    std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let held = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    held.try_lock_exclusive()
        .expect("the test takes the watch lock first");

    let rt = rt();
    let (addr, shared) = rt.block_on(spawn_mock(MockState::default()));
    std::env::set_var("FUTURE_GOAL_AGENT_ADDR", &addr);
    let before = note_kinds(&open_store(&cr), &gid);

    for _ in 0..5 {
        cli_ok(&["status", "--goal", &gid]);
    }

    let store = open_store(&cr);
    assert_eq!(
        note_kinds(&store, &gid),
        before,
        "a watched goal's poll must not queue notes of its own"
    );
    assert!(
        shared.lock().unwrap().prompt_calls.is_empty(),
        "a watched goal's poll must not deliver — the watcher owns delivery"
    );
}

/// R2 (the re-arm): a goal whose watcher is gone gets another one from an
/// ordinary `status` poll. This is the failure the evidence names — the
/// `watch.lock` froze at 00:48 and no `supervisor watch` process existed for
/// hours, so nothing ticked.
///
/// Driven through the real binary: a test harness's `current_exe` is not the CLI,
/// so the in-process tests above cannot observe a spawn (and correctly must not
/// spawn one). The copy is named `future`, the spelling whose group verb the
/// watchdog has to use.
#[test]
fn a_status_poll_restarts_a_watcher_that_died() {
    use std::time::{Duration, Instant};

    struct Teardown {
        root: String,
        goal: String,
    }
    impl Drop for Teardown {
        fn drop(&mut self) {
            let Ok(mut store) = future_goal::store::Store::open(&self.root) else {
                return;
            };
            if !supervision::running(&store, &self.goal) {
                return;
            }
            let _ = store.append(Event::GoalCancelled {
                goal_id: self.goal.clone(),
                reason: "test teardown".into(),
                ts: now_epoch(),
            });
            let deadline = Instant::now() + Duration::from_secs(15);
            while Instant::now() < deadline && supervision::running(&store, &self.goal) {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    let cr = cli_root();
    let (gid, _todo) = goal_with_a_dead_holder(&cr, "status re-arms the watcher");
    let dir = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let copy = dir.path().join("future.exe");
    #[cfg(not(windows))]
    let copy = dir.path().join("future");
    std::fs::copy(env!("CARGO_BIN_EXE_future-goal"), &copy).expect("copy the built binary");
    let teardown = Teardown {
        root: cr.root.clone(),
        goal: gid.clone(),
    };

    let stdout_path = dir.path().join("status.stdout");
    let stderr_path = dir.path().join("status.stderr");
    let status = std::process::Command::new(&copy)
        .args(["status", "--goal", &gid])
        .env("FUTURE_GOAL_ROOT", &cr.root)
        // The watchdog's own delivery must not reach an agent from this test.
        .env("FUTURE_GOAL_AGENT_ADDR", "http://127.0.0.1:1")
        .env_remove("FUTURE_GOAL_NO_DETACH")
        .stdin(std::process::Stdio::null())
        .stdout(std::fs::File::create(&stdout_path).unwrap())
        .stderr(std::fs::File::create(&stderr_path).unwrap())
        .status()
        .expect("the future-named copy runs");
    let stdout = std::fs::read_to_string(&stdout_path).unwrap_or_default();
    let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
    assert!(
        status.success(),
        "a status poll must succeed: {stdout}\n{stderr}"
    );

    let store = open_store(&cr);
    assert!(
        supervision::running(&store, &gid),
        "`status` must leave a watcher behind for an unwatched, supervised goal: \
         {stdout}\n{stderr}"
    );
    drop(teardown);
}

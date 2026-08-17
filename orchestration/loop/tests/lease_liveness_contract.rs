//! Lease-liveness contract: a dead holder's lease is reclaimed
//! automatically (kill -0 probe on the recorded holder pid), eliminating
//! the manual `lease release` dance after killing a run.
//! Missing holder_pid (pre-liveness ledgers) keeps the old hard error.

use future_loop::state::Todo;
use future_loop::store::{Event, Store};
use future_loop::work_items::task_lease::{claim, ClaimOutcome};

fn now() -> u64 {
    future_loop::state::now_epoch()
}

fn open_claimed_todo(root: &str, holder_pid: Option<u32>) -> Todo {
    let mut store = Store::open(root).unwrap();
    let goal = future_loop::state::Goal::new("g1", "obj", "/tmp");
    store.register(&goal).unwrap();
    let ts = now();
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "task"),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoClaimed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "holder".into(),
            lease_expires_at: ts + 3600,
            holder_pid,
            ts,
        })
        .unwrap();
    store.replay("g1").unwrap().unwrap().todos[0].clone()
}

#[test]
fn live_holder_refuses_claim() {
    let dir = tempfile::tempdir().unwrap();
    let todo = open_claimed_todo(dir.path().to_str().unwrap(), Some(std::process::id()));
    let mut t = todo.clone();
    // Our own pid is alive — the claim must stay a hard error.
    let err = claim(&mut t, "other", 3600, now()).unwrap_err();
    assert!(format!("{err}").contains("active lease"), "{err}");
}

#[test]
fn dead_holder_is_reclaimed_with_steal() {
    let dir = tempfile::tempdir().unwrap();
    // 999999 sits above both Linux pid_max (32768) and macOS's range —
    // kill -0 always reports ESRCH, i.e. a dead holder.
    let todo = open_claimed_todo(dir.path().to_str().unwrap(), Some(999_999));
    let mut t = todo.clone();
    let outcome = claim(&mut t, "successor", 3600, now()).unwrap();
    assert_eq!(
        outcome,
        ClaimOutcome {
            idempotent: false,
            steal: true
        }
    );
    assert_eq!(t.claimed_by.as_deref(), Some("successor"));
    assert_eq!(t.holder_pid, Some(std::process::id()));
}

#[test]
fn missing_holder_pid_keeps_hard_error() {
    let dir = tempfile::tempdir().unwrap();
    // Pre-liveness ledger: no holder_pid recorded → no probe → old behavior.
    let todo = open_claimed_todo(dir.path().to_str().unwrap(), None);
    let mut t = todo.clone();
    let err = claim(&mut t, "other", 3600, now()).unwrap_err();
    assert!(format!("{err}").contains("active lease"), "{err}");
}

#[test]
fn atomic_claim_path_reclaims_dead_holder() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap();
    let mut store = Store::open(root).unwrap();
    let goal = future_loop::state::Goal::new("g1", "obj", "/tmp");
    store.register(&goal).unwrap();
    let ts = now();
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "task"),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoClaimed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "holder".into(),
            lease_expires_at: ts + 3600,
            holder_pid: Some(999_999),
            ts,
        })
        .unwrap();
    drop(store);

    // The atomic check+append claim path must reclaim the dead holder too.
    let store = Store::open(root).unwrap();
    let ok = store.try_claim_todo("g1", "t1", "successor", 3600).unwrap();
    assert!(ok, "dead holder must not block the atomic claim path");
    let g = store.replay("g1").unwrap().unwrap();
    assert_eq!(g.todos[0].claimed_by.as_deref(), Some("successor"));
}

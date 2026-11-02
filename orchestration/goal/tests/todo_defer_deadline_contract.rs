//! Contract: a relative deferral written by `todo update --resume-when N`
//! (`resume_when = "defer:N"`) is anchored to the moment the EVENT was written,
//! never to read time (bug h3-C1).
//!
//! The old projection resolved `defer:N` inside the replay fold with
//! `SystemTime::now() + N`, so every read re-armed the deadline to read-time +
//! N: the todo never became due (a 3 s deferral was still "not yet due" 30 s
//! later) and the goal could never reach terminal closure. Replay must be a
//! pure function of the ledger — the same reason `Store::replay` restores
//! `created_at` from the ledger instead of re-stamping it from the read clock.

use future_goal::console;
use future_goal::state::{now_epoch, Goal, Todo, TodoStatus};
use future_goal::store::{Event, Store};
use std::io::Write;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn fresh_store(tag: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(tag);
    std::fs::create_dir_all(&root).unwrap();
    (dir, root.to_string_lossy().into_owned())
}

/// Append one raw ledger line, bypassing the typed writer — this is the shape
/// an OLDER binary produced (no absolute deadline field existed yet).
fn append_raw_line(root: &str, goal_id: &str, line: &str) {
    let path = Store::open(root)
        .unwrap()
        .goal_dir(goal_id)
        .join("events.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(file, "{line}").unwrap();
}

/// A `defer:N` line written a minute ago must be DUE now: the deadline is
/// `event.ts + N`, not `read-time + N`.
#[test]
fn legacy_relative_defer_anchors_to_the_event_write_time() {
    let (_d, root) = fresh_store("defer-legacy");
    let mut store = Store::open(&root).unwrap();
    let goal = Goal::new("g1", "legacy defer", "/tmp");
    store.register(&goal).unwrap();
    let written = now_epoch() - 60;
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts: written,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "work"),
            ts: written,
        })
        .unwrap();
    // Pre-fix ledger shape: `defer:5` with no absolute deadline.
    append_raw_line(
        &root,
        "g1",
        &format!(
            "{{\"kind\":\"todo_updated\",\"goal_id\":\"g1\",\"todo_id\":\"t1\",\
             \"text\":null,\"status\":null,\"evidence\":null,\"note\":null,\
             \"priority\":null,\"resume_when\":\"defer:5\",\"blocks\":null,\
             \"acceptance\":null,\"owner\":null,\"ts\":{written}}}"
        ),
    );

    let g = store.replay("g1").unwrap().unwrap();
    let t = g.todo("t1").unwrap();
    assert_eq!(t.status, TodoStatus::Deferred);
    assert_eq!(t.resume_when_text.as_deref(), Some("defer:5"));
    let deadline = t.resume_when.expect("a numeric defer sets a real deadline");
    assert_eq!(
        deadline,
        UNIX_EPOCH + Duration::from_secs(written + 5),
        "deadline must be event.ts + N (write time), not read-time + N"
    );
    assert!(
        t.is_due_deferred(SystemTime::now()),
        "a 5 s deferral written 60 s ago is long overdue"
    );
}

/// Replaying the SAME ledger twice must produce an identical projection — a
/// read-time-relative deadline makes the read a function of the wall clock.
#[test]
fn replaying_one_ledger_twice_yields_an_identical_goal() {
    let (_d, root) = fresh_store("defer-stable");
    let mut store = Store::open(&root).unwrap();
    let goal = Goal::new("g1", "stable replay", "/tmp");
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "work"),
            ts: now_epoch(),
        })
        .unwrap();
    // A written line with a RELATIVE defer (the legacy on-disk shape: no
    // absolute deadline field).
    append_raw_line(
        &root,
        "g1",
        &format!(
            "{{\"kind\":\"todo_updated\",\"goal_id\":\"g1\",\"todo_id\":\"t1\",\
             \"resume_when\":\"defer:5\",\"ts\":{}}}",
            now_epoch()
        ),
    );

    let first = store.replay("g1").unwrap().unwrap();
    let second = store.replay("g1").unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap(),
        "replay must be a pure function of the ledger, not of the read clock"
    );
}

/// End-to-end regression (the h3-C1 repro): `--resume-when 1` must actually
/// expire, and the deadline must not move when the ledger is read again.
#[test]
fn cli_one_second_deferral_becomes_due() {
    with_root("cli-defer-due", |root| {
        cli(&[
            "goal",
            "init",
            "--objective",
            "defer due",
            "--cwd",
            "/tmp",
            "--goal-id",
            "g1",
        ])
        .unwrap();
        cli(&["todo", "add", "--goal", "g1", "--text", "defer me"]).unwrap();
        let todo_id = todo_id_by_text(root, "g1", "defer me");
        cli(&[
            "todo",
            "update",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--resume-when",
            "1",
        ])
        .unwrap();

        let store = Store::open(root).unwrap();
        let g = store.replay("g1").unwrap().unwrap();
        let deadline = g
            .todo(&todo_id)
            .unwrap()
            .resume_when
            .expect("a numeric defer sets a real deadline");
        assert!(
            !g.todo(&todo_id).unwrap().is_due_deferred(SystemTime::now()),
            "a 1 s deferral is not due the instant it is written"
        );

        std::thread::sleep(Duration::from_millis(1000));

        let g = store.replay("g1").unwrap().unwrap();
        let t = g.todo(&todo_id).unwrap();
        assert_eq!(
            t.resume_when,
            Some(deadline),
            "the deadline must be a fixed instant, not re-armed on read"
        );
        assert!(
            t.is_due_deferred(SystemTime::now()),
            "a 1 s deferral must be due one second later"
        );
    });
}

// ── CLI harness (FUTURE_GOAL_ROOT is process-global) ──────────────────────

fn with_root<F: FnOnce(&str)>(tag: &str, f: F) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("future-goal-defer-{tag}-{}", uuid_like()));
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.join(".future/goal");
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("FUTURE_GOAL_ROOT", root.to_str().unwrap());
    f(root.to_str().unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

fn uuid_like() -> String {
    format!(
        "{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn cli(args: &[&str]) -> Result<(), String> {
    console::run(
        "future-goal",
        args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    )
    .map_err(|e| format!("{e:#}"))
}

fn todo_id_by_text(root: &str, goal: &str, text_prefix: &str) -> String {
    let store = Store::open(root).unwrap();
    let g = store.replay(goal).unwrap().unwrap();
    g.todos
        .iter()
        .find(|t| t.text.contains(text_prefix))
        .map(|t| t.id.clone())
        .unwrap_or_else(|| panic!("no todo with text containing {text_prefix:?}"))
}

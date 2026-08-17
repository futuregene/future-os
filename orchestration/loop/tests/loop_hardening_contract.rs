//! Contract tests for the 2026-08 loop hardening pass:
//!   1. `todo update --resume-when N` (numeric) sets a REAL deadline
//!      (`defer:N` → resume_when = now + N secs), so deferred/monitor todos
//!      become due again instead of being stuck forever (previously the raw
//!      string was stored and never parsed to SystemTime).
//!   2. `todo complete` refuses to complete a todo blocked by an OPEN
//!      user gate / blocker — the manual CLI bypass of gate enforcement
//!      (which the run loop already applies at schedule time).
//!
//! These exercise the real CLI entry (`console::run`) against an isolated
//! `FUTURE_LOOP_ROOT`, so they cover the parse → event → projection path.

use future_loop::console;
use future_loop::state::TodoStatus;
use future_loop::store::Store;

fn with_root<F: FnOnce(&str)>(tag: &str, f: F) {
    // FUTURE_LOOP_ROOT is process-global; tests run in parallel, so
    // serialize all CLI tests behind one mutex (each still gets its own
    // isolated root dir).
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("future-loop-hardening-{tag}-{}", uuid_like()));
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.join(".future/loop");
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("FUTURE_LOOP_ROOT", root.to_str().unwrap());
    f(root.to_str().unwrap());
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
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
        "future-loop",
        args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    )
    .map_err(|e| format!("{e:#}"))
}

/// Find a todo id in a goal by text prefix.
fn todo_id_by_text(root: &str, goal: &str, text_prefix: &str) -> String {
    let store = Store::open(root).unwrap();
    let g = store.replay(goal).unwrap().unwrap();
    g.todos
        .iter()
        .find(|t| t.text.contains(text_prefix))
        .map(|t| t.id.clone())
        .unwrap_or_else(|| panic!("no todo with text containing {text_prefix:?}"))
}

/// End-to-end: goal init → todo add → todo update --resume-when 30 →
/// status projection shows a REAL SystemTime deadline (~30s out).
#[test]
fn cli_resume_when_numeric_sets_real_deadline() {
    with_root("cli-defer", |root| {
        cli(&[
            "goal",
            "init",
            "--objective",
            "defer test",
            "--cwd",
            "/tmp",
            "--goal-id",
            "g1",
        ])
        .unwrap();
        cli(&[
            "todo",
            "add",
            "--goal",
            "g1",
            "--text",
            "deferred work",
            "--priority",
            "P1",
        ])
        .unwrap();
        let todo_id = todo_id_by_text(root, "g1", "deferred work");
        cli(&[
            "todo",
            "update",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--resume-when",
            "30",
        ])
        .unwrap();

        let store = Store::open(root).unwrap();
        let g = store.replay("g1").unwrap().unwrap();
        let t = g.todo(&todo_id).unwrap();
        assert_eq!(t.status, TodoStatus::Deferred);
        assert_eq!(t.resume_when_text.as_deref(), Some("defer:30"));
        let rw = t
            .resume_when
            .expect("numeric resume-when must set SystemTime");
        let remaining = rw
            .duration_since(std::time::SystemTime::now())
            .unwrap_or_default();
        assert!(
            remaining.as_secs() <= 30 && remaining.as_secs() >= 25,
            "deadline should be ~30s in the future, got {remaining:?}"
        );
    });
}

/// End-to-end: non-numeric `--resume-when` stays a text-only hint (Deferred
/// status, NO SystemTime deadline).
#[test]
fn cli_resume_when_text_keeps_no_deadline() {
    with_root("cli-text", |root| {
        cli(&[
            "goal",
            "init",
            "--objective",
            "text test",
            "--cwd",
            "/tmp",
            "--goal-id",
            "g1",
        ])
        .unwrap();
        cli(&[
            "todo",
            "add",
            "--goal",
            "g1",
            "--text",
            "hint work",
            "--priority",
            "P1",
        ])
        .unwrap();
        let todo_id = todo_id_by_text(root, "g1", "hint work");
        cli(&[
            "todo",
            "update",
            "--goal",
            "g1",
            "--todo-id",
            &todo_id,
            "--resume-when",
            "when-ci-passes",
        ])
        .unwrap();

        let store = Store::open(root).unwrap();
        let g = store.replay("g1").unwrap().unwrap();
        let t = g.todo(&todo_id).unwrap();
        assert_eq!(t.status, TodoStatus::Deferred);
        assert_eq!(t.resume_when_text.as_deref(), Some("when-ci-passes"));
        assert!(
            t.resume_when.is_none(),
            "text-only resume-when: no deadline"
        );
    });
}

/// `todo complete` on a todo blocked by an OPEN user gate must be REJECTED
/// by the CLI (bail with a helpful message), even with --no-follow-up.
#[test]
fn cli_complete_blocked_by_open_gate_rejected() {
    with_root("cli-gate", |root| {
        cli(&[
            "goal",
            "init",
            "--objective",
            "gate test",
            "--cwd",
            "/tmp",
            "--goal-id",
            "g1",
        ])
        .unwrap();
        cli(&[
            "todo",
            "add",
            "--goal",
            "g1",
            "--text",
            "gated work",
            "--priority",
            "P0",
        ])
        .unwrap();
        let t1_id = todo_id_by_text(root, "g1", "gated work");
        // gate g2 blocks t1
        cli(&[
            "todo",
            "add",
            "--goal",
            "g1",
            "--role",
            "user",
            "--class",
            "user_gate",
            "--gate-question",
            "Approve the gated work?",
            "--text",
            "Approve the gated work?",
            "--blocks",
            &t1_id,
        ])
        .unwrap();
        let g2_id = todo_id_by_text(root, "g1", "Approve the gated work?");

        // t1 blocks on g2; completing t1 while g2 is open must fail.
        let err = cli(&[
            "todo",
            "complete",
            "--goal",
            "g1",
            "--todo-id",
            &t1_id,
            "--no-follow-up",
            "--evidence",
            "fixture evidence for completion contract",
        ])
        .expect_err("completing a gate-blocked todo must be rejected");
        assert!(
            err.contains(&g2_id) && err.contains("open gate"),
            "error should name the open gate: {err}"
        );

        // state unchanged: t1 still open, g2 still open.
        let store = Store::open(root).unwrap();
        let g = store.replay("g1").unwrap().unwrap();
        assert_eq!(g.todo(&t1_id).unwrap().status, TodoStatus::Open);
        assert_eq!(g.todo(&g2_id).unwrap().status, TodoStatus::Open);
    });
}

/// After resolving the gate, the previously blocked todo completes cleanly.
#[test]
fn cli_complete_after_gate_resolved() {
    with_root("cli-gate-resolved", |root| {
        cli(&[
            "goal",
            "init",
            "--objective",
            "gate2",
            "--cwd",
            "/tmp",
            "--goal-id",
            "g1",
        ])
        .unwrap();
        cli(&[
            "todo",
            "add",
            "--goal",
            "g1",
            "--text",
            "gated work",
            "--priority",
            "P0",
        ])
        .unwrap();
        let t1_id = todo_id_by_text(root, "g1", "gated work");
        cli(&[
            "todo",
            "add",
            "--goal",
            "g1",
            "--role",
            "user",
            "--class",
            "user_gate",
            "--gate-question",
            "Approve?",
            "--text",
            "Approve?",
            "--blocks",
            &t1_id,
        ])
        .unwrap();
        let g2_id = todo_id_by_text(root, "g1", "Approve?");

        cli(&[
            "gate",
            "resolve",
            "--goal",
            "g1",
            "--todo-id",
            &g2_id,
            "--decision",
            "approved",
        ])
        .unwrap();
        cli(&[
            "todo",
            "complete",
            "--goal",
            "g1",
            "--todo-id",
            &t1_id,
            "--no-follow-up",
            "--evidence",
            "after approval",
        ])
        .unwrap();

        let store = Store::open(root).unwrap();
        let g = store.replay("g1").unwrap().unwrap();
        assert_eq!(g.todo(&t1_id).unwrap().status, TodoStatus::Done);
        assert_eq!(g.todo(&g2_id).unwrap().status, TodoStatus::Done);
    });
}

/// Resolving the gate itself is never blocked (a gate is not blocked by
/// anything), and completing a NON-gate-blocked todo still works.
#[test]
fn cli_complete_unblocked_todo_ok() {
    with_root("cli-free", |root| {
        cli(&[
            "goal",
            "init",
            "--objective",
            "free",
            "--cwd",
            "/tmp",
            "--goal-id",
            "g1",
        ])
        .unwrap();
        cli(&[
            "todo",
            "add",
            "--goal",
            "g1",
            "--text",
            "free work",
            "--priority",
            "P1",
        ])
        .unwrap();
        let t1_id = todo_id_by_text(root, "g1", "free work");
        cli(&[
            "todo",
            "complete",
            "--goal",
            "g1",
            "--todo-id",
            &t1_id,
            "--no-follow-up",
            "--evidence",
            "fixture evidence for completion contract",
        ])
        .unwrap();
        let store = Store::open(root).unwrap();
        let g = store.replay("g1").unwrap().unwrap();
        assert_eq!(g.todo(&t1_id).unwrap().status, TodoStatus::Done);
    });
}

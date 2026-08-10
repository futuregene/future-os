//! Shared helpers for the future-loop coverage drive tests.
//!
//! Each integration-test binary compiles this module and uses a subset of it.
#![allow(dead_code)]
//!
//! Two invocation styles exist side by side:
//!   * in-process (`cli*`) — fast, but shares the process-global
//!     FUTURE_LOOP_ROOT, so every invocation is serialized behind CLI_LOCK
//!     and each test gets its own tempdir root.
//!   * subprocess (`bin*` in the subprocess drive file) — isolated env, and
//!     the only safe way to cover `std::process::exit` / stdin-driven paths.

pub mod mock_agent;

use std::sync::MutexGuard;

/// Serializes in-process CLI invocations (FUTURE_LOOP_ROOT is process-global;
/// tests run on parallel threads).
pub static CLI_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Holds the lock + tempdir for one in-process CLI scenario.
pub struct CliRoot {
    pub _guard: MutexGuard<'static, ()>,
    pub _dir: tempfile::TempDir,
    /// Loop state root (FUTURE_LOOP_ROOT).
    pub root: String,
    /// Scratch cwd for goals (kept inside the tempdir).
    pub cwd: String,
}

pub fn cli_root() -> CliRoot {
    let guard = CLI_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("loop-root");
    std::fs::create_dir_all(&root).unwrap();
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    std::env::set_var("FUTURE_LOOP_ROOT", &root);
    CliRoot {
        _guard: guard,
        _dir: dir,
        root: root.to_string_lossy().into_owned(),
        cwd: cwd.to_string_lossy().into_owned(),
    }
}

/// Invoke the real CLI entry in-process.
pub fn cli(args: &[&str]) -> Result<(), String> {
    future_loop::console::run(
        "future-loop",
        args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
    )
    .map_err(|e| format!("{e:#}"))
}

pub fn cli_ok(args: &[&str]) {
    cli(args).unwrap_or_else(|e| panic!("cli {args:?} should succeed: {e}"));
}

pub fn cli_err(args: &[&str]) -> String {
    match cli(args) {
        Ok(()) => panic!("cli {args:?} should fail"),
        Err(e) => e,
    }
}

/// `goal init` with an explicit id + cwd inside the temp root.
pub fn init_goal(cr: &CliRoot, objective: &str) -> String {
    let gid = format!("goal_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    cli_ok(&[
        "goal",
        "init",
        "--objective",
        objective,
        "--goal-id",
        &gid,
        "--cwd",
        &cr.cwd,
    ]);
    gid
}

/// Add a plain advancement todo; returns its id.
pub fn add_todo(cr: &CliRoot, goal: &str, text: &str) -> String {
    cli_ok(&["todo", "add", "--goal", goal, "--text", text]);
    todo_id_by_text(&cr.root, goal, text)
}

pub fn todo_id_by_text(root: &str, goal: &str, needle: &str) -> String {
    let store = future_loop::store::Store::open(root).unwrap();
    let g = store.replay(goal).unwrap().unwrap();
    g.todos
        .iter()
        .find(|t| t.text.contains(needle))
        .map(|t| t.id.clone())
        .unwrap_or_else(|| panic!("no todo containing {needle:?}"))
}

/// The onboarding todo added by `goal init` (always the first todo).
pub fn first_todo_id(root: &str, goal: &str) -> String {
    let store = future_loop::store::Store::open(root).unwrap();
    let g = store.replay(goal).unwrap().unwrap();
    g.todos
        .first()
        .map(|t| t.id.clone())
        .expect("goal has the onboarding todo")
}

pub fn open_store(cr: &CliRoot) -> future_loop::store::Store {
    future_loop::store::Store::open(&cr.root).unwrap()
}

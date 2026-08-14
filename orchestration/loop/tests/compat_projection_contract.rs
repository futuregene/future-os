//! Project-local projection tests: the on-disk layout keeps everything under
//! the project (GOAL.md at the project root, ACTIVE_GOAL_STATE.md under
//! `.future/loop/goals/<id>/`), and todo anchors render with a stable,
//! URL-encoded format.

use future_loop::compat::{
    acquire_active_state_lock, future_loop_status, future_loop_task_class,
    release_active_state_lock, rfc3339, write_active_state, write_goal_doc,
};
use future_loop::state::{Goal, TaskClass, Todo, TodoStatus};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("future-loop-compat-{tag}-{}", nano()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}
fn nano() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

// ── enum values match reference exactly ────────────────────────────────────────
#[test]
fn enum_values_match_loopx() {
    assert_eq!(
        future_loop_task_class(TaskClass::Advancement),
        "advancement_task"
    );
    assert_eq!(future_loop_task_class(TaskClass::UserGate), "user_gate");
    assert_eq!(future_loop_task_class(TaskClass::UserAction), "user_action");
    assert_eq!(
        future_loop_task_class(TaskClass::Monitor),
        "continuous_monitor"
    );
    assert_eq!(future_loop_task_class(TaskClass::Blocker), "blocker");
    assert_eq!(future_loop_status(TodoStatus::Open), "open");
    assert_eq!(future_loop_status(TodoStatus::Done), "done");
    assert_eq!(future_loop_status(TodoStatus::Blocked), "blocked");
    assert_eq!(future_loop_status(TodoStatus::Deferred), "deferred");
}

// ── file layout (project-local) ───────────────────────────────────────────
#[test]
fn file_layout_is_project_local() {
    let proj = tmp_root("layout");
    let mut goal = Goal::new("g1", "objective", &proj);
    goal.add(Todo::user_gate("ug", "Approve X", &[]));
    goal.add(Todo::advancement("at", "work"));

    write_goal_doc(&proj, &goal.objective).unwrap();
    write_active_state(
        &std::path::Path::new(&proj).join(".future/loop/goals/g1"),
        &goal,
    )
    .unwrap();

    assert!(std::path::Path::new(&proj).join("GOAL.md").exists());
    let state = std::path::Path::new(&proj).join(".future/loop/goals/g1/ACTIVE_GOAL_STATE.md");
    assert!(state.exists());
    // O2 lock liveness: the lock sidecar is acquired (pid written) and
    // released after each projection write — it must not linger as a
    // permanent artifact.
    let lock = std::path::Path::new(&proj).join(".future/loop/goals/g1/ACTIVE_GOAL_STATE.md.lock");
    assert!(!lock.exists(), "lock sidecar is released after the write");
    // While held it carries the holder's pid.
    let goal_dir = std::path::Path::new(&proj).join(".future/loop/goals/g1");
    let held = acquire_active_state_lock(&goal_dir).unwrap();
    let held_pid = std::fs::read_to_string(&held).unwrap();
    assert_eq!(held_pid.trim(), std::process::id().to_string());
    release_active_state_lock(&held);
    assert!(!lock.exists());
    // Nothing reference-layout outside the project: no .codex, no .loopx.
    assert!(!std::path::Path::new(&proj).join(".codex").exists());
    assert!(!std::path::Path::new(&proj).join(".loopx").exists());
}

// ── todo anchors render with LoopX's exact format ──────────────────────────
#[test]
fn todo_anchors_match_future_loop_format() {
    let proj = tmp_root("anchor");
    let mut goal = Goal::new("g1", "objective", &proj);
    // Plain user_gate: no goal_bound/global_gate (reference only renders them on
    // bootstrap-injected gates) — explicit scope flags render them.
    goal.add(Todo::user_gate("ug", "Decide X", &[]));
    goal.add(Todo::advancement("at", "agent work"));
    write_active_state(
        &std::path::Path::new(&proj).join(".future/loop/goals/g1"),
        &goal,
    )
    .unwrap();
    let md = std::fs::read_to_string(format!("{proj}/.future/loop/goals/g1/ACTIVE_GOAL_STATE.md"))
        .unwrap();
    // user gate anchor: task_class present; scope flags absent by default
    assert!(md.contains("task_class=user_gate"));
    assert!(
        !md.contains("goal_bound=true"),
        "plain user_gate has no goal_bound"
    );
    // default advancement omits task_class (reference demo behavior)
    let at_anchor = md.lines().find(|l| l.contains("todo_id=at")).unwrap_or("");
    assert!(
        !at_anchor.contains("task_class"),
        "default advancement hides task_class"
    );
    // timestamps URL-encode the '+' of the tz offset — any offset (CI runs
    // UTC, dev machines vary, e.g. +08:00); colons stay literal.
    let updated_at_line = md.lines().find(|l| l.contains("updated_at=")).unwrap_or("");
    assert!(
        updated_at_line.contains("%2B"),
        "offset must be URL-encoded like reference (line: {updated_at_line})"
    );

    // Explicit gate scope renders the reference flags.
    let proj2 = tmp_root("anchor2");
    let mut goal2 = Goal::new("g2", "objective", &proj2);
    let mut g = Todo::user_gate("ug2", "Decide Y", &[]);
    g = g.with_gate_scope(true, true);
    goal2.add(g);
    write_active_state(
        &std::path::Path::new(&proj2).join(".future/loop/goals/g2"),
        &goal2,
    )
    .unwrap();
    let md2 = std::fs::read_to_string(format!(
        "{proj2}/.future/loop/goals/g2/ACTIVE_GOAL_STATE.md"
    ))
    .unwrap();
    assert!(md2.contains("goal_bound=true global_gate=true"));
}

// ── timestamp format ───────────────────────────────────────────────────────
#[test]
fn rfc3339_matches_future_loop_shape() {
    let ts = rfc3339(1785893919);
    assert!(ts.starts_with("2026-08-05T"), "got {ts}");
    assert!(
        ts.contains('+') || ts.contains("+08"),
        "offset present: {ts}"
    );
}

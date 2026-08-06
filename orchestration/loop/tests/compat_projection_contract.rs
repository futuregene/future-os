//! LoopX-compatible projection tests: the on-disk layout mirrors real LoopX
//! (GOAL.md / .loopx/registry.json / ACTIVE_GOAL_STATE.md / runs/), field
//! sets match, and todo anchors render with LoopX's exact format.

use future_loop::compat::{
    loopx_status, loopx_task_class, rfc3339, write_active_state, write_goal_doc, write_registry,
};
use future_loop::state::{Goal, TaskClass, Todo, TodoStatus};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("loopx-compat-{tag}-{}", nano()));
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

// ── enum values match LoopX exactly ────────────────────────────────────────
#[test]
fn enum_values_match_loopx() {
    assert_eq!(loopx_task_class(TaskClass::Advancement), "advancement_task");
    assert_eq!(loopx_task_class(TaskClass::UserGate), "user_gate");
    assert_eq!(loopx_task_class(TaskClass::UserAction), "user_action");
    assert_eq!(loopx_task_class(TaskClass::Monitor), "continuous_monitor");
    assert_eq!(loopx_task_class(TaskClass::Blocker), "blocker");
    assert_eq!(loopx_status(TodoStatus::Open), "open");
    assert_eq!(loopx_status(TodoStatus::Done), "done");
    assert_eq!(loopx_status(TodoStatus::Blocked), "blocked");
    assert_eq!(loopx_status(TodoStatus::Deferred), "deferred");
}

// ── file layout parity ─────────────────────────────────────────────────────
#[test]
fn file_layout_matches_loopx() {
    let proj = tmp_root("layout");
    let mut goal = Goal::new("g1", "objective", &proj);
    goal.add(Todo::user_gate("ug", "Approve X", &[]));
    goal.add(Todo::advancement("at", "work"));

    write_goal_doc(&proj, &goal.objective).unwrap();
    let goals = vec![&goal];
    write_registry(&proj, &goals, "/tmp/runtime").unwrap();
    write_active_state(&proj, &goal).unwrap();

    assert!(std::path::Path::new(&proj).join("GOAL.md").exists());
    assert!(std::path::Path::new(&proj)
        .join(".loopx/registry.json")
        .exists());
    let state = std::path::Path::new(&proj).join(".codex/goals/g1/ACTIVE_GOAL_STATE.md");
    assert!(state.exists());
    assert!(std::path::Path::new(&proj)
        .join(".codex/goals/g1/ACTIVE_GOAL_STATE.md.lock")
        .exists());
}

#[test]
fn registry_field_set_matches_loopx() {
    let proj = tmp_root("reg");
    let mut goal = Goal::new("g1", "objective", &proj);
    goal.add(Todo::advancement("t", "work"));
    write_registry(&proj, &[&goal], "/tmp/rt").unwrap();
    let d: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(format!("{proj}/.loopx/registry.json")).unwrap(),
    )
    .unwrap();
    let top: std::collections::BTreeSet<String> = d.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        top,
        [
            "common_runtime_root",
            "goals",
            "schema_version",
            "updated_at"
        ]
        .into_iter()
        .map(String::from)
        .collect()
    );
    let g = &d["goals"][0];
    let gkeys: std::collections::BTreeSet<String> =
        g.as_object().unwrap().keys().cloned().collect();
    for expected in [
        "id",
        "domain",
        "status",
        "role",
        "parent_goal_id",
        "repo",
        "state_file",
        "authority_sources",
        "adapter",
        "spawn_policy",
        "coordination",
        "execution_profile",
        "guards",
        "next_probe",
    ] {
        assert!(gkeys.contains(expected), "registry missing {expected}");
    }
    assert_eq!(g["id"], "g1");
    assert_eq!(g["status"], "active");
}

// ── todo anchors render with LoopX's exact format ──────────────────────────
#[test]
fn todo_anchors_match_loopx_format() {
    let proj = tmp_root("anchor");
    let mut goal = Goal::new("g1", "objective", &proj);
    // Plain user_gate: no goal_bound/global_gate (LoopX only renders them on
    // bootstrap-injected gates) — explicit scope flags render them.
    goal.add(Todo::user_gate("ug", "Decide X", &[]));
    goal.add(Todo::advancement("at", "agent work"));
    write_active_state(&proj, &goal).unwrap();
    let md =
        std::fs::read_to_string(format!("{proj}/.codex/goals/g1/ACTIVE_GOAL_STATE.md")).unwrap();
    // user gate anchor: task_class present; scope flags absent by default
    assert!(md.contains("task_class=user_gate"));
    assert!(
        !md.contains("goal_bound=true"),
        "plain user_gate has no goal_bound"
    );
    // default advancement omits task_class (LoopX demo behavior)
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
        "offset must be URL-encoded like LoopX (line: {updated_at_line})"
    );

    // Explicit gate scope renders the LoopX flags.
    let proj2 = tmp_root("anchor2");
    let mut goal2 = Goal::new("g2", "objective", &proj2);
    let mut g = Todo::user_gate("ug2", "Decide Y", &[]);
    g = g.with_gate_scope(true, true);
    goal2.add(g);
    write_active_state(&proj2, &goal2).unwrap();
    let md2 =
        std::fs::read_to_string(format!("{proj2}/.codex/goals/g2/ACTIVE_GOAL_STATE.md")).unwrap();
    assert!(md2.contains("goal_bound=true global_gate=true"));
}

// ── timestamp format ───────────────────────────────────────────────────────
#[test]
fn rfc3339_matches_loopx_shape() {
    let ts = rfc3339(1785893919);
    assert!(ts.starts_with("2026-08-05T"), "got {ts}");
    assert!(
        ts.contains('+') || ts.contains("+08"),
        "offset present: {ts}"
    );
}

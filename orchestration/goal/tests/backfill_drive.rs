//! Coverage drive for `backfill.rs` — markdown parsing branches (sections,
//! markers, metadata keys, continuations), class/priority derivation,
//! privacy redaction, and the workbench file reader.

use future_goal::backfill::{
    active_state_markdown, backfill_event_id, backfill_todo_events, parse_markdown_todos,
};
use future_goal::projection::privacy::PrivacyLevel;
use future_goal::state::TaskClass;

/// A workbench exercising every parser branch.
const FULL_MD: &str = "---\nstatus: active\n---\n\n# Active Goal State\n\n\
## Not A Todo Section\n\n\
- [ ] ignored line (no role yet)\n\n\
## Agent Todo\n\n\
- [ ] [P0] First task\n  <!-- future-goal:todo todo_id=todo_a1 status=open action_kind=shell claimed_by=agent-1 monitor_target=file:x monitor_policy=exists cadence=15m goal_bound=true updated_at=2026-08-05T12:00:00+00:00 -->\n\
- [x] [P2] Done task\n  <!-- future-goal:todo todo_id=todo_a2 status=done no_followup=true evidence=great%20success completed_at=2026-08-05T13:00:00+00:00 updated_at=not-a-date note=check%2Bplus -->\n\
- [-] Deferred task\n\
- [ ] \n\
- [ ] Continued task\n  this continuation text joins the record\n\
- [malformed line without close bracket\n\n\
## User Todo / Owner Review\n\n\
- [ ] Decide scope\n  <!-- future-goal:todo task_class=user_action -->\n\
- [ ] Gate without explicit class (user default)\n\
- [ ] External blocker\n  <!-- future-goal:todo task_class=blocker -->\n\
- [ ] Watch the deploy\n  <!-- future-goal:todo task_class=continuous_monitor -->\n\
- [ ] Explicit monitor\n  <!-- future-goal:todo task_class=monitor global_gate=true -->\n";

#[test]
fn parse_markdown_full_branch_matrix() {
    let records = parse_markdown_todos(FULL_MD);
    // 9 records (the no-role line and the empty-text line are skipped; the
    // malformed no-bracket line is skipped).
    assert_eq!(records.len(), 9, "{records:?}");
    let first = &records[0];
    assert_eq!(first.role, "agent");
    assert_eq!(first.monitor_target.as_deref(), Some("file:x"));
    assert_eq!(first.monitor_policy.as_deref(), Some("exists"));
    assert_eq!(first.cadence.as_deref(), Some("15m"));
    assert!(first.goal_bound);
    assert_eq!(first.claimed_by.as_deref(), Some("agent-1"));
    let done = &records[1];
    assert_eq!(done.status, "done");
    assert!(done.no_followup);
    assert_eq!(
        done.evidence.as_deref(),
        Some("great success"),
        "url-decoded"
    );
    assert_eq!(done.note.as_deref(), Some("check+plus"));
    assert_eq!(records[2].status, "deferred");
    // Continuation text joins the todo text.
    let cont = records
        .iter()
        .find(|r| r.text.contains("continuation text"))
        .unwrap();
    assert!(cont.text.contains("Continued task"));
    // User-role records.
    let user_action = records
        .iter()
        .find(|r| r.text.contains("Decide scope"))
        .unwrap();
    assert_eq!(user_action.role, "user");
    assert_eq!(user_action.task_class.as_deref(), Some("user_action"));
}

#[test]
fn backfill_events_class_priority_and_privacy() {
    let outcome = backfill_todo_events(FULL_MD, "g1", PrivacyLevel::LocalPrivate).unwrap();
    // 8 todos; events = adds + 1 claim (agent-1) + 1 complete (done task).
    let completes: Vec<_> = outcome
        .events
        .iter()
        .filter(|e| matches!(e.event, future_goal::store::Event::TodoCompleted { .. }))
        .collect();
    assert_eq!(completes.len(), 1);
    let claims: Vec<_> = outcome
        .events
        .iter()
        .filter(|e| matches!(e.event, future_goal::store::Event::TodoClaimed { .. }))
        .collect();
    assert_eq!(claims.len(), 1);
    let classes: Vec<TaskClass> = outcome
        .events
        .iter()
        .filter_map(|e| match &e.event {
            future_goal::store::Event::TodoAdded { todo, .. } => Some(todo.class),
            _ => None,
        })
        .collect();
    assert!(classes.contains(&TaskClass::UserAction));
    assert!(classes.contains(&TaskClass::UserGate), "user default class");
    assert!(classes.contains(&TaskClass::Blocker));
    assert_eq!(
        classes.iter().filter(|c| **c == TaskClass::Monitor).count(),
        2,
        "continuous_monitor and monitor both map to Monitor"
    );
    // Priorities from [P0]/[P2] prefixes.
    let p0 = outcome.events.iter().find_map(|e| match &e.event {
        future_goal::store::Event::TodoAdded { todo, .. } if todo.text.contains("First task") => {
            Some(todo.priority)
        }
        _ => None,
    });
    assert_eq!(p0, Some(future_goal::state::Priority::P0));
    // A record without todo_id gets a content-derived id (deterministic).
    let digest_todo = outcome.events.iter().find_map(|e| match &e.event {
        future_goal::store::Event::TodoAdded { todo, .. }
            if todo.text.contains("Deferred task") =>
        {
            Some(todo.id.clone())
        }
        _ => None,
    });
    assert!(digest_todo.as_deref().unwrap().starts_with("todo-"));
    // Public-safe privacy passes evidence through the redactor (secret
    // patterns would be masked; plain text passes through).
    let public = backfill_todo_events(FULL_MD, "g1", PrivacyLevel::PublicSafe).unwrap();
    let ev = public.events.iter().find_map(|e| match &e.event {
        future_goal::store::Event::TodoAdded { todo, .. } if todo.id == "todo_a2" => {
            todo.evidence.clone()
        }
        _ => None,
    });
    assert!(ev.is_some());
    // Empty goal id / no records → errors.
    assert!(backfill_todo_events(FULL_MD, "", PrivacyLevel::LocalPrivate).is_err());
    assert!(backfill_todo_events("# nothing", "g1", PrivacyLevel::LocalPrivate).is_err());
}

#[test]
fn backfill_ids_are_deterministic() {
    assert_eq!(
        backfill_event_id("g", "t", "add"),
        backfill_event_id("g", "t", "add")
    );
    assert!(backfill_event_id("g", "t", "add").starts_with("backfill-add-"));
}

#[test]
fn active_state_markdown_read_paths() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_string_lossy().into_owned();
    // Missing file → error mentioning the path.
    assert!(active_state_markdown(&root, "goal_x").is_err());
    // Present file → contents. The workbench sits directly under the STATE
    // ROOT (`<root>/goals/<id>/`), not under `<root>/.future/goal/`.
    let p = dir.path().join("goals/goal_x");
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("ACTIVE_GOAL_STATE.md"), "# state").unwrap();
    assert_eq!(active_state_markdown(&root, "goal_x").unwrap(), "# state");
}

/// State paths must follow `FUTURE_GOAL_ROOT`: with the override set the
/// workbench is read from there, and the project-relative layout would point at
/// a directory that holds nothing.
#[test]
fn active_state_markdown_follows_the_state_root_override() {
    // `FUTURE_GOAL_ROOT` is process-global; each integration test file is its
    // own process, so a file-local lock is enough to keep this single env
    // mutation off the other tests here.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    /// Removes the override even if an assertion below panics.
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            std::env::remove_var("FUTURE_GOAL_ROOT");
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let root = dir.path().join("override-root");
    std::fs::create_dir_all(root.join("goals/goal_x")).unwrap();
    std::fs::write(
        root.join("goals/goal_x/ACTIVE_GOAL_STATE.md"),
        "# from root",
    )
    .unwrap();
    // A decoy in the project-relative location that must NOT be picked up
    // while the override is set.
    std::fs::create_dir_all(project.join(".future/goal/goals/goal_x")).unwrap();
    std::fs::write(
        project.join(".future/goal/goals/goal_x/ACTIVE_GOAL_STATE.md"),
        "# decoy",
    )
    .unwrap();
    let project = project.to_string_lossy().into_owned();

    let without = active_state_markdown(
        &future_goal::store::state_root_for(project.as_ref()),
        "goal_x",
    )
    .unwrap();
    std::env::set_var("FUTURE_GOAL_ROOT", &root);
    let _restore = Restore;
    let via_override = active_state_markdown(
        &future_goal::store::state_root_for(project.as_ref()),
        "goal_x",
    )
    .unwrap();

    assert_eq!(via_override, "# from root", "the override must win");
    assert_eq!(without, "# decoy", "without the override the project wins");
}

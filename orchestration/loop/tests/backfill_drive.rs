//! Coverage drive for `backfill.rs` — markdown parsing branches (sections,
//! markers, metadata keys, continuations), class/priority derivation,
//! privacy redaction, and the workbench file reader.

use future_loop::backfill::{
    active_state_markdown, backfill_event_id, backfill_todo_events, parse_markdown_todos,
};
use future_loop::projection::privacy::PrivacyLevel;
use future_loop::state::TaskClass;

/// A workbench exercising every parser branch.
const FULL_MD: &str = "---\nstatus: active\n---\n\n# Active Goal State\n\n\
## Not A Todo Section\n\n\
- [ ] ignored line (no role yet)\n\n\
## Agent Todo\n\n\
- [ ] [P0] First task\n  <!-- future-loop:todo todo_id=todo_a1 status=open action_kind=shell claimed_by=agent-1 monitor_target=file:x monitor_policy=exists cadence=15m goal_bound=true updated_at=2026-08-05T12:00:00+00:00 -->\n\
- [x] [P2] Done task\n  <!-- future-loop:todo todo_id=todo_a2 status=done no_followup=true evidence=great%20success completed_at=2026-08-05T13:00:00+00:00 updated_at=not-a-date note=check%2Bplus -->\n\
- [-] Deferred task\n\
- [ ] \n\
- [ ] Continued task\n  this continuation text joins the record\n\
- [malformed line without close bracket\n\n\
## User Todo / Owner Review\n\n\
- [ ] Decide scope\n  <!-- future-loop:todo task_class=user_action -->\n\
- [ ] Gate without explicit class (user default)\n\
- [ ] External blocker\n  <!-- future-loop:todo task_class=blocker -->\n\
- [ ] Watch the deploy\n  <!-- future-loop:todo task_class=continuous_monitor -->\n\
- [ ] Explicit monitor\n  <!-- future-loop:todo task_class=monitor global_gate=true -->\n";

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
    assert_eq!(done.evidence.as_deref(), Some("great success"), "url-decoded");
    assert_eq!(done.note.as_deref(), Some("check+plus"));
    assert_eq!(records[2].status, "deferred");
    // Continuation text joins the todo text.
    let cont = records.iter().find(|r| r.text.contains("continuation text")).unwrap();
    assert!(cont.text.contains("Continued task"));
    // User-role records.
    let user_action = records.iter().find(|r| r.text.contains("Decide scope")).unwrap();
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
        .filter(|e| matches!(e.event, future_loop::store::Event::TodoCompleted { .. }))
        .collect();
    assert_eq!(completes.len(), 1);
    let claims: Vec<_> = outcome
        .events
        .iter()
        .filter(|e| matches!(e.event, future_loop::store::Event::TodoClaimed { .. }))
        .collect();
    assert_eq!(claims.len(), 1);
    let classes: Vec<TaskClass> = outcome
        .events
        .iter()
        .filter_map(|e| match &e.event {
            future_loop::store::Event::TodoAdded { todo, .. } => Some(todo.class),
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
        future_loop::store::Event::TodoAdded { todo, .. } if todo.text.contains("First task") => {
            Some(todo.priority)
        }
        _ => None,
    });
    assert_eq!(p0, Some(future_loop::state::Priority::P0));
    // A record without todo_id gets a content-derived id (deterministic).
    let digest_todo = outcome.events.iter().find_map(|e| match &e.event {
        future_loop::store::Event::TodoAdded { todo, .. } if todo.text.contains("Deferred task") => {
            Some(todo.id.clone())
        }
        _ => None,
    });
    assert!(digest_todo.as_deref().unwrap().starts_with("todo-"));
    // Public-safe privacy passes evidence through the redactor (secret
    // patterns would be masked; plain text passes through).
    let public = backfill_todo_events(FULL_MD, "g1", PrivacyLevel::PublicSafe).unwrap();
    let ev = public.events.iter().find_map(|e| match &e.event {
        future_loop::store::Event::TodoAdded { todo, .. } if todo.id == "todo_a2" => {
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
    let cwd = dir.path().to_string_lossy().into_owned();
    // Missing file → error mentioning the path.
    assert!(active_state_markdown(&cwd, "goal_x").is_err());
    // Present file → contents.
    let p = dir
        .path()
        .join(".future/loop/goals/goal_x");
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("ACTIVE_GOAL_STATE.md"), "# state").unwrap();
    assert_eq!(active_state_markdown(&cwd, "goal_x").unwrap(), "# state");
}

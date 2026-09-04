//! Supervisor projection (G-16) — the supervisor event log for one goal,
//! polled via `supervisor events --goal G`. Two durable, projection-only
//! event kinds surface here (both ledgered first, then pushed — see
//! `notify_supervisor` in console.rs):
//!
//! - `ProgressReported` — a worker's mid-run milestone note (the `report`
//!   command), advisory and never a push.
//! - `SupervisorNote` — the durable half of the dual-mode notify
//!   (todo completed/failed/infra-stop/ask_user/host_died).

use crate::store::{Event, Store};

pub const SUPERVISOR_EVENT_PROJECTION_SCHEMA_VERSION: &str = "supervisor_event_projection_v1";

/// Project the supervisor event log for one goal.
pub fn build_supervisor_event_projection(
    store: &Store,
    goal_id: &str,
) -> anyhow::Result<serde_json::Value> {
    let events = store.events(goal_id)?;
    let mut progress: Vec<(String, String, String, u64)> = vec![];
    let mut notes: Vec<(String, String, String, String, u64)> = vec![];
    for stored in &events {
        match &stored.event {
            // Worker mid-run progress notes — advisory, projection-only (see
            // `report` command). Collected for the supervisor's idle-loop
            // consumption; never a push.
            Event::ProgressReported {
                goal_id: g,
                agent_id,
                todo_id,
                message,
                ts,
            } if g == goal_id => {
                progress.push((agent_id.clone(), todo_id.clone(), message.clone(), *ts))
            }
            // Supervisor intervention notes (todo completed/failed/infra-stop/
            // ask_user/host_died) — the durable half of the dual-mode notify
            // (ledgered first, then pushed). Polled via `supervisor events`.
            Event::SupervisorNote {
                goal_id: g,
                todo_id,
                note_kind,
                message,
                dedup_key,
                ts,
            } if g == goal_id => notes.push((
                todo_id.clone(),
                note_kind.clone(),
                message.clone(),
                dedup_key.clone(),
                *ts,
            )),
            _ => {}
        }
    }
    let progress_items: Vec<serde_json::Value> = progress
        .into_iter()
        .map(|(agent_id, todo_id, message, ts)| {
            serde_json::json!({
                "agent_id": agent_id,
                "todo_id": todo_id,
                "message": message,
                "ts": ts,
            })
        })
        .collect();
    let note_items: Vec<serde_json::Value> = notes
        .into_iter()
        .map(|(todo_id, kind, message, dedup_key, ts)| {
            serde_json::json!({
                "todo_id": todo_id,
                "kind": kind,
                "message": message,
                "dedup_key": dedup_key,
                "ts": ts,
            })
        })
        .collect();
    Ok(serde_json::json!({
        "ok": true,
        "schema_version": SUPERVISOR_EVENT_PROJECTION_SCHEMA_VERSION,
        "goal_id": goal_id,
        "progress_count": progress_items.len(),
        "progress": progress_items,
        "note_count": note_items.len(),
        "notes": note_items,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Goal;
    use crate::store::Store;

    fn tmp_root(tag: &str) -> String {
        let dir = std::env::temp_dir().join(format!(
            "future-loop-p3-supervisor-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().into_owned()
    }

    fn open_goal(store: &mut Store, goal_id: &str) {
        let goal = Goal::new(goal_id, "objective", "/tmp");
        store.register(&goal).unwrap();
        store
            .append(Event::GoalStarted {
                goal_id: goal_id.into(),
                ts: goal.created_at,
            })
            .unwrap();
    }

    #[test]
    fn progress_reports_surface_in_projection() {
        let root = tmp_root("progress");
        let mut store = Store::open(&root).unwrap();
        open_goal(&mut store, "g1");
        store
            .append(Event::ProgressReported {
                goal_id: "g1".into(),
                agent_id: "agent-b".into(),
                todo_id: "todo-1".into(),
                message: "submitted attempt 34444, waiting on score".into(),
                ts: 100,
            })
            .unwrap();
        // A report for another goal must not leak into this projection.
        let goal2 = crate::state::Goal::new("g2", "obj2", "/tmp");
        store.register(&goal2).unwrap();
        store
            .append(Event::GoalStarted {
                goal_id: "g2".into(),
                ts: goal2.created_at,
            })
            .unwrap();
        store
            .append(Event::ProgressReported {
                goal_id: "g2".into(),
                agent_id: "agent-c".into(),
                todo_id: "".into(),
                message: "other goal".into(),
                ts: 101,
            })
            .unwrap();
        let projection = build_supervisor_event_projection(&store, "g1").unwrap();
        assert_eq!(projection["progress_count"], 1);
        assert_eq!(projection["progress"][0]["agent_id"], "agent-b");
        assert_eq!(projection["progress"][0]["todo_id"], "todo-1");
        assert_eq!(
            projection["progress"][0]["message"],
            "submitted attempt 34444, waiting on score"
        );
        assert_eq!(projection["progress"][0]["ts"], 100);
        // Projection-only: replay must not mutate the goal kanban.
        let goal = store.replay("g1").unwrap().unwrap();
        assert!(goal.todos.is_empty());
    }
}

//! Stale-latest-run projection (G-5) — warning when the active state was
//! updated AFTER the latest recorded run, mirroring LoopX
//! `control_plane/runtime/stale_latest_run.py`: don't trust
//! latest_run-derived routing before a refresh-state.

use std::path::Path;

use serde::Serialize;

use crate::state::Goal;

#[derive(Debug, Clone, Serialize)]
pub struct StaleRunWarning {
    pub kind: String,
    pub source: String,
    pub severity: String,
    pub requires_refresh_state: bool,
    pub reason: String,
    pub active_state_updated_at: Option<u64>,
    pub latest_run_recorded_at: Option<u64>,
    pub recommended_action: String,
}

/// Compare the newest state touch (max todo updated_at, plus the
/// next_action file mtime when present) against the latest run's
/// `recorded_at`. Newer state ⇒ stale-latest-run warning.
pub fn stale_latest_run(goal: &Goal, goal_dir: &Path) -> Option<StaleRunWarning> {
    let latest_run = goal.history.iter().map(|r| r.recorded_at).max();
    let mut state_updated_at = goal.todos.iter().map(|t| t.updated_at).max();
    // The next_action projection file is part of active state.
    let na_path = goal_dir.join("next_action.txt");
    let na_mtime = std::fs::metadata(&na_path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|mtime| mtime.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
        .map(|secs| secs.as_secs());
    state_updated_at =
        na_mtime.map_or(state_updated_at, |e| Some(state_updated_at.map_or(e, |v| v.max(e))));
    let (Some(state_at), Some(run_at)) = (state_updated_at, latest_run) else {
        return None;
    };
    if state_at <= run_at {
        return None;
    }
    Some(StaleRunWarning {
        kind: "stale_latest_run_projection".to_string(),
        source: "active_state_vs_latest_run".to_string(),
        severity: "warning".to_string(),
        requires_refresh_state: true,
        reason: "active_state_updated_after_latest_run".to_string(),
        active_state_updated_at: Some(state_at),
        latest_run_recorded_at: Some(run_at),
        recommended_action: "run refresh-state before trusting latest_run-derived routing"
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RunRecord, Todo};

    fn run(recorded_at: u64) -> RunRecord {
        RunRecord {
            turn: 1,
            todo_id: "t1".to_string(),
            run_id: "r1".to_string(),
            terminal_state: "completed".to_string(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: String::new(),
            recorded_at,
            spend_source: Some("run".to_string()),
            validation: None,
        }
    }

    #[test]
    fn warns_when_state_is_newer_than_latest_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut goal = Goal::new("g", "objective", "/tmp");
        let mut todo = Todo::advancement("t1", "work");
        todo.updated_at = 2_000;
        goal.add(todo);
        goal.history.push(run(1_000));
        let warning = stale_latest_run(&goal, dir.path());
        assert!(warning.is_some());
        assert_eq!(
            warning.unwrap().reason,
            "active_state_updated_after_latest_run"
        );
    }

    #[test]
    fn no_warning_when_run_is_newer() {
        let dir = tempfile::tempdir().unwrap();
        let mut goal = Goal::new("g", "objective", "/tmp");
        let mut todo = Todo::advancement("t1", "work");
        todo.updated_at = 500;
        goal.add(todo);
        goal.history.push(run(1_000));
        assert!(stale_latest_run(&goal, dir.path()).is_none());
    }

    #[test]
    fn no_warning_without_runs() {
        let dir = tempfile::tempdir().unwrap();
        let goal = Goal::new("g", "objective", "/tmp");
        assert!(stale_latest_run(&goal, dir.path()).is_none());
    }
}

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

/// Compare the newest state touch (max todo updated_at, plus the last write of
/// the `next_action` projection) against the latest run's `recorded_at`. Newer
/// state ⇒ stale-latest-run warning.
pub fn stale_latest_run(goal: &Goal, goal_dir: &Path) -> Option<StaleRunWarning> {
    let latest_run = goal.history.iter().map(|r| r.recorded_at).max();
    let mut state_updated_at = goal.todos.iter().map(|t| t.updated_at).max();
    // The next_action projection is part of active state.
    let na_touched_at = next_action_touched_at(goal_dir);
    state_updated_at = na_touched_at.map_or(state_updated_at, |e| {
        Some(state_updated_at.map_or(e, |v| v.max(e)))
    });
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

/// When the `next_action` projection last changed, on the ledger's clock.
///
/// The stamp the store writes beside the projection is the answer, because it
/// carries a timestamp this machine wrote and the ledger's `recorded_at` shares
/// its clock. The file system's modification time is **not**: a goal directory on
/// a share whose server clock runs minutes off dates every projection write with
/// that share's clock, and comparing it with the ledger mixes two clocks — a fast
/// share produces a permanent "state is newer than the last run" warning that no
/// refresh can clear (the warning exists to prompt exactly that refresh), and a
/// slow one hides the warning the refresh was meant to raise.
///
/// A goal last written before the stamp existed has none, so its modification
/// time is the only signal left. It is clamped to `now` on the way in: a share
/// running ahead is the direction that produces the warning nobody can clear,
/// while a share running behind can only delay one until the next write stamps
/// the projection properly.
fn next_action_touched_at(goal_dir: &Path) -> Option<u64> {
    let stamp = goal_dir.join(crate::store::NEXT_ACTION_STAMP_FILE);
    if let Some(secs) = std::fs::read_to_string(&stamp)
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
    {
        return Some(secs.min(crate::state::now_epoch()));
    }
    let modified = std::fs::metadata(goal_dir.join("next_action.txt"))
        .and_then(|meta| meta.modified())
        .ok()?
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(modified.min(crate::state::now_epoch()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{RunRecord, Todo};

    fn run(recorded_at: u64) -> RunRecord {
        RunRecord {
            agent_id: None,
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
            failure_kind: None,
            truncation: None,
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

    /// The projection's stamp is what dates it, and it is read from the file's
    /// content — never from the modification time a share would stamp with its own
    /// clock.
    #[test]
    fn the_projection_is_dated_by_its_stamp_not_by_the_file_system() {
        let dir = tempfile::tempdir().unwrap();
        // Text written "now" by the share, stamped two hours in the past by it.
        std::fs::write(dir.path().join("next_action.txt"), "do the next thing").unwrap();
        std::fs::write(dir.path().join("next_action.stamp"), "5000").unwrap();
        let when = next_action_touched_at(dir.path()).expect("a stamp is a touch");
        assert_eq!(when, 5000, "the content decides, not the mtime");

        // A goal that predates the stamp: the mtime is the only signal left.
        std::fs::remove_file(dir.path().join("next_action.stamp")).unwrap();
        let legacy = next_action_touched_at(dir.path()).expect("the mtime is a touch");
        assert_ne!(legacy, 5000);
        assert!(
            legacy > 1_700_000_000,
            "it is this machine's clock: {legacy}"
        );
    }

    /// A share whose clock runs *ahead* is the direction that hurts: the projection
    /// would look permanently newer than the last run, so the warning would never
    /// clear however many refresh-state passes the user ran. A timestamp in the
    /// future is clamped to now instead.
    #[test]
    fn a_projection_dated_in_the_future_cannot_pin_the_warning() {
        let dir = tempfile::tempdir().unwrap();
        let run_at = crate::state::now_epoch() - 60;
        let mut goal = Goal::new("g", "objective", "/tmp");
        goal.history.push(run(run_at));
        std::fs::write(
            dir.path().join("next_action.stamp"),
            (crate::state::now_epoch() + 7_200).to_string(),
        )
        .unwrap();

        let warning = stale_latest_run(&goal, dir.path()).expect("state is newer than the run");
        assert_eq!(
            warning.active_state_updated_at,
            Some(crate::state::now_epoch())
        );
        assert_eq!(warning.latest_run_recorded_at, Some(run_at));
    }

    /// Both halves of the comparison have to come from the same clock, which is why
    /// this one does not read the file system at all.
    #[test]
    fn a_projection_stamped_before_the_run_raises_no_warning() {
        let dir = tempfile::tempdir().unwrap();
        let mut goal = Goal::new("g", "objective", "/tmp");
        goal.history.push(run(9_000));
        std::fs::write(dir.path().join("next_action.txt"), "do the next thing").unwrap();
        std::fs::write(dir.path().join("next_action.stamp"), "8000").unwrap();
        assert!(stale_latest_run(&goal, dir.path()).is_none());
    }
}

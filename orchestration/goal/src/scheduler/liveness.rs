//! Automation liveness (P1-3①) — "is the scheduler itself alive?"
//! meta-monitoring, the native subset of LoopX
//! `control_plane/scheduler/automation_liveness.py`.
//!
//! Every `scheduler tick` lands a [`crate::store::Event::SchedulerTicked`]
//! heartbeat (per goal + agent). A liveness check compares now against the
//! latest heartbeat; when the silence exceeds the threshold the check
//! records an [`crate::store::Event::AutomationLivenessAlert`] (folded into
//! `goal.liveness_alerts`) and the attention projection escalates the goal
//! to a high-severity operator item until a fresh heartbeat recovers it.
//! This is the overnight-unattended-goal safety net: a dead host automation
//! surfaces instead of silently starving the goal.

use serde::{Deserialize, Serialize};

/// Schema version of the liveness evaluation projection (LoopX
/// `AUTOMATION_LIVENESS_SCHEMA_VERSION`).
pub const AUTOMATION_LIVENESS_SCHEMA_VERSION: &str = "automation_liveness_v0";

/// Default silence threshold: 2h. A goal whose cadence tops out at the
/// hourly class must tick at least this often; anything longer means the
/// host automation (cron / codex-app / loop driver) is dead or stuck.
pub const DEFAULT_LIVENESS_THRESHOLD_SECS: u64 = 2 * 60 * 60;

/// Alert cooldown: a sustained breach re-alerts at most this often so a dead
/// automation does not flood the event ledger (each alert is one event).
pub const LIVENESS_ALERT_COOLDOWN_SECS: u64 = 60 * 60;

/// Liveness state labels (stable wire values).
pub const LIVENESS_ALIVE: &str = "alive";
pub const LIVENESS_BREACH: &str = "breach";
pub const LIVENESS_NO_HEARTBEAT: &str = "no_heartbeat";

/// The liveness evaluation for one (goal, agent) automation scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LivenessEvaluation {
    pub schema_version: String,
    pub goal_id: String,
    pub agent_id: String,
    /// Latest heartbeat (epoch secs); `None` = the automation never ticked.
    pub last_tick_at: Option<u64>,
    /// Silence so far (epoch secs); `None` when there is no heartbeat.
    pub elapsed_secs: Option<u64>,
    pub threshold_secs: u64,
    /// `alive` | `breach` | `no_heartbeat`.
    pub state: String,
}

/// Evaluate liveness for one automation scope. Pure + deterministic: the
/// caller supplies `last_tick_at` (max of the heartbeat-event ts and the
/// persisted scheduler-state `updated_at`) and `now`.
///
/// - no heartbeat → `no_heartbeat` (automation never installed — NOT a
///   breach: nothing was expected to run);
/// - silence > threshold → `breach`;
/// - otherwise → `alive` (equality with the threshold is still alive).
pub fn evaluate_liveness(
    goal_id: &str,
    agent_id: &str,
    last_tick_at: Option<u64>,
    now: u64,
    threshold_secs: u64,
) -> LivenessEvaluation {
    let elapsed = last_tick_at.map(|t| now.saturating_sub(t));
    let state = match elapsed {
        None => LIVENESS_NO_HEARTBEAT,
        Some(e) if e > threshold_secs => LIVENESS_BREACH,
        Some(_) => LIVENESS_ALIVE,
    };
    LivenessEvaluation {
        schema_version: AUTOMATION_LIVENESS_SCHEMA_VERSION.to_string(),
        goal_id: goal_id.to_string(),
        agent_id: agent_id.to_string(),
        last_tick_at,
        elapsed_secs: elapsed,
        threshold_secs,
        state: state.to_string(),
    }
}

/// Should a breach append a new alert event? Cooldown dedup: at most one
/// alert per `LIVENESS_ALERT_COOLDOWN_SECS` window per scope (LoopX
/// automation_liveness alerts escalate, they do not spam).
pub fn alert_due(last_alert_at: Option<u64>, now: u64) -> bool {
    last_alert_at.is_none_or(|t| now.saturating_sub(t) >= LIVENESS_ALERT_COOLDOWN_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_heartbeat_is_not_a_breach() {
        let eval = evaluate_liveness("g1", "codex-app", None, 1_000_000, 100);
        assert_eq!(eval.state, LIVENESS_NO_HEARTBEAT);
        assert_eq!(eval.last_tick_at, None);
        assert_eq!(eval.elapsed_secs, None);
    }

    #[test]
    fn fresh_heartbeat_is_alive() {
        let eval = evaluate_liveness("g1", "codex-app", Some(1_000_000), 1_000_050, 100);
        assert_eq!(eval.state, LIVENESS_ALIVE);
        assert_eq!(eval.elapsed_secs, Some(50));
    }

    #[test]
    fn silence_past_threshold_breaches() {
        let eval = evaluate_liveness("g1", "codex-app", Some(1_000_000), 1_000_101, 100);
        assert_eq!(eval.state, LIVENESS_BREACH);
        assert_eq!(eval.elapsed_secs, Some(101));
    }

    #[test]
    fn threshold_boundary_is_alive_and_clock_skew_is_clamped() {
        // Exactly at the threshold: alive (breach is strictly greater).
        let eval = evaluate_liveness("g1", "codex-app", Some(1_000_000), 1_000_100, 100);
        assert_eq!(eval.state, LIVENESS_ALIVE);
        // now BEFORE the heartbeat (clock skew) clamps to 0, never breaches.
        let skewed = evaluate_liveness("g1", "codex-app", Some(1_000_100), 1_000_000, 100);
        assert_eq!(skewed.state, LIVENESS_ALIVE);
        assert_eq!(skewed.elapsed_secs, Some(0));
    }

    #[test]
    fn alert_cooldown_dedups() {
        assert!(alert_due(None, 1_000_000));
        let alerted = 1_000_000;
        assert!(!alert_due(Some(alerted), alerted + 60));
        assert!(alert_due(
            Some(alerted),
            alerted + LIVENESS_ALERT_COOLDOWN_SECS
        ));
    }
}

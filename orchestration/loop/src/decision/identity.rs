//! Identity gate — LoopX fail-closed: `quota should-run --agent-id` requires
//! the identity to be a registered coordination peer. An unregistered
//! identity ⇒ `automation_prompt_upgrade_required`, no delivery.

use crate::contract::{ShouldRunPacket, TurnMode, UserChannel};
use crate::state::Goal;

use super::packet;
use super::primary_action::agent_channel;

/// Fail-closed identity gate. Returns the blocked packet when `agent_id` is
/// present but not a registered peer; `None` lets the pipeline continue.
pub(crate) fn identity_gate(goal: &Goal, agent_id: Option<&str>) -> Option<ShouldRunPacket> {
    if goal.is_registered_agent(agent_id) {
        return None;
    }
    // LoopX: state=blocked_health, status=quota_collection_failed, ok=false.
    let mut p = packet(
        goal,
        crate::quota::error_codes::DecisionReasonCode::IdentityNotRegistered,
        "skip",
        false,
        "automation_prompt_upgrade_required",
        TurnMode::WaitMonitor,
        "quota should-run --agent-id requires coordination.registered_agents; \
         register this agent identity first",
        UserChannel::none(),
        agent_channel(None, None, None, false, false, true),
    );
    p.ok = false;
    p.state = "blocked_health".to_string();
    p.status = "quota_collection_failed".to_string();
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Goal;

    #[test]
    fn anonymous_identity_passes() {
        let g = Goal::new("g", "objective", "/tmp");
        assert!(identity_gate(&g, None).is_none());
    }

    #[test]
    fn registered_identity_passes() {
        let mut g = Goal::new("g", "objective", "/tmp");
        g.register_agent("a1", vec![]);
        assert!(identity_gate(&g, Some("a1")).is_none());
    }

    #[test]
    fn unregistered_identity_fails_closed() {
        let g = Goal::new("g", "objective", "/tmp");
        let p = identity_gate(&g, Some("ghost")).expect("unregistered identity must block");
        assert!(!p.ok);
        assert!(!p.should_run);
        assert!(!p.actionable_by_codex);
        assert_eq!(p.state, "blocked_health");
        assert_eq!(p.status, "quota_collection_failed");
        assert_eq!(p.decision, "skip");
        assert_eq!(p.effective_action, "automation_prompt_upgrade_required");
        assert_eq!(p.reason_code, "identity_not_registered");
        assert!(p.reason.contains("register this agent identity"));
    }
}

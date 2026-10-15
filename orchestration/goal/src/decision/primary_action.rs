//! Agent channel (primary action) composition — the bounded action the agent
//! is asked to attempt this turn, plus the delivery/quiet flags that shape
//! the interaction contract.

use crate::contract::AgentChannel;

/// Build the agent channel for a decision outcome: the primary action text,
/// the selected todo, the fallback todo, and the delivery/quiet semantics.
pub(crate) fn agent_channel(
    primary_action: Option<String>,
    selected_todo: Option<String>,
    fallback_todo: Option<String>,
    must_attempt: bool,
    delivery_allowed: bool,
    quiet_noop_allowed: bool,
) -> AgentChannel {
    AgentChannel {
        must_attempt,
        delivery_allowed,
        quiet_noop_allowed,
        primary_action,
        selected_todo,
        fallback_todo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_channel_passes_through_all_semantics() {
        let ch = agent_channel(
            Some("run it".into()),
            Some("T1".into()),
            Some("T2".into()),
            true,
            true,
            false,
        );
        assert_eq!(ch.primary_action.as_deref(), Some("run it"));
        assert_eq!(ch.selected_todo.as_deref(), Some("T1"));
        assert_eq!(ch.fallback_todo.as_deref(), Some("T2"));
        assert!(ch.must_attempt);
        assert!(ch.delivery_allowed);
        assert!(!ch.quiet_noop_allowed);
    }

    #[test]
    fn quiet_noop_channel_disables_delivery() {
        let ch = agent_channel(None, None, None, false, false, true);
        assert!(!ch.must_attempt);
        assert!(!ch.delivery_allowed);
        assert!(ch.quiet_noop_allowed);
        assert_eq!(ch.primary_action, None);
        assert_eq!(ch.selected_todo, None);
        assert_eq!(ch.fallback_todo, None);
    }
}

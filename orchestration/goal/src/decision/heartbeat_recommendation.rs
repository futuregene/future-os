//! Heartbeat recommendation — how the host should behave between turns
//! (LoopX: `heartbeat_recommendation` on the packet).

use crate::contract::{HeartbeatRecommendation, TurnMode};

/// Compose the heartbeat recommendation for the given interaction mode.
pub(crate) fn recommendation(mode: TurnMode, must_attempt: bool) -> HeartbeatRecommendation {
    HeartbeatRecommendation {
        recommended_mode: match mode {
            TurnMode::Terminal => "terminal_no_followup".to_string(),
            TurnMode::WaitMonitor => "quiet_wait".to_string(),
            TurnMode::AskUser => "ask_user".to_string(),
            _ => "steering_audit_then_one_step".to_string(),
        },
        notify: if must_attempt {
            "DONT_NOTIFY".to_string()
        } else {
            "NOTIFY_ON_GATE".to_string()
        },
        spend_policy: "append exactly one heartbeat spend only after a bounded progress segment is validated and written back".to_string(),
        reason: "eligible goal requires the standard steering audit before delivery".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::TurnMode;

    #[test]
    fn recommended_mode_maps_each_turn_mode() {
        assert_eq!(
            recommendation(TurnMode::Terminal, false).recommended_mode,
            "terminal_no_followup"
        );
        assert_eq!(
            recommendation(TurnMode::WaitMonitor, false).recommended_mode,
            "quiet_wait"
        );
        assert_eq!(
            recommendation(TurnMode::AskUser, false).recommended_mode,
            "ask_user"
        );
        for mode in [
            TurnMode::BoundedDelivery,
            TurnMode::MonitorPoll,
            TurnMode::Replan,
        ] {
            assert_eq!(
                recommendation(mode, true).recommended_mode,
                "steering_audit_then_one_step"
            );
        }
    }

    #[test]
    fn notify_flag_follows_must_attempt() {
        assert_eq!(
            recommendation(TurnMode::BoundedDelivery, true).notify,
            "DONT_NOTIFY"
        );
        assert_eq!(
            recommendation(TurnMode::WaitMonitor, false).notify,
            "NOTIFY_ON_GATE"
        );
    }
}

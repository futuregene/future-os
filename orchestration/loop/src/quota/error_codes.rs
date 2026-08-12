//! Machine-readable quota / decision reason codes (P1-1①).
//!
//! LoopX `control_plane/quota/error_codes.py` maps quota-state collection
//! failures to stable string codes, and every quota decision carries a
//! compact machine classification alongside the prose `reason`. Pre-P1-1,
//! the kernel emitted free-text reasons only, so consumers (status / TUI /
//! desktop) had to substring-match prose. This module enumerates both
//! surfaces in the typed-RPC oneof style: every variant has a stable
//! snake_case wire code (`as_str` == serde), and the kernel stamps
//! `ShouldRunPacket.reason_code` at construction time.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Quota state collection failures (LoopX `quota_error_code`). These classify
/// the errors a quota read-model collector can hit while loading persisted
/// quota/scheduler state, so an operator surface can render a stable code
/// instead of an OS-specific error string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaErrorCode {
    /// Persisted state is not valid JSON (LoopX: JSONDecodeError).
    QuotaStateInvalidJson,
    /// Caller supplied malformed arguments (LoopX: ValueError).
    QuotaInvalidArguments,
    /// State file unreadable due to permissions (LoopX: PermissionError).
    QuotaStatePermissionDenied,
    /// State file I/O failed (LoopX: OSError).
    QuotaStateIoFailed,
    /// Required field / file missing (LoopX: KeyError).
    QuotaStateMissingField,
    /// State parsed but has the wrong shape (LoopX: TypeError).
    QuotaStateShapeError,
    /// Anything else (LoopX: fallback).
    QuotaUnexpectedCollectionError,
}

impl QuotaErrorCode {
    /// Stable wire code (identical to the serde representation).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QuotaStateInvalidJson => "quota_state_invalid_json",
            Self::QuotaInvalidArguments => "quota_invalid_arguments",
            Self::QuotaStatePermissionDenied => "quota_state_permission_denied",
            Self::QuotaStateIoFailed => "quota_state_io_failed",
            Self::QuotaStateMissingField => "quota_state_missing_field",
            Self::QuotaStateShapeError => "quota_state_shape_error",
            Self::QuotaUnexpectedCollectionError => "quota_unexpected_collection_error",
        }
    }

    /// Classify an I/O error the way LoopX classifies OSError subclasses.
    pub fn from_io_error(err: &std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound => Self::QuotaStateMissingField,
            std::io::ErrorKind::PermissionDenied => Self::QuotaStatePermissionDenied,
            _ => Self::QuotaStateIoFailed,
        }
    }

    /// Classify a JSON parse failure (LoopX: JSONDecodeError).
    pub fn from_serde_error(_err: &serde_json::Error) -> Self {
        Self::QuotaStateInvalidJson
    }
}

impl fmt::Display for QuotaErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Decision reason codes — one variant per kernel exit path in
/// [`crate::decision::decide_for`]. The prose `reason` stays human-facing;
/// this code is the machine contract (typed-RPC oneof style: stable wire
/// strings, exhaustive variants, no wildcards).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReasonCode {
    /// Goal was cancelled — automation stopped, state retained.
    GoalCancelled,
    /// `--agent-id` present but not a registered coordination peer
    /// (fail-closed identity gate; LoopX `automation_prompt_upgrade_required`).
    IdentityNotRegistered,
    /// Open user gate(s) — the ask channel owns the turn.
    OpenUserGate,
    /// Runnable advancement todo selected (first attempt).
    RunnableTodo,
    /// Runnable advancement todo selected (repair attempt N > 1).
    RepairAttempt,
    /// Surface-only progress loop breached the outcome floor → replan.
    OutcomeFloorBreach,
    /// Advancement todo(s) exhausted the repair budget → replan.
    RepairBudgetExhausted,
    /// External blocker(s) open with no runnable fallback → quiet wait.
    BlockedNoFallback,
    /// Completed advancement without closure intent → replan obligation.
    SuccessionClosureMissing,
    /// Monitor stalled (consecutive no-change polls over threshold) → replan.
    MonitorStalled,
    /// Monitor due — one read-only poll.
    MonitorDue,
    /// Monitor(s) present, none due — quiet wait with backoff.
    MonitorBackoff,
    /// Acceptance gap(s) open with no runnable work → replan.
    AcceptanceGapOpen,
    /// Deferred todo(s) not yet due — quiet wait.
    DeferredNotDue,
    /// Validated closure — todos done, gaps closed, closure intent declared.
    ValidatedClosure,
}

impl DecisionReasonCode {
    /// Stable wire code (identical to the serde representation).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GoalCancelled => "goal_cancelled",
            Self::IdentityNotRegistered => "identity_not_registered",
            Self::OpenUserGate => "open_user_gate",
            Self::RunnableTodo => "runnable_todo",
            Self::RepairAttempt => "repair_attempt",
            Self::OutcomeFloorBreach => "outcome_floor_breach",
            Self::RepairBudgetExhausted => "repair_budget_exhausted",
            Self::BlockedNoFallback => "blocked_no_fallback",
            Self::SuccessionClosureMissing => "succession_closure_missing",
            Self::MonitorStalled => "monitor_stalled",
            Self::MonitorDue => "monitor_due",
            Self::MonitorBackoff => "monitor_backoff",
            Self::AcceptanceGapOpen => "acceptance_gap_open",
            Self::DeferredNotDue => "deferred_not_due",
            Self::ValidatedClosure => "validated_closure",
        }
    }

    /// Parse a wire code back (read-model side; returns `None` for unknown
    /// codes so forward-compatible readers degrade instead of failing).
    pub fn parse(code: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(code.to_string())).ok()
    }
}

impl fmt::Display for DecisionReasonCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_error_codes_wire_roundtrip() {
        let all = [
            QuotaErrorCode::QuotaStateInvalidJson,
            QuotaErrorCode::QuotaInvalidArguments,
            QuotaErrorCode::QuotaStatePermissionDenied,
            QuotaErrorCode::QuotaStateIoFailed,
            QuotaErrorCode::QuotaStateMissingField,
            QuotaErrorCode::QuotaStateShapeError,
            QuotaErrorCode::QuotaUnexpectedCollectionError,
        ];
        for code in all {
            let json = serde_json::to_string(&code).unwrap();
            assert_eq!(json, format!("\"{}\"", code.as_str()));
            let back: QuotaErrorCode = serde_json::from_str(&json).unwrap();
            assert_eq!(back, code);
            assert_eq!(code.to_string(), code.as_str());
        }
        // LoopX wire names (parity anchor).
        assert_eq!(
            QuotaErrorCode::QuotaUnexpectedCollectionError.as_str(),
            "quota_unexpected_collection_error"
        );
    }

    #[test]
    fn quota_error_code_classification() {
        let not_found = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(
            QuotaErrorCode::from_io_error(&not_found),
            QuotaErrorCode::QuotaStateMissingField
        );
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            QuotaErrorCode::from_io_error(&denied),
            QuotaErrorCode::QuotaStatePermissionDenied
        );
        let other = std::io::Error::from(std::io::ErrorKind::ConnectionReset);
        assert_eq!(
            QuotaErrorCode::from_io_error(&other),
            QuotaErrorCode::QuotaStateIoFailed
        );
        let json_err = serde_json::from_str::<serde_json::Value>("{nope").unwrap_err();
        assert_eq!(
            QuotaErrorCode::from_serde_error(&json_err),
            QuotaErrorCode::QuotaStateInvalidJson
        );
    }

    #[test]
    fn decision_reason_codes_wire_roundtrip() {
        let all = [
            DecisionReasonCode::GoalCancelled,
            DecisionReasonCode::IdentityNotRegistered,
            DecisionReasonCode::OpenUserGate,
            DecisionReasonCode::RunnableTodo,
            DecisionReasonCode::RepairAttempt,
            DecisionReasonCode::OutcomeFloorBreach,
            DecisionReasonCode::RepairBudgetExhausted,
            DecisionReasonCode::BlockedNoFallback,
            DecisionReasonCode::SuccessionClosureMissing,
            DecisionReasonCode::MonitorStalled,
            DecisionReasonCode::MonitorDue,
            DecisionReasonCode::MonitorBackoff,
            DecisionReasonCode::AcceptanceGapOpen,
            DecisionReasonCode::DeferredNotDue,
            DecisionReasonCode::ValidatedClosure,
        ];
        // 15 variants — one per kernel exit path.
        assert_eq!(all.len(), 15);
        let mut seen = std::collections::HashSet::new();
        for code in all {
            assert!(seen.insert(code.as_str()), "duplicate wire code {code}");
            let json = serde_json::to_string(&code).unwrap();
            assert_eq!(json, format!("\"{}\"", code.as_str()));
            assert_eq!(DecisionReasonCode::parse(code.as_str()), Some(code));
            assert_eq!(code.to_string(), code.as_str());
        }
        assert_eq!(DecisionReasonCode::parse("future_unknown_code"), None);
    }
}

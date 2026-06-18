//! Approval policy evaluation (stub)
//!
//! This module is the hook point for future rule-based auto-approval.
//! Today it always returns `AskUser`, meaning every approval request
//! goes to the user for manual decision.
//!
//! Future: load rules from `approval_rules` table, match against
//! tool_name/arguments, and return AutoApprove/AutoReject when a rule matches.

use super::approval::ApprovalShape;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum PolicyDecision {
    AskUser,
    AutoApprove,
    AutoReject(String),
}

/// Evaluate approval policy for a tool call.
///
/// Currently always returns `AskUser`. Future implementation will:
/// 1. Load rules from `approval_rules` table (by workspace_id)
/// 2. Match against tool_name / arguments using `match_kind` / `match_value`
/// 3. Return `AutoApprove` or `AutoReject` if a rule matches
/// 4. Fall back to `AskUser` if no rule matches
pub fn evaluate_policy(
    _cwd: &str,
    _tool_name: &str,
    _arguments: &serde_json::Value,
    _shape: &ApprovalShape,
) -> PolicyDecision {
    // Stub: always ask user.
    // Future: implement rule matching here.
    PolicyDecision::AskUser
}

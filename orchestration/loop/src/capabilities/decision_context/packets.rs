//! Public-safe packet contracts for decision_context (P1-4) — LoopX
//! `capabilities/decision_context/packets.py`, compact set.
//!
//! Two contracts:
//!   - [`DecisionContextPacket`] (`decision_context_packet_v0`) — the
//!     assembled decision context: a goal-boundary header plus one typed
//!     section per provider (run history / outcome streak / quota status).
//!     Ids, counters and enums only — never free text — so a packet can be
//!     persisted inside a public-safe decision-replay case without leaking
//!     private content (todo text, gap descriptions, paths).
//!   - [`DecisionOutcomeReceipt`] (`decision_outcome_receipt_v0`) — the
//!     outcome-feedback writeback: links a settled outcome back to the
//!     anchored decision and to the digest of the context packet the
//!     decision was assembled from (the tamper-evident audit link).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DECISION_CONTEXT_PACKET_SCHEMA_VERSION: &str = "decision_context_packet_v0";
pub const DECISION_OUTCOME_RECEIPT_SCHEMA_VERSION: &str = "decision_outcome_receipt_v0";

/// The decision is fresh; no outcome observed yet.
pub const VERIFICATION_PENDING: &str = "pending";
/// The outcome confirms the decision held up.
pub const VERIFICATION_VERIFIED: &str = "verified";
/// The outcome contradicts the decision (the context was wrong or stale).
pub const VERIFICATION_REFUTED: &str = "refuted";
/// The outcome neither confirms nor contradicts (observer could not decide).
pub const VERIFICATION_INCONCLUSIVE: &str = "inconclusive";

/// LoopX `DECISION_OUTCOME_VERIFICATION_STATUSES`.
pub const DECISION_OUTCOME_VERIFICATION_STATUSES: [&str; 4] = [
    VERIFICATION_PENDING,
    VERIFICATION_VERIFIED,
    VERIFICATION_REFUTED,
    VERIFICATION_INCONCLUSIVE,
];

/// Normalize a verification-status token (case/whitespace-insensitive);
/// `None` for unknown values.
pub fn normalize_verification_status(value: &str) -> Option<&'static str> {
    match value.trim().to_lowercase().as_str() {
        VERIFICATION_PENDING => Some(VERIFICATION_PENDING),
        VERIFICATION_VERIFIED => Some(VERIFICATION_VERIFIED),
        VERIFICATION_REFUTED => Some(VERIFICATION_REFUTED),
        VERIFICATION_INCONCLUSIVE => Some(VERIFICATION_INCONCLUSIVE),
        _ => None,
    }
}

/// Terminal statuses a pending receipt may settle to (settle targets).
pub fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        VERIFICATION_VERIFIED | VERIFICATION_REFUTED | VERIFICATION_INCONCLUSIVE
    )
}

/// First 20 hex chars of the sha256 of `content` (LoopX `_packet_ref`
/// digest shape).
pub fn digest20(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let bytes = hasher.finalize();
    bytes[..10].iter().map(|b| format!("{b:02x}")).collect()
}

/// Run-history section (provider `run_history`): how the goal's turns have
/// been landing. Counts + terminal-state enums only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunHistorySection {
    pub run_count: u64,
    /// Turns that produced a validated artifact (tools invoked + evidence)
    /// — the outcome-floor materiality rule (executor writeback).
    pub material_runs: u64,
    /// `terminal_state` of the most recent runs, newest first (≤3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_terminal_states: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<u64>,
}

/// Outcome-streak section (provider `outcome_streak`): the surface-only
/// progress loop counter and its configured floor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeStreakSection {
    /// Consecutive turns without a material outcome.
    pub surface_streak: u32,
    /// Configured floor (0 = disabled).
    pub threshold: u32,
    /// `threshold > 0 && surface_streak >= threshold` — the kernel replans
    /// on this (decision::stall::outcome_floor_breach).
    pub floor_breached: bool,
}

/// Quota-status section (provider `quota_status`): the slot budget view the
/// packet's quota snapshot is compiled from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaStatusSection {
    pub allowed_slots: u64,
    /// Slots spent per the run history (the kernel's spend counter).
    pub spent_slots: u64,
    /// Slots spent per the `QuotaSpent` event projection (G-3). Divergence
    /// from `spent_slots` is a read-model drift signal.
    #[serde(default)]
    pub projected_spent_slots: u32,
}

/// Semantic-history section (provider `semantic_history`, G13 ③): the
/// goal-level bounded semantic event summaries (kind / todo_id / summary /
/// ts — summaries are truncated at write time, public-safe).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticHistorySection {
    pub schema_version: String,
    pub cap: usize,
    /// Newest-last, bounded to `cap`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<crate::decision::goal_frontier::semantic_history::SemanticEvent>,
}

/// The assembled decision context (LoopX evidence packet, compact set).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionContextPacket {
    pub schema_version: String,
    pub goal_id: String,
    /// Goal lifecycle status at assembly time (`active` / `cancelled`) —
    /// the kernel's cancelled branch reads it.
    pub goal_status: String,
    pub assembled_at: u64,
    /// Provider ids in assembly order (deterministic: registration order).
    pub providers: Vec<String>,
    /// Unsatisfied acceptance gap ids at assembly time (ids are public-safe
    /// tokens; gap descriptions never leave private state).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_acceptance_gaps: Vec<String>,
    pub run_history: RunHistorySection,
    pub outcome_streak: OutcomeStreakSection,
    pub quota: QuotaStatusSection,
    /// G13 ③: semantic event history (bounded) — provider `semantic_history`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_history: Option<SemanticHistorySection>,
}

impl DecisionContextPacket {
    /// sha256-20 of the canonical JSON (struct field order is fixed, so the
    /// serialization is deterministic). An outcome receipt carries this
    /// digest back as the tamper-evident link to the context the decision
    /// was assembled from.
    pub fn digest(&self) -> String {
        let canonical = serde_json::to_string(self).unwrap_or_default();
        digest20(&canonical)
    }
}

/// Outcome-feedback receipt (LoopX outcome receipt, compact set). Identity
/// is content-addressed on (goal, decision anchor, seq) so it is stable
/// across the pending → settled transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionOutcomeReceipt {
    pub schema_version: String,
    /// `decision-outcome-{digest20(goal_id:decision_id:seq)}`.
    pub receipt_id: String,
    pub goal_id: String,
    /// Anchor to the persisted decision: `turn-{N}` (matches
    /// `HeartbeatReceiptRecorded.turn_instance_id` / `DecisionSummary.turn`).
    pub decision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// `effective_action` of the anchored decision.
    pub accepted_decision: String,
    #[serde(default)]
    pub reason_code: String,
    /// Digest of the context packet the decision was assembled from
    /// (empty when settled without an assembled packet at hand).
    #[serde(default)]
    pub context_digest: String,
    pub verification_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Per-decision feedback sequence (1-based) — the G-3 dedupe anchor for
    /// otherwise-identical settles appended within the same second.
    #[serde(default)]
    pub seq: u32,
    pub recorded_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<u64>,
}

impl DecisionOutcomeReceipt {
    /// Content-addressed receipt identity (stable across settle).
    pub fn derive_receipt_id(goal_id: &str, decision_id: &str, seq: u32) -> String {
        format!(
            "decision-outcome-{}",
            digest20(&format!("{goal_id}:{decision_id}:{seq}"))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_status_normalization() {
        assert_eq!(
            normalize_verification_status(" Verified "),
            Some(VERIFICATION_VERIFIED)
        );
        assert_eq!(
            normalize_verification_status("REFUTED"),
            Some(VERIFICATION_REFUTED)
        );
        assert_eq!(
            normalize_verification_status("inconclusive"),
            Some(VERIFICATION_INCONCLUSIVE)
        );
        assert_eq!(
            normalize_verification_status("pending"),
            Some(VERIFICATION_PENDING)
        );
        assert_eq!(normalize_verification_status("bogus"), None);
        assert!(is_terminal_status(VERIFICATION_VERIFIED));
        assert!(is_terminal_status(VERIFICATION_REFUTED));
        assert!(is_terminal_status(VERIFICATION_INCONCLUSIVE));
        assert!(!is_terminal_status(VERIFICATION_PENDING));
    }

    #[test]
    fn packet_digest_is_deterministic_and_content_sensitive() {
        let packet = DecisionContextPacket {
            schema_version: DECISION_CONTEXT_PACKET_SCHEMA_VERSION.to_string(),
            goal_id: "g1".to_string(),
            goal_status: "active".to_string(),
            assembled_at: 100,
            providers: vec!["run_history".to_string()],
            open_acceptance_gaps: vec![],
            run_history: RunHistorySection::default(),
            outcome_streak: OutcomeStreakSection::default(),
            quota: QuotaStatusSection::default(),
            semantic_history: None,
        };
        assert_eq!(packet.digest(), packet.digest());
        assert_eq!(packet.digest().len(), 20);
        let mut other = packet.clone();
        other.goal_status = "cancelled".to_string();
        assert_ne!(packet.digest(), other.digest());
    }

    #[test]
    fn receipt_id_is_stable_per_decision_and_seq() {
        let a = DecisionOutcomeReceipt::derive_receipt_id("g1", "turn-3", 1);
        let b = DecisionOutcomeReceipt::derive_receipt_id("g1", "turn-3", 1);
        let c = DecisionOutcomeReceipt::derive_receipt_id("g1", "turn-3", 2);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("decision-outcome-"));
    }
}

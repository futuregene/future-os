//! Decision summary projection (P1-1②) + receipt writeback (P1-1③).
//!
//! LoopX `control_plane/quota/decision_summary.py` compacts each quota
//! decision into a `QuotaDecisionPacket` projection that CLI / status / host
//! surfaces consume without re-running the kernel; `heartbeat_receipt.py`
//! and `scheduler_ack.py` record the corresponding receipts. Pre-P1-1 the
//! decision reached consumers only through the one-shot turn envelope —
//! nothing was persisted, so status/TUI/desktop had to re-derive the
//! decision from goal state.
//!
//! This module owns:
//!   - [`DecisionSummary`] — the compact, serializable decision projection
//!     (LoopX `compact_quota_decision`);
//!   - the ledger writeback helper [`record_turn_decision`] — appends one
//!     `DecisionSummaryRecorded` + one `HeartbeatReceiptRecorded` event per
//!     executed turn (both projection-only: replay ignores them);
//!   - the read model ([`decision_summaries`] / [`latest_decision_summary`])
//!     over the raw ledger, reused by `quota decisions` and available to
//!     any external consumer reading the event log.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::contract::ShouldRunPacket;
use crate::store::{Event, Store, StoredEvent};

pub const DECISION_SUMMARY_SCHEMA_VERSION: &str = "quota_decision_summary_v0";

/// Compact decision projection (LoopX `QuotaDecisionPacket`): the fields a
/// consumer needs to render the decision without re-running the kernel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionSummary {
    pub schema_version: String,
    pub goal_id: String,
    /// Agent the decision was compiled for (`None` = anonymous path).
    #[serde(default)]
    pub agent_id: Option<String>,
    pub decision: String,
    pub should_run: bool,
    pub effective_action: String,
    /// P1-1① machine-readable reason code
    /// (`quota::error_codes::DecisionReasonCode` wire string).
    #[serde(default)]
    pub reason_code: String,
    /// Turn mode (`TurnMode::as_str`).
    pub mode: String,
    pub state: String,
    #[serde(default)]
    pub selected_todo: Option<String>,
    pub spent_slots: u64,
    pub allowed_slots: u64,
    pub normal_delivery_allowed: bool,
    pub recovery_delivery_allowed: bool,
    pub self_repair_allowed: bool,
    pub safe_bypass_allowed: bool,
    #[serde(default)]
    pub safe_bypass_kind: Option<String>,
    #[serde(default)]
    pub blocked_action_scope: Option<String>,
    /// Goal-level monotonic turn counter at record time (0 = recorded outside
    /// a run loop). The caller offsets the run-local turn by the number of
    /// turns already started on the goal, so receipts stay distinguishable
    /// across separate `run` processes (timeout turns included).
    #[serde(default)]
    pub turn: u32,
}

impl DecisionSummary {
    /// Compact a full kernel packet into the projection (LoopX
    /// `compact_quota_decision` — same field selection).
    pub fn from_packet(packet: &ShouldRunPacket, agent_id: Option<&str>, turn: u32) -> Self {
        Self {
            schema_version: DECISION_SUMMARY_SCHEMA_VERSION.to_string(),
            goal_id: packet.goal_id.clone(),
            agent_id: agent_id.map(str::to_string),
            decision: packet.decision.clone(),
            should_run: packet.should_run,
            effective_action: packet.effective_action.clone(),
            reason_code: packet.reason_code.clone(),
            mode: packet.interaction_contract.mode.as_str().to_string(),
            state: packet.state.clone(),
            selected_todo: packet
                .interaction_contract
                .agent_channel
                .selected_todo
                .clone(),
            spent_slots: packet.quota.spent_slots,
            allowed_slots: packet.quota.allowed_slots,
            normal_delivery_allowed: packet.normal_delivery_allowed,
            recovery_delivery_allowed: packet.recovery_delivery_allowed,
            self_repair_allowed: packet.self_repair_allowed,
            safe_bypass_allowed: packet.safe_bypass_allowed,
            safe_bypass_kind: packet.safe_bypass_kind.clone(),
            blocked_action_scope: packet.blocked_action_scope.clone(),
            turn,
        }
    }
}

/// Read model: all persisted decision summaries in ledger order.
pub fn decision_summaries(events: &[StoredEvent]) -> Vec<&DecisionSummary> {
    events
        .iter()
        .filter_map(|stored| match &stored.event {
            Event::DecisionSummaryRecorded { summary, .. } => Some(summary),
            _ => None,
        })
        .collect()
}

/// Read model: the most recent persisted decision summary, if any.
pub fn latest_decision_summary(events: &[StoredEvent]) -> Option<&DecisionSummary> {
    decision_summaries(events).into_iter().next_back()
}

/// Per-turn writeback (P1-1②+③): persist the compact decision projection
/// and the heartbeat receipt for one executed turn. Both events are
/// projection-only — replay never folds them into goal state, so recording
/// can never change future decisions.
///
/// `turn_instance_id` anchors the receipt the way LoopX keys heartbeat
/// receipts on (goal, agent, run/turn instance, todo). The `turn` argument is
/// a GOAL-LEVEL monotonic counter (not the run-local turn), so receipts stay
/// distinguishable across separate `run` processes — the caller offsets it.
pub fn record_turn_decision(
    store: &mut Store,
    packet: &ShouldRunPacket,
    agent_id: Option<&str>,
    turn: u32,
) -> Result<()> {
    let now = crate::state::now_epoch();
    let summary = DecisionSummary::from_packet(packet, agent_id, turn);
    store.append(Event::DecisionSummaryRecorded {
        goal_id: packet.goal_id.clone(),
        summary,
        ts: now,
    })?;
    store.append(Event::HeartbeatReceiptRecorded {
        goal_id: packet.goal_id.clone(),
        agent_id: agent_id.map(str::to_string),
        turn_instance_id: format!("turn-{turn}"),
        todo_id: packet
            .interaction_contract
            .agent_channel
            .selected_todo
            .clone(),
        decision: packet.decision.clone(),
        reason_code: packet.reason_code.clone(),
        ts: now,
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::decide;
    use crate::state::{Goal, Todo};

    #[test]
    fn summary_compacts_the_packet() {
        let mut g = Goal::new("g1", "objective", "/tmp");
        g.add(Todo::advancement("t1", "do work"));
        let packet = decide(&g, std::time::SystemTime::now());
        let s = DecisionSummary::from_packet(&packet, Some("agent-1"), 3);
        assert_eq!(s.schema_version, DECISION_SUMMARY_SCHEMA_VERSION);
        assert_eq!(s.goal_id, "g1");
        assert_eq!(s.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(s.decision, "run");
        assert!(s.should_run);
        assert_eq!(s.effective_action, "normal_run");
        assert_eq!(s.reason_code, "runnable_todo");
        assert_eq!(s.mode, "bounded_delivery");
        assert_eq!(s.selected_todo.as_deref(), Some("t1"));
        assert_eq!(
            s.allowed_slots,
            crate::quota::slot_accounting::QUOTA_ALLOWED_SLOTS
        );
        assert!(s.normal_delivery_allowed);
        assert!(!s.self_repair_allowed);
        assert_eq!(s.turn, 3);
    }

    #[test]
    fn summary_serde_roundtrip_and_legacy_default() {
        let mut g = Goal::new("g1", "objective", "/tmp");
        g.add(Todo::advancement("t1", "do work"));
        let packet = decide(&g, std::time::SystemTime::now());
        let s = DecisionSummary::from_packet(&packet, None, 0);
        let json = serde_json::to_string(&s).unwrap();
        let back: DecisionSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);

        // Legacy shape: pre-P1-1 summaries without reason_code/turn parse.
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value.as_object_mut().unwrap().remove("reason_code");
        value.as_object_mut().unwrap().remove("turn");
        let legacy: DecisionSummary = serde_json::from_value(value).unwrap();
        assert_eq!(legacy.reason_code, "");
        assert_eq!(legacy.turn, 0);
    }
}

//! Audited feedback from decision outcomes into the ledger and reward
//! memory (P1-4) — LoopX `capabilities/decision_context/outcome_feedback.py`,
//! compact set.
//!
//! Lifecycle: a receipt begins `pending` at decision time (carrying the
//! context digest — the tamper-evident link to the packet the decision was
//! assembled from) and settles to a terminal status (verified / refuted /
//! inconclusive) once the outcome is observed. The settle is the writeback:
//! one `DecisionOutcomeRecorded` ledger event (projection-only, like the
//! P1-1 decision summaries) plus — for decisive outcomes — one reward
//! signal under the `decision_outcome` source, so reward_memory learns
//! which decisions held up.
//!
//! Audited = fail closed: feedback must anchor to a persisted decision
//! summary (`DecisionSummaryRecorded` for the same goal + turn); settling
//! against a decision that never happened is rejected.

use anyhow::{bail, Result};

use super::packets::{
    is_terminal_status, normalize_verification_status, DecisionContextPacket,
    DecisionOutcomeReceipt, DECISION_OUTCOME_RECEIPT_SCHEMA_VERSION, VERIFICATION_INCONCLUSIVE,
    VERIFICATION_PENDING, VERIFICATION_REFUTED, VERIFICATION_VERIFIED,
};
use crate::capabilities::reward_memory as rm;
use crate::contract::ShouldRunPacket;
use crate::store::{Event, Store, StoredEvent};

/// Begin a pending receipt for a decision made against `context`.
/// `decision_id` anchors the receipt (`turn-{N}`, matching
/// `HeartbeatReceiptRecorded.turn_instance_id`). `seq` is the per-decision
/// feedback sequence ([`next_seq`]) — the G-3 dedupe anchor.
pub fn begin_receipt(
    packet: &ShouldRunPacket,
    context: &DecisionContextPacket,
    decision_id: &str,
    agent_id: Option<&str>,
    seq: u32,
    now: u64,
) -> DecisionOutcomeReceipt {
    DecisionOutcomeReceipt {
        schema_version: DECISION_OUTCOME_RECEIPT_SCHEMA_VERSION.to_string(),
        receipt_id: DecisionOutcomeReceipt::derive_receipt_id(&packet.goal_id, decision_id, seq),
        goal_id: packet.goal_id.clone(),
        decision_id: decision_id.to_string(),
        agent_id: agent_id.map(str::to_string),
        accepted_decision: packet.effective_action.clone(),
        reason_code: packet.reason_code.clone(),
        context_digest: context.digest(),
        verification_status: VERIFICATION_PENDING.to_string(),
        note: None,
        seq,
        recorded_at: now,
        settled_at: None,
    }
}

/// Settle a pending receipt to a terminal status. Fails closed on an
/// unknown status, on `pending` (not a settle target), and on a
/// double-settle (a settled receipt is immutable — record a new one with
/// the next seq instead).
pub fn settle_receipt(
    receipt: &mut DecisionOutcomeReceipt,
    status: &str,
    note: Option<String>,
    now: u64,
) -> Result<()> {
    if receipt.verification_status != VERIFICATION_PENDING {
        bail!(
            "receipt {} already settled ({}) — record a new receipt instead",
            receipt.receipt_id,
            receipt.verification_status
        );
    }
    let status = normalize_verification_status(status)
        .ok_or_else(|| anyhow::anyhow!("unknown verification status `{status}`"))?;
    if !is_terminal_status(status) {
        bail!("verification status `{status}` is not a terminal settle target");
    }
    receipt.verification_status = status.to_string();
    receipt.note = note;
    receipt.settled_at = Some(now);
    Ok(())
}

/// Deterministic outcome → reward-signal mapping (phase 1): verified = 1.0,
/// refuted = 0.0, inconclusive carries no score (the observer could not
/// decide); `pending` is not an outcome and yields no signal.
pub fn outcome_signal(status: &str) -> Option<(&'static str, Option<f64>)> {
    match status {
        VERIFICATION_VERIFIED => Some(("verified", Some(1.0))),
        VERIFICATION_REFUTED => Some(("refuted", Some(0.0))),
        VERIFICATION_INCONCLUSIVE => Some(("inconclusive", None)),
        _ => None,
    }
}

/// Read model: all settled outcome receipts in ledger order.
pub fn decision_outcomes(events: &[StoredEvent]) -> Vec<&DecisionOutcomeReceipt> {
    events
        .iter()
        .filter_map(|stored| match &stored.event {
            Event::DecisionOutcomeRecorded { receipt, .. } => Some(receipt),
            _ => None,
        })
        .collect()
}

/// The latest receipt recorded against one decision anchor, if any.
pub fn outcome_for<'a>(
    events: &'a [StoredEvent],
    decision_id: &str,
) -> Option<&'a DecisionOutcomeReceipt> {
    decision_outcomes(events)
        .into_iter()
        .rev()
        .find(|r| r.decision_id == decision_id)
}

/// The next per-decision feedback sequence (1 + the number of receipts
/// already recorded against this decision) — mirrors reward_memory's
/// per-todo seq as the G-3 content-id dedupe anchor.
pub fn next_seq(events: &[StoredEvent], decision_id: &str) -> u32 {
    let count = events
        .iter()
        .filter(|se| matches!(&se.event, Event::DecisionOutcomeRecorded { receipt, .. } if receipt.decision_id == decision_id))
        .count();
    count as u32 + 1
}

/// The settle writeback (`decision-context feedback`): anchor the receipt
/// to the persisted decision summary for `turn`, append the
/// `DecisionOutcomeRecorded` event, and — for decisive outcomes — ingest
/// the reward signal under the `decision_outcome` source.
///
/// `context_digest` links the receipt to the assembled packet when the
/// caller has one at hand (e.g. a replay-recorded case); empty otherwise.
pub fn record_outcome_feedback(
    store: &mut Store,
    goal_id: &str,
    turn: u32,
    status: &str,
    note: Option<String>,
    agent_id: Option<String>,
    context_digest: &str,
) -> Result<DecisionOutcomeReceipt> {
    let events = store.events(goal_id)?;
    let decision_id = format!("turn-{turn}");
    // Audited: the feedback must anchor to a persisted decision. Use the
    // latest summary for the turn for the decision payload.
    let summary = crate::quota::decision_summary::decision_summaries(&events)
        .into_iter()
        .rev()
        .find(|s| s.turn == turn)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no persisted decision summary for goal {goal_id} turn {turn} — feedback must anchor to a real decision"
            )
        })?;
    let status = normalize_verification_status(status).ok_or_else(|| {
        anyhow::anyhow!("--status must be one of: verified, refuted, inconclusive")
    })?;
    let now = crate::state::now_epoch();
    let seq = next_seq(&events, &decision_id);
    let mut receipt = DecisionOutcomeReceipt {
        schema_version: DECISION_OUTCOME_RECEIPT_SCHEMA_VERSION.to_string(),
        receipt_id: DecisionOutcomeReceipt::derive_receipt_id(goal_id, &decision_id, seq),
        goal_id: goal_id.to_string(),
        decision_id: decision_id.clone(),
        agent_id: agent_id.clone(),
        accepted_decision: summary.effective_action.clone(),
        reason_code: summary.reason_code.clone(),
        context_digest: context_digest.to_string(),
        verification_status: VERIFICATION_PENDING.to_string(),
        note: None,
        seq,
        recorded_at: now,
        settled_at: None,
    };
    settle_receipt(&mut receipt, status, note, now)?;
    store.append(Event::DecisionOutcomeRecorded {
        goal_id: goal_id.to_string(),
        receipt: receipt.clone(),
        ts: now,
    })?;
    // Reward-memory writeback (LoopX: audited feedback into reward memory):
    // decisive outcomes become a `decision_outcome` signal, scoped to the
    // anchored decision's selected todo (empty = goal-scoped).
    if let Some((signal, score)) = outcome_signal(status) {
        let todo_id = summary.selected_todo.clone().unwrap_or_default();
        let reward_seq = rm::next_seq(&store.events(goal_id)?, &todo_id);
        store.append(Event::RewardSignalRecorded {
            goal_id: goal_id.to_string(),
            todo_id,
            agent_id,
            run_id: None,
            source: rm::SOURCE_DECISION_OUTCOME.to_string(),
            signal: signal.to_string(),
            score,
            note: Some(format!("decision {decision_id} {}", receipt.receipt_id)),
            seq: reward_seq,
            ts: now,
        })?;
    }
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::decision_context::assembler::assemble_decision_context;
    use crate::state::{Goal, Todo};

    fn decision_fixture() -> (Goal, ShouldRunPacket, DecisionContextPacket) {
        let mut goal = Goal::new("g1", "objective", "/tmp");
        goal.add(Todo::advancement("T1", "work"));
        let packet = crate::decision::decide(&goal, std::time::SystemTime::now());
        let context = assemble_decision_context(&goal);
        (goal, packet, context)
    }

    #[test]
    fn receipt_lifecycle_pending_to_settled() {
        let (_goal, packet, context) = decision_fixture();
        let mut receipt = begin_receipt(&packet, &context, "turn-1", Some("agent-a"), 1, 100);
        assert_eq!(receipt.verification_status, VERIFICATION_PENDING);
        assert_eq!(receipt.settled_at, None);
        assert_eq!(receipt.context_digest, context.digest());
        assert_eq!(receipt.accepted_decision, packet.effective_action);
        assert_eq!(
            receipt.receipt_id,
            DecisionOutcomeReceipt::derive_receipt_id("g1", "turn-1", 1)
        );
        settle_receipt(&mut receipt, "verified", Some("held up".to_string()), 200).unwrap();
        assert_eq!(receipt.verification_status, VERIFICATION_VERIFIED);
        assert_eq!(receipt.settled_at, Some(200));
        assert_eq!(receipt.note.as_deref(), Some("held up"));
    }

    #[test]
    fn settle_fails_closed_on_bad_status_pending_and_double_settle() {
        let (_goal, packet, context) = decision_fixture();
        let mut receipt = begin_receipt(&packet, &context, "turn-1", None, 1, 100);
        assert!(settle_receipt(&mut receipt, "bogus", None, 200).is_err());
        assert!(settle_receipt(&mut receipt, "pending", None, 200).is_err());
        settle_receipt(&mut receipt, "refuted", None, 200).unwrap();
        let err = settle_receipt(&mut receipt, "verified", None, 300).unwrap_err();
        assert!(err.to_string().contains("already settled"), "{err}");
    }

    #[test]
    fn outcome_signal_mapping() {
        assert_eq!(
            outcome_signal(VERIFICATION_VERIFIED),
            Some(("verified", Some(1.0)))
        );
        assert_eq!(
            outcome_signal(VERIFICATION_REFUTED),
            Some(("refuted", Some(0.0)))
        );
        assert_eq!(
            outcome_signal(VERIFICATION_INCONCLUSIVE),
            Some(("inconclusive", None))
        );
        assert_eq!(outcome_signal(VERIFICATION_PENDING), None);
    }
}

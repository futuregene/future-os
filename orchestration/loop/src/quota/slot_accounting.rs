//! Slot accounting (G-7) — spend-source classification and per-goal spend
//! breakdown, replacing the P0 `QUOTA_ALLOWED_SLOTS` constant + raw
//! `history.len()` count.
//!
//! LoopX `control_plane/quota/slot_accounting.py` (841 lines) classifies
//! every slot spend by source (`heartbeat` / `controller` / `adapter` /
//! `visible-goal`) so quota is observable: which kind of turn consumed the
//! allowance. We use the plan's three-way taxonomy (`run` / `agent` /
//! `heartbeat`):
//!
//!   - `run`       — bounded-delivery turns that actually executed work
//!   - `agent`     — agent-claimed execution (a claimed todo being worked)
//!   - `heartbeat` — quota-neutral activity: monitor polls (no spend on
//!     no-change), scheduler acks, state refreshes
//!
//! The classification is stamped on each [`RunRecord`] at writeback time
//! (`spend_source`); this module owns the constant and the accounting rules.

use crate::contract::TurnMode;
use crate::state::RunRecord;

/// Daily slot allowance (LoopX quota budget; P0 had this inline in
/// `decision/mod.rs` — it now lives in the quota subdomain and is
/// re-exported by the decision kernel for packet parity).
pub const QUOTA_ALLOWED_SLOTS: u64 = 1440;

/// Spend-source taxonomy (plan §5.2 G-7: run/agent/heartbeat).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotSpendSource {
    /// A bounded-delivery turn that executed work.
    Run,
    /// An agent-claimed execution slice.
    Agent,
    /// Quota-neutral: monitor polls, scheduler acks, state refreshes.
    Heartbeat,
}

impl SlotSpendSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SlotSpendSource::Run => "run",
            SlotSpendSource::Agent => "agent",
            SlotSpendSource::Heartbeat => "heartbeat",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "run" => Some(SlotSpendSource::Run),
            "agent" => Some(SlotSpendSource::Agent),
            "heartbeat" => Some(SlotSpendSource::Heartbeat),
            _ => None,
        }
    }
}

/// Classify a turn by its interaction mode. Monitor polls and non-delivery
/// modes are quota-neutral (heartbeat); delivery modes spend.
pub fn classify_mode(mode: TurnMode) -> SlotSpendSource {
    match mode {
        TurnMode::BoundedDelivery => SlotSpendSource::Run,
        TurnMode::MonitorPoll => SlotSpendSource::Heartbeat,
        TurnMode::AskUser | TurnMode::WaitMonitor | TurnMode::Replan | TurnMode::Terminal => {
            SlotSpendSource::Heartbeat
        }
    }
}

/// Classify a recorded run without a mode hint (e.g. replayed runs.jsonl):
/// completed runs spent, everything else is quota-neutral.
pub fn classify_record(record: &RunRecord) -> SlotSpendSource {
    match record
        .spend_source
        .as_deref()
        .and_then(SlotSpendSource::parse)
    {
        Some(source) => source,
        None if record.terminal_state == "completed" => SlotSpendSource::Run,
        None => SlotSpendSource::Heartbeat,
    }
}

/// Whether a turn consumes a quota slot (LoopX spend rules: unchanged
/// monitor polls and scheduler acks never spend).
pub fn slot_spend(record: &RunRecord) -> u64 {
    match classify_record(record) {
        SlotSpendSource::Heartbeat => 0,
        SlotSpendSource::Run | SlotSpendSource::Agent => 1,
    }
}

/// Per-source spend breakdown for a goal's history.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SpendBreakdown {
    pub allowed_slots: u64,
    pub spent_slots: u64,
    pub by_source: Vec<(SlotSpendSource, u64)>,
}

impl SpendBreakdown {
    pub fn source_count(&self, source: SlotSpendSource) -> u64 {
        self.by_source
            .iter()
            .find(|(s, _)| *s == source)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }
}

/// Account a goal's history: total spent (counted) + per-source split.
pub fn account_history(history: &[RunRecord]) -> SpendBreakdown {
    let mut run = 0u64;
    let mut agent = 0u64;
    let mut heartbeat = 0u64;
    for record in history {
        let spent = slot_spend(record);
        match classify_record(record) {
            SlotSpendSource::Run => run += spent,
            SlotSpendSource::Agent => agent += spent,
            SlotSpendSource::Heartbeat => heartbeat += spent,
        }
    }
    SpendBreakdown {
        allowed_slots: QUOTA_ALLOWED_SLOTS,
        spent_slots: run + agent + heartbeat,
        by_source: vec![
            (SlotSpendSource::Run, run),
            (SlotSpendSource::Agent, agent),
            (SlotSpendSource::Heartbeat, heartbeat),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(todo_id: &str, state: &str, source: Option<&str>) -> RunRecord {
        RunRecord {
            turn: 1,
            todo_id: todo_id.to_string(),
            run_id: "run-1".to_string(),
            terminal_state: state.to_string(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: String::new(),
            recorded_at: 0,
            spend_source: source.map(|s| s.to_string()),
            validation: None,
            failure_kind: None,
            truncation: None,
        }
    }

    #[test]
    fn mode_classification() {
        assert_eq!(
            classify_mode(TurnMode::BoundedDelivery),
            SlotSpendSource::Run
        );
        assert_eq!(
            classify_mode(TurnMode::MonitorPoll),
            SlotSpendSource::Heartbeat
        );
        assert_eq!(classify_mode(TurnMode::Replan), SlotSpendSource::Heartbeat);
        assert_eq!(
            classify_mode(TurnMode::Terminal),
            SlotSpendSource::Heartbeat
        );
    }

    #[test]
    fn monitor_polls_never_spend() {
        let poll = record("M1", "polled", Some("heartbeat"));
        assert_eq!(slot_spend(&poll), 0);
        let run = record("T1", "completed", Some("run"));
        assert_eq!(slot_spend(&run), 1);
    }

    #[test]
    fn legacy_records_without_source_default_to_completed_runs() {
        let completed = record("T1", "completed", None);
        assert_eq!(classify_record(&completed), SlotSpendSource::Run);
        let errored = record("T1", "error", None);
        assert_eq!(classify_record(&errored), SlotSpendSource::Heartbeat);
        assert_eq!(slot_spend(&errored), 0);
    }

    #[test]
    fn breakdown_sums_by_source() {
        let history = vec![
            record("T1", "completed", Some("run")),
            record("T1", "completed", Some("run")),
            record("T2", "completed", Some("agent")),
            record("M1", "polled", Some("heartbeat")),
            record("M2", "polled", Some("heartbeat")),
            record("M2", "polled", Some("heartbeat")),
        ];
        let b = account_history(&history);
        assert_eq!(b.allowed_slots, QUOTA_ALLOWED_SLOTS);
        assert_eq!(b.spent_slots, 3);
        assert_eq!(b.source_count(SlotSpendSource::Run), 2);
        assert_eq!(b.source_count(SlotSpendSource::Agent), 1);
        assert_eq!(b.source_count(SlotSpendSource::Heartbeat), 0);
    }
}

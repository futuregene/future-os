//! Scheduler arbitration — derive scheduler authority from the final
//! interaction contract (LoopX: `control_plane/scheduler/arbitration.py`).
//!
//! G-2 + G-11: the 9 scheduler dispositions plus the consistency validation
//! that fails closed to `CONSISTENCY_REPAIR` when the final contract is
//! structurally inconsistent. Lower-level quota fields never participate in
//! branch selection (reference rule): only the final interaction contract's
//! schema / mode / channels decide.
//!
//! Rollout (refactor plan §5.1 trade-off): observe-only shipped first — every
//! packet carried its [`SchedulerArbitration`] record with behavior
//! unchanged. [`ARBITRATION_ENFORCEMENT`] is now `true`: `CONSISTENCY_REPAIR`
//! fails closed (`repair_action`: rebuild the contract, then rerun quota
//! before applying scheduler cadence).
//!
//! Mode vocabulary: our typed [`TurnMode`] is mapped onto the LoopX
//! interaction-contract mode strings the classifier branches on, so
//! dispositions and reason_codes align value-for-value with LoopX
//! (`terminal_no_followup`, `user_gate`, `monitor_quiet_skip`, ...).
//! `WaitMonitor` currently conflates blockers / monitors-none-due / deferred
//! in one turn mode; it maps to `monitor_quiet_skip` (the dominant
//! monitor-wait case) until monitor metadata is refined (G-12).

use serde::Serialize;

use crate::contract::{InteractionContract, ShouldRunPacket, TurnMode};

/// reference scheduler-arbitration record schema version.
pub const SCHEDULER_ARBITRATION_SCHEMA_VERSION: &str = "scheduler_arbitration_v0";

/// Fail-closed repair action (reference `consistency_error.repair_action`).
pub const CONSISTENCY_REPAIR_ACTION: &str =
    "rebuild interaction_contract from the current quota decision, then rerun \
     quota before applying scheduler cadence";

/// Rollout switch: `true` (enforced) makes `CONSISTENCY_REPAIR` fail closed —
/// an inconsistent final interaction contract rewrites the packet to the
/// `consistency_repair` decision with the `control_plane_repair` cadence.
pub const ARBITRATION_ENFORCEMENT: bool = true;

/// The 9 scheduler dispositions, aligned value-for-value with LoopX
/// `SchedulerDisposition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerDisposition {
    /// Validated closure: stop recurring automation until explicit resume.
    TerminalStop,
    /// Agent is monitor-only: quiet poll keeps due monitors responsive.
    AgentMonitorOnlyWait,
    /// The contract requires an agent attempt; keep the active cadence.
    ActiveWork,
    /// Registered agent has no in-scope candidate; wait for handoff/reassign.
    AgentScopeWait,
    /// Final contract is structurally inconsistent; repair projection first.
    ConsistencyRepair,
    /// Concrete human decision is the next unlock; surface once, then wait.
    HumanGate,
    /// Monitor-only quiet polls stay alive on a slower cadence.
    MonitorWait,
    /// Quota blocks delivery and no immediate path is projected; slow poll.
    QuietWait,
    /// Source is unchanged; back off until fresh evidence or a handoff.
    UnchangedWait,
}

impl SchedulerDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            SchedulerDisposition::TerminalStop => "terminal_stop",
            SchedulerDisposition::AgentMonitorOnlyWait => "agent_monitor_only_wait",
            SchedulerDisposition::ActiveWork => "active_work",
            SchedulerDisposition::AgentScopeWait => "agent_scope_wait",
            SchedulerDisposition::ConsistencyRepair => "consistency_repair",
            SchedulerDisposition::HumanGate => "human_gate",
            SchedulerDisposition::MonitorWait => "monitor_wait",
            SchedulerDisposition::QuietWait => "quiet_wait",
            SchedulerDisposition::UnchangedWait => "unchanged_wait",
        }
    }
}

/// The arbitration record: disposition + reason_code + contract mode + any
/// structural errors (reference `SchedulerArbitration`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchedulerArbitration {
    pub disposition: SchedulerDisposition,
    pub reason_code: String,
    pub mode: String,
    pub errors: Vec<String>,
}

impl SchedulerArbitration {
    /// True when the final contract is structurally consistent.
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// The fail-closed repair payload, present only when inconsistent
    /// (reference `consistency_error()`).
    pub fn consistency_error(&self) -> Option<serde_json::Value> {
        if self.ok() {
            return None;
        }
        Some(serde_json::json!({
            "schema_version": SCHEDULER_ARBITRATION_SCHEMA_VERSION,
            "reason_code": self.reason_code,
            "mode": if self.mode.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(self.mode.clone()) },
            "errors": self.errors,
            "repair_action": CONSISTENCY_REPAIR_ACTION,
        }))
    }
}

/// Map our typed turn mode onto the reference interaction-contract mode string
/// the classifier branches on.
fn future_loop_mode(mode: TurnMode) -> &'static str {
    match mode {
        TurnMode::BoundedDelivery => "bounded_delivery",
        TurnMode::AskUser => "user_gate",
        TurnMode::MonitorPoll => "monitor_due",
        TurnMode::WaitMonitor => "monitor_quiet_skip",
        TurnMode::Replan => "successor_replan_required",
        TurnMode::Terminal => "terminal_no_followup",
    }
}

/// reference `_classify_disposition`: branch order is normative and preserved.
pub fn classify_disposition(
    mode: &str,
    user_required: bool,
    must_attempt: bool,
    quiet_noop_allowed: bool,
    agent_scope_frontier_actions: &[&str],
) -> (SchedulerDisposition, String) {
    if mode == "terminal_no_followup" {
        return (SchedulerDisposition::TerminalStop, mode.to_string());
    }
    if mode == "agent_monitor_only" {
        return (SchedulerDisposition::AgentMonitorOnlyWait, mode.to_string());
    }
    if user_required && !must_attempt {
        return (
            SchedulerDisposition::HumanGate,
            "interaction_blocking_user_gate".to_string(),
        );
    }
    if mode == "monitor_quiet_skip" {
        return (
            SchedulerDisposition::MonitorWait,
            "interaction_monitor_quiet_wait".to_string(),
        );
    }
    if mode == "successor_replan_required" && must_attempt {
        return (
            SchedulerDisposition::ActiveWork,
            "interaction_successor_replan_required".to_string(),
        );
    }
    if agent_scope_frontier_actions.contains(&mode) {
        return (
            SchedulerDisposition::AgentScopeWait,
            "interaction_agent_scope_wait".to_string(),
        );
    }
    if mode == "mapped_noop_if_unchanged" {
        return (
            SchedulerDisposition::UnchangedWait,
            "interaction_unchanged_wait".to_string(),
        );
    }
    if must_attempt {
        return (
            SchedulerDisposition::ActiveWork,
            "interaction_agent_attempt_required".to_string(),
        );
    }
    if quiet_noop_allowed {
        return (
            SchedulerDisposition::QuietWait,
            "interaction_quiet_noop_allowed".to_string(),
        );
    }
    (
        SchedulerDisposition::QuietWait,
        "interaction_delivery_not_allowed".to_string(),
    )
}

/// Derive scheduler authority from the final interaction contract. Structural
/// contradictions inside the contract fail closed to `CONSISTENCY_REPAIR`
/// (reference `build_scheduler_arbitration`). The typed contract guarantees
/// channel presence and boolean typing (LoopX's dict-level missing/bool-type
/// checks are compile-time here); the remaining checks mirror reference exactly.
pub fn build_scheduler_arbitration(
    contract: &InteractionContract,
    agent_scope_frontier_actions: &[&str],
) -> SchedulerArbitration {
    let mut errors: Vec<String> = Vec::new();

    if contract.schema_version != crate::contract::INTERACTION_CONTRACT_SCHEMA_VERSION {
        errors.push("interaction_contract.schema_version_mismatch".to_string());
    }

    let mode = future_loop_mode(contract.mode);
    let user_required = contract.user_channel.action_required;
    let must_attempt = contract.agent_channel.must_attempt;
    let delivery_allowed = contract.agent_channel.delivery_allowed;
    let quiet_noop_allowed = contract.agent_channel.quiet_noop_allowed;

    if delivery_allowed && !must_attempt {
        errors.push("interaction_contract.delivery_without_attempt".to_string());
    }
    if quiet_noop_allowed && (must_attempt || delivery_allowed || user_required) {
        errors.push("interaction_contract.quiet_noop_conflicts_with_required_action".to_string());
    }
    if matches!(mode, "terminal_no_followup" | "agent_monitor_only")
        && (user_required || must_attempt || delivery_allowed || !quiet_noop_allowed)
    {
        errors.push("interaction_contract.terminal_conflicts_with_open_action".to_string());
    }

    let (disposition, reason_code) = classify_disposition(
        mode,
        user_required,
        must_attempt,
        quiet_noop_allowed,
        agent_scope_frontier_actions,
    );

    if errors.is_empty() {
        SchedulerArbitration {
            disposition,
            reason_code,
            mode: mode.to_string(),
            errors: vec![],
        }
    } else {
        // Fail closed; order-preserving dedup (LoopX: dict.fromkeys).
        let mut seen = std::collections::HashSet::new();
        errors.retain(|e| seen.insert(e.clone()));
        SchedulerArbitration {
            disposition: SchedulerDisposition::ConsistencyRepair,
            reason_code: "scheduler_interaction_contract_inconsistent".to_string(),
            mode: mode.to_string(),
            errors,
        }
    }
}

/// Record the arbitration on a packet (observe-only). When `enforce` is set
/// and the contract is inconsistent, the packet fails closed to a
/// `consistency_repair` decision with the repair scheduler cadence (LoopX
/// scheduler_hint `CONSISTENCY_REPAIR` branch).
pub fn apply_arbitration(packet: &mut ShouldRunPacket, enforce: bool) {
    let arbitration = build_scheduler_arbitration(&packet.interaction_contract, &[]);
    packet.scheduler_arbitration = Some(arbitration.clone());
    if enforce && arbitration.disposition == SchedulerDisposition::ConsistencyRepair {
        packet.ok = false;
        packet.should_run = false;
        packet.decision = "consistency_repair".to_string();
        packet.effective_action = "consistency_repair".to_string();
        packet.state = "waiting".to_string();
        packet.status = "consistency_repair".to_string();
        packet.recommended_action = CONSISTENCY_REPAIR_ACTION.to_string();
        packet.normal_delivery_allowed = false;
        packet.actionable_by_codex = false;
        packet.scheduler_hint.action = "repair_interaction_contract_projection".to_string();
        packet.scheduler_hint.cadence_class = "control_plane_repair".to_string();
    }
}

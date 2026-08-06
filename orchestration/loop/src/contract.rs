//! The typed decision protocol — what `quota should-run` emits.
//!
//! Mirrors LoopX's `future_loop_interaction_contract_v0`: a versioned packet with
//! three channels (user / agent / CLI), plus the auxiliary contracts the
//! host consumes (work lane, execution obligation, automation liveness,
//! scheduler hint, quota). All structs serialize to JSON so the packet can
//! travel to any host over stdout/gRPC.

use serde::Serialize;

use crate::decision::arbitration::SchedulerArbitration;

/// reference interaction-contract schema version the arbitration layer validates
/// against (LoopX: `INTERACTION_CONTRACT_SCHEMA_VERSION`).
pub const INTERACTION_CONTRACT_SCHEMA_VERSION: &str = "future_loop_interaction_contract_v0";

/// Closed set of turn modes (LoopX: `interaction_contract.mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnMode {
    /// Agent must attempt one runnable todo, then spend once after validation.
    BoundedDelivery,
    /// Concrete human decision required; agent may have fallback work.
    AskUser,
    /// A monitor is due: at most one read-only poll, no spend on no-change.
    MonitorPoll,
    /// Monitors exist but none due: quiet wait, do not poll, do not stop.
    WaitMonitor,
    /// State/frontier must change (stalled monitor / acceptance gap / a
    /// completed todo that never declared successor or no-follow-up).
    Replan,
    /// Validated closure: todos done, no gaps, closure intent declared.
    Terminal,
}

impl TurnMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            TurnMode::BoundedDelivery => "bounded_delivery",
            TurnMode::AskUser => "ask_user",
            TurnMode::MonitorPoll => "monitor_poll",
            TurnMode::WaitMonitor => "wait_monitor",
            TurnMode::Replan => "replan",
            TurnMode::Terminal => "terminal",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UserChannel {
    pub action_required: bool,
    #[serde(rename = "notify")]
    pub notify: String,
    pub question: Option<String>,
    pub todo_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentChannel {
    pub must_attempt: bool,
    pub delivery_allowed: bool,
    pub quiet_noop_allowed: bool,
    pub primary_action: Option<String>,
    pub selected_todo: Option<String>,
    pub fallback_todo: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CliChannel {
    pub next_cli_actions: Vec<String>,
    pub spend_allowed_now: bool,
    pub spend_after_validation: bool,
    pub spend_policy: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractionContract {
    pub schema_version: String,
    pub mode: TurnMode,
    pub user_channel: UserChannel,
    pub agent_channel: AgentChannel,
    pub cli_channel: CliChannel,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkLaneContract {
    pub schema_version: String,
    pub lane: String,
    pub obligation: String,
    pub must_attempt_work: bool,
    pub reason_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionObligation {
    pub must_attempt_work: bool,
    pub kind: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutomationLiveness {
    pub keep_active: bool,
    pub pause_allowed: bool,
    pub action: String,
    pub reason: String,
    /// reference pause policy: pause/delete only after a bounded self-repair or
    /// replan path is itself stuck for more eligible turns.
    pub pause_policy: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeartbeatRecommendation {
    pub recommended_mode: String,
    pub notify: String,
    pub spend_policy: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchedulerHint {
    pub schema_version: String,
    pub action: String,
    pub cadence_class: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_due_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuotaSnapshot {
    pub compute: f64,
    pub allowed_slots: u64,
    pub spent_slots: u64,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionProfileSnapshot {
    pub cadence: String,
    pub spend_rule: String,
    pub outcome_floor_streak_threshold: u32,
    pub surface_streak: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplanAckSnapshot {
    pub recorded: bool,
    pub delta_kinds: Vec<String>,
    pub frontier_delta_present: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthoritySnapshot {
    pub write_scope: Vec<String>,
    pub requires_approval: Vec<String>,
    pub pending_approvals: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BoundarySnapshot {
    pub leaks: Vec<String>,
    pub public_safe: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrontierProjection {
    pub replan_required: bool,
    pub current_agent_advancement: usize,
    pub unclaimed_advancement: usize,
    pub acceptance_gaps: usize,
    pub monitors_open: usize,
    pub monitors_due: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TerminalClosure {
    pub kind: String,
    pub derived: bool,
    pub source: String,
}

/// reference rollout-event envelope carried on quota packets.
#[derive(Debug, Clone, Serialize)]
pub struct RolloutEvent {
    pub schema_version: String,
    pub event_id: String,
    pub event_kind: String,
    pub recorded_at: String,
    pub status: String,
}

/// The full packet emitted by the decision kernel (reference `should_run`).
/// Field set mirrors LoopX's top-level keys (mode/state/status/waiting_on/
/// source/recommended_action/rollout_event + the auxiliary contracts).
#[derive(Debug, Clone, Serialize)]
pub struct ShouldRunPacket {
    pub ok: bool,
    pub mode: String,
    pub goal_id: String,
    pub decision: String,
    pub should_run: bool,
    pub effective_action: String,
    pub reason: String,
    pub state: String,
    pub waiting_on: String,
    pub status: String,
    pub source: String,
    pub recommended_action: String,
    pub rollout_event: Option<RolloutEvent>,
    pub status_health_ok: bool,
    pub normal_delivery_allowed: bool,
    pub recovery_delivery_allowed: bool,
    pub self_repair_allowed: bool,
    pub capability_repair_allowed: bool,
    pub workspace_repair_allowed: bool,
    pub actionable_by_codex: bool,
    pub requires_user_action: bool,
    pub action_required: bool,
    pub selected_todo: Option<serde_json::Value>,
    pub open_count: usize,
    pub blocked_action_scope: Option<String>,
    pub safe_bypass_allowed: bool,
    pub safe_bypass_kind: Option<String>,
    pub safe_bypass_policy: Option<String>,
    pub lifecycle_phase: String,
    pub lifecycle_flags: Vec<String>,
    pub project_asset_source: Option<String>,
    pub active_state_next_action: Option<String>,
    pub latest_run_recommended_action: Option<String>,
    pub interaction_contract: InteractionContract,
    pub work_lane_contract: WorkLaneContract,
    pub execution_obligation: ExecutionObligation,
    pub automation_liveness: AutomationLiveness,
    pub heartbeat_recommendation: HeartbeatRecommendation,
    pub scheduler_hint: SchedulerHint,
    pub quota: QuotaSnapshot,
    pub execution_profile: ExecutionProfileSnapshot,
    pub replan_ack: ReplanAckSnapshot,
    pub authority: AuthoritySnapshot,
    pub boundary: BoundarySnapshot,
    /// reference field name for the frontier projection (aliased for parity).
    #[serde(rename = "goal_frontier_projection")]
    pub frontier_projection: FrontierProjection,
    pub agent_todo_summary: Option<crate::state::TodoSummary>,
    pub user_todo_summary: Option<crate::state::TodoSummary>,
    pub todo_summary_projection: Option<serde_json::Value>,
    pub goal_boundary: Option<serde_json::Value>,
    pub plan_summary: Option<serde_json::Value>,
    pub todo_write_hint: Option<String>,
    pub agent_identity: Option<serde_json::Value>,
    pub agent_lane_frontier_hint: Option<String>,
    pub agent_lane_next_action: Option<String>,
    pub goal_route_hint: Option<String>,
    pub task_scope: Option<String>,
    pub handoff_readiness: Option<serde_json::Value>,
    pub long_task_cadence_hint: Option<serde_json::Value>,
    pub promotion_readiness_warning: Option<String>,
    pub autonomous_backlog_candidates: Option<serde_json::Value>,
    pub protocol_action_packet: Option<serde_json::Value>,
    /// G-2/G-11 scheduler arbitration record — observe-only by default
    /// (records the 9-disposition classification without blocking); flip
    /// `ARBITRATION_ENFORCEMENT` to fail closed on CONSISTENCY_REPAIR.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduler_arbitration: Option<SchedulerArbitration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_closure: Option<TerminalClosure>,
}

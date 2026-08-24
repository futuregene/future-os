//! The decision kernel — `quota should-run` compiled from goal state.
//!
//! Pure function of state (+ clock), emitting the full `ShouldRunPacket`.
//! Pipeline order (LoopX: identity → boundary → gate → frontier → contract):
//!   1. user gates (scoped: independent fallback still delivers)
//!   2. runnable advancement todos
//!   3. succession replan obligation (done todos without closure intent)
//!   4. monitors (stall → replan; due → one poll; none due → wait)
//!   5. acceptance gaps with no work → replan
//!   6. validated closure → terminal_no_followup
//!
//! No I/O, no LLM: this is the deterministic contract hosts consume.
//!
//! Subdomain layout (G-1 kernel repackaging):
//!   - [`self`]    pipeline orchestration (`decide` / `decide_for` / `packet`)
//!   - [`arbitration`] G-2/G-11 scheduler arbitration: 9 dispositions + consistency repair
//!   - [`identity`]     fail-closed identity gate (registered peers only)
//!   - [`oscillation`]  A→V→A→V signature-pair detection over run history (③)
//!   - [`stall`]        stall semantics: outcome floor, repair budget, monitor stall
//!   - [`monitor`]      monitor evaluation (stalled / due / quiet-wait backoff)
//!   - [`frontier`]     runnable sort, work lane, frontier projection
//!   - [`boundary`]     boundary scan snapshot
//!   - [`goal_boundary`] goal boundary prompt + packet JSON composition
//!   - [`heartbeat_recommendation`] heartbeat recommendation composition
//!   - [`primary_action`] agent channel (primary action) composition

pub mod arbitration;
mod boundary;
mod frontier;
mod goal_boundary;
pub mod goal_frontier;
mod heartbeat_recommendation;
mod identity;
pub(crate) mod monitor;
pub(crate) mod oscillation;
mod primary_action;
pub(crate) mod stall;

use std::time::SystemTime;

use crate::contract::{
    AgentChannel, AuthoritySnapshot, AutomationLiveness, CliChannel, ExecutionObligation,
    ExecutionProfileSnapshot, InteractionContract, QuotaSnapshot, ReplanAckSnapshot, RolloutEvent,
    SchedulerHint, ShouldRunPacket, TerminalClosure, TurnMode, UserChannel, WorkLaneContract,
};
use crate::state::{Goal, TaskClass, Todo, TodoStatus};

use self::arbitration::{apply_arbitration, ARBITRATION_ENFORCEMENT};
use self::boundary::boundary_snapshot;
use self::frontier::{frontier_projection, lane, sorted_runnable};
use self::goal_boundary::goal_boundary_json;
use self::heartbeat_recommendation::recommendation;
use self::identity::identity_gate;
use self::monitor::{monitor_outcome, MonitorOutcome};
use self::oscillation::oscillation_replan_reason;
use self::primary_action::agent_channel;
use self::stall::{
    is_monitor_stalled, outcome_floor_breach, repair_exhausted, repair_exhausted_reason,
};
use crate::quota::error_codes::DecisionReasonCode;

pub use self::arbitration::{
    build_scheduler_arbitration, SchedulerArbitration, SchedulerDisposition,
    SCHEDULER_ARBITRATION_SCHEMA_VERSION,
};
pub use self::goal_boundary::compose_goal_boundary;
pub use self::monitor::{monitor_poll_classification, MONITOR_NO_CHANGE_BACKOFF_SECS};
pub use self::oscillation::OSCILLATION_PATTERN_LEN;
pub use self::stall::{MAX_REPAIR_ATTEMPTS, MONITOR_NO_CHANGE_REPLAN_THRESHOLD};
pub use crate::quota::slot_accounting::QUOTA_ALLOWED_SLOTS;
pub use crate::state::now_epoch;

/// B: LLM-zombie threshold — consecutive turns on one todo with NO
/// write-class tool (write/edit/shell) that force a replan. A worker that
/// lands nothing material is a silent-LLM-loop zombie; restarting the same
/// session replays the same failing context. Two turns is tight enough to
/// catch the loop before the burn compounds, loose enough that a single
/// quiet-turn (e.g. a long think before the first write) never trips it.
pub const LLM_ZOMBIE_TURN_THRESHOLD: u32 = 2;

/// should-run decision compiler. Pure: injectable clock, no I/O.
/// `agent_id`: when present, must be a registered peer (reference fail-closed:
/// unregistered identity ⇒ `automation_prompt_upgrade_required`, no delivery).
pub fn decide(goal: &Goal, now: SystemTime) -> ShouldRunPacket {
    decide_for(goal, now, None)
}

pub fn decide_for(goal: &Goal, now: SystemTime, agent_id: Option<&str>) -> ShouldRunPacket {
    // ── 0a. Cancelled goals never run (automation stopped, state kept). ──
    if goal.status == "cancelled" {
        let mut p = packet(
            goal,
            DecisionReasonCode::GoalCancelled,
            "skip",
            false,
            "cancelled",
            TurnMode::Terminal,
            "goal was cancelled — automation stopped, state retained for audit",
            UserChannel::none(),
            AgentChannel {
                must_attempt: false,
                delivery_allowed: false,
                quiet_noop_allowed: true,
                primary_action: None,
                selected_todo: None,
                fallback_todo: None,
            },
        );
        p.status = "cancelled".to_string();
        return p;
    }

    // ── 0. Identity gate (LoopX: quota --agent-id requires
    //       coordination.registered_agents; anonymous path allowed). ──────
    if let Some(p) = identity_gate(goal, agent_id) {
        return p;
    }

    // ── 1. User gates (scoped semantics; only gates trigger the ask channel).
    //        Non-blocking user_actions surface in the user channel but never
    //        freeze the agent. ─────────────────────────────────────────────
    let gates: Vec<&Todo> = goal.open_gates().collect();
    let user_actions: Vec<&Todo> = goal.open_of(TaskClass::UserAction).collect();
    let runnable = sorted_runnable(goal, agent_id);

    if !gates.is_empty() {
        let question = gates
            .iter()
            .filter_map(|g| g.gate_question.clone().or_else(|| Some(g.text.clone())))
            .collect::<Vec<_>>()
            .join(" | ");
        let mut question = question;
        if !user_actions.is_empty() {
            let actions = user_actions
                .iter()
                .map(|a| a.text.clone())
                .collect::<Vec<_>>()
                .join(" | ");
            question = format!("{question} | (actions) {actions}");
        }
        let fallback = runnable.first().map(|t| t.id.clone());
        let has_fallback = fallback.is_some();
        return packet(
            goal,
            DecisionReasonCode::OpenUserGate,
            "run",
            true,
            "ask_user",
            TurnMode::AskUser,
            &format!(
                "open user gate(s): {}",
                gates
                    .iter()
                    .map(|g| g.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            UserChannel {
                action_required: true,
                notify: "NOTIFY".to_string(),
                question: Some(question),
                todo_ids: gates.iter().map(|g| g.id.clone()).collect(),
            },
            agent_channel(
                runnable.first().map(|t| t.text.clone()),
                None,
                fallback,
                has_fallback,
                has_fallback,
                false,
            ),
        );
    }

    // ── 2. Runnable advancement. ─────────────────────────────────────────
    let retryable: Vec<&Todo> = runnable
        .into_iter()
        .filter(|t| t.failed_attempts <= MAX_REPAIR_ATTEMPTS)
        .collect();
    // ── 2b. Outcome floor (LoopX: surface-only progress loop). ──────────
    if let Some(todo) = retryable.first() {
        if let Some(reason) = outcome_floor_breach(goal) {
            return replan_packet(goal, DecisionReasonCode::OutcomeFloorBreach, &reason);
        }
        // Oscillation guard (LoopX 对比改进项 ③): the goal's recent delivery
        // outcomes strictly alternate accept/reject (A→V→A→V) — the
        // action/verify flip-flop that burns spend without converging.
        // Force a frontier-changing replan instead of the next delivery.
        if let Some(reason) = oscillation_replan_reason(goal) {
            return replan_packet(goal, DecisionReasonCode::OscillationDetected, &reason);
        }
        let attempt = todo.failed_attempts + 1;
        let (reason, code) = if attempt > 1 {
            (
                format!("repair attempt {attempt} for todo {}", todo.id),
                DecisionReasonCode::RepairAttempt,
            )
        } else {
            (
                format!("runnable todo {}", todo.id),
                DecisionReasonCode::RunnableTodo,
            )
        };
        // Non-blocking user actions surface in the user channel alongside
        // delivery (LoopX: user_action never freezes the agent).
        let user_channel = if !user_actions.is_empty() {
            UserChannel {
                action_required: true,
                notify: "NOTIFY".to_string(),
                question: Some(
                    user_actions
                        .iter()
                        .map(|a| a.text.clone())
                        .collect::<Vec<_>>()
                        .join(" | "),
                ),
                todo_ids: user_actions.iter().map(|a| a.id.clone()).collect(),
            }
        } else {
            UserChannel::none()
        };
        return packet(
            goal,
            code,
            "run",
            true,
            "normal_run",
            TurnMode::BoundedDelivery,
            &reason,
            user_channel,
            agent_channel(
                Some(todo.text.clone()),
                Some(todo.id.clone()),
                None,
                true,
                true,
                false,
            ),
        );
    }
    if repair_exhausted(goal) {
        let reason = repair_exhausted_reason(goal)
            .unwrap_or_else(|| "advancement todo(s) exhausted repair budget".to_string());
        return replan_packet(goal, DecisionReasonCode::RepairBudgetExhausted, &reason);
    }

    // ── 2d. B: LLM-health zombie detection — a worker whose recent turns all
    //       ended with NO write-class tool activity (write/edit/shell) is not
    //       making material progress; it is either stuck in a silent LLM loop
    //       or reasoning without ever landing an artifact. Relaunching the
    //       same session replays the same context and re-hits the same wall.
    //       Surface a replan so the orchestrator restarts the worker with a
    //       FRESH session (the durable loop state carries the context via the
    //       turn envelope — nothing is lost). Counts only turns for this
    //       todo, not the goal at large. ───────────────────────────────────
    if let Some(todo) = retryable.first() {
        let no_progress_turns = goal
            .turn_no_progress
            .iter()
            .filter(|np| np.todo_id == todo.id)
            .count() as u32;
        if no_progress_turns >= LLM_ZOMBIE_TURN_THRESHOLD {
            return replan_packet(
                goal,
                DecisionReasonCode::RepairBudgetExhausted,
                &format!(
                    "LLM zombie: todo {} produced {} turns with no write-class tool (write/edit/shell) — the worker is stuck without landing an artifact; restart it with a fresh session (context replays from the ledger)",
                    todo.id, no_progress_turns
                ),
            );
        }
    }

    // ── 2c. Blocked by an external blocker with no fallback: quiet wait. ──
    let blockers: Vec<&Todo> = goal.open_of(TaskClass::Blocker).collect();
    if !blockers.is_empty() {
        return packet(
            goal,
            DecisionReasonCode::BlockedNoFallback,
            "wait",
            false,
            "quiet_wait",
            TurnMode::WaitMonitor,
            &format!(
                "waiting on blocker(s): {} — no runnable fallback",
                blockers
                    .iter()
                    .map(|b| b.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            UserChannel::none(),
            agent_channel(None, None, None, false, false, true),
        );
    }

    // ── 3. Succession replan obligation (LoopX: completed advancement
    //       without successor/no-follow-up). ─────────────────────────────
    let unclosed = goal.completed_without_closure_intent();
    if !unclosed.is_empty() {
        return replan_packet(
            goal,
            DecisionReasonCode::SuccessionClosureMissing,
            &format!(
                "completed advancement without closure intent: {} — complete must declare successor or --no-follow-up",
                unclosed.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", ")
            ),
        );
    }

    // ── 4. Monitors. ────────────────────────────────────────────────────
    match monitor_outcome(goal, now) {
        MonitorOutcome::Stalled(stalled) => {
            return replan_packet(
                goal,
                DecisionReasonCode::MonitorStalled,
                &format!(
                    "monitor {} stalled ({} consecutive no-change polls)",
                    stalled.id, stalled.consecutive_no_change
                ),
            );
        }
        MonitorOutcome::Due(due) => {
            return packet(
                goal,
                DecisionReasonCode::MonitorDue,
                "run",
                true,
                "monitor_poll",
                TurnMode::MonitorPoll,
                &format!(
                    "monitor {} due — one read-only poll, no spend on no-change",
                    due[0].id
                ),
                UserChannel::none(),
                agent_channel(
                    Some(due[0].text.clone()),
                    Some(due[0].id.clone()),
                    None,
                    true,
                    true,
                    false,
                ),
            );
        }
        MonitorOutcome::Waiting(next_due_ms) => {
            return packet(
                goal,
                DecisionReasonCode::MonitorBackoff,
                "wait",
                false,
                "quiet_wait",
                TurnMode::WaitMonitor,
                "monitor(s) present, none due — quiet wait with backoff",
                UserChannel::none(),
                agent_channel(None, None, None, false, false, true),
            )
            .with_next_due_ms(next_due_ms);
        }
        MonitorOutcome::None => {}
    }

    // ── 5. Acceptance gaps with no work left. ───────────────────────────
    let gaps = goal.unsatisfied_gaps();
    if !gaps.is_empty() {
        return replan_packet(
            goal,
            DecisionReasonCode::AcceptanceGapOpen,
            &format!(
                "acceptance gap(s) open with no runnable work: {}",
                gaps.iter()
                    .map(|g| g.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }

    // ── 5b. Pending deferred work (not yet due): quiet wait, automation
    //       stays alive — never terminal while deferred work is pending. ──
    if goal
        .todos
        .iter()
        .any(|t| t.status == TodoStatus::Deferred && !t.is_due_deferred(now))
    {
        return packet(
            goal,
            DecisionReasonCode::DeferredNotDue,
            "wait",
            false,
            "quiet_wait",
            TurnMode::WaitMonitor,
            "deferred todo(s) not yet due — quiet wait with backoff",
            UserChannel::none(),
            agent_channel(None, None, None, false, false, true),
        );
    }

    // ── 5c. Open advancement work exists but none of it is runnable for
    //        THIS agent (leased to peers). Quiet wait — never terminal: a
    //        leased todo is not a closed goal, and falling through to
    //        closure here parks every subsequent run in a skip loop until
    //        the lease expires. (Learned from goal_c86c1a0aa7b8.) ────────
    if goal.todos.iter().any(|t| {
        t.class == TaskClass::Advancement
            && (t.status == TodoStatus::Open
                || (t.status == TodoStatus::Deferred && t.is_due_deferred(now)))
    }) {
        return packet(
            goal,
            DecisionReasonCode::WorkLeasedToOthers,
            "wait",
            false,
            "quiet_wait",
            TurnMode::WaitMonitor,
            "open advancement(s) leased to other agents — quiet wait, goal is not closed",
            UserChannel::none(),
            agent_channel(None, None, None, false, false, true),
        );
    }

    // ── 6. Validated closure — the terminal judgement (G13 ④): closure is
    //       validated from complete sources (todos done/superseded, every
    //       acceptance gap satisfied, closure intent declared, no pending
    //       deferred work) and the judgement enumerates the gap detail.
    //       `terminal` ⇔ `Goal::is_terminal()` — the judgement is the single
    //       authoritative gate here; any remaining blocker surfaces as a
    //       replan with the explicit gap list (defensive: the pipeline above
    //       catches every blocker before this point). ───────────────────────
    let judgement = crate::decision::goal_frontier::terminal::terminal_judgement(goal);
    if !judgement.terminal {
        let gaps: Vec<String> = judgement
            .gaps
            .iter()
            .map(|g| {
                let id = g.gap_id.as_deref().or(g.todo_id.as_deref()).unwrap_or("-");
                format!("{}:{}", g.kind, id)
            })
            .collect();
        return replan_packet(
            goal,
            DecisionReasonCode::AcceptanceGapOpen,
            &format!("terminal judgement open gaps: {}", gaps.join(", ")),
        );
    }
    let mut p = packet(
        goal,
        DecisionReasonCode::ValidatedClosure,
        "skip",
        false,
        "terminal_no_followup",
        TurnMode::Terminal,
        "validated closure: todos done, gaps closed, closure intent declared — stop recurring automation until an explicit resume",
        UserChannel::none(),
        agent_channel(None, None, None, false, false, true),
    );
    p.terminal_closure = Some(TerminalClosure {
        kind: "no_followup".to_string(),
        derived: true,
        source: "validated_goal_closure".to_string(),
    });
    p
}

fn replan_packet(goal: &Goal, code: DecisionReasonCode, reason: &str) -> ShouldRunPacket {
    packet(
        goal,
        code,
        "replan",
        true,
        "replan",
        TurnMode::Replan,
        reason,
        UserChannel::none(),
        agent_channel(None, None, None, true, false, false),
    )
}

#[allow(clippy::too_many_arguments)]
fn packet(
    goal: &Goal,
    reason_code: DecisionReasonCode,
    decision: &str,
    should_run: bool,
    effective_action: &str,
    mode: TurnMode,
    reason: &str,
    user_channel: UserChannel,
    agent_channel: AgentChannel,
) -> ShouldRunPacket {
    let must_attempt = agent_channel.must_attempt;
    let gates = goal.open_gates().count();
    let done_unclosed = goal.completed_without_closure_intent().len();
    let monitor_stalled = goal.open_monitors().any(is_monitor_stalled);
    let replan_required = match mode {
        TurnMode::Replan => true,
        _ => {
            gates > 0 || done_unclosed > 0 || monitor_stalled || !goal.unsatisfied_gaps().is_empty()
        }
    };
    let spent = goal.history.len() as u64;
    let mut p = ShouldRunPacket {
        ok: true,
        mode: "should-run".to_string(),
        goal_id: goal.goal_id.clone(),
        decision: decision.to_string(),
        should_run,
        effective_action: effective_action.to_string(),
        reason: reason.to_string(),
        reason_code: reason_code.as_str().to_string(),
        state: if should_run { "eligible".to_string() } else { "waiting".to_string() },
        waiting_on: "codex".to_string(),
        status: match mode {
            TurnMode::Terminal => "validated_closure".to_string(),
            TurnMode::Replan => "replan_required".to_string(),
            TurnMode::AskUser => "waiting_on_user".to_string(),
            TurnMode::WaitMonitor => "quiet_wait".to_string(),
            _ => "connected_without_run".to_string(),
        },
        source: "run_history".to_string(),
        recommended_action: match mode {
            TurnMode::Terminal => "validated closure; stop recurring automation".to_string(),
            TurnMode::AskUser => "resolve the user gate before gated work".to_string(),
            TurnMode::Replan => "record a frontier-changing replan delta".to_string(),
            _ => reason.to_string(),
        },
        rollout_event: Some(RolloutEvent {
            schema_version: "future_loop_rollout_event_v0".to_string(),
            event_id: uuid::Uuid::new_v4().simple().to_string(),
            event_kind: "quota_should_run".to_string(),
            recorded_at: crate::compat::rfc3339(crate::state::now_epoch()),
            status: decision.to_string(),
        }),
        status_health_ok: true,
        normal_delivery_allowed: mode == TurnMode::BoundedDelivery,
        recovery_delivery_allowed: false,
        self_repair_allowed: mode == TurnMode::Replan,
        // The capability framework (and its per-tool quota) was removed:
        // the capability-repair lane is permanently closed. The packet field
        // stays for wire compatibility with consumers of the legacy packet.
        capability_repair_allowed: false,
        workspace_repair_allowed: false,
        actionable_by_codex: should_run,
        requires_user_action: mode == TurnMode::AskUser,
        action_required: mode == TurnMode::AskUser,
        selected_todo: agent_channel.selected_todo.as_ref().map(|todo_id| {
            let todo = goal.todo(todo_id);
            serde_json::json!({
                "schema_version": "quota_selected_todo_v0",
                "source": "agent_todo_summary.first_executable_items",
                "todo_id": todo_id,
                "index": todo.map(|t| t.index).unwrap_or(0),
                "role": "agent",
                "priority": todo.map(|t| t.priority.to_string()).unwrap_or_default(),
                "status": "open",
                "task_class": todo.map(|t| crate::compat::future_loop_task_class(t.class)).unwrap_or(""),
                "action_kind": todo.and_then(|t| t.action_kind.clone()).unwrap_or_default(),
                "text": todo.map(|t| t.text.clone()).unwrap_or_default(),
                "agent_id": "",
                "selected_by": "unclaimed_todo",
                "confidence": "candidate",
                "claim_required_before_work": true,
            })
        }),
        open_count: goal.todos.iter().filter(|t| t.role == crate::state::TodoRole::User && t.status == crate::state::TodoStatus::Open).count(),
        blocked_action_scope: None,
        safe_bypass_allowed: false,
        safe_bypass_kind: None,
        safe_bypass_policy: None,
        lifecycle_phase: if mode == TurnMode::Terminal { "closed".to_string() } else { "connected".to_string() },
        lifecycle_flags: if mode == TurnMode::Terminal { vec!["closed".to_string()] } else { vec!["connected".to_string()] },
        project_asset_source: Some("project_asset".to_string()),
        active_state_next_action: goal.next_action.clone(),
        latest_run_recommended_action: goal.next_action.clone(),
        interaction_contract: InteractionContract {
            schema_version: crate::contract::INTERACTION_CONTRACT_SCHEMA_VERSION.to_string(),
            mode,
            user_channel: user_channel.clone(),
            agent_channel: agent_channel.clone(),
            cli_channel: CliChannel {
                next_cli_actions: vec![
                    "loopx refresh-state".to_string(),
                    "loopx quota spend-slot".to_string(),
                ],
                spend_allowed_now: false,
                spend_after_validation: true,
                spend_policy: "spend once after validated writeback".to_string(),
            },
        },
        work_lane_contract: WorkLaneContract {
            schema_version: "work_lane_contract_v1".to_string(),
            lane: lane(goal).to_string(),
            obligation: "advance_one_bounded_segment".to_string(),
            must_attempt_work: must_attempt,
            reason_codes: if agent_channel.selected_todo.is_some() {
                vec!["open_agent_todo".to_string()]
            } else if gates > 0 {
                vec!["open_user_gate".to_string()]
            } else if !goal.unsatisfied_gaps().is_empty() {
                vec!["acceptance_gap".to_string()]
            } else {
                vec!["no_runnable_work".to_string()]
            },
        },
        execution_obligation: ExecutionObligation {
            must_attempt_work: must_attempt,
            kind: "work_lane_contract".to_string(),
            reason: "work_lane_contract.obligation is the machine execution contract".to_string(),
        },
        automation_liveness: AutomationLiveness {
            keep_active: !matches!(mode, TurnMode::Terminal),
            pause_allowed: mode == TurnMode::Terminal,
            action: match mode {
                TurnMode::Terminal => "pause".to_string(),
                TurnMode::WaitMonitor => "quiet_wait".to_string(),
                _ => "execute_bounded_work".to_string(),
            },
            reason: if mode == TurnMode::Terminal {
                "validated closure — no recurring automation".to_string()
            } else {
                "execution obligation keeps the loop alive".to_string()
            },
            pause_policy: "pause/delete only after a bounded self-repair or replan path is itself stuck for more eligible turns".to_string(),
        },
        heartbeat_recommendation: recommendation(mode, must_attempt),
        scheduler_hint: SchedulerHint {
            schema_version: "scheduler_hint_v0".to_string(),
            action: match mode {
                TurnMode::Terminal => "stop".to_string(),
                TurnMode::WaitMonitor => "wait_until_due".to_string(),
                _ => "tick_next".to_string(),
            },
            cadence_class: match mode {
                TurnMode::Terminal => "terminal".to_string(),
                TurnMode::WaitMonitor => "monitor_backoff".to_string(),
                _ => "bounded_segment".to_string(),
            },
            next_due_ms: None,
        },
        quota: QuotaSnapshot {
            compute: 1.0,
            allowed_slots: QUOTA_ALLOWED_SLOTS,
            spent_slots: spent,
            state: if should_run { "eligible".to_string() } else { "waiting".to_string() },
        },
        execution_profile: ExecutionProfileSnapshot {
            cadence: goal.execution_profile.cadence.clone(),
            spend_rule: goal.execution_profile.spend_rule.clone(),
            outcome_floor_streak_threshold: goal.execution_profile.outcome_floor_streak_threshold,
            surface_streak: goal.outcome_streak,
        },
        replan_ack: ReplanAckSnapshot {
            recorded: goal.replan_ack.as_ref().map(|a| a.recorded).unwrap_or(false),
            delta_kinds: goal
                .replan_ack
                .as_ref()
                .map(|a| a.delta_kinds.clone())
                .unwrap_or_default(),
            frontier_delta_present: goal
                .replan_ack
                .as_ref()
                .map(|a| a.has_frontier_delta())
                .unwrap_or(false),
        },
        authority: AuthoritySnapshot {
            write_scope: goal.authority.write_scope.clone(),
            requires_approval: goal.authority.requires_approval.clone(),
            pending_approvals: vec![],
        },
        boundary: boundary_snapshot(goal),
        agent_todo_summary: Some(goal.todo_summary()),
        user_todo_summary: Some(goal.todo_summary()),
        todo_summary_projection: None,
        goal_boundary: Some(goal_boundary_json(goal)),
        plan_summary: None,
        todo_write_hint: Some("loopx todo update".to_string()),
        agent_identity: Some(serde_json::json!({})),
        agent_lane_frontier_hint: None,
        agent_lane_next_action: None,
        goal_route_hint: Some(mode.as_str().to_string()),
        task_scope: Some("goal".to_string()),
        handoff_readiness: None,
        long_task_cadence_hint: None,
        promotion_readiness_warning: None,
        autonomous_backlog_candidates: None,
        protocol_action_packet: None,
        frontier_projection: frontier_projection(goal, replan_required),
        scheduler_arbitration: None,
        terminal_closure: None,
        decision_freshness: goal.decision_freshness.clone(),
    };
    // G-2/G-11: record the scheduler arbitration (observe-only by default).
    apply_arbitration(&mut p, ARBITRATION_ENFORCEMENT);
    p
}

impl UserChannel {
    fn none() -> Self {
        Self {
            action_required: false,
            notify: "DONT_NOTIFY".to_string(),
            question: None,
            todo_ids: vec![],
        }
    }
}

impl ShouldRunPacket {
    fn with_next_due_ms(mut self, next_due_ms: Option<u64>) -> Self {
        self.scheduler_hint.next_due_ms = next_due_ms;
        self
    }
}

// Re-exported helpers for the executor and scenarios.

/// Compose the per-turn packet: todo + resolved gate decisions + prior
/// evidence. Moved to `turn_envelope.rs` (G-9) — the P0 signature is kept
/// here for call-site stability; see [`crate::turn_envelope`] for the full
/// envelope (instruction + context + evidence + decision summary).
pub use crate::turn_envelope::{compose_turn_envelope, compose_turn_message};

/// Completion contract helper: the caller decides successor vs no-follow-up
/// when completing a todo; the kernel refuses silent completion.
pub fn complete_todo(goal: &mut Goal, todo_id: &str, no_follow_up: bool, successors: Vec<String>) {
    if let Some(t) = goal.todo_mut(todo_id) {
        t.complete(no_follow_up, successors);
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut t = s.chars().take(max).collect::<String>();
        t.push('…');
        t
    }
}

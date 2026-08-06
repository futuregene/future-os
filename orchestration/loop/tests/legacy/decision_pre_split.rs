// The decision kernel — `quota should-run` compiled from goal state.
//
// Pure function of state (+ clock), emitting the full `ShouldRunPacket`.
// Pipeline order (LoopX: identity → boundary → gate → frontier → contract):
//   1. user gates (scoped: independent fallback still delivers)
//   2. runnable advancement todos
//   3. succession replan obligation (done todos without closure intent)
//   4. monitors (stall → replan; due → one poll; none due → wait)
//   5. acceptance gaps with no work → replan
//   6. validated closure → terminal_no_followup
//
// No I/O, no LLM: this is the deterministic contract hosts consume.
//
// GENERATED test artifact — verbatim copy of
// `snapshots/decision.rs.pre-split` (source commit 85372a53,
// SHA-256 4ac8c78e7e3f489e1304e57ce0e9f5dbc3bebc965b013913b487b89b9bebf165),
// with mechanical transforms only: `crate::` → `future_loop::`, function renames,
// inherent impls → local traits (integration tests may not impl foreign
// types), and the G-2/G-11 `scheduler_arbitration: None` field. Do not edit
// by hand; regenerate from the snapshot if the baseline moves, then rustfmt.

use std::time::SystemTime;

use future_loop::contract::{
    AgentChannel, AuthoritySnapshot, AutomationLiveness, BoundarySnapshot, CliChannel,
    ExecutionObligation, ExecutionProfileSnapshot, FrontierProjection, HeartbeatRecommendation,
    InteractionContract, QuotaSnapshot, ReplanAckSnapshot, RolloutEvent, SchedulerHint,
    ShouldRunPacket, TerminalClosure, TurnMode, UserChannel, WorkLaneContract,
};
use future_loop::state::{Goal, TaskClass, Todo, TodoStatus};

pub const MONITOR_NO_CHANGE_REPLAN_THRESHOLD: u32 = 3;
pub const MAX_REPAIR_ATTEMPTS: u32 = 1;
pub const QUOTA_ALLOWED_SLOTS: u64 = 1440;

/// should-run decision compiler. Pure: injectable clock, no I/O.
/// `agent_id`: when present, must be a registered peer (LoopX fail-closed:
/// unregistered identity ⇒ `automation_prompt_upgrade_required`, no delivery).
pub fn legacy_decide(goal: &Goal, now: SystemTime) -> ShouldRunPacket {
    legacy_decide_for(goal, now, None)
}

pub fn legacy_decide_for(goal: &Goal, now: SystemTime, agent_id: Option<&str>) -> ShouldRunPacket {
    // ── 0. Identity gate (LoopX: quota --agent-id requires
    //       coordination.registered_agents; anonymous path allowed). ──────
    if !goal.is_registered_agent(agent_id) {
        // Fail-closed identity gate (LoopX: state=blocked_health,
        // status=quota_collection_failed, ok=false).
        let mut p = legacy_packet(
            goal,
            "skip",
            false,
            "automation_prompt_upgrade_required",
            TurnMode::WaitMonitor,
            "quota should-run --agent-id requires coordination.registered_agents; \
             register this agent identity first",
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
        p.ok = false;
        p.state = "blocked_health".to_string();
        p.status = "quota_collection_failed".to_string();
        return p;
    }

    // ── 1. User gates (scoped semantics; only gates trigger the ask channel).
    //        Non-blocking user_actions surface in the user channel but never
    //        freeze the agent. ─────────────────────────────────────────────
    let gates: Vec<&Todo> = goal.open_gates().collect();
    let user_actions: Vec<&Todo> = goal.open_of(future_loop::state::TaskClass::UserAction).collect();
    let mut runnable: Vec<&Todo> = goal.runnable_advancement_for(agent_id).collect();
    // Priority sort: P0 before P1 before P2 (LoopX sorts the frontier).
    runnable.sort_by_key(|t| t.priority);

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
        return legacy_packet(
            goal,
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
            AgentChannel {
                must_attempt: fallback.is_some(),
                delivery_allowed: fallback.is_some(),
                quiet_noop_allowed: false,
                primary_action: runnable.first().map(|t| t.text.clone()),
                selected_todo: None,
                fallback_todo: fallback,
            },
        );
    }

    // ── 2. Runnable advancement. ─────────────────────────────────────────
    let retryable: Vec<&Todo> = runnable
        .into_iter()
        .filter(|t| t.failed_attempts <= MAX_REPAIR_ATTEMPTS)
        .collect();
    // ── 2b. Outcome floor (LoopX: surface-only progress loop). ──────────
    if let Some(todo) = retryable.first() {
        let threshold = goal.execution_profile.outcome_floor_streak_threshold;
        if threshold > 0 && goal.outcome_streak >= threshold {
            return legacy_replan_packet(
                goal,
                &format!(
                    "outcome floor: {surface_streak} consecutive turns without a material outcome (threshold {threshold})",
                    surface_streak = goal.outcome_streak
                ),
            );
        }
        let attempt = todo.failed_attempts + 1;
        let reason = if attempt > 1 {
            format!("repair attempt {attempt} for todo {}", todo.id)
        } else {
            format!("runnable todo {}", todo.id)
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
        return legacy_packet(
            goal,
            "run",
            true,
            "normal_run",
            TurnMode::BoundedDelivery,
            &reason,
            user_channel,
            AgentChannel {
                must_attempt: true,
                delivery_allowed: true,
                quiet_noop_allowed: false,
                primary_action: Some(todo.text.clone()),
                selected_todo: Some(todo.id.clone()),
                fallback_todo: None,
            },
        );
    }
    if goal
        .open_of(TaskClass::Advancement)
        .any(|t| t.failed_attempts > MAX_REPAIR_ATTEMPTS)
    {
        return legacy_replan_packet(goal, "advancement todo(s) exhausted repair budget");
    }

    // ── 2c. Blocked by an external blocker with no fallback: quiet wait. ──
    let blockers: Vec<&Todo> = goal.open_of(TaskClass::Blocker).collect();
    if !blockers.is_empty() {
        return legacy_packet(
            goal,
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
            AgentChannel {
                must_attempt: false,
                delivery_allowed: false,
                quiet_noop_allowed: true,
                primary_action: None,
                selected_todo: None,
                fallback_todo: None,
            },
        );
    }

    // ── 3. Succession replan obligation (LoopX: completed advancement
    //       without successor/no-follow-up). ─────────────────────────────
    let unclosed = goal.completed_without_closure_intent();
    if !unclosed.is_empty() {
        return legacy_replan_packet(
            goal,
            &format!(
                "completed advancement without closure intent: {} — complete must declare successor or --no-follow-up",
                unclosed.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", ")
            ),
        );
    }

    // ── 4. Monitors. ────────────────────────────────────────────────────
    let monitors: Vec<&Todo> = goal.open_monitors().collect();
    if let Some(stalled) = monitors
        .iter()
        .find(|m| m.consecutive_no_change >= MONITOR_NO_CHANGE_REPLAN_THRESHOLD)
    {
        return legacy_replan_packet(
            goal,
            &format!(
                "monitor {} stalled ({} consecutive no-change polls)",
                stalled.id, stalled.consecutive_no_change
            ),
        );
    }
    let due: Vec<&Todo> = monitors
        .iter()
        .filter(|m| m.resume_when.is_some_and(|d| d <= now))
        .copied()
        .collect();
    if !due.is_empty() {
        return legacy_packet(
            goal,
            "run",
            true,
            "monitor_poll",
            TurnMode::MonitorPoll,
            &format!(
                "monitor {} due — one read-only poll, no spend on no-change",
                due[0].id
            ),
            UserChannel::none(),
            AgentChannel {
                must_attempt: true,
                delivery_allowed: true,
                quiet_noop_allowed: false,
                primary_action: Some(due[0].text.clone()),
                selected_todo: Some(due[0].id.clone()),
                fallback_todo: None,
            },
        );
    }
    if !monitors.is_empty() {
        let next_due = monitors.iter().filter_map(|m| m.resume_when).min();
        return legacy_packet(
            goal,
            "wait",
            false,
            "quiet_wait",
            TurnMode::WaitMonitor,
            "monitor(s) present, none due — quiet wait with backoff",
            UserChannel::none(),
            AgentChannel {
                must_attempt: false,
                delivery_allowed: false,
                quiet_noop_allowed: true,
                primary_action: None,
                selected_todo: None,
                fallback_todo: None,
            },
        )
        .with_next_due_ms(
            next_due.and_then(|d| d.duration_since(now).ok().map(|x| x.as_millis() as u64)),
        );
    }

    // ── 5. Acceptance gaps with no work left. ───────────────────────────
    let gaps = goal.unsatisfied_gaps();
    if !gaps.is_empty() {
        return legacy_replan_packet(
            goal,
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
        return legacy_packet(
            goal,
            "wait",
            false,
            "quiet_wait",
            TurnMode::WaitMonitor,
            "deferred todo(s) not yet due — quiet wait with backoff",
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
    }

    // ── 6. Validated closure. ───────────────────────────────────────────
    let mut p = legacy_packet(
        goal,
        "skip",
        false,
        "terminal_no_followup",
        TurnMode::Terminal,
        "validated closure: todos done, gaps closed, closure intent declared — stop recurring automation until an explicit resume",
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
    p.terminal_closure = Some(TerminalClosure {
        kind: "no_followup".to_string(),
        derived: true,
        source: "validated_goal_closure".to_string(),
    });
    p
}

fn legacy_replan_packet(goal: &Goal, reason: &str) -> ShouldRunPacket {
    legacy_packet(
        goal,
        "replan",
        true,
        "replan",
        TurnMode::Replan,
        reason,
        UserChannel::none(),
        AgentChannel {
            must_attempt: true,
            delivery_allowed: false,
            quiet_noop_allowed: false,
            primary_action: None,
            selected_todo: None,
            fallback_todo: None,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn legacy_packet(
    goal: &Goal,
    decision: &str,
    should_run: bool,
    effective_action: &str,
    mode: TurnMode,
    reason: &str,
    user_channel: UserChannel,
    agent_channel: AgentChannel,
) -> ShouldRunPacket {
    let lane = if goal.open_of(TaskClass::Monitor).next().is_some() {
        "monitor"
    } else {
        "advancement_task"
    };
    let must_attempt = agent_channel.must_attempt;
    let gates = goal.open_gates().count();
    let done_unclosed = goal.completed_without_closure_intent().len();
    let monitor_stalled = goal
        .open_monitors()
        .any(|m| m.consecutive_no_change >= MONITOR_NO_CHANGE_REPLAN_THRESHOLD);
    let replan_required = match mode {
        TurnMode::Replan => true,
        _ => {
            gates > 0 || done_unclosed > 0 || monitor_stalled || !goal.unsatisfied_gaps().is_empty()
        }
    };
    let spent = goal.history.len() as u64;
    ShouldRunPacket {
        ok: true,
        mode: "should-run".to_string(),
        goal_id: goal.goal_id.clone(),
        decision: decision.to_string(),
        should_run,
        effective_action: effective_action.to_string(),
        reason: reason.to_string(),
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
            schema_version: "loopx_rollout_event_v0".to_string(),
            event_id: uuid::Uuid::new_v4().simple().to_string(),
            event_kind: "quota_should_run".to_string(),
            recorded_at: future_loop::compat::rfc3339(future_loop::state::now_epoch()),
            status: decision.to_string(),
        }),
        status_health_ok: true,
        normal_delivery_allowed: mode == TurnMode::BoundedDelivery,
        recovery_delivery_allowed: false,
        self_repair_allowed: mode == TurnMode::Replan,
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
                "task_class": todo.map(|t| future_loop::compat::loopx_task_class(t.class)).unwrap_or(""),
                "action_kind": todo.and_then(|t| t.action_kind.clone()).unwrap_or_default(),
                "text": todo.map(|t| t.text.clone()).unwrap_or_default(),
                "agent_id": "",
                "selected_by": "unclaimed_todo",
                "confidence": "candidate",
                "claim_required_before_work": true,
            })
        }),
        open_count: goal.todos.iter().filter(|t| t.role == future_loop::state::TodoRole::User && t.status == future_loop::state::TodoStatus::Open).count(),
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
            schema_version: "loopx_interaction_contract_v0".to_string(),
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
            lane: lane.to_string(),
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
        heartbeat_recommendation: HeartbeatRecommendation {
            recommended_mode: match mode {
                TurnMode::Terminal => "terminal_no_followup".to_string(),
                TurnMode::WaitMonitor => "quiet_wait".to_string(),
                TurnMode::AskUser => "ask_user".to_string(),
                _ => "steering_audit_then_one_step".to_string(),
            },
            notify: if must_attempt { "DONT_NOTIFY".to_string() } else { "NOTIFY_ON_GATE".to_string() },
            spend_policy: "append exactly one heartbeat spend only after a bounded progress segment is validated and written back".to_string(),
            reason: "eligible goal requires the standard steering audit before delivery".to_string(),
        },
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
        boundary: BoundarySnapshot {
            leaks: future_loop::state::boundary_scan_leaks(&goal.objective),
            public_safe: future_loop::state::boundary_scan_leaks(&goal.objective).is_empty(),
        },
        agent_todo_summary: Some(goal.todo_summary()),
        user_todo_summary: Some(goal.todo_summary()),
        todo_summary_projection: None,
        goal_boundary: Some(serde_json::json!({
            "repo": goal.cwd,
            "write_scope": goal.authority.write_scope,
        })),
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
        frontier_projection: FrontierProjection {
            replan_required,
            current_agent_advancement: goal
                .runnable_advancement()
                .filter(|t| t.failed_attempts > 0)
                .count(),
            unclaimed_advancement: goal.runnable_advancement().count(),
            acceptance_gaps: goal.unsatisfied_gaps().len(),
            monitors_open: goal.open_monitors().count(),
            monitors_due: goal
                .open_monitors()
                .filter(|m| m.resume_when.is_some_and(|d| d <= now_epoch_as_time()))
                .count(),
        },
        terminal_closure: None,
        // G-2/G-11 delta (the ONLY struct change since the pre-split baseline):
        // the arbitration record is added post-hoc by `apply_arbitration`.
        scheduler_arbitration: None,
    }
}

trait UserChannelNone {
    fn none() -> Self;
}
impl UserChannelNone for UserChannel {
    fn none() -> Self {
        Self {
            action_required: false,
            notify: "DONT_NOTIFY".to_string(),
            question: None,
            todo_ids: vec![],
        }
    }
}

trait PacketExt {
    fn with_next_due_ms(self, next_due_ms: Option<u64>) -> Self;
}
impl PacketExt for ShouldRunPacket {
    fn with_next_due_ms(mut self, next_due_ms: Option<u64>) -> Self {
        self.scheduler_hint.next_due_ms = next_due_ms;
        self
    }
}

fn now_epoch_as_time() -> SystemTime {
    SystemTime::now()
}

// Re-exported helpers for the executor and scenarios.
pub use future_loop::state::now_epoch;

/// Compose the stable goal boundary appended once to the session system prompt.
pub fn compose_goal_boundary(goal: &Goal) -> String {
    let acceptance = if goal.acceptance.is_empty() {
        "see per-todo acceptance".to_string()
    } else {
        goal.acceptance
            .iter()
            .map(|g| format!("- {} ({})", g.description, g.id))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "You are an executor working under a deterministic control plane.\n\
         \n\
         GOAL: {}\n\
         ACCEPTANCE (what 'done' means):\n{}\n\
         \n\
         Rules:\n\
         - Work exactly one todo per turn. Do not invent work outside the goal.\n\
         - Write evidence: report concrete results (file paths, values, diffs).\n\
         - When the todo is complete, stop — do not continue into extra work.\n\
         - You cannot change the goal or its acceptance.",
        goal.objective, acceptance,
    )
}

/// Compose the per-turn packet: todo + resolved gate decisions + prior evidence.
pub fn compose_turn_message(
    goal: &Goal,
    todo: &Todo,
    prev: Option<&future_loop::state::RunRecord>,
) -> String {
    let mut prompt = format!("TODO {}: {}", todo.id, todo.text);
    if let Some(gate_ids) = todo.blocked_by_gate.as_deref() {
        let decisions: Vec<String> = gate_ids
            .split(',')
            .filter_map(|gid| goal.todo(gid))
            .filter(|g| g.status == TodoStatus::Done)
            .filter_map(|g| g.decision.clone().map(|d| format!("{}: {}", g.id, d)))
            .collect();
        if !decisions.is_empty() {
            prompt.push_str(&format!(
                "\n\nResolved gate decision(s): {}",
                decisions.join("; ")
            ));
        }
    }
    if let Some(p) = prev {
        prompt.push_str(&format!(
            "\n\nEvidence from the previous turn (todo {}):\n{}",
            p.todo_id,
            truncate(&p.evidence, 1_200)
        ));
    }
    prompt.push_str("\n\nComplete the todo and report what you did and observed.");
    prompt
}

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

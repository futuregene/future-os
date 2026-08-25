//! Dashboard projections — every payload is derived fresh from a replay of
//! the event ledger, so the UI is always a faithful read model of CLI state.

use std::time::SystemTime;

use anyhow::Result;
use serde::Serialize;

use crate::decision;
use crate::state::{
    now_epoch, DeliveryState, FailureKind, Goal, RunRecord, TaskClass, Todo, TodoStatus,
};
use crate::store::Store;
use crate::work_items::{attention, replan_obligation, task_graph};

/// Serialize a SystemTime as epoch seconds (JSON has no SystemTime).
fn epoch_secs(t: Option<SystemTime>) -> Option<u64> {
    t.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

#[derive(Debug, Clone, Serialize)]
pub struct TodoView {
    pub id: String,
    pub index: u32,
    pub title: String,
    pub text: String,
    pub class: String,
    pub status: String,
    pub priority: String,
    pub role: String,
    pub gate_question: Option<String>,
    pub decision: Option<String>,
    pub note: Option<String>,
    pub blocked_by: Vec<String>,
    pub blocked: bool,
    pub claimed_by: Option<String>,
    pub lease_expires_at: Option<u64>,
    pub holder_pid: Option<u32>,
    pub holder_alive: Option<bool>,
    pub evidence: Option<String>,
    pub validator: Option<String>,
    pub acceptance: Option<String>,
    pub failed_attempts: u32,
    pub max_validation_attempts: u32,
    pub monitor_target: Option<String>,
    pub monitor_cadence: Option<String>,
    pub monitor_due_at: Option<u64>,
    pub consecutive_no_change: u32,
    pub resume_when_text: Option<String>,
    pub successor_ids: Vec<String>,
    pub no_follow_up: bool,
    pub archive_state: String,
    pub updated_at: u64,
    pub completed_at: Option<u64>,
    pub passed_validation: bool,
    // ── run rollup for the cost column ───────────────────────────────
    pub runs: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    pub first_run_at: Option<u64>,
    pub last_run_at: Option<u64>,
    pub last_holder: Option<String>,
}

fn class_label(t: &Todo) -> &'static str {
    match t.class {
        TaskClass::Advancement => "advancement",
        TaskClass::UserGate => "user_gate",
        TaskClass::UserAction => "user_action",
        TaskClass::Monitor => "monitor",
        TaskClass::Blocker => "blocker",
    }
}

fn status_label(t: &Todo) -> &'static str {
    match t.status {
        TodoStatus::Open => "open",
        TodoStatus::Done => "done",
        TodoStatus::Superseded => "superseded",
        TodoStatus::Deferred => "deferred",
        TodoStatus::Blocked => "blocked",
    }
}

fn todo_view(goal: &Goal, t: &Todo) -> TodoView {
    let mut runs = 0u32;
    let mut tokens_in = 0u64;
    let mut tokens_out = 0u64;
    let mut cost = 0.0f64;
    let mut first_run_at: Option<u64> = None;
    let mut last_run_at: Option<u64> = None;
    for r in goal.history.iter().filter(|r| r.todo_id == t.id) {
        runs += 1;
        tokens_in += r.tokens_in_delta;
        tokens_out += r.tokens_out_delta;
        cost += r.cost_delta;
        first_run_at = Some(first_run_at.map_or(r.recorded_at, |f: u64| f.min(r.recorded_at)));
        last_run_at = Some(last_run_at.map_or(r.recorded_at, |l: u64| l.max(r.recorded_at)));
    }
    TodoView {
        id: t.id.clone(),
        index: t.index,
        title: t.title.clone(),
        text: t.text.clone(),
        class: class_label(t).to_string(),
        status: status_label(t).to_string(),
        priority: t.priority.to_string(),
        role: match t.role {
            crate::state::TodoRole::Agent => "agent",
            crate::state::TodoRole::User => "user",
        }
        .to_string(),
        gate_question: t.gate_question.clone(),
        decision: t.decision.clone(),
        note: t.note.clone(),
        blocked_by: task_graph::predecessors_of(t),
        blocked: goal.is_blocked(t),
        claimed_by: t.claimed_by.clone(),
        lease_expires_at: t.lease_expires_at,
        holder_pid: t.holder_pid,
        holder_alive: t.holder_pid.map(crate::compat::pid_alive),
        evidence: t.evidence.clone(),
        validator: t.validator.clone(),
        acceptance: t.acceptance.clone(),
        failed_attempts: t.failed_attempts,
        max_validation_attempts: t.max_validation_attempts,
        monitor_target: t.monitor_target.clone(),
        monitor_cadence: t.monitor_cadence.clone(),
        monitor_due_at: epoch_secs(t.resume_when),
        consecutive_no_change: t.consecutive_no_change,
        resume_when_text: t.resume_when_text.clone(),
        successor_ids: t.successor_ids.clone(),
        no_follow_up: t.no_follow_up,
        archive_state: t.archive_state.clone(),
        updated_at: t.updated_at,
        completed_at: t.completed_at,
        passed_validation: goal.has_passed_validation(&t.id),
        runs,
        tokens_in,
        tokens_out,
        cost: nnz(cost),
        first_run_at,
        last_run_at,
        last_holder: t.claimed_by.clone(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentView {
    pub id: String,
    pub capabilities: Vec<String>,
    pub workspaces: Vec<String>,
    pub last_heartbeat: Option<u64>,
    pub heartbeat_age_secs: Option<u64>,
    pub active_leases: Vec<String>,
    // ── run rollup for the cost column ───────────────────────────────
    pub runs: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    pub first_run_at: Option<u64>,
    pub last_run_at: Option<u64>,
}

fn agent_views(goal: &Goal, now: u64) -> Vec<AgentView> {
    let mut ids: Vec<String> = goal.registered_agents.clone();
    for a in goal.scheduler_heartbeats.keys() {
        if !ids.iter().any(|x| x == a) {
            ids.push(a.clone());
        }
    }
    // Agents inferred from lease claims (never registered explicitly).
    for t in &goal.todos {
        if let Some(c) = &t.claimed_by {
            if !ids.iter().any(|x| x == c) {
                ids.push(c.clone());
            }
        }
    }
    ids.sort();
    ids.into_iter()
        .map(|id| {
            let profile = goal.agent_profiles.iter().find(|p| p.id == id);
            let hb = goal.scheduler_heartbeats.get(&id).copied();
            let mut leases: Vec<&Todo> = goal
                .todos
                .iter()
                .filter(|t| {
                    t.claimed_by.as_deref() == Some(id.as_str())
                        && t.lease_expires_at.is_some_and(|e| e > now)
                })
                .collect();
            leases.sort_by_key(|t| t.index);
            let mut runs = 0u32;
            let mut tokens_in = 0u64;
            let mut tokens_out = 0u64;
            let mut cost = 0.0f64;
            let mut first_run_at: Option<u64> = None;
            let mut last_run_at: Option<u64> = None;
            for r in goal
                .history
                .iter()
                .filter(|r| r.run_id.starts_with(&format!("{id}-")))
            {
                runs += 1;
                tokens_in += r.tokens_in_delta;
                tokens_out += r.tokens_out_delta;
                cost += r.cost_delta;
                first_run_at =
                    Some(first_run_at.map_or(r.recorded_at, |f: u64| f.min(r.recorded_at)));
                last_run_at =
                    Some(last_run_at.map_or(r.recorded_at, |l: u64| l.max(r.recorded_at)));
            }
            AgentView {
                id: id.clone(),
                capabilities: profile.map(|p| p.capabilities.clone()).unwrap_or_default(),
                workspaces: profile.map(|p| p.workspaces.clone()).unwrap_or_default(),
                last_heartbeat: hb,
                heartbeat_age_secs: hb.map(|h| now.saturating_sub(h)),
                active_leases: leases.into_iter().map(|t| t.id.clone()).collect(),
                runs,
                tokens_in,
                tokens_out,
                cost: nnz(cost),
                first_run_at,
                last_run_at,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct SpendBucket {
    pub runs: u32,
    pub slots: u64,
    pub cost: f64,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

impl SpendBucket {
    fn fold(&mut self, r: &RunRecord) {
        self.runs += 1;
        self.slots += crate::quota::slot_accounting::slot_spend(r);
        self.cost = nnz(self.cost + r.cost_delta);
        self.tokens_in += r.tokens_in_delta;
        self.tokens_out += r.tokens_out_delta;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OutcomeSplit {
    pub succeeded: u32,
    pub verify_failed: u32,
    pub infra_failed: u32,
    pub errored: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpendView {
    pub runs_24h: SpendBucket,
    pub runs_7d: SpendBucket,
    pub total: SpendBucket,
    pub outcomes_7d: OutcomeSplit,
}

fn failure_kind_of(_goal: &Goal, r: &RunRecord) -> FailureKind {
    r.failure_kind
        .unwrap_or_else(|| crate::executor::classify_failure(r))
}

fn spend_view(goal: &Goal, now: u64) -> SpendView {
    let cutoff_24h = now.saturating_sub(24 * 3600);
    let cutoff_7d = now.saturating_sub(7 * 24 * 3600);
    let mut v = SpendView {
        runs_24h: SpendBucket {
            runs: 0,
            slots: 0,
            cost: 0.0,
            tokens_in: 0,
            tokens_out: 0,
        },
        runs_7d: SpendBucket {
            runs: 0,
            slots: 0,
            cost: 0.0,
            tokens_in: 0,
            tokens_out: 0,
        },
        total: SpendBucket {
            runs: 0,
            slots: 0,
            cost: 0.0,
            tokens_in: 0,
            tokens_out: 0,
        },
        outcomes_7d: OutcomeSplit {
            succeeded: 0,
            verify_failed: 0,
            infra_failed: 0,
            errored: 0,
        },
    };
    for r in &goal.history {
        v.total.fold(r);
        if r.recorded_at >= cutoff_24h {
            v.runs_24h.fold(r);
        }
        if r.recorded_at >= cutoff_7d {
            v.runs_7d.fold(r);
            match failure_kind_of(goal, r) {
                FailureKind::None => v.outcomes_7d.succeeded += 1,
                FailureKind::ScienceVerifyFailed => v.outcomes_7d.verify_failed += 1,
                FailureKind::InfraRecoverable => v.outcomes_7d.infra_failed += 1,
                FailureKind::HardError => v.outcomes_7d.errored += 1,
            }
        }
    }
    v
}

/// JSON payloads must not carry `-0.0` (an empty run history sums to
/// negative zero via f64 and renders as "$-0.00" in the UI).
fn nnz(x: f64) -> f64 {
    if x == 0.0 {
        0.0
    } else {
        x
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GoalCard {
    pub goal_id: String,
    pub objective: String,
    pub cwd: String,
    pub status: String,
    pub created_at: u64,
    pub todos_open: usize,
    pub todos_done: usize,
    pub todos_total: usize,
    pub open_gates: usize,
    pub runs_total: usize,
    pub cost_total: f64,
    pub decision: String,
    pub decision_reason: String,
    pub waiting_on: String,
    pub severity: Option<String>,
    pub terminal: bool,
    pub cancelled: bool,
    pub last_run_at: Option<u64>,
}

fn goal_card(goal: &Goal, now: u64) -> GoalCard {
    let packet = decision::decide_for(goal, SystemTime::now(), None);
    let item = attention::goal_attention_item(goal);
    GoalCard {
        goal_id: goal.goal_id.clone(),
        objective: goal.objective.clone(),
        cwd: goal.cwd.clone(),
        status: goal.status.clone(),
        created_at: goal.created_at,
        todos_open: goal
            .todos
            .iter()
            .filter(|t| t.status == TodoStatus::Open)
            .count(),
        todos_done: goal
            .todos
            .iter()
            .filter(|t| t.status == TodoStatus::Done)
            .count(),
        todos_total: goal.todos.len(),
        open_gates: goal.open_gates().count(),
        runs_total: goal.history.len(),
        cost_total: nnz(goal.history.iter().map(|r| r.cost_delta).sum()),
        decision: packet.decision.clone(),
        decision_reason: packet.reason.clone(),
        waiting_on: item
            .as_ref()
            .map(|i| i.waiting_on.clone())
            .unwrap_or_else(|| packet.waiting_on.clone()),
        severity: item.as_ref().map(|i| i.severity.clone()),
        terminal: goal.is_terminal(),
        cancelled: goal.status == "cancelled",
        last_run_at: goal
            .history
            .iter()
            .map(|r| r.recorded_at)
            .max()
            .filter(|_| now > 0),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OverviewTotals {
    pub goals: usize,
    pub active: usize,
    pub terminal: usize,
    pub cancelled: usize,
    pub open_gates: usize,
    pub open_todos: usize,
    pub runs_24h: u32,
    pub cost_24h: f64,
    pub runs_7d: u32,
    pub cost_7d: f64,
    pub slots_7d: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub generated_at: u64,
    pub root: String,
    pub totals: OverviewTotals,
    pub attention: attention::AttentionQueue,
    pub goals: Vec<GoalCard>,
}

/// Replay every registered goal (a goal whose ledger fails to replay is
/// skipped — one corrupt goal must not blank the whole dashboard).
fn replay_all(store: &Store) -> Vec<Goal> {
    store
        .registry()
        .iter()
        .filter_map(|e| store.replay(&e.goal_id).ok().flatten())
        .collect()
}

pub fn overview(store: &Store) -> Result<Overview> {
    let now = now_epoch();
    let goals = replay_all(store);
    let mut cards: Vec<GoalCard> = goals.iter().map(|g| goal_card(g, now)).collect();
    // Triage order: goals needing a human first, then actionable, then the rest.
    cards.sort_by(|a, b| {
        fn rank(c: &GoalCard) -> u8 {
            if c.cancelled || c.terminal {
                3
            } else if c.open_gates > 0 {
                0
            } else if c.severity.as_deref() == Some("high") {
                1
            } else {
                2
            }
        }
        rank(a).cmp(&rank(b)).then(b.created_at.cmp(&a.created_at))
    });
    let items: Vec<attention::AttentionItem> = goals
        .iter()
        .filter_map(attention::goal_attention_item)
        .collect();
    let queue = attention::build_attention_queue(items);
    let cutoff_24h = now.saturating_sub(24 * 3600);
    let cutoff_7d = now.saturating_sub(7 * 24 * 3600);
    let mut totals = OverviewTotals {
        goals: goals.len(),
        active: 0,
        terminal: 0,
        cancelled: 0,
        open_gates: 0,
        open_todos: 0,
        runs_24h: 0,
        cost_24h: 0.0,
        runs_7d: 0,
        cost_7d: 0.0,
        slots_7d: 0,
    };
    for g in &goals {
        if g.status == "cancelled" {
            totals.cancelled += 1;
        } else if g.is_terminal() {
            totals.terminal += 1;
        } else {
            totals.active += 1;
        }
        totals.open_gates += g.open_gates().count();
        totals.open_todos += g
            .todos
            .iter()
            .filter(|t| t.status == TodoStatus::Open)
            .count();
        for r in &g.history {
            if r.recorded_at >= cutoff_24h {
                totals.runs_24h += 1;
                totals.cost_24h = nnz(totals.cost_24h + r.cost_delta);
            }
            if r.recorded_at >= cutoff_7d {
                totals.runs_7d += 1;
                totals.cost_7d = nnz(totals.cost_7d + r.cost_delta);
                totals.slots_7d += crate::quota::slot_accounting::slot_spend(r);
            }
        }
    }
    Ok(Overview {
        generated_at: now,
        root: store.root_path(),
        totals,
        attention: queue,
        goals: cards,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct GoalDetail {
    pub goal_id: String,
    pub objective: String,
    pub cwd: String,
    pub status: String,
    pub created_at: u64,
    pub next_action: Option<String>,
    pub terminal: bool,
    pub decision: serde_json::Value,
    pub attention: Option<attention::AttentionItem>,
    pub replan_obligations: Vec<replan_obligation::ReplanObligation>,
    pub todos: Vec<TodoView>,
    pub agents: Vec<AgentView>,
    pub acceptance: Vec<crate::state::AcceptanceGap>,
    pub deliveries: Vec<DeliveryState>,
    pub unvalidated_deliveries: Vec<String>,
    pub spend: SpendView,
    pub runs: Vec<RunRecord>,
    pub semantic_history: Vec<crate::decision::goal_frontier::semantic_history::SemanticEvent>,
    pub frontier: crate::decision::goal_frontier::FrontierShow,
    pub task_graph: Option<task_graph::TaskGraph>,
    pub liveness_alerts: Vec<crate::state::LivenessAlert>,
    pub authority: crate::state::Authority,
    pub execution_profile: crate::state::ExecutionProfile,
    pub event_count: usize,
}

pub fn goal_detail(store: &Store, goal_id: &str) -> Result<Option<GoalDetail>> {
    let Some(goal) = store.replay(goal_id)? else {
        return Ok(None);
    };
    let now = now_epoch();
    let packet = decision::decide_for(&goal, SystemTime::now(), None);
    let mut todos: Vec<TodoView> = goal.todos.iter().map(|t| todo_view(&goal, t)).collect();
    todos.sort_by_key(|t| t.index);
    let mut runs = goal.history.clone();
    runs.sort_by_key(|r| std::cmp::Reverse(r.recorded_at));
    runs.truncate(200);
    let graph = task_graph::build_task_graph(&goal).ok();
    let event_count = store.events(goal_id).map(|e| e.len()).unwrap_or(0);
    Ok(Some(GoalDetail {
        goal_id: goal.goal_id.clone(),
        objective: goal.objective.clone(),
        cwd: goal.cwd.clone(),
        status: goal.status.clone(),
        created_at: goal.created_at,
        next_action: goal.next_action.clone(),
        terminal: goal.is_terminal(),
        decision: serde_json::to_value(&packet)?,
        attention: attention::goal_attention_item(&goal),
        replan_obligations: replan_obligation::detect_obligations(&goal),
        todos,
        agents: agent_views(&goal, now),
        acceptance: goal.acceptance.clone(),
        deliveries: goal.delivery_states.clone(),
        unvalidated_deliveries: goal
            .unvalidated_deliveries()
            .iter()
            .map(|t| t.id.clone())
            .collect(),
        spend: spend_view(&goal, now),
        runs,
        semantic_history: goal.semantic_history.clone(),
        frontier: crate::decision::goal_frontier::frontier_show(&goal),
        task_graph: graph,
        liveness_alerts: goal.liveness_alerts.clone(),
        authority: goal.authority.clone(),
        execution_profile: goal.execution_profile.clone(),
        event_count,
    }))
}

pub fn runs_page(store: &Store, goal_id: &str, limit: usize) -> Result<Option<Vec<RunRecord>>> {
    let Some(goal) = store.replay(goal_id)? else {
        return Ok(None);
    };
    let mut runs = goal.history.clone();
    runs.sort_by_key(|r| std::cmp::Reverse(r.recorded_at));
    runs.truncate(limit.min(500));
    Ok(Some(runs))
}

#[derive(Debug, Clone, Serialize)]
pub struct EventView {
    pub event_id: String,
    pub kind: String,
    pub ts: u64,
    pub event: serde_json::Value,
}

pub fn events_page(store: &Store, goal_id: &str, limit: usize) -> Result<Option<Vec<EventView>>> {
    if !store.registered(goal_id) {
        return Ok(None);
    }
    // Raw ledger lines: the stored envelope is flattened (kind/goal_id/ts
    // plus the payload live at the top level, next to the provenance
    // fields), so we project directly instead of re-deriving from `Event`.
    let mut lines = store.raw_ledger_lines(goal_id)?;
    lines.reverse();
    let views = lines
        .into_iter()
        .take(limit.min(500))
        .map(|line| {
            let value: serde_json::Value =
                serde_json::from_str(&line).unwrap_or_else(|_| serde_json::json!({"raw": line}));
            EventView {
                event_id: value
                    .get("event_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                kind: value
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                ts: value.get("ts").and_then(|v| v.as_u64()).unwrap_or(0),
                event: value,
            }
        })
        .collect();
    Ok(Some(views))
}

// The dashboard is strictly read-only: it projects the event ledger and
// never appends to it. Mutations (gate resolve, goal cancel, …) stay in
// the CLI (`future loop gate resolve`, `future loop goal cancel`).

/// Compact per-goal push payload for the SSE stream (avoids resending full
/// details every tick; the detail view refetches on `goals` events anyway).
pub fn goals_push(store: &Store) -> Result<Vec<GoalCard>> {
    let now = now_epoch();
    Ok(replay_all(store)
        .iter()
        .map(|g| goal_card(g, now))
        .collect())
}

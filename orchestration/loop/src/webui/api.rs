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

fn goal_card(goal: &Goal, created_at: u64, now: u64) -> GoalCard {
    let packet = decision::decide_for(goal, SystemTime::now(), None);
    let item = attention::goal_attention_item(goal);
    GoalCard {
        goal_id: goal.goal_id.clone(),
        objective: goal.objective.clone(),
        cwd: goal.cwd.clone(),
        status: goal.status.clone(),
        created_at,
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
    // Registry timestamps are stable across replays; the replayed goal's
    // `created_at` is not (the ledger ignores GoalStarted, so replay stamps
    // it with "now" — which would make every fingerprint change per tick).
    let created_by: std::collections::HashMap<String, u64> = store
        .registry()
        .iter()
        .map(|e| (e.goal_id.clone(), e.created_at))
        .collect();
    let mut cards: Vec<GoalCard> = goals
        .iter()
        .map(|g| {
            goal_card(
                g,
                created_by.get(&g.goal_id).copied().unwrap_or(g.created_at),
                now,
            )
        })
        .collect();
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
pub struct ActiveRunView {
    pub run_id: String,
    pub session_id: String,
    pub agent_id: Option<String>,
    pub todo_id: Option<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    pub started_at: Option<u64>,
    pub last_activity_at: Option<u64>,
    pub active: bool,
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
    pub active_runs: Vec<ActiveRunView>,
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
    // Stable creation timestamp (see overview(): the replayed goal's
    // created_at is re-stamped with "now" on every replay).
    let created_at = store
        .registry()
        .iter()
        .find(|e| e.goal_id == goal_id)
        .map(|e| e.created_at)
        .unwrap_or(goal.created_at);
    Ok(Some(GoalDetail {
        goal_id: goal.goal_id.clone(),
        objective: goal.objective.clone(),
        cwd: goal.cwd.clone(),
        status: goal.status.clone(),
        created_at,
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
        active_runs: scan_active_runs(&store.root_path(), goal_id),
    }))
}

/// Live in-flight run projections: scan `<root>/runs/*.live.jsonl` (written
/// by the orchestrator's `execute_turn` header + teed `usage` events) and
/// expose real-time in/out tokens + cost for runs whose stream is still
/// active. The header line carries the goal/agent/todo association; `usage`
/// lines carry per-request token/cost that we sum (each request emits exactly
/// one `usage` event, so summing never double-counts). Read-only: never
/// touches the ledger.
const ACTIVE_RUN_WINDOW_SECS: u64 = 90;

fn scan_active_runs(root: &str, goal_id: &str) -> Vec<ActiveRunView> {
    let now = now_epoch();
    let runs_dir = std::path::Path::new(root).join("runs");
    let entries = match std::fs::read_dir(&runs_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<ActiveRunView> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !name.ends_with(".live") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut agent_id = None;
        let mut todo_id = None;
        let mut session_id = None;
        let mut run_id = None;
        let mut header_goal_id: Option<String> = None;
        let mut started_at: Option<u64> = None;
        let mut last_activity_at: Option<u64> = None;
        let mut tokens_in = 0u64;
        let mut tokens_out = 0u64;
        let mut cost = 0.0f64;
        let mut finished = false;
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(ty) = v.get("type").and_then(|x| x.as_str()) else {
                continue;
            };
            if let Some(ts) = v.get("wall_ts").and_then(|x| x.as_u64()) {
                last_activity_at = Some(last_activity_at.map_or(ts, |c| c.max(ts)));
                if started_at.is_none() {
                    started_at = Some(ts);
                }
            }
            match ty {
                "run_header" => {
                    agent_id = v.get("agent_id").and_then(|x| x.as_str()).map(String::from);
                    todo_id = v.get("todo_id").and_then(|x| x.as_str()).map(String::from);
                    session_id = v
                        .get("session_id")
                        .and_then(|x| x.as_str())
                        .map(String::from);
                    run_id = v.get("run_id").and_then(|x| x.as_str()).map(String::from);
                    header_goal_id = v.get("goal_id").and_then(|x| x.as_str()).map(String::from);
                }
                "usage" => {
                    if let Some(u) = v.get("usage").and_then(|x| x.as_object()) {
                        tokens_in += u
                            .get("prompt_tokens")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(0)
                            .max(0) as u64;
                        tokens_out += u
                            .get("completion_tokens")
                            .and_then(|x| x.as_i64())
                            .unwrap_or(0)
                            .max(0) as u64;
                        let cc = u
                            .get("credit_cost")
                            .and_then(|x| x.as_f64())
                            .or_else(|| {
                                u.get("credit_cost")
                                    .and_then(|x| x.as_str())
                                    .and_then(|s| s.parse::<f64>().ok())
                            })
                            .unwrap_or(0.0);
                        cost += cc.max(0.0);
                    }
                }
                "agent_end" => {
                    finished = true;
                }
                _ => {}
            }
        }
        if header_goal_id.as_deref() != Some(goal_id) {
            continue;
        }
        let active = !finished
            && last_activity_at.is_some_and(|ts| now.saturating_sub(ts) < ACTIVE_RUN_WINDOW_SECS);
        out.push(ActiveRunView {
            run_id: run_id
                .unwrap_or_else(|| name.strip_suffix(".live").unwrap_or(name).to_string()),
            session_id: session_id.unwrap_or_default(),
            agent_id,
            todo_id,
            tokens_in,
            tokens_out,
            cost: nnz(cost),
            started_at,
            last_activity_at,
            active,
        });
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.last_activity_at));
    out
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
    let created_by: std::collections::HashMap<String, u64> = store
        .registry()
        .iter()
        .map(|e| (e.goal_id.clone(), e.created_at))
        .collect();
    Ok(replay_all(store)
        .iter()
        .map(|g| {
            goal_card(
                g,
                created_by.get(&g.goal_id).copied().unwrap_or(g.created_at),
                now,
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_live(root: &std::path::Path, name: &str, lines: Vec<String>) {
        let dir = root.join("runs");
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(dir.join(name)).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
    }

    #[test]
    fn scan_active_runs_sums_usage_and_filters_by_goal() {
        let now = now_epoch();
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        // Active run (g1): header + two usage events, no agent_end.
        write_live(
            p,
            "run_a.live.jsonl",
            vec![
                serde_json::json!({"type":"run_header","wall_ts":now,"run_id":"run_a","session_id":"s1","agent_id":"mac-w1","todo_id":"todo_t1","goal_id":"g1"}).to_string(),
                serde_json::json!({"type":"usage","wall_ts":now+1,"usage":{"prompt_tokens":10,"completion_tokens":5,"credit_cost":0.001}}).to_string(),
                serde_json::json!({"type":"usage","wall_ts":now+2,"usage":{"prompt_tokens":20,"completion_tokens":8,"credit_cost":0.002}}).to_string(),
            ],
        );
        // Finished run (g1): header + one usage + agent_end.
        write_live(
            p,
            "run_b.live.jsonl",
            vec![
                serde_json::json!({"type":"run_header","wall_ts":now-200,"run_id":"run_b","session_id":"s2","agent_id":"mac-w2","todo_id":"todo_t2","goal_id":"g1"}).to_string(),
                serde_json::json!({"type":"usage","wall_ts":now-199,"usage":{"prompt_tokens":5,"completion_tokens":3,"credit_cost":0.0005}}).to_string(),
                serde_json::json!({"type":"agent_end","wall_ts":now-198}).to_string(),
            ],
        );
        // Foreign goal (g2): header only — must be filtered out.
        write_live(
            p,
            "run_c.live.jsonl",
            vec![serde_json::json!({"type":"run_header","wall_ts":now,"run_id":"run_c","session_id":"s3","agent_id":"mac-w3","todo_id":"todo_t3","goal_id":"g2"}).to_string()],
        );
        // Legacy run (no header): must be filtered out.
        write_live(
            p,
            "run_d.live.jsonl",
            vec![serde_json::json!({"type":"usage","wall_ts":now,"usage":{"prompt_tokens":99,"completion_tokens":99}}).to_string()],
        );

        let runs = scan_active_runs(p.to_str().unwrap(), "g1");
        assert_eq!(runs.len(), 2, "only g1 runs with headers are returned");

        let a = runs.iter().find(|r| r.run_id == "run_a").unwrap();
        assert!(a.active, "no agent_end and recent wall_ts ⇒ active");
        assert_eq!(a.agent_id.as_deref(), Some("mac-w1"));
        assert_eq!(a.todo_id.as_deref(), Some("todo_t1"));
        assert_eq!(a.tokens_in, 30);
        assert_eq!(a.tokens_out, 13);
        assert!((a.cost - 0.003).abs() < 1e-9);

        let b = runs.iter().find(|r| r.run_id == "run_b").unwrap();
        assert!(!b.active, "agent_end marks the run finished");
        assert_eq!(b.tokens_in, 5);
        assert!((b.cost - 0.0005).abs() < 1e-9);
    }

    // ── projection fixtures ────────────────────────────────────────────────

    fn rec(
        todo_id: &str,
        run_id: &str,
        recorded_at: u64,
        kind: FailureKind,
        cost: f64,
    ) -> RunRecord {
        RunRecord {
            turn: 1,
            todo_id: todo_id.to_string(),
            run_id: run_id.to_string(),
            terminal_state: "completed".to_string(),
            error: None,
            tokens_in_delta: 10,
            tokens_out_delta: 5,
            cost_delta: cost,
            tools: vec![],
            evidence: String::new(),
            recorded_at,
            spend_source: None,
            validation: None,
            failure_kind: Some(kind),
            truncation: None,
        }
    }

    fn open_store_with_goal() -> (Store, tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().to_string_lossy()).unwrap();
        let mut goal = Goal::new("g1", "objective", "/tmp");
        goal.add(Todo::advancement("T1", "work"));
        let mut done = Todo::advancement("T2", "done work");
        done.complete(true, vec![]);
        goal.add(done);
        goal.add(Todo::user_gate("G1", "approve?", &["T3"]));
        goal.add(Todo::monitor(
            "M1",
            "watch",
            std::time::Duration::from_secs(60),
        ));
        goal.register_agent("a1", vec!["shell".to_string()]);
        goal.scheduler_heartbeats.insert("a1".to_string(), 100);
        let ts = goal.created_at;
        store.register(&goal).unwrap();
        store
            .append(crate::store::Event::GoalStarted {
                goal_id: "g1".into(),
                ts,
            })
            .unwrap();
        for t in [
            Todo::advancement("T1", "work"),
            {
                let mut d = Todo::advancement("T2", "done work");
                d.complete(true, vec![]);
                d
            },
            Todo::user_gate("G1", "approve?", &["T3"]),
            Todo::monitor("M1", "watch", std::time::Duration::from_secs(60)),
        ] {
            store
                .append(crate::store::Event::TodoAdded {
                    goal_id: "g1".into(),
                    todo: t,
                    ts,
                })
                .unwrap();
        }
        store
            .append(crate::store::Event::AgentOnboarded {
                goal_id: "g1".into(),
                agent_id: "a1".into(),
                capabilities: vec!["shell".into()],
                workspaces: vec![],
                ts,
            })
            .unwrap();
        let now = now_epoch();
        store
            .append_run("g1", &rec("T1", "a1-run", now, FailureKind::None, 0.5))
            .unwrap();
        store
            .append_run(
                "g1",
                &rec("T1", "a1-run2", now, FailureKind::ScienceVerifyFailed, 0.25),
            )
            .unwrap();
        store
            .append_run(
                "g1",
                &rec("T1", "a1-run3", now, FailureKind::InfraRecoverable, 0.125),
            )
            .unwrap();
        store
            .append_run(
                "g1",
                &rec("T1", "a1-run4", now, FailureKind::HardError, 0.0625),
            )
            .unwrap();
        (store, dir, "g1".to_string())
    }

    #[test]
    fn epoch_secs_and_label_helpers() {
        assert!(epoch_secs(None).is_none());
        assert_eq!(epoch_secs(Some(SystemTime::UNIX_EPOCH)), Some(0));

        use crate::state::TodoStatus;
        let t = Todo::advancement("x", "x");
        assert_eq!(class_label(&t), "advancement");
        assert_eq!(status_label(&t), "open");
        let mut m = Todo::monitor("m", "w", std::time::Duration::from_secs(1));
        assert_eq!(class_label(&m), "monitor");
        m.status = TodoStatus::Done;
        assert_eq!(status_label(&m), "done");
    }

    #[test]
    fn nnz_normalizes_zero() {
        assert_eq!(nnz(0.0), 0.0);
        assert_eq!(nnz(-0.0), 0.0);
        assert_eq!(nnz(1.5), 1.5);
    }

    #[test]
    fn overview_and_goals_push_project_goal() {
        let (store, _dir, _gid) = open_store_with_goal();
        let ov = overview(&store).unwrap();
        assert_eq!(ov.totals.goals, 1);
        assert_eq!(ov.totals.active, 1);
        assert_eq!(ov.goals.len(), 1);
        assert_eq!(ov.goals[0].goal_id, "g1");
        assert_eq!(ov.goals[0].todos_total, 4);
        assert_eq!(ov.goals[0].open_gates, 1);

        let push = goals_push(&store).unwrap();
        assert_eq!(push.len(), 1);
        assert_eq!(push[0].goal_id, "g1");
    }

    #[test]
    fn goal_detail_projects_todos_agents_spend_and_runs() {
        let (store, _dir, gid) = open_store_with_goal();
        let detail = goal_detail(&store, &gid).unwrap().unwrap();
        assert_eq!(detail.goal_id, "g1");
        assert_eq!(detail.todos.len(), 4);
        assert_eq!(detail.agents.len(), 1);
        assert_eq!(detail.agents[0].id, "a1");
        assert_eq!(detail.agents[0].capabilities, vec!["shell".to_string()]);
        assert_eq!(detail.runs.len(), 4);
        // spend totals fold all four runs.
        assert_eq!(detail.spend.total.runs, 4);
        // outcomes split: one per failure kind.
        assert_eq!(detail.spend.outcomes_7d.succeeded, 1);
        assert_eq!(detail.spend.outcomes_7d.verify_failed, 1);
        assert_eq!(detail.spend.outcomes_7d.infra_failed, 1);
        assert_eq!(detail.spend.outcomes_7d.errored, 1);
    }

    #[test]
    fn goal_detail_missing_goal_is_none() {
        let (store, _dir, _gid) = open_store_with_goal();
        assert!(goal_detail(&store, "nope").unwrap().is_none());
    }

    #[test]
    fn runs_page_and_events_page_cover_empty_and_missing() {
        let (store, _dir, gid) = open_store_with_goal();
        let runs = runs_page(&store, &gid, 2).unwrap().unwrap();
        assert_eq!(runs.len(), 2, "limit truncates to the newest two");
        assert!(runs_page(&store, "nope", 10).unwrap().is_none());

        let events = events_page(&store, &gid, 10).unwrap().unwrap();
        assert!(!events.is_empty());
        assert!(events_page(&store, "nope", 10).unwrap().is_none());
    }

    #[test]
    fn label_helpers_cover_every_variant() {
        use crate::state::{TaskClass, TodoStatus};
        for (class, label) in [
            (TaskClass::Advancement, "advancement"),
            (TaskClass::UserGate, "user_gate"),
            (TaskClass::UserAction, "user_action"),
            (TaskClass::Monitor, "monitor"),
            (TaskClass::Blocker, "blocker"),
        ] {
            let mut t = Todo::advancement("t", "x");
            t.class = class;
            assert_eq!(class_label(&t), label);
        }
        for (status, label) in [
            (TodoStatus::Open, "open"),
            (TodoStatus::Done, "done"),
            (TodoStatus::Superseded, "superseded"),
            (TodoStatus::Deferred, "deferred"),
            (TodoStatus::Blocked, "blocked"),
        ] {
            let mut t = Todo::advancement("t", "x");
            t.status = status;
            assert_eq!(status_label(&t), label);
        }
    }

    #[test]
    fn agent_views_infer_heartbeat_and_lease_agents() {
        let mut goal = Goal::new("g", "o", "/tmp");
        goal.register_agent("a1", vec!["shell".to_string()]);
        // Heartbeat-only agent (never registered) + lease-inferred agent.
        goal.scheduler_heartbeats.insert("hb_only".to_string(), 100);
        let mut leased = Todo::advancement("T1", "work");
        let now = now_epoch();
        leased.claimed_by = Some("lease_only".to_string());
        leased.lease_expires_at = Some(now + 60);
        goal.add(leased);
        let mut expired = Todo::advancement("T2", "expired lease");
        expired.claimed_by = Some("lease_only".to_string());
        expired.lease_expires_at = Some(now.saturating_sub(1));
        goal.add(expired);

        let views = agent_views(&goal, now);
        let ids: Vec<&str> = views.iter().map(|v| v.id.as_str()).collect();
        assert!(ids.contains(&"a1"));
        assert!(ids.contains(&"hb_only"));
        assert!(ids.contains(&"lease_only"));

        let hb = views.iter().find(|v| v.id == "hb_only").unwrap();
        assert_eq!(hb.last_heartbeat, Some(100));
        assert!(hb.heartbeat_age_secs.is_some());

        let lease = views.iter().find(|v| v.id == "lease_only").unwrap();
        assert_eq!(lease.active_leases, vec!["T1".to_string()]);
    }

    #[test]
    fn scan_active_runs_skips_non_live_non_header_and_foreign() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path();
        let runs = p.join("runs");
        std::fs::create_dir_all(&runs).unwrap();
        // Not a .jsonl file.
        std::fs::write(runs.join("note.txt"), b"x").unwrap();
        // Not a .live.jsonl (missing .live stem).
        std::fs::write(runs.join("plain.jsonl"), b"{}").unwrap();
        // Unreadable JSON lines are skipped.
        std::fs::write(runs.join("junk.live.jsonl"), b"not-json\n").unwrap();
        // A foreign goal header.
        std::fs::write(
            runs.join("foreign.live.jsonl"),
            b"{\"type\":\"run_header\",\"goal_id\":\"other\",\"agent_id\":\"a\",\"wall_ts\":1}\n",
        )
        .unwrap();
        // A run with no agent_id header field is still projected (agent_id
        // becomes None — the header carries no agent to map).
        std::fs::write(
            runs.join("noagent.live.jsonl"),
            b"{\"type\":\"run_header\",\"goal_id\":\"g1\",\"wall_ts\":1}\n",
        )
        .unwrap();
        // A valid run with usage string credit_cost + agent_end.
        std::fs::write(
            runs.join("good.live.jsonl"),
            concat!(
                "{\"type\":\"run_header\",\"goal_id\":\"g1\",\"agent_id\":\"a1\",\"session_id\":\"s\",\"run_id\":\"r\",\"todo_id\":\"t\",\"wall_ts\":100}\n",
                "{\"type\":\"usage\",\"wall_ts\":101,\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"credit_cost\":\"0.5\"}}\n",
                "{\"type\":\"agent_end\",\"wall_ts\":102}\n",
            ),
        )
        .unwrap();

        let views = scan_active_runs(p.to_str().unwrap(), "g1");
        assert_eq!(views.len(), 2);
        let v = views
            .iter()
            .find(|v| v.agent_id.as_deref() == Some("a1"))
            .unwrap();
        assert_eq!(v.tokens_in, 5);
        assert_eq!(v.tokens_out, 2);
        assert!(!v.active, "agent_end marks finished");
        assert!((v.cost - 0.5).abs() < 1e-9);
        let noagent = views.iter().find(|v| v.agent_id.is_none()).unwrap();
        assert_eq!(noagent.run_id, "noagent");
    }

    #[test]
    fn replay_all_skips_a_corrupt_ledger() {
        let (store, _dir, gid) = open_store_with_goal();
        // Truncate the ledger so replay fails → the goal is skipped.
        let ledger = store.goal_dir(&gid).join("events.jsonl");
        std::fs::write(&ledger, "not-json\n").unwrap();
        assert!(replay_all(&store).is_empty());
        // overview still returns Ok with zero goals (one corrupt goal must
        // not blank the whole dashboard).
        let ov = overview(&store).unwrap();
        assert_eq!(ov.totals.goals, 0);
    }
}

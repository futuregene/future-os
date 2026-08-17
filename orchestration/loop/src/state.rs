//! Durable goal state — the control-plane kernel of the FutureOS loop.
//!
//! Mirrors LoopX's state substrate, distilled: a goal owns a work graph
//! (todos), acceptance gaps, and a run-history ledger. `store.rs` persists
//! these as an append-only event log and rebuilds this state by replay.
//! Decision *inputs* live here; the decision *compiler* lives in
//! `decision.rs` (LoopX: quota.py::build_quota_should_run).

use std::time::{Duration, SystemTime};

/// Priority (LoopX: P0/P1/P2 — the decision kernel sorts the frontier by
/// priority before anything else). Serialized with the EXACT reference values.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    Default,
)]
pub enum Priority {
    #[serde(rename = "P0")]
    P0,
    #[serde(rename = "P1")]
    #[default]
    P1,
    #[serde(rename = "P2")]
    P2,
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Priority::P0 => "P0",
                Priority::P1 => "P1",
                Priority::P2 => "P2",
            }
        )
    }
}

/// Task class — LoopX's six-way todo taxonomy. Serialized with the EXACT
/// reference values (advancement_task / continuous_monitor / user_gate /
/// user_action / blocker). The Kanban "column" is a projection of these axes,
/// never a single stored string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TaskClass {
    /// Runnable agent work (advancement).
    #[serde(rename = "advancement_task")]
    Advancement,
    /// A concrete human decision is required before dependent work may run.
    #[serde(rename = "user_gate")]
    UserGate,
    /// A non-blocking human to-do: the user channel surfaces it, but it does
    /// NOT freeze the agent (LoopX: user_action vs user_gate distinction).
    #[serde(rename = "user_action")]
    UserAction,
    /// Periodic read-only observation of an external target.
    #[serde(rename = "continuous_monitor")]
    Monitor,
    /// An external blocker (missing dependency, awaiting authority, waiting
    /// on another lane) that gates dependent todos until resolved.
    #[serde(rename = "blocker")]
    Blocker,
}

/// Lifecycle status — reference values open/done/blocked/deferred, plus our
/// superseded extension (reference performs supersede as an operation; keeping it
/// as a status preserves our event-sourced replay).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TodoStatus {
    #[serde(rename = "open")]
    Open,
    #[serde(rename = "done")]
    Done,
    /// Superseded: a better route was discovered; the todo is no longer
    /// runnable and must not block closure (reference 0623 `supersede`).
    #[serde(rename = "superseded")]
    Superseded,
    /// Deferred: held until `resume_when`; due deferred todos return to Open
    /// and rejoin the frontier (LoopX: deferred + resume_when).
    #[serde(rename = "deferred")]
    Deferred,
    /// Explicitly blocked (LoopX: status blocked — the todo is held, not open).
    #[serde(rename = "blocked")]
    Blocked,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Todo {
    pub id: String,
    pub text: String,
    /// Short title (LoopX: title vs text separation).
    pub title: String,
    pub class: TaskClass,
    pub status: TodoStatus,
    pub priority: Priority,
    /// Agent vs user ownership (LoopX: role is orthogonal to task_class).
    pub role: TodoRole,
    /// Ordering / identity-stability index within the goal (LoopX: index).
    pub index: u32,
    /// Action kind token (LoopX: shell/github/... — quota re-entry routing
    /// input).
    pub action_kind: Option<String>,
    /// Concrete question a UserGate poses (never a vague "waiting for owner").
    pub gate_question: Option<String>,
    /// Resolved decision payload once a gate closes (scoped evidence that must
    /// flow into blocked todos' packets).
    pub decision: Option<String>,
    /// Human note (LoopX: `todo update --note`; rendered as note= in anchors).
    pub note: Option<String>,
    /// Deferred resume condition (LoopX: `--resume-when capacity_available:...`).
    pub resume_when_text: Option<String>,
    /// Monitor only: next due time for one poll.
    pub resume_when: Option<SystemTime>,
    /// Monitor only: consecutive no-change polls (per-monitor counter).
    pub consecutive_no_change: u32,
    /// Monitor only (G-12): the external target this monitor observes
    /// (reference `monitor_target` — e.g. an endpoint / file / URL).
    pub monitor_target: Option<String>,
    /// Monitor only (G-12): the poll policy. reference vocabulary:
    /// `material_transition_only` | `read_only_observation_then_no_spend_if_unchanged`.
    pub monitor_policy: Option<String>,
    /// Monitor only (G-12): recurrence cadence — cadence class
    /// (`hourly` / `daily` / `weekly` / `once`) or an interval string
    /// (`15m`, `1h`, `2d`) consumed by the scheduler state machine
    /// (`rrule_for_cadence_class` / `monitor_cadence_secs`).
    pub monitor_cadence: Option<String>,
    /// Advancement only: which user gate ids block this todo.
    pub blocked_by_gate: Option<String>,
    /// Repair bookkeeping: failed attempts so far.
    pub failed_attempts: u32,
    /// Completion contract (LoopX): a completed advancement todo must declare
    /// either a successor or no-follow-up, else a succession replan
    /// obligation is raised.
    pub successor_ids: Vec<String>,
    pub no_follow_up: bool,
    /// Completion evidence (LoopX: recorded on the todo at complete time).
    pub evidence: Option<String>,
    /// Claim/lease (reference task-lease): which agent lane owns this slice and
    /// when the lease expires. Expired leases return the todo to the frontier.
    pub claimed_by: Option<String>,
    pub lease_expires_at: Option<u64>,
    /// Lease liveness: pid of the run process that holds the current claim
    /// (written at claim time). A dead holder's lease is reclaimed
    /// automatically by the claim path (`task_lease` + `try_claim_todo`),
    /// eliminating the manual `lease release` dance after killing a run.
    #[serde(default)]
    pub holder_pid: Option<u32>,
    /// Gate scope flags (LoopX: goal_bound / global_gate — set by bootstrap
    /// flows, not by plain todo add).
    pub goal_bound: bool,
    pub global_gate: bool,
    /// Audit timestamps (LoopX: updated_at / completed_at).
    pub updated_at: u64,
    pub completed_at: Option<u64>,
    /// Archive state (LoopX: "active" | "archived").
    pub archive_state: String,
    /// Associated repository (LoopX: task_repository).
    pub task_repository: Option<String>,
    /// Continuation policy (LoopX: independent_handoff | same_agent_non_delivery).
    pub continuation_policy: Option<String>,
    /// Required write scopes (LoopX: required_write_scope).
    pub required_write_scope: Vec<String>,
    /// Independent validator command (`todo add --verify "cmd"`): the kernel
    /// runs it in the goal cwd after each turn; exit 0 completes the todo
    /// (validated), non-zero keeps it open for bounded repair.
    pub validator: Option<String>,
    /// Completion acceptance contract (`todo add --acceptance "a,b"`): the
    /// completion evidence must contain EVERY comma-separated token
    /// (case-insensitive) — e.g. an external attempt id — else `todo complete`
    /// refuses unless `--force`. Encodes "done ≠ delivered" acceptance
    /// criteria as a hard check instead of a text convention.
    #[serde(default)]
    pub acceptance: Option<String>,
    /// How many failed validation attempts are tolerated before the kernel
    /// replans and surfaces to the user (default 3).
    #[serde(default = "default_max_validation_attempts")]
    pub max_validation_attempts: u32,
}

/// Default validator retry budget (the reference: default_max_validation_attempts).
pub fn default_max_validation_attempts() -> u32 {
    3
}

/// Role (LoopX: role — agent vs user; orthogonal to task_class).
/// Serialized with the EXACT reference values ("agent" / "user").
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TodoRole {
    #[serde(rename = "agent")]
    Agent,
    #[serde(rename = "user")]
    User,
}

impl Todo {
    pub fn advancement(id: &str, text: &str) -> Self {
        Self::new(id, text, TaskClass::Advancement)
    }

    pub fn user_gate(id: &str, question: &str, blocks: &[&str]) -> Self {
        let mut t = Self::new(id, question, TaskClass::UserGate);
        t.gate_question = Some(question.to_string());
        t.blocking(blocks)
    }

    pub fn monitor(id: &str, text: &str, due_in: Duration) -> Self {
        let mut t = Self::new(id, text, TaskClass::Monitor);
        t.resume_when = Some(SystemTime::now() + due_in);
        t
    }

    /// G-12: attach monitor metadata (target / policy / cadence) and derive
    /// the first due time from the cadence when no explicit delay is given.
    /// Cadence accepts a cadence class (`hourly`/`daily`/`weekly`/`once`) or
    /// an interval string (`15m`, `1h`, `2d`); `once` monitors get no
    /// recurrence rrule and keep the explicit `due_in` delay.
    pub fn monitor_with(
        id: &str,
        text: &str,
        target: Option<&str>,
        policy: Option<&str>,
        cadence: Option<&str>,
        due_in: Duration,
    ) -> Self {
        let mut t = Self::monitor(id, text, due_in);
        t.monitor_target = target.map(|s| s.to_string());
        t.monitor_policy = policy.map(|s| s.to_string());
        t.monitor_cadence = cadence.map(|s| s.to_string());
        if let Some(cad) = cadence {
            if let Some(secs) = crate::scheduler::state::monitor_cadence_secs(cad) {
                t.resume_when = Some(SystemTime::now() + Duration::from_secs(secs));
            }
        }
        t
    }

    /// Set the monitor target (G-12).
    pub fn with_monitor_target(mut self, target: &str) -> Self {
        self.monitor_target = Some(target.to_string());
        self
    }

    /// Set the monitor poll policy (G-12).
    pub fn with_monitor_policy(mut self, policy: &str) -> Self {
        self.monitor_policy = Some(policy.to_string());
        self
    }

    /// Set the monitor recurrence cadence (G-12).
    pub fn with_monitor_cadence(mut self, cadence: &str) -> Self {
        self.monitor_cadence = Some(cadence.to_string());
        self
    }

    pub fn blocker(id: &str, text: &str, blocks: &[&str]) -> Self {
        let t = Self::new(id, text, TaskClass::Blocker);
        t.blocking(blocks)
    }

    fn new(id: &str, text: &str, class: TaskClass) -> Self {
        let now = now_epoch();
        let role = match class {
            TaskClass::UserGate | TaskClass::UserAction => TodoRole::User,
            _ => TodoRole::Agent,
        };
        Self {
            id: id.to_string(),
            text: text.to_string(),
            title: text.to_string(),
            class,
            status: TodoStatus::Open,
            priority: Priority::default(),
            role,
            index: 0,
            action_kind: None,
            gate_question: None,
            decision: None,
            note: None,
            resume_when_text: None,
            resume_when: None,
            consecutive_no_change: 0,
            monitor_target: None,
            monitor_policy: None,
            monitor_cadence: None,
            blocked_by_gate: None,
            failed_attempts: 0,
            successor_ids: vec![],
            no_follow_up: false,
            evidence: None,
            claimed_by: None,
            lease_expires_at: None,
            holder_pid: None,
            goal_bound: false,
            global_gate: false,
            updated_at: now,
            completed_at: None,
            archive_state: "active".to_string(),
            task_repository: None,
            continuation_policy: None,
            required_write_scope: vec![],
            validator: None,
            max_validation_attempts: default_max_validation_attempts(),
            acceptance: None,
        }
    }

    /// Assign the goal-relative index (LoopX: index for ordering/identity).
    pub fn at_index(mut self, index: u32) -> Self {
        self.index = index;
        self
    }

    /// Set the short title (defaults to text).
    pub fn with_title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }

    /// Associated repository (LoopX: task_repository).
    pub fn with_repository(mut self, repo: &str) -> Self {
        self.task_repository = Some(repo.to_string());
        self
    }

    /// Continuation policy (LoopX: independent_handoff | same_agent_non_delivery).
    pub fn with_continuation_policy(mut self, policy: &str) -> Self {
        self.continuation_policy = Some(policy.to_string());
        self
    }

    /// Required write scopes (LoopX: required_write_scope).
    pub fn with_write_scopes(mut self, scopes: &[&str]) -> Self {
        self.required_write_scope = scopes.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Archive the todo (LoopX: archive_state "archived").
    pub fn archive(&mut self) {
        self.archive_state = "archived".to_string();
        self.updated_at = now_epoch();
    }

    /// Set priority (P0/P1/P2) — the decision kernel sorts by it.
    pub fn at_priority(mut self, p: Priority) -> Self {
        self.priority = p;
        self
    }

    /// Declare the action kind (LoopX: shell/github/...) for quota
    /// re-entry routing.
    pub fn with_action_kind(mut self, kind: &str) -> Self {
        self.action_kind = Some(kind.to_string());
        self
    }

    /// Attach a human note (LoopX: --note).
    pub fn with_note(mut self, note: &str) -> Self {
        self.note = Some(note.to_string());
        self
    }

    /// Declare gate scope (reference bootstrap sets goal_bound/global_gate).
    pub fn with_gate_scope(mut self, goal_bound: bool, global_gate: bool) -> Self {
        self.goal_bound = goal_bound;
        self.global_gate = global_gate;
        self
    }

    pub fn user_action(id: &str, text: &str) -> Self {
        Self::new(id, text, TaskClass::UserAction)
    }

    /// Deferred todo: not runnable until `resume_when` passes.
    pub fn deferred(id: &str, text: &str, resume_in: Duration) -> Self {
        let mut t = Self::new(id, text, TaskClass::Advancement);
        t.status = TodoStatus::Deferred;
        t.resume_when = Some(SystemTime::now() + resume_in);
        t
    }

    /// Deferred todos whose resume time passed become runnable again.
    pub fn is_due_deferred(&self, now: SystemTime) -> bool {
        self.status == TodoStatus::Deferred && self.resume_when.is_some_and(|d| d <= now)
    }

    /// Claim a slice: succeeds only when open AND (unclaimed OR the previous
    /// lease expired). Returns false if another agent holds a live lease
    /// (LoopX: claim is not ownership; lease is the bounded execution window).
    pub fn claim(&mut self, agent_id: &str, lease_secs: u64, now_epoch: u64) -> bool {
        if self.status != TodoStatus::Open {
            return false;
        }
        if let Some(expires) = self.lease_expires_at {
            if expires > now_epoch && self.claimed_by.as_deref() != Some(agent_id) {
                return false;
            }
        }
        self.claimed_by = Some(agent_id.to_string());
        self.lease_expires_at = Some(now_epoch + lease_secs);
        true
    }

    /// Whether this todo is currently claimed by another agent (live lease).
    pub fn claimed_by_other(&self, agent_id: Option<&str>, now_epoch: u64) -> bool {
        match (&self.claimed_by, self.lease_expires_at) {
            (Some(owner), Some(expires)) => expires > now_epoch && agent_id != Some(owner.as_str()),
            _ => false,
        }
    }

    pub fn blocking(mut self, gate_ids: &[&str]) -> Self {
        if !gate_ids.is_empty() {
            self.blocked_by_gate = Some(gate_ids.join(","));
        }
        self
    }

    /// Completion contract: mark done AND record the closure intent (LoopX:
    /// `todo complete --no-follow-up` / `--successor-todo-id`).
    pub fn complete(&mut self, no_follow_up: bool, successor_ids: Vec<String>) {
        let now = now_epoch();
        self.status = TodoStatus::Done;
        self.no_follow_up = no_follow_up;
        self.successor_ids = successor_ids;
        self.completed_at = Some(now);
        self.updated_at = now;
    }

    /// Record completion evidence (LoopX: `todo complete --evidence`).
    pub fn set_evidence(&mut self, evidence: &str) {
        self.evidence = Some(evidence.to_string());
        self.updated_at = now_epoch();
    }
}

// ── Task validation (the reference: future_loop_turn_task_validation_v0) ────────

pub const MAX_VALIDATION_SCHEMA_VERSION: &str = "future_loop_turn_task_validation_v0";

/// Independent task-validation status — mirrors the reference's turn
/// task-validation state machine: passed / progress / failed / inconclusive /
/// unavailable / not_required.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ValidationStatus {
    /// Independent validation satisfied — the turn is validated-ok.
    #[serde(rename = "passed")]
    Passed,
    /// Non-zero exit: made progress but did not pass — keep iterating (repair).
    #[serde(rename = "progress")]
    Progress,
    /// Validation failed — repair required.
    #[serde(rename = "failed")]
    Failed,
    /// Receipt is invalid / result ambiguous — treat as needing repair.
    #[serde(rename = "inconclusive")]
    Inconclusive,
    /// No validator attached to this todo — material results default to this.
    #[serde(rename = "unavailable")]
    Unavailable,
    /// Validation declared but not required for this todo (no validator set).
    #[serde(rename = "not_required")]
    NotRequired,
}

/// Recovery direction for a non-passed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RecoveryKind {
    #[serde(rename = "repair_required")]
    RepairRequired,
    #[serde(rename = "replan_required")]
    ReplanRequired,
}

/// One independent-validation receipt attached to a run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskValidation {
    pub schema_version: String,
    pub status: ValidationStatus,
    pub validator_kind: String,
    pub summary: String,
    pub recovery_kind: Option<RecoveryKind>,
    pub exit_code: Option<i32>,
    pub ok: bool,
}

/// Build a validation receipt (the reference receipt rules: failed /
/// inconclusive / unavailable require a recovery direction; successful
/// statuses must not carry one).
pub fn task_validation_receipt(
    status: ValidationStatus,
    validator_kind: &str,
    summary: &str,
    recovery_kind: Option<RecoveryKind>,
    exit_code: Option<i32>,
) -> TaskValidation {
    let ok = matches!(
        status,
        ValidationStatus::Passed | ValidationStatus::NotRequired
    );
    let recovery = match status {
        ValidationStatus::Failed
        | ValidationStatus::Inconclusive
        | ValidationStatus::Unavailable => {
            Some(recovery_kind.unwrap_or(RecoveryKind::RepairRequired))
        }
        _ => None,
    };
    TaskValidation {
        schema_version: MAX_VALIDATION_SCHEMA_VERSION.to_string(),
        status,
        validator_kind: validator_kind.to_string(),
        summary: summary.to_string(),
        recovery_kind: recovery,
        exit_code,
        ok,
    }
}

/// One recorded bounded turn (spend ledger entry).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunRecord {
    pub turn: u32,
    pub todo_id: String,
    pub run_id: String,
    /// Independent task-validation receipt (`todo add --verify`); absent on
    /// legacy ledger lines → read as not required.
    #[serde(default)]
    pub validation: Option<TaskValidation>,
    pub terminal_state: String,
    pub error: Option<String>,
    pub tokens_in_delta: u64,
    pub tokens_out_delta: u64,
    pub cost_delta: f64,
    pub tools: Vec<String>,
    pub evidence: String,
    pub recorded_at: u64,
    /// G-7 slot-accounting classification (`run` / `agent` / `heartbeat`),
    /// stamped at writeback time. Absent on legacy ledger lines → derived
    /// from `terminal_state` by [`crate::quota::slot_accounting`].
    pub spend_source: Option<String>,
}

/// An acceptance condition not yet satisfied by evidence. Terminal closure
/// requires every gap satisfied — "open_count == 0" alone is never done.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AcceptanceGap {
    pub id: String,
    pub description: String,
    pub satisfied: bool,
}

/// Execution profile (LoopX: execution_profile — cadence, spend rule, and
/// the outcome floor that rejects surface-only progress loops).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionProfile {
    pub cadence: String,
    pub spend_rule: String,
    /// Consecutive surface-only turns tolerated before the kernel requires a
    /// material outcome (0 = floor disabled).
    pub outcome_floor_streak_threshold: u32,
}

impl Default for ExecutionProfile {
    fn default() -> Self {
        Self {
            cadence: "bounded_progress_segment".to_string(),
            spend_rule: "spend_only_after_artifact_validation_writeback".to_string(),
            outcome_floor_streak_threshold: 0,
        }
    }
}

/// Replan acknowledgment (LoopX: autonomous replan ACK must carry a frontier
/// delta — vision patch / no-follow-up / successor — to clear the obligation).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReplanAck {
    pub recorded: bool,
    pub delta_kinds: Vec<String>,
    pub at: u64,
}

impl ReplanAck {
    pub fn has_frontier_delta(&self) -> bool {
        self.recorded
            && self.delta_kinds.iter().any(|k| {
                matches!(
                    k.as_str(),
                    "vision_patch" | "no_followup" | "successor_or_supersede" | "runnable_todo_set"
                )
            })
    }
}

/// Whether a delta kind changes the machine-visible frontier (LoopX:
/// repair_delta_kinds_have_frontier_delta).
pub fn delta_kind_changes_frontier(kind: &str) -> bool {
    matches!(
        kind,
        "vision_patch"
            | "no_followup"
            | "successor_or_supersede"
            | "runnable_todo_set"
            | "user_gate"
            | "blocker"
            | "monitor_target"
            | "active_state_next_action"
            | "goal_boundary_projection"
    )
}

/// P0-2: latest delivery-outcome state per work item — rebuilt by replay
/// from `DeliveryOutcomeRecorded` / `FollowthroughCreated` events. The
/// signal chain: `delivered` (pending verification) → `verified` / `failed`
/// / `rework` (terminal resolutions). Domain rules live in
/// [`crate::work_items::delivery_outcome`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeliveryState {
    pub todo_id: String,
    /// `delivered` (pending) | `verified` | `failed` | `rework`.
    pub outcome: String,
    pub note: Option<String>,
    /// Run-turn counter at delivery time (0 = recorded without run context).
    pub delivered_turn: u32,
    /// Follow-up todo auto-created by outcome_followthrough (P0-2②) — the
    /// dedupe stamp so the follow-through fires exactly once per cycle.
    pub followthrough_todo_id: Option<String>,
    /// Per-todo outcome sequence number of the latest event (audit ordering;
    /// also what makes each cycle's events content-distinct).
    pub seq: u32,
    pub updated_at: u64,
}

/// Agent peer profile (LoopX: coordination.agent_profiles — a registered
/// peer plus the capabilities it declares, kept as descriptive metadata).
/// `workspaces` is the P0-1 workspace guard declaration: the normalized
/// absolute path set this agent writes
/// into (empty = undeclared → the guard is fail-open for this agent).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub workspaces: Vec<String>,
}

impl AgentProfile {
    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }
}

/// Goal authority (LoopX: authority / boundary_authority — separates "can
/// see / propose / execute / commit" and declares write scope + approval
/// gates for irreversible actions).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Authority {
    /// Directories the executor may write inside (empty = no writes).
    pub write_scope: Vec<String>,
    /// Action kinds that require explicit approval (a gate is raised):
    /// e.g. "publish", "production-action", "merge".
    pub requires_approval: Vec<String>,
}

impl Default for Authority {
    fn default() -> Self {
        Self {
            write_scope: vec![],
            requires_approval: vec![
                "publish".to_string(),
                "production-action".to_string(),
                "merge".to_string(),
            ],
        }
    }
}

impl Authority {
    /// Whether an action kind needs a user gate before the agent may execute
    /// it (LoopX: scoped operator gate — approval binds to the exact action).
    pub fn approval_required_for(&self, action_kind: &str) -> bool {
        self.requires_approval.iter().any(|k| k == action_kind)
    }
}

/// Public/private boundary scan: flags evidence text that may leak private
/// material into public evidence (LoopX: public-safe evidence).
/// Prototype rule: absolute HOME paths and credential-ish tokens.
pub fn boundary_scan_leaks(text: &str) -> Vec<String> {
    let mut leaks = vec![];
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && text.contains(&home) {
        leaks.push(format!("absolute home path leak: {home}"));
    }
    for marker in [".ssh", "auth.json", "token=", "api_key"] {
        if text.contains(marker) {
            leaks.push(format!("sensitive marker `{marker}` in evidence"));
        }
    }
    leaks
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Goal {
    pub goal_id: String,
    pub objective: String,
    pub cwd: String,
    /// Goal lifecycle status: "active" | "cancelled" (goal cancel keeps state,
    /// stops automation; delete removes the registry entry entirely).
    #[serde(default = "default_goal_status")]
    pub status: String,
    pub acceptance: Vec<AcceptanceGap>,
    pub todos: Vec<Todo>,
    pub history: Vec<RunRecord>,
    /// Active-state "Next Action" text (must stay in sync with the todo
    /// frontier or the projection-gap check flags drift).
    pub next_action: Option<String>,
    /// Registered agent peers (LoopX: coordination.registered_agents).
    pub registered_agents: Vec<String>,
    /// Registered peers with declared capabilities (descriptive metadata;
    /// `workspaces` feeds the workspace guard).
    pub agent_profiles: Vec<AgentProfile>,
    pub execution_profile: ExecutionProfile,
    /// Consecutive turns without a material outcome (outcome floor).
    pub outcome_streak: u32,
    pub replan_ack: Option<ReplanAck>,
    pub authority: Authority,
    pub created_at: u64,
    /// Goal-relative todo index counter (LoopX: index for ordering).
    pub next_index: u32,
    /// G-3: quota slots spent as recorded by QuotaSpent events (replay
    /// projection — the authoritative spend ledger stays runs.jsonl).
    pub quota_spent_slots: u32,
    /// P0-2: per-work-item delivery outcome states (latest event wins —
    /// folded from DeliveryOutcomeRecorded / FollowthroughCreated).
    pub delivery_states: Vec<DeliveryState>,
    /// P1-2②: replay-time freshness stamp of the event ledger this state
    /// was rebuilt from (in-memory only — never persisted; `None` for
    /// hand-built goals). The decision kernel copies it onto every
    /// `ShouldRunPacket` as `decision_freshness`.
    #[serde(skip)]
    pub decision_freshness: Option<DecisionFreshness>,
    /// P1-3①: latest scheduler heartbeat per agent (epoch secs), folded
    /// from `SchedulerTicked`. The liveness check compares now against this.
    #[serde(default)]
    pub scheduler_heartbeats: std::collections::BTreeMap<String, u64>,
    /// P1-3①: automation liveness breach alerts, folded from
    /// `AutomationLivenessAlert` (append-only; recovery is derived by
    /// comparing against `scheduler_heartbeats`).
    #[serde(default)]
    pub liveness_alerts: Vec<LivenessAlert>,
    /// O3: idle-turn no-progress records, folded from `TurnNoProgress`
    /// (append-only; detection + bookkeeping, no auto-injection).
    #[serde(default)]
    pub turn_no_progress: Vec<TurnNoProgressRecord>,
    /// G13 ①: timestamps of frontier-changing events (todo added/completed/
    /// superseded, gate resolved, frontier-delta replan ack, todo archived),
    /// folded during replay — the reset markers for outcome-continuity
    /// segments ([`crate::decision::goal_frontier::outcome_continuity`]).
    #[serde(default)]
    pub frontier_change_ts: Vec<u64>,
    /// G13 ③: goal-level bounded semantic event history (recent
    /// [`crate::decision::goal_frontier::semantic_history::SEMANTIC_HISTORY_CAP`]
    /// summaries, oldest dropped) — folded from the event ledger during
    /// replay; a standalone goal-level projection (public-safe summaries).
    #[serde(default)]
    pub semantic_history: Vec<crate::decision::goal_frontier::semantic_history::SemanticEvent>,
    /// G13 ②: explicit replan rule set (folded from `ReplanRuleSetUpdated`;
    /// latest wins). `None` → the default rule set applies.
    #[serde(default)]
    pub replan_rule_set: Option<crate::decision::goal_frontier::replan_rules::ReplanRuleSet>,
}

/// P1-3①: one automation-liveness breach alert (LoopX automation_liveness):
/// the scheduler heartbeat for `agent_id` went silent past the threshold.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LivenessAlert {
    pub agent_id: String,
    pub elapsed_secs: u64,
    pub threshold_secs: u64,
    /// 1-based alert ordinal for this (goal, agent) scope.
    pub consecutive: u32,
    pub ts: u64,
}

/// O3: one idle-turn (no-progress) breach — the turn ended with no write-class
/// tool (write/edit/shell) started inside the no-progress window. Folded from
/// `TurnNoProgress` events; the orchestrator nudges via the todo update
/// steering channel (the loop itself never auto-injects).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TurnNoProgressRecord {
    pub goal_id: String,
    pub todo_id: String,
    pub agent_id: Option<String>,
    /// Seconds since the last write-class tool start (turn start when none).
    pub idle_secs: u64,
    /// Total tool calls observed this turn (all classes).
    pub tool_calls_total: u32,
    pub ts: u64,
}

/// O3: default idle-turn no-progress window (15 minutes). A turn that ends
/// without any write-class tool (write/edit/shell) starting inside this
/// window is ledgered as `TurnNoProgress`.
pub const TURN_NO_PROGRESS_IDLE_SECS_DEFAULT: u64 = 15 * 60;

/// O3: effective no-progress window (secs). The `FUTURE_LOOP_NO_PROGRESS_SECS`
/// env var shrinks it in tests (mirrors the FUTURE_LOOP_AGENT_ADDR hook);
/// invalid or non-positive values fall back to the 15-minute default.
pub fn no_progress_idle_secs() -> u64 {
    std::env::var("FUTURE_LOOP_NO_PROGRESS_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(TURN_NO_PROGRESS_IDLE_SECS_DEFAULT)
}

/// P1-2②: freshness stamp of the event ledger a decision was compiled
/// against (LoopX `runtime/decision_freshness.py` — decisions annotate the
/// max seq / timestamp of the state they read, so a consumer can tell a
/// stale decision from a fresh one without re-running the kernel).
/// Stamped by `Store::replay`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DecisionFreshness {
    /// Ledger length at read time = the 1-based sequence number of the
    /// newest event the decision state was rebuilt from.
    pub events_max_seq: u64,
    /// Newest event timestamp (epoch secs) among the events read (`None`
    /// when the ledger was empty).
    #[serde(default)]
    pub events_max_ts: Option<u64>,
    /// When the state was replayed (epoch secs).
    pub read_at: u64,
}

/// Default goal lifecycle status.
pub fn default_goal_status() -> String {
    "active".to_string()
}

impl Goal {
    pub fn new(goal_id: &str, objective: &str, cwd: &str) -> Self {
        Self {
            goal_id: goal_id.to_string(),
            objective: objective.to_string(),
            cwd: cwd.to_string(),
            status: "active".to_string(),
            acceptance: vec![],
            todos: vec![],
            history: vec![],
            next_action: None,
            registered_agents: vec![],
            agent_profiles: vec![],
            execution_profile: ExecutionProfile::default(),
            outcome_streak: 0,
            replan_ack: None,
            authority: Authority::default(),
            created_at: now_epoch(),
            next_index: 0,
            quota_spent_slots: 0,
            delivery_states: vec![],
            decision_freshness: None,
            scheduler_heartbeats: std::collections::BTreeMap::new(),
            liveness_alerts: vec![],
            turn_no_progress: vec![],
            frontier_change_ts: vec![],
            semantic_history: vec![],
            replan_rule_set: None,
        }
    }

    pub fn with_acceptance(mut self, gaps: Vec<(&str, &str)>) -> Self {
        self.acceptance = gaps
            .into_iter()
            .map(|(id, description)| AcceptanceGap {
                id: id.to_string(),
                description: description.to_string(),
                satisfied: false,
            })
            .collect();
        self
    }

    pub fn add(&mut self, todo: Todo) {
        self.next_index += 1;
        let mut todo = todo;
        if todo.index == 0 {
            todo.index = self.next_index;
        }
        self.todos.push(todo);
    }

    /// Archive a todo (LoopX: archive_state "archived").
    pub fn archive_todo(&mut self, todo_id: &str) {
        if let Some(t) = self.todo_mut(todo_id) {
            t.archive();
        }
    }

    pub fn todo(&self, id: &str) -> Option<&Todo> {
        self.todos.iter().find(|t| t.id == id)
    }

    pub fn todo_mut(&mut self, id: &str) -> Option<&mut Todo> {
        self.todos.iter_mut().find(|t| t.id == id)
    }

    pub fn open_of(&self, class: TaskClass) -> impl Iterator<Item = &Todo> {
        self.todos
            .iter()
            .filter(move |t| t.class == class && t.status == TodoStatus::Open)
    }

    pub fn open_gates(&self) -> impl Iterator<Item = &Todo> {
        self.open_of(TaskClass::UserGate)
    }

    /// Open blocking sources: user gates AND external blockers. Both gate
    /// dependent todos (LoopX: blocker task class).
    pub fn open_blocking_sources(&self) -> impl Iterator<Item = &Todo> {
        self.todos.iter().filter(|t| {
            (t.class == TaskClass::UserGate || t.class == TaskClass::Blocker)
                && t.status == TodoStatus::Open
        })
    }

    pub fn open_monitors(&self) -> impl Iterator<Item = &Todo> {
        self.open_of(TaskClass::Monitor)
    }

    /// Open advancement todos NOT blocked by any open gate.
    pub fn runnable_advancement(&self) -> impl Iterator<Item = &Todo> {
        self.runnable_advancement_for(None)
    }

    /// Whether `t` is held out of the frontier by an unresolved predecessor.
    ///
    /// `blocked_by_gate` carries predecessor ids (comma-joined). An id blocks
    /// `t` when it names:
    /// - an OPEN gate/blocker (existing behavior: gates freeze linked work
    ///   until resolved), or
    /// - a plain todo that has not reached Done/Superseded (todo→todo
    ///   dependency — `--blocks` on `todo add`/`todo update`). Previously
    ///   these ids were only ever compared against the open-gate set, so
    ///   todo→todo blocks silently never took effect; they are now enforced.
    ///
    /// Unknown predecessor ids do NOT block here (liveness); the
    /// `task-graph` projection fails closed on them instead.
    pub fn is_blocked(&self, t: &Todo) -> bool {
        let Some(ids) = t.blocked_by_gate.as_deref() else {
            return false;
        };
        ids.split(',').any(|gid| {
            let gid = gid.trim();
            if gid.is_empty() {
                return false;
            }
            match self.todo(gid) {
                Some(pred) => match pred.class {
                    TaskClass::UserGate | TaskClass::Blocker => pred.status == TodoStatus::Open,
                    _ => !matches!(pred.status, TodoStatus::Done | TodoStatus::Superseded),
                },
                None => false,
            }
        })
    }

    /// Identity-scoped frontier (LoopX: registered peers see their own slice;
    /// unclaimed work wakes every eligible peer; a live lease held by another
    /// agent hides the todo from this frontier).
    pub fn runnable_advancement_for<'a>(
        &'a self,
        agent_id: Option<&'a str>,
    ) -> impl Iterator<Item = &'a Todo> + 'a {
        let now_sys = SystemTime::now();
        let now = now_epoch();
        self.todos.iter().filter(move |t| {
            // Open OR due-deferred (returns to the frontier) advancement.
            (t.class == TaskClass::Advancement
                && (t.status == TodoStatus::Open || t.is_due_deferred(now_sys)))
                && !t.claimed_by_other(agent_id, now)
                && !self.is_blocked(t)
        })
    }

    /// Whether `agent_id` is a registered peer of this goal (LoopX:
    /// coordination.registered_agents — the precondition for quota --agent-id).
    pub fn is_registered_agent(&self, agent_id: Option<&str>) -> bool {
        match agent_id {
            None => true, // anonymous path allowed
            Some(id) => self.registered_agents.iter().any(|a| a == id),
        }
    }

    /// The capabilities an agent declared for this goal (empty = none).
    pub fn agent_capabilities(&self, agent_id: &str) -> Vec<String> {
        self.agent_profiles
            .iter()
            .find(|p| p.id == agent_id)
            .map(|p| p.capabilities.clone())
            .unwrap_or_default()
    }

    /// Register an agent with optional declared capabilities.
    pub fn register_agent(&mut self, agent_id: &str, capabilities: Vec<String>) {
        if !self.registered_agents.iter().any(|a| a == agent_id) {
            self.registered_agents.push(agent_id.to_string());
        }
        self.agent_profiles.retain(|p| p.id != agent_id);
        self.agent_profiles.push(AgentProfile {
            id: agent_id.to_string(),
            capabilities,
            workspaces: vec![],
        });
    }

    /// The latest delivery-outcome state for a work item (P0-2).
    pub fn delivery_state(&self, todo_id: &str) -> Option<&DeliveryState> {
        self.delivery_states.iter().find(|d| d.todo_id == todo_id)
    }

    /// Fold a `DeliveryOutcomeRecorded` event into the read model (latest
    /// wins). A fresh `delivered` resets the cycle: the turn stamp moves and
    /// the follow-through stamp clears, so a re-delivered item can fire its
    /// own follow-through later. Resolutions keep the original turn stamp.
    pub fn apply_delivery_outcome(
        &mut self,
        todo_id: &str,
        outcome: &str,
        note: Option<String>,
        delivered_turn: u32,
        seq: u32,
        ts: u64,
    ) {
        let fresh_delivery = outcome == crate::work_items::delivery_outcome::OUTCOME_DELIVERED;
        if let Some(d) = self
            .delivery_states
            .iter_mut()
            .find(|d| d.todo_id == todo_id)
        {
            d.outcome = outcome.to_string();
            if note.is_some() {
                d.note = note;
            }
            if fresh_delivery {
                d.delivered_turn = delivered_turn;
                d.followthrough_todo_id = None;
            }
            d.seq = seq;
            d.updated_at = ts;
        } else {
            self.delivery_states.push(DeliveryState {
                todo_id: todo_id.to_string(),
                outcome: outcome.to_string(),
                note,
                delivered_turn: if fresh_delivery { delivered_turn } else { 0 },
                followthrough_todo_id: None,
                seq,
                updated_at: ts,
            });
        }
    }

    /// Fold a `FollowthroughCreated` event (P0-2②) — stamps the auto-created
    /// follow-up todo on the pending delivery so it fires exactly once.
    pub fn apply_followthrough(&mut self, source_todo_id: &str, followup_todo_id: &str, ts: u64) {
        if let Some(d) = self
            .delivery_states
            .iter_mut()
            .find(|d| d.todo_id == source_todo_id)
        {
            d.followthrough_todo_id = Some(followup_todo_id.to_string());
            d.updated_at = ts;
        }
    }

    /// Done advancement todos that declared NO successor and NO no-follow-up
    /// (LoopX: `completed_advancement_without_successor` — completion must
    /// declare closure intent, else a succession replan obligation is raised).
    pub fn completed_without_closure_intent(&self) -> Vec<&Todo> {
        self.todos
            .iter()
            .filter(|t| {
                t.class == TaskClass::Advancement
                    && t.status == TodoStatus::Done
                    && t.successor_ids.is_empty()
                    && !t.no_follow_up
            })
            .collect()
    }

    pub fn unsatisfied_gaps(&self) -> Vec<&AcceptanceGap> {
        self.acceptance.iter().filter(|g| !g.satisfied).collect()
    }

    pub fn satisfy_gap(&mut self, id: &str) {
        if let Some(g) = self.acceptance.iter_mut().find(|g| g.id == id) {
            g.satisfied = true;
        }
    }

    pub fn supersede(&mut self, todo_id: &str) {
        if let Some(t) = self.todo_mut(todo_id) {
            t.status = TodoStatus::Superseded;
        }
    }

    /// Terminal closure — LoopX: NOT derivable from open_count alone. Every
    /// todo is done OR superseded AND no pending (open or not-yet-due
    /// deferred) work AND no acceptance gap AND every done advancement
    /// declared closure intent (successor or no-follow-up).
    pub fn is_terminal(&self) -> bool {
        let now = SystemTime::now();
        self.todos.iter().all(|t| {
            t.status != TodoStatus::Open
                && t.status != TodoStatus::Blocked
                && (t.status != TodoStatus::Deferred || t.is_due_deferred(now))
        }) && self.unsatisfied_gaps().is_empty()
            && self.completed_without_closure_intent().is_empty()
    }

    /// The validated terminal mode: `Some(())` only when closure is derived
    /// from complete sources (LoopX: `terminal_no_followup` from
    /// validated_goal_closure, never hand-written).
    pub fn terminal_closure(&self) -> Option<()> {
        self.is_terminal().then_some(())
    }

    /// todo_summary aggregation (LoopX: todo_summary_v0) — per-role counts,
    /// source proof, and terminal closure proof. The summary is a PROJECTION
    /// derived from canonical state, never a second source of truth.
    pub fn todo_summary(&self) -> TodoSummary {
        let user_open = self
            .todos
            .iter()
            .filter(|t| t.role == TodoRole::User && t.status == TodoStatus::Open)
            .count();
        let user_done = self
            .todos
            .iter()
            .filter(|t| t.role == TodoRole::User && t.status == TodoStatus::Done)
            .count();
        let agent_open = self
            .todos
            .iter()
            .filter(|t| t.role == TodoRole::Agent && t.status == TodoStatus::Open)
            .count();
        let agent_done = self
            .todos
            .iter()
            .filter(|t| t.role == TodoRole::Agent && t.status == TodoStatus::Done)
            .count();
        let monitor_open = self
            .todos
            .iter()
            .filter(|t| t.class == TaskClass::Monitor && t.status == TodoStatus::Open)
            .count();
        let no_followup_count = self.todos.iter().filter(|t| t.no_follow_up).count();
        let closure_proof = TerminalClosureProof {
            all_todos_done: if self.is_terminal() {
                true
            } else {
                self.todos.iter().all(|t| t.status == TodoStatus::Done)
            },
            derived: true,
            no_followup_count: no_followup_count as u32,
            monitor_open_count: monitor_open as u32,
            successor_gap_count: self.completed_without_closure_intent().len() as u32,
            schema_version: "todo_terminal_closure_proof_v0".to_string(),
        };
        TodoSummary {
            schema_version: "todo_summary_v0".to_string(),
            user_open,
            user_done,
            agent_open,
            agent_done,
            monitor_open,
            source_proof: TodoSourceProof {
                derived: true,
                item_count: self.todos.len() as u32,
                schema_version: "todo_source_proof_v0".to_string(),
            },
            terminal_closure_proof: closure_proof,
        }
    }
}

/// todo_summary_v0 projection (LoopX).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TodoSummary {
    pub schema_version: String,
    pub user_open: usize,
    pub user_done: usize,
    pub agent_open: usize,
    pub agent_done: usize,
    pub monitor_open: usize,
    pub source_proof: TodoSourceProof,
    pub terminal_closure_proof: TerminalClosureProof,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TodoSourceProof {
    pub derived: bool,
    pub item_count: u32,
    pub schema_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TerminalClosureProof {
    pub all_todos_done: bool,
    pub derived: bool,
    pub no_followup_count: u32,
    pub monitor_open_count: u32,
    pub successor_gap_count: u32,
    pub schema_version: String,
}

pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

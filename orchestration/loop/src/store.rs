// P1-1 + G-16 + G12 projection-only events: read from
//!
//!   registry.json    — known goals (id, objective, cwd, status, authority)
//!   <goal>/events.jsonl   — append-only event ledger (the canonical truth)
//!   <goal>/runs.jsonl     — run history (spend ledger)
//!   backups/<ts>/    — point-in-time snapshots (loopx backup / restore)
//!
//! Active state is a READ MODEL rebuilt by replaying the event ledger; the
//! projection-gap check detects when the active-state Next Action drifts from
//! the todo frontier (LoopX: state_projection_gap → self-repair). Appends are
//! guarded by an advisory file lock so concurrent processes cannot interleave
//! a line (LoopX: file_lock).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::state::{Goal, RunRecord, Todo};

const REGISTRY_FILE: &str = "registry.json";
const EVENTS_FILE: &str = "events.jsonl";
const RUNS_FILE: &str = "runs.jsonl";
const NEXT_ACTION_FILE: &str = "next_action.txt";
const BACKUPS_DIR: &str = "backups";
const SCHEMA_FILE: &str = "schema.json";
/// Per-goal ledger read diagnostics (O1): written when the read path skips
/// lines carrying event kinds this binary does not know (a newer binary
/// wrote the ledger). Surfaced by `status` / `diagnose` / `store verify`.
const READ_DIAGNOSTICS_FILE: &str = "read_diagnostics.json";

// ── Event ledger ───────────────────────────────────────────────────────────

/// Current event-store schema version (G-6). Bumped whenever the event
/// surface changes shape; legacy ledgers carry `future_loop_event_store_v0` and
/// are migrated through [`crate::migration::migration_steps`] on read.
pub const EVENT_STORE_SCHEMA_VERSION: &str = "future_loop_event_store_v1";
/// The pre-G-3 ledger schema (plain `{"kind": ...}` lines, no event ids).
pub const LEGACY_EVENT_STORE_SCHEMA_VERSION: &str = "future_loop_event_store_v0";
/// Producer tag for ordinary kernel appends (reference default producer).
pub const DEFAULT_EVENT_PRODUCER: &str = "loopx.event_sourced_state";

/// The persisted ledger line (G-3): the event plus its content-derived id
/// and optional provenance. `#[serde(flatten)]` keeps the on-disk shape
/// `{"event_id": "...", "kind": "...", ...}` — the `kind` tag merges into
/// the same object, so legacy readers that ignore unknown fields still parse
/// new lines and new readers default `event_id` for legacy lines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEvent {
    #[serde(default)]
    pub event_id: String,
    #[serde(default)]
    pub producer: Option<String>,
    /// G-3 backfill provenance (source file / section / line).
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub source_section: Option<String>,
    #[serde(default)]
    pub source_line: Option<u64>,
    /// G-4 privacy level of the event payload (`public_safe` /
    /// `local_private` / `private_pointer`).
    #[serde(default)]
    pub privacy: Option<String>,
    /// Reserved fencing token (schema reservation only — NOT enforced): a
    /// monotonically increasing per-ledger token a future fencing authority
    /// will issue to writers so a stale/zombie writer can be fenced off in a
    /// multi-replica deployment. Kernel appends never populate it and no
    /// validation rejects missing/regressing tokens yet; old ledger lines
    /// read as `None`, and `None` is omitted from serialization so the
    /// on-disk line shape is byte-identical to pre-reservation ledgers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fencing_token: Option<u64>,
    #[serde(flatten)]
    pub event: Event,
}

impl StoredEvent {
    /// The event id, falling back to a content-derived id for legacy lines
    /// that predate G-3 (stable across replays once migrated).
    pub fn effective_id(&self) -> String {
        if self.event_id.is_empty() {
            derive_event_id(&self.event)
        } else {
            self.event_id.clone()
        }
    }
}

/// FNV-1a 64-bit content digest (deterministic across processes and Rust
/// versions — the identity anchor for event ids). 16 hex chars.
pub fn content_digest(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Content-derived event id (G-3): `evt-<digest16>` over the canonical event
/// JSON — identical events produce identical ids (idempotency), differing
/// content under the same id is a conflict (StateEventConflictError).
pub fn derive_event_id(event: &Event) -> String {
    let json = serde_json::to_string(event).unwrap_or_default();
    format!("evt-{}", &content_digest(json.as_bytes())[..16])
}

/// Same derivation over a raw ledger line (used by the migration path where
/// the line has not been parsed into a typed [`Event`] yet).
pub fn derive_event_id_from_value(value: &serde_json::Value) -> String {
    let json = serde_json::to_string(value).unwrap_or_default();
    format!("evt-{}", &content_digest(json.as_bytes())[..16])
}

/// Strip the envelope fields so fingerprints compare event CONTENT only
/// (idempotent re-append must ignore producer/source/privacy differences).
fn event_part(value: &serde_json::Value) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    if let Some(obj) = value.as_object() {
        for (key, value) in obj {
            if !matches!(
                key.as_str(),
                "event_id"
                    | "producer"
                    | "source_ref"
                    | "source_section"
                    | "source_line"
                    | "privacy"
                    // Fencing tokens are writer metadata, not event content:
                    // a re-append that differs only in the token stays an
                    // idempotent no-op (same envelope rule as `producer`).
                    | "fencing_token"
            ) {
                map.insert(key.clone(), value.clone());
            }
        }
    }
    serde_json::Value::Object(map)
}

/// Fingerprint of an event line (event content only).
pub fn event_fingerprint(value: &serde_json::Value) -> String {
    serde_json::to_string(&event_part(value)).unwrap_or_default()
}

// Variants carry whole todos/run records; the ledger is append-only JSONL so
// the size difference is irrelevant at runtime.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    GoalStarted {
        goal_id: String,
        ts: u64,
    },
    TodoAdded {
        goal_id: String,
        todo: Todo,
        ts: u64,
    },
    TodoCompleted {
        goal_id: String,
        todo_id: String,
        no_follow_up: bool,
        successor_ids: Vec<String>,
        evidence: Option<String>,
        ts: u64,
    },
    TodoSuperseded {
        goal_id: String,
        todo_id: String,
        ts: u64,
    },
    /// Field-level todo update (the reference: `todo update` — text / status /
    /// evidence / note / priority / resume-when / blocks). Done must go through
    /// `todo complete` (closure-intent contract).
    TodoUpdated {
        goal_id: String,
        todo_id: String,
        text: Option<String>,
        status: Option<String>,
        evidence: Option<String>,
        note: Option<String>,
        priority: Option<String>,
        resume_when: Option<String>,
        /// Replace the blocking set (`--blocks a,b`); `Some([])` clears it.
        /// Absent (`None`) leaves the current blocking set untouched. Old
        /// events serialized without this field deserialize as `None`.
        #[serde(default)]
        blocks: Option<Vec<String>>,
        /// Completion acceptance contract (`--acceptance "a,b"`); `Some("")`
        /// clears it, absent leaves it untouched. See `Todo::acceptance`.
        #[serde(default)]
        acceptance: Option<String>,
        ts: u64,
    },
    /// Stop automation while retaining state (the reference: goal cancel).
    GoalCancelled {
        goal_id: String,
        reason: String,
        ts: u64,
    },
    GateResolved {
        goal_id: String,
        todo_id: String,
        decision: String,
        note: Option<String>,
        ts: u64,
    },
    GapSatisfied {
        goal_id: String,
        gap_id: String,
        ts: u64,
    },
    RunRecorded {
        goal_id: String,
        record: RunRecord,
        ts: u64,
    },
    /// Claim/lease: an agent owns this slice until lease_expires_at.
    TodoClaimed {
        goal_id: String,
        todo_id: String,
        agent_id: String,
        lease_expires_at: u64,
        /// Pid of the claiming run process (lease liveness — see Todo.holder_pid).
        holder_pid: Option<u32>,
        ts: u64,
    },
    /// Register an agent peer for the goal (LoopX: coordination.registered_agents).
    /// `workspaces` is the P0-1 workspace-guard declaration (normalized
    /// absolute paths the agent writes into; empty = undeclared/fail-open).
    /// Old events without the field deserialize as empty.
    AgentRegistered {
        goal_id: String,
        agent_id: String,
        #[serde(default)]
        workspaces: Vec<String>,
        ts: u64,
    },
    /// Onboard an agent with declared capabilities (descriptive metadata —
    /// kept on the event for recipe/agent-list surfaces; the runnability
    /// gate is gone). Old events without the field deserialize as empty.
    AgentOnboarded {
        goal_id: String,
        agent_id: String,
        #[serde(default)]
        capabilities: Vec<String>,
        #[serde(default)]
        workspaces: Vec<String>,
        ts: u64,
    },
    /// P0-1: advisory write-lock record — an agent with declared workspaces
    /// claimed a todo and now occupies its workspace set. `forced` marks a
    /// claim that overrode a live workspace conflict via `--force`.
    /// Projection-only: occupancy is derived from profiles + live leases;
    /// this event is the audit trail (agent list / history / todo-event).
    WorkspaceLockAcquired {
        goal_id: String,
        agent_id: String,
        todo_id: String,
        paths: Vec<String>,
        forced: bool,
        ts: u64,
    },
    /// Replan acknowledgment with frontier-delta kinds (vision patch /
    /// no-follow-up / successor…). Clearing a replan obligation requires a
    /// frontier-changing delta (LoopX: replan ACK contract).
    ReplanAcked {
        goal_id: String,
        delta_kinds: Vec<String>,
        ts: u64,
    },
    /// Execution profile update (outcome floor threshold, …).
    ProfileSet {
        goal_id: String,
        outcome_floor_streak_threshold: u32,
        ts: u64,
    },
    /// Authority declaration update (write scope + approval gates).
    AuthoritySet {
        goal_id: String,
        write_scope: Vec<String>,
        requires_approval: Vec<String>,
        ts: u64,
    },
    /// Archive a todo (LoopX: archive_state "archived").
    TodoArchived {
        goal_id: String,
        todo_id: String,
        ts: u64,
    },
    /// G-8: a monitor poll was executed (decision-path writeback). `result`
    /// is `changed` (material transition — monitor closes) or `no_change`
    /// (unchanged — consecutive counter advances, no spend); `no_change_count`
    /// is the resulting counter so replay is exact and idempotent.
    MonitorPolled {
        goal_id: String,
        todo_id: String,
        result: String,
        no_change_count: u32,
        ts: u64,
    },
    /// O3: idle-turn detection — a turn ended with no write-class tool
    /// (write/edit/shell) started inside the no-progress window. Detection +
    /// bookkeeping only (no auto-injection): orchestrators nudge via a `todo
    /// update` picked up at the next turn. `agent_id` is `None` for anonymous runs.
    TurnNoProgress {
        goal_id: String,
        todo_id: String,
        #[serde(default)]
        agent_id: Option<String>,
        /// Seconds since the last write-class tool start (turn start when none).
        idle_secs: u64,
        /// Total tool calls observed this turn (all classes).
        tool_calls_total: u32,
        ts: u64,
    },
    /// G-3: a quota slot was spent (reference QUOTA_SPENT). Recorded alongside
    /// the run ledger; `source` mirrors the slot-accounting spend source
    /// (`run` / `agent` / `heartbeat`) and `slots` the count spent (1 per
    /// bounded turn; monitor no-change polls never spend).
    QuotaSpent {
        goal_id: String,
        run_id: String,
        todo_id: String,
        source: String,
        slots: u32,
        ts: u64,
    },
    /// G-3: evidence attached to a todo independently of completion (LoopX
    /// EVIDENCE_ATTACHED).
    EvidenceAttached {
        goal_id: String,
        todo_id: String,
        evidence: String,
        ts: u64,
    },
    /// G-13: lease renewed by the current owner (extends `lease_expires_at`).
    TodoRenewed {
        goal_id: String,
        todo_id: String,
        agent_id: String,
        lease_expires_at: u64,
        ts: u64,
    },
    /// G-13: lease released early by its owner (claim cleared).
    TodoReleased {
        goal_id: String,
        todo_id: String,
        agent_id: String,
        ts: u64,
    },
    /// G-13: lease expired without renewal (claim cleared; a steal re-claims
    /// the slice via a fresh TodoClaimed).
    TodoExpired {
        goal_id: String,
        todo_id: String,
        ts: u64,
    },
    /// P0-2①: post-delivery outcome signal — `delivered` (pending
    /// verification) → `verified` / `failed` / `rework` (the three terminal
    /// resolutions). Recorded automatically when an advancement todo
    /// completes, and manually via `delivery record`. `delivered_turn` is the
    /// run-turn counter at delivery time (0 = recorded without run context).
    DeliveryOutcomeRecorded {
        goal_id: String,
        todo_id: String,
        outcome: String,
        note: Option<String>,
        delivered_turn: u32,
        /// Per-todo outcome sequence number (1-based, from the read model at
        /// append time). Distinguishes cycles: a re-delivery after
        /// failed/rework would otherwise content-collide with the earlier
        /// `delivered` event (same todo/turn/note within one second) and be
        /// swallowed by the G-3 idempotent-append dedupe.
        #[serde(default)]
        seq: u32,
        ts: u64,
    },
    /// P0-2②: outcome_followthrough fired — a delivered-but-unverified work
    /// item aged past the turn threshold, so a follow-up todo was
    /// auto-created (the followup itself is the TodoAdded event; this event
    /// stamps the source delivery so the follow-through fires exactly once).
    FollowthroughCreated {
        goal_id: String,
        source_todo_id: String,
        followup_todo_id: String,
        turns_overdue: u32,
        ts: u64,
    },
    DecisionSummaryRecorded {
        goal_id: String,
        summary: crate::quota::decision_summary::DecisionSummary,
        ts: u64,
    },
    /// P1-1③: heartbeat receipt — the per-turn heartbeat packet was issued
    /// to a host executor with this decision (LoopX `heartbeat_receipt.py`).
    /// `turn_instance_id` anchors the receipt the way LoopX keys on
    /// (goal, agent, run/turn instance); `todo_id` is the selected todo when
    /// the turn had one. Projection-only (audit trail).
    HeartbeatReceiptRecorded {
        goal_id: String,
        #[serde(default)]
        agent_id: Option<String>,
        turn_instance_id: String,
        #[serde(default)]
        todo_id: Option<String>,
        decision: String,
        #[serde(default)]
        reason_code: String,
        ts: u64,
    },
    /// P1-1③: scheduler ack — the host scheduler acknowledged the cadence
    /// hint it applied (LoopX `scheduler_ack.py`). Recorded via
    /// `scheduler ack`; `source` identifies the acking surface
    /// (`scheduler_cli`, `codex_app`, …). Projection-only (audit trail).
    SchedulerAcked {
        goal_id: String,
        agent_id: String,
        action: String,
        #[serde(default)]
        cadence_class: String,
        #[serde(default)]
        rrule: Option<String>,
        source: String,
        ts: u64,
    },
    /// P1-3①: scheduler heartbeat — every `scheduler tick` lands one (LoopX
    /// automation_liveness heartbeat). `rrule` is the cadence in effect
    /// after the tick. Folded into `goal.scheduler_heartbeats`; the
    /// liveness check compares now against the latest heartbeat per
    /// (goal, agent).
    SchedulerTicked {
        goal_id: String,
        agent_id: String,
        action: String,
        #[serde(default)]
        rrule: Option<String>,
        ts: u64,
    },
    /// P1-3①: automation liveness breach alert — the tick heartbeat went
    /// silent past the threshold. Folded into `goal.liveness_alerts`; the
    /// attention projection escalates the goal to the operator until a
    /// fresh heartbeat recovers the automation.
    AutomationLivenessAlert {
        goal_id: String,
        agent_id: String,
        elapsed_secs: u64,
        threshold_secs: u64,
        consecutive: u32,
        ts: u64,
    },
    /// G-16: a supervisor proposed a decision for a target agent (LoopX
    /// SUPERVISOR_PROPOSED). Projection-only — supervisor state is read from
    /// the event log, not folded into goal state.
    SupervisorProposed {
        goal_id: String,
        supervisor_agent_id: String,
        decision_id: String,
        decision_kind: String,
        target_agent_id: String,
        required_host_capabilities: Vec<String>,
        decision: String,
        ts: u64,
    },
    /// G-16: a host execution receipt recorded against a supervisor proposal
    /// (reference SUPERVISOR_RECEIPT_RECORDED). An `executed` receipt requires an
    /// authority ref (validated by the supervisor domain before append).
    SupervisorReceiptRecorded {
        goal_id: String,
        decision_id: String,
        receipt_id: String,
        adapter_id: String,
        outcome: String,
        authority_ref: Option<String>,
        rollback_ref: Option<String>,
        ts: u64,
    },
    /// P1-2③: projection self-healing audit — a read model drifted past
    /// the repair threshold and was rebuilt from its source of truth
    /// (`projection` = `run_index`: rescan of the run files on disk,
    /// non-destructive with a backup). Recorded by both the automatic
    /// run-path hook and `store verify --repair`. Projection-only: replay
    /// ignores it; the drift/rebuild counters make the repair auditable
    /// from the ledger (history / todo-event / status).
    ProjectionRepaired {
        goal_id: String,
        projection: String,
        drift_count: usize,
        missing_rows: usize,
        stale_rows: usize,
        duplicate_rows: usize,
        rows_written: usize,
        backup_path: String,
        ts: u64,
    },
    MultiAgentContractSet {
        goal_id: String,
        contract: crate::agents::multi_agent::MultiAgentContract,
        ts: u64,
    },
    /// G12: named agent recipe added (capabilities / workspaces / default
    /// priority). Re-adding a name is allowed — lookups resolve the latest
    /// event. Projection-only.
    AgentRecipeAdded {
        goal_id: String,
        recipe: crate::agents::multi_agent::AgentRecipe,
        ts: u64,
    },
    /// G12: role succession occurred — a primary's lease expired or its
    /// heartbeat went silent past the threshold, so the declared backup was
    /// promoted. Projection-only: the succession read model and the
    /// attention hint read the ledger.
    SuccessionOccurred {
        goal_id: String,
        primary: String,
        backup: String,
        reason: String,
        ts: u64,
    },
    /// G13 ②: replan rule set updated — the goal's explicit rule set (full
    /// replace; latest wins). `rule_ids` empty ⇒ reset to the default rule
    /// set. Folded into `Goal::replan_rule_set` on replay.
    ReplanRuleSetUpdated {
        goal_id: String,
        rule_set_version: String,
        #[serde(default)]
        rule_ids: Vec<String>,
        ts: u64,
    },
}

impl Event {
    fn goal_id(&self) -> &str {
        match self {
            Event::GoalStarted { goal_id, .. }
            | Event::TodoAdded { goal_id, .. }
            | Event::TodoCompleted { goal_id, .. }
            | Event::TodoSuperseded { goal_id, .. }
            | Event::TodoUpdated { goal_id, .. }
            | Event::GoalCancelled { goal_id, .. }
            | Event::GateResolved { goal_id, .. }
            | Event::GapSatisfied { goal_id, .. }
            | Event::RunRecorded { goal_id, .. }
            | Event::TodoClaimed { goal_id, .. }
            | Event::AgentRegistered { goal_id, .. }
            | Event::AgentOnboarded { goal_id, .. }
            | Event::WorkspaceLockAcquired { goal_id, .. }
            | Event::ReplanAcked { goal_id, .. }
            | Event::ProfileSet { goal_id, .. }
            | Event::AuthoritySet { goal_id, .. }
            | Event::TodoArchived { goal_id, .. }
            | Event::MonitorPolled { goal_id, .. }
            | Event::TurnNoProgress { goal_id, .. }
            | Event::QuotaSpent { goal_id, .. }
            | Event::EvidenceAttached { goal_id, .. }
            | Event::TodoRenewed { goal_id, .. }
            | Event::TodoReleased { goal_id, .. }
            | Event::TodoExpired { goal_id, .. }
            | Event::DeliveryOutcomeRecorded { goal_id, .. }
            | Event::FollowthroughCreated { goal_id, .. }
            | Event::DecisionSummaryRecorded { goal_id, .. }
            | Event::HeartbeatReceiptRecorded { goal_id, .. }
            | Event::SchedulerAcked { goal_id, .. }
            | Event::SchedulerTicked { goal_id, .. }
            | Event::AutomationLivenessAlert { goal_id, .. }
            | Event::SupervisorProposed { goal_id, .. }
            | Event::SupervisorReceiptRecorded { goal_id, .. }
            | Event::ProjectionRepaired { goal_id, .. }
            | Event::MultiAgentContractSet { goal_id, .. }
            | Event::AgentRecipeAdded { goal_id, .. }
            | Event::SuccessionOccurred { goal_id, .. }
            | Event::ReplanRuleSetUpdated { goal_id, .. } => goal_id,
        }
    }
}

/// Event timestamp (every variant carries `ts`). Used by the P1-2②
/// decision-freshness stamp; 0 when the field is absent (defensive — the
/// schema guarantees it). Derived via serde so new variants never strand
/// this accessor.
pub fn event_ts(event: &Event) -> u64 {
    serde_json::to_value(event)
        .ok()
        .and_then(|v| v.get("ts").and_then(|t| t.as_u64()))
        .unwrap_or(0)
}

// ── Store ──────────────────────────────────────────────────────────────────

/// Outcome of an atomic claim: whether the claim was appended, and
/// whether it stole an expired/dead holder's lease (for audit output).
pub struct AtomicClaimOutcome {
    pub claimed: bool,
    pub stolen: bool,
}

pub struct Store {
    root: PathBuf,
    /// In-memory registry: goal_id → (objective, cwd).
    registry: Vec<RegistryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub goal_id: String,
    pub objective: String,
    pub cwd: String,
    pub status: String,
    pub created_at: u64,
}

impl Store {
    pub fn open(root: &str) -> Result<Self> {
        let root = PathBuf::from(root);
        fs::create_dir_all(&root)?;
        let registry = load_registry(&root)?;
        Ok(Self { root, registry })
    }

    pub fn goal_dir(&self, goal_id: &str) -> PathBuf {
        self.root.join(format!("goals/{goal_id}"))
    }

    fn ensure_goal_dir(&self, goal_id: &str) -> Result<PathBuf> {
        let dir = self.goal_dir(goal_id);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    // ── Registry ────────────────────────────────────────────────────────

    pub fn registered(&self, goal_id: &str) -> bool {
        self.registry.iter().any(|g| g.goal_id == goal_id)
    }

    pub fn register(&mut self, goal: &Goal) -> Result<()> {
        if !self.registered(&goal.goal_id) {
            self.registry.push(RegistryEntry {
                goal_id: goal.goal_id.clone(),
                objective: goal.objective.clone(),
                cwd: goal.cwd.clone(),
                status: "active".to_string(),
                created_at: goal.created_at,
            });
            self.save_registry()?;
        }
        Ok(())
    }

    fn save_registry(&self) -> Result<()> {
        let path = self.root.join(REGISTRY_FILE);
        let json = serde_json::to_string_pretty(&self.registry)?;
        fs::write(&path, json).context("write registry")?;
        Ok(())
    }

    pub fn root_path(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    pub fn registry(&self) -> &[RegistryEntry] {
        &self.registry
    }

    // ── Event ledger (canonical truth) ──────────────────────────────────

    /// Append an event with a content-derived id (G-3). Returns the event id.
    /// Idempotent: appending an event whose content already exists under the
    /// same id is a no-op (reference AppendOnlyStateEventStore.append); appending
    /// the same id with DIFFERENT content fails closed with a conflict.
    pub fn append(&mut self, event: Event) -> Result<String> {
        self.append_with_meta(event, None, None, None, None, None, None)
    }

    /// Append an event with optional explicit id + provenance (G-3 backfill
    /// and G-4 privacy stamps). `event_id` is content-derived when None.
    #[allow(clippy::too_many_arguments)]
    pub fn append_with_meta(
        &mut self,
        event: Event,
        event_id: Option<String>,
        producer: Option<String>,
        source_ref: Option<String>,
        source_section: Option<String>,
        source_line: Option<u64>,
        privacy: Option<String>,
    ) -> Result<String> {
        let goal_id = event.goal_id().to_string();
        if !self.registered(&goal_id) {
            bail!("goal `{goal_id}` is not registered — register before appending events");
        }
        let dir = self.ensure_goal_dir(&goal_id)?;
        let event_id = event_id.unwrap_or_else(|| derive_event_id(&event));
        let stored = StoredEvent {
            event_id: event_id.clone(),
            producer,
            source_ref,
            source_section,
            source_line,
            privacy,
            fencing_token: None,
            event,
        };
        let line = format!("{}\n", serde_json::to_string(&stored)?);
        append_event_locked(dir.join(EVENTS_FILE), &line, &event_id).context("append event")?;
        self.ensure_schema_stamp(&goal_id)?;
        Ok(event_id)
    }

    /// Read the raw ledger lines (no migration transforms) — for verification.
    pub fn raw_ledger_lines(&self, goal_id: &str) -> Result<Vec<String>> {
        let path = self.goal_dir(goal_id).join(EVENTS_FILE);
        if !path.exists() {
            return Ok(vec![]);
        }
        Ok(fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect())
    }

    /// Atomically claim `todo_id` for `agent_id`: lease-state check and the
    /// claim append happen under the SAME exclusive file lock, closing the
    /// decide→claim TOCTOU race between concurrent `run --agent-id` processes
    /// (previously both could pass the free-lease check and the last claim
    /// won in the ledger → the same todo got executed twice).
    pub fn try_claim_todo(
        &self,
        goal_id: &str,
        todo_id: &str,
        agent_id: &str,
        lease_secs: u64,
    ) -> Result<AtomicClaimOutcome> {
        use std::io::Write;
        let now = crate::state::now_epoch();
        // Normalize the TTL here (0 → default, >max → error) so every
        // caller gets identical expiry semantics to the non-atomic
        // task_lease path — previously a 0 default on the manual
        // `lease claim` path minted an already-expired lease.
        let lease_secs = crate::work_items::task_lease::normalize_ttl(lease_secs)?;
        let dir = self.ensure_goal_dir(goal_id)?;
        let path = dir.join(EVENTS_FILE);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)?;
        file.lock_exclusive()?;
        let result = (|| -> Result<AtomicClaimOutcome> {
            let existing = fs::read_to_string(&path).unwrap_or_default();
            // Reconstruct the current lease for this todo from the ledger
            // (StoredEvent flattens the Event payload to top level).
            let mut lease: Option<(String, u64, Option<u32>)> = None;
            for line in existing.lines().filter(|l| !l.trim().is_empty()) {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                if v.get("todo_id").and_then(|t| t.as_str()) != Some(todo_id) {
                    continue;
                }
                match v.get("kind").and_then(|k| k.as_str()).unwrap_or("") {
                    "todo_claimed" => {
                        let agent = v
                            .get("agent_id")
                            .and_then(|a| a.as_str())
                            .unwrap_or("")
                            .to_string();
                        let exp = v
                            .get("lease_expires_at")
                            .and_then(|e| e.as_u64())
                            .unwrap_or(0);
                        let pid = v
                            .get("holder_pid")
                            .and_then(|p| p.as_u64())
                            .map(|p| p as u32);
                        lease = Some((agent, exp, pid));
                    }
                    "todo_released" => lease = None,
                    // Mirror replay (apply): an expiry record clears the
                    // claim too, so a steal/expiry is honored by the atomic
                    // claim path exactly like by projection replay.
                    "todo_expired" => lease = None,
                    _ => {}
                }
            }
            if let Some((holder, exp, holder_pid)) = &lease {
                if *exp > now && holder != agent_id {
                    // Lease liveness: a dead holder's claim is reclaimed
                    // automatically (mirrors the task_lease steal path).
                    let holder_dead = holder_pid
                        .map(|p| !crate::compat::pid_alive(p))
                        .unwrap_or(false);
                    if !holder_dead {
                        return Ok(AtomicClaimOutcome {
                            claimed: false,
                            stolen: false,
                        });
                    }
                }
            }
            // Steal when the prior lease belongs to another agent: a live
            // lease with a live holder already returned `claimed=false`
            // above, so reaching here means the lease lapsed or its holder
            // is dead. We do NOT append a TodoExpired marker — the fresh
            // TodoClaimed supersedes the old claim in replay order, and a
            // content-derived TodoExpired id would collide with any manual
            // expiry appended in the same second (idempotent dedup would
            // silently drop one of them).
            let stolen = lease
                .as_ref()
                .is_some_and(|(holder, _, _)| holder != agent_id);
            let expires_at = now + lease_secs;
            let event = Event::TodoClaimed {
                goal_id: goal_id.to_string(),
                todo_id: todo_id.to_string(),
                agent_id: agent_id.to_string(),
                lease_expires_at: expires_at,
                holder_pid: Some(std::process::id()),
                ts: now,
            };
            let stored = StoredEvent {
                event_id: derive_event_id(&event),
                producer: None,
                source_ref: None,
                source_section: None,
                source_line: None,
                privacy: None,
                fencing_token: None,
                event,
            };
            let line = format!("{}\n", serde_json::to_string(&stored)?);
            file.write_all(line.as_bytes())?;
            Ok(AtomicClaimOutcome {
                claimed: true,
                stolen,
            })
        })();
        let _ = file.unlock();
        result
    }

    /// The parsed ledger (G-3/G-6 read path): legacy lines are migrated
    /// in-memory to the current schema and get a content-derived id.
    pub fn events(&self, goal_id: &str) -> Result<Vec<StoredEvent>> {
        let dir = self.goal_dir(goal_id);
        let from = self
            .goal_schema_version(goal_id)
            .unwrap_or_else(|| LEGACY_EVENT_STORE_SCHEMA_VERSION.to_string());
        read_ledger(&dir, &from)
    }

    /// Ledger read diagnostics (O1): the sidecar written by the last ledger
    /// read when unknown-kind lines were skipped, or `None` when the ledger
    /// read clean. Surfaced by `status` / `diagnose` / `store verify`.
    pub fn ledger_read_diagnostics(&self, goal_id: &str) -> Option<serde_json::Value> {
        read_diagnostics(&self.goal_dir(goal_id))
    }

    /// Verify the ledger for id/conflict integrity (G-3): duplicate event
    /// ids with identical content are counted (idempotent duplicates); a
    /// duplicate id with DIFFERENT content is a conflict (fail closed).
    pub fn verify(&self, goal_id: &str) -> Result<LedgerReport> {
        let dir = self.goal_dir(goal_id);
        let schema = self
            .goal_schema_version(goal_id)
            .unwrap_or_else(|| LEGACY_EVENT_STORE_SCHEMA_VERSION.to_string());
        verify_ledger(&dir, goal_id, &schema)
    }

    // ── Schema stamp (G-6) ───────────────────────────────────────────────

    pub fn goal_schema_version(&self, goal_id: &str) -> Option<String> {
        let path = self.goal_dir(goal_id).join(SCHEMA_FILE);
        let text = fs::read_to_string(path).ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        let raw = value
            .get("event_store_schema_version")
            .and_then(|v| v.as_str())?;
        // Normalize the pre-rename schema tokens (legacy goals stamped with
        // `loopx_event_store_v0/v1` load transparently).
        Some(match raw {
            "loopx_event_store_v1" => EVENT_STORE_SCHEMA_VERSION.to_string(),
            "loopx_event_store_v0" => LEGACY_EVENT_STORE_SCHEMA_VERSION.to_string(),
            other => other.to_string(),
        })
    }

    fn ensure_schema_stamp(&self, goal_id: &str) -> Result<()> {
        let dir = self.ensure_goal_dir(goal_id)?;
        let path = dir.join(SCHEMA_FILE);
        if path.exists() {
            return Ok(());
        }
        let payload = serde_json::json!({
            "event_store_schema_version": EVENT_STORE_SCHEMA_VERSION,
            "created_at": crate::state::now_epoch(),
        });
        fs::write(path, serde_json::to_string_pretty(&payload)? + "\n")
            .context("write schema stamp")?;
        Ok(())
    }

    /// Rebuild active goal state by replaying the event ledger (LoopX:
    /// canonical event/state → active state is a reconstructible read model).
    pub fn replay(&self, goal_id: &str) -> Result<Option<Goal>> {
        let Some(entry) = self.registry.iter().find(|g| g.goal_id == goal_id) else {
            return Ok(None);
        };
        let path = self.goal_dir(goal_id).join(EVENTS_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let mut goal = Goal::new(goal_id, &entry.objective, &entry.cwd);
        let from = self
            .goal_schema_version(goal_id)
            .unwrap_or_else(|| LEGACY_EVENT_STORE_SCHEMA_VERSION.to_string());
        let ledger = read_ledger(&self.goal_dir(goal_id), &from).context("read ledger")?;
        // P1-2②: stamp the decision-freshness read model — arbitration
        // decisions compiled from this state carry the max ledger seq /
        // newest event ts they were rebuilt against.
        goal.decision_freshness = Some(crate::state::DecisionFreshness {
            events_max_seq: ledger.len() as u64,
            events_max_ts: ledger
                .iter()
                .map(|stored| event_ts(&stored.event))
                .filter(|ts| *ts > 0)
                .max(),
            read_at: crate::state::now_epoch(),
        });
        for stored in ledger {
            apply(&mut goal, stored.event);
        }
        // Restore Next Action (active-state field kept alongside the ledger).
        let na_path = self.goal_dir(goal_id).join(NEXT_ACTION_FILE);
        if na_path.exists() {
            goal.next_action = Some(
                fs::read_to_string(&na_path)
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            );
        }
        // Restore run history.
        let runs_path = self.goal_dir(goal_id).join(RUNS_FILE);
        if runs_path.exists() {
            let text = fs::read_to_string(&runs_path).unwrap_or_default();
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                if let Ok(r) = serde_json::from_str::<RunRecord>(line) {
                    goal.history.push(r);
                }
            }
        }
        Ok(Some(goal))
    }

    // ── Active-state projections ────────────────────────────────────────

    pub fn set_next_action(&self, goal_id: &str, next_action: &str) -> Result<()> {
        let dir = self.ensure_goal_dir(goal_id)?;
        fs::write(dir.join(NEXT_ACTION_FILE), next_action).context("write next_action")?;
        Ok(())
    }

    pub fn append_run(&self, goal_id: &str, record: &RunRecord) -> Result<()> {
        let dir = self.ensure_goal_dir(goal_id)?;
        let mut line = serde_json::to_string(record)?;
        line.push('\n');
        append_locked(dir.join(RUNS_FILE), line.as_bytes()).context("append run")?;
        Ok(())
    }

    // ── Backup / restore (LoopX: state_backup / state_migration SOP) ────

    /// Snapshot one goal's state files into backups/<ts>/ (events + runs +
    /// next_action + schema stamp + scheduler-state). Point-in-time restore
    /// uses the same files. G-10: the scheduler state directory is included
    /// so a restore does not silently reset progression (P1 risk:
    /// replay/backup interaction). G-6: the schema stamp travels with the
    /// backup so a restore rolls the schema version back in time with it.
    pub fn backup_goal(&self, goal_id: &str) -> Result<String> {
        let dir = self.goal_dir(goal_id);
        if !dir.join(EVENTS_FILE).exists() {
            bail!("goal {goal_id} has no state to back up");
        }
        let ts = crate::state::now_epoch();
        let dest = self.root.join(BACKUPS_DIR).join(format!("{ts}-{goal_id}"));
        fs::create_dir_all(&dest)?;
        for file in [EVENTS_FILE, RUNS_FILE, NEXT_ACTION_FILE, SCHEMA_FILE] {
            let src = dir.join(file);
            if src.exists() {
                fs::copy(&src, dest.join(file))?;
            }
        }
        copy_dir_if_present(&dir.join("scheduler-state"), &dest.join("scheduler-state"))?;
        // Registry snapshot (goal entry).
        self.registry
            .iter()
            .find(|g| g.goal_id == goal_id)
            .map(|entry| -> Result<()> {
                let json = serde_json::to_string_pretty(entry)?;
                fs::write(dest.join("registry-entry.json"), json)?;
                Ok(())
            })
            .transpose()?;
        Ok(dest.to_string_lossy().into_owned())
    }

    /// Restore a goal from a backup directory (overwrites current state files).
    pub fn restore_goal(&self, goal_id: &str, backup_dir: &str) -> Result<()> {
        let src = PathBuf::from(backup_dir);
        if !src.join(EVENTS_FILE).exists() {
            bail!("backup at {backup_dir} has no events.jsonl");
        }
        let dest = self.ensure_goal_dir(goal_id)?;
        for file in [EVENTS_FILE, RUNS_FILE, NEXT_ACTION_FILE, SCHEMA_FILE] {
            let s = src.join(file);
            if s.exists() {
                fs::copy(&s, dest.join(file))?;
            }
        }
        copy_dir_if_present(&src.join("scheduler-state"), &dest.join("scheduler-state"))?;
        Ok(())
    }

    /// Remove a goal entirely: registry entry + per-goal state directory.
    /// Irreversible — callers must gate with `--force`.
    pub fn delete_goal(&mut self, goal_id: &str) -> Result<()> {
        let before = self.registry.len();
        self.registry.retain(|g| g.goal_id != goal_id);
        if self.registry.len() == before {
            bail!("goal `{goal_id}` not found in registry");
        }
        self.save_registry()?;
        let dir = self.goal_dir(goal_id);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    pub fn backups(&self, goal_id: &str) -> Vec<String> {
        let dir = self.root.join(BACKUPS_DIR);
        let Ok(entries) = fs::read_dir(&dir) else {
            return vec![];
        };
        entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .ends_with(&format!("-{goal_id}"))
            })
            .map(|e| e.path().to_string_lossy().into_owned())
            .collect()
    }
}

/// Append under an advisory lock so concurrent processes cannot interleave
/// a line in the middle of an event/run (LoopX: file_lock).
fn append_locked(path: PathBuf, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    file.lock_exclusive()?;
    let result = {
        let mut f = &file;
        f.write_all(bytes)
    };
    let _ = FileExt::unlock(&file);
    result
}

/// Append one event line under the advisory lock with G-3 idempotency +
/// conflict detection (reference `AppendOnlyStateEventStore.append` →
/// `StateEventConflictError`): the same event id with identical content is
/// skipped (idempotent replay/backfill re-run); the same id with different
/// content fails closed.
fn append_event_locked(path: PathBuf, line: &str, event_id: &str) -> Result<()> {
    use std::io::Write;
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&path)?;
    file.lock_exclusive()?;
    let result = (|| -> Result<()> {
        let existing = fs::read_to_string(&path).unwrap_or_default();
        let new_value: serde_json::Value = serde_json::from_str(line).context("serialize event")?;
        let new_fingerprint = event_fingerprint(&new_value);
        for existing_line in existing.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(existing_line) else {
                continue;
            };
            let existing_id = value.get("event_id").and_then(|v| v.as_str()).unwrap_or("");
            if existing_id != event_id {
                continue;
            }
            if event_fingerprint(&value) == new_fingerprint {
                // Idempotent re-append (same event content) — nothing to do.
                return Ok(());
            }
            bail!("conflicting event_id `{event_id}` — same id, different content (StateEventConflictError)");
        }
        let mut f = &file;
        f.write_all(line.as_bytes())?;
        Ok(())
    })();
    let _ = FileExt::unlock(&file);
    result
}

/// Classify a [`StoredEvent`] deserialization failure as an unknown `kind`
/// variant (O1). Relies purely on serde's error text plus a non-empty `kind`
/// string present in the line — no hand-maintained kind list, so a kind added
/// by a newer binary is tolerated without shipping a new enum here.
fn is_unknown_kind_error(value: &serde_json::Value, err: &serde_json::Error) -> Option<String> {
    let kind = value.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    if kind.is_empty() {
        return None;
    }
    err.to_string()
        .contains("unknown variant")
        .then(|| kind.to_string())
}

/// Persist (or clear) the ledger read diagnostics sidecar (O1). Best-effort:
/// a failed write must never take down the read path. `skipped` holds
/// (1-based line number, kind) pairs for lines skipped as unknown-kind.
fn write_read_diagnostics(dir: &Path, skipped: &[(usize, String)]) {
    let path = dir.join(READ_DIAGNOSTICS_FILE);
    if skipped.is_empty() {
        let _ = fs::remove_file(&path);
        return;
    }
    let mut kinds: Vec<&str> = skipped.iter().map(|(_, k)| k.as_str()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    let note = format!(
        "{} unknown-kind event(s) skipped — binary older than ledger, please upgrade",
        skipped.len()
    );
    let payload = serde_json::json!({
        "skipped_unknown_kinds": skipped.len(),
        "skipped_lines": skipped.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
        "unknown_kinds": kinds,
        "note": note,
    });
    let _ = fs::write(
        path,
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );
}

/// Read the ledger read diagnostics sidecar, if present.
pub fn read_diagnostics(dir: &Path) -> Option<serde_json::Value> {
    let text = fs::read_to_string(dir.join(READ_DIAGNOSTICS_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Read + parse the ledger, migrating legacy lines in-memory to the current
/// schema (G-6 read path) and backfilling content-derived ids for lines that
/// predate G-3. Identical duplicate ids are collapsed to the first occurrence
/// (idempotent dedupe); conflicting duplicates fail closed.
///
/// O1: lines carrying an event `kind` this binary does not know (written by
/// a newer binary) are SKIPPED with a warning recorded to
/// [`READ_DIAGNOSTICS_FILE`] instead of hard-failing the whole ledger read;
/// structural errors (missing fields / wrong types on a known kind) still
/// fail closed.
fn read_ledger(dir: &Path, from_schema: &str) -> Result<Vec<StoredEvent>> {
    let path = dir.join(EVENTS_FILE);
    if !path.exists() {
        write_read_diagnostics(dir, &[]);
        return Ok(vec![]);
    }
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut out: Vec<StoredEvent> = vec![];
    let mut skipped: Vec<(usize, String)> = vec![];
    for (line_number, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let mut value: serde_json::Value =
            serde_json::from_str(line).context(format!("parse event line {}", line_number + 1))?;
        crate::migration::migrate_event_line(&mut value, from_schema, EVENT_STORE_SCHEMA_VERSION)
            .context(format!("migrate event line {}", line_number + 1))?;
        let stored: StoredEvent = match serde_json::from_value(value.clone()) {
            Ok(stored) => stored,
            Err(err) => {
                if let Some(kind) = is_unknown_kind_error(&value, &err) {
                    skipped.push((line_number + 1, kind));
                    continue;
                }
                return Err(anyhow::anyhow!(
                    "parse event line {}: {err}",
                    line_number + 1
                ));
            }
        };
        let id = stored.effective_id();
        let fingerprint = event_fingerprint(
            &serde_json::to_value(&stored.event).unwrap_or(serde_json::Value::Null),
        );
        if let Some(prior) = seen.get(&id) {
            if prior != &fingerprint {
                bail!(
                    "conflicting event_id `{id}` at line {} (StateEventConflictError)",
                    line_number + 1
                );
            }
            continue; // identical duplicate — collapse
        }
        seen.insert(id, fingerprint);
        out.push(stored);
    }
    write_read_diagnostics(dir, &skipped);
    Ok(out)
}

/// Ledger integrity report (G-3 `store verify`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct LedgerReport {
    pub goal_id: String,
    pub schema_version: String,
    pub total_events: usize,
    pub unique_events: usize,
    pub idempotent_duplicates: usize,
    pub legacy_lines_without_id: usize,
    /// O1: lines whose `kind` this binary does not know (skipped by the read
    /// path, kept here so verify surfaces the tolerance surface).
    pub skipped_unknown_kinds: usize,
    pub unknown_kinds: Vec<String>,
    pub conflicts: Vec<String>,
    pub ok: bool,
}

/// Scan the ledger for duplicate ids / content conflicts (no migrations
/// applied — raw lines).
pub fn verify_ledger(dir: &Path, goal_id: &str, schema: &str) -> Result<LedgerReport> {
    let path = dir.join(EVENTS_FILE);
    if !path.exists() {
        return Ok(LedgerReport {
            goal_id: goal_id.to_string(),
            schema_version: schema.to_string(),
            total_events: 0,
            unique_events: 0,
            idempotent_duplicates: 0,
            legacy_lines_without_id: 0,
            skipped_unknown_kinds: 0,
            unknown_kinds: vec![],
            conflicts: vec![],
            ok: true,
        });
    }
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut by_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut idempotent_duplicates = 0usize;
    let mut legacy_lines_without_id = 0usize;
    let mut conflicts: Vec<String> = vec![];
    let mut total = 0usize;
    let mut skipped_unknown_kinds = 0usize;
    let mut unknown_kinds: Vec<String> = vec![];
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        total += 1;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            conflicts.push("unparsable line".to_string());
            continue;
        };
        // O1: count the lines the read path would tolerate-and-skip.
        if let Err(err) = serde_json::from_value::<StoredEvent>(value.clone()) {
            if let Some(kind) = is_unknown_kind_error(&value, &err) {
                skipped_unknown_kinds += 1;
                if !unknown_kinds.contains(&kind) {
                    unknown_kinds.push(kind);
                }
            }
        }
        let raw_id = value
            .get("event_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let id = if raw_id.is_empty() {
            legacy_lines_without_id += 1;
            derive_event_id_from_value(&value)
        } else {
            raw_id
        };
        let fingerprint = event_fingerprint(&value);
        match by_id.get(&id) {
            Some(prior) if prior == &fingerprint => idempotent_duplicates += 1,
            Some(_) => {
                if !conflicts.contains(&id) {
                    conflicts.push(id.clone());
                }
            }
            None => {
                by_id.insert(id, fingerprint);
            }
        }
    }
    let ok = conflicts.is_empty();
    unknown_kinds.sort();
    Ok(LedgerReport {
        goal_id: goal_id.to_string(),
        schema_version: schema.to_string(),
        total_events: total,
        unique_events: by_id.len(),
        idempotent_duplicates,
        legacy_lines_without_id,
        skipped_unknown_kinds,
        unknown_kinds,
        conflicts,
        ok,
    })
}

fn load_registry(root: &Path) -> Result<Vec<RegistryEntry>> {
    let path = root.join(REGISTRY_FILE);
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = fs::read_to_string(&path)?;
    // Dual format: the native array, or the reference-compatible map
    // {"goals":[...]} written by earlier productized builds (fields id/repo
    // map onto goal_id/cwd).
    let v: serde_json::Value = serde_json::from_str(&text).context("parse registry")?;
    let items: Vec<serde_json::Value> = match &v {
        serde_json::Value::Array(a) => a.clone(),
        serde_json::Value::Object(m) => m
            .get("goals")
            .and_then(|g| g.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => bail!("registry is neither an array nor a {{goals:[...]}} object"),
    };
    items
        .into_iter()
        .map(|g| {
            let obj = g
                .as_object()
                .ok_or_else(|| anyhow::anyhow!("registry entry is not an object"))?;
            let str_of = |k: &str, alt: &str| -> String {
                obj.get(k)
                    .or_else(|| obj.get(alt))
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            Ok(RegistryEntry {
                goal_id: str_of("goal_id", "id"),
                objective: str_of("objective", "objective"),
                cwd: str_of("cwd", "repo"),
                status: str_of("status", "status"),
                created_at: obj.get("created_at").and_then(|x| x.as_u64()).unwrap_or(0),
            })
        })
        .collect()
}

/// Copy a directory tree when present (used for the scheduler-state dir in
/// backup/restore — G-10).
fn copy_dir_if_present(src: &Path, dest: &Path) -> Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_if_present(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn apply(goal: &mut Goal, event: Event) {
    match event {
        Event::GoalStarted { .. } => {}
        Event::TodoAdded { todo, ts, .. } => {
            let mut t = todo;
            // Assign the goal-relative index on replay too (identity/order).
            if t.index == 0 {
                goal.next_index += 1;
                t.index = goal.next_index;
            }
            goal.todos.push(t);
            // G13 ①: a new todo changes the frontier (segment reset marker).
            goal.frontier_change_ts.push(ts);
        }
        Event::TodoCompleted {
            todo_id,
            no_follow_up,
            successor_ids,
            evidence,
            ts,
            ..
        } => {
            // G13 ③: summary first — `complete` takes `successor_ids` by value.
            let summary = if no_follow_up {
                "no-follow-up".to_string()
            } else if !successor_ids.is_empty() {
                format!("successor {}", successor_ids.join(","))
            } else {
                "completed".to_string()
            };
            if let Some(t) = goal.todo_mut(&todo_id) {
                t.complete(no_follow_up, successor_ids);
                // The event ts is authoritative for the completion stamp
                // (wall-clock replay is second-granular; GateResolved already
                // applies the same rule).
                t.completed_at = Some(ts);
                t.updated_at = ts;
                if let Some(e) = evidence {
                    t.evidence = Some(e);
                }
            }
            // G13 ①: completion moves the frontier (segment reset marker).
            goal.frontier_change_ts.push(ts);
            // G13 ③: semantic history fold.
            goal.record_semantic_event(
                crate::decision::goal_frontier::semantic_history::KIND_TODO_COMPLETED,
                Some(&todo_id),
                &summary,
                ts,
            );
        }
        Event::TodoSuperseded { todo_id, ts, .. } => {
            goal.supersede(&todo_id);
            goal.frontier_change_ts.push(ts);
            goal.record_semantic_event(
                crate::decision::goal_frontier::semantic_history::KIND_TODO_SUPERSEDED,
                Some(&todo_id),
                "superseded",
                ts,
            );
        }
        Event::TodoUpdated {
            todo_id,
            text,
            status,
            evidence,
            note,
            priority,
            resume_when,
            blocks,
            acceptance,
            ts,
            ..
        } => {
            if let Some(t) = goal.todo_mut(&todo_id) {
                if let Some(x) = text {
                    t.text = x;
                }
                if let Some(x) = evidence {
                    t.evidence = Some(x);
                }
                if let Some(x) = note {
                    t.note = Some(x);
                }
                if let Some(a) = acceptance {
                    t.acceptance = if a.trim().is_empty() { None } else { Some(a) };
                }
                if let Some(b) = blocks {
                    // `--blocks a,b` replaces the blocking set; `--blocks ""`
                    // clears it. Mirrors Todo::blocking()'s join encoding.
                    if b.is_empty() {
                        t.blocked_by_gate = None;
                    } else {
                        t.blocked_by_gate = Some(b.join(","));
                    }
                }
                if let Some(p) = priority.as_deref() {
                    t.priority = match p.to_uppercase().as_str() {
                        "P0" => crate::state::Priority::P0,
                        "P2" => crate::state::Priority::P2,
                        _ => crate::state::Priority::P1,
                    };
                }
                if let Some(rw) = resume_when {
                    t.resume_when_text = Some(rw.clone());
                    t.status = crate::state::TodoStatus::Deferred;
                    // `defer:N` (written by `todo update --resume-when N` with a
                    // numeric N) sets a REAL deadline: resume_when = now + N
                    // seconds, so the deferred/monitor todo becomes due again.
                    // Any other value is a text-only hint (no deadline) and
                    // keeps the legacy behavior.
                    if let Some(secs) = rw
                        .strip_prefix("defer:")
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        t.resume_when = Some(
                            std::time::SystemTime::now() + std::time::Duration::from_secs(secs),
                        );
                    }
                }
                if let Some(st) = status.as_deref() {
                    match st {
                        "open" => t.status = crate::state::TodoStatus::Open,
                        "blocked" => t.status = crate::state::TodoStatus::Blocked,
                        "deferred" => t.status = crate::state::TodoStatus::Deferred,
                        "superseded" => t.status = crate::state::TodoStatus::Superseded,
                        _ => {}
                    }
                }
                t.updated_at = ts;
            }
        }
        Event::GoalCancelled { .. } => goal.status = "cancelled".to_string(),
        Event::GateResolved {
            goal_id: _,
            todo_id,
            decision,
            note,
            ts,
        } => {
            if let Some(t) = goal.todo_mut(&todo_id) {
                // Gate resolution: done WITHOUT a no-follow-up marker (LoopX
                // renders gate resolution without no_followup=).
                t.status = crate::state::TodoStatus::Done;
                t.decision = Some(decision);
                t.completed_at = Some(ts);
                t.updated_at = ts;
                if let Some(n) = note {
                    t.note = Some(n);
                }
            }
            // G13 ①: a resolved gate unblocks the frontier (segment reset).
            goal.frontier_change_ts.push(ts);
            goal.record_semantic_event(
                crate::decision::goal_frontier::semantic_history::KIND_GATE_RESOLVED,
                Some(&todo_id),
                "gate resolved",
                ts,
            );
        }
        Event::GapSatisfied { gap_id, ts, .. } => {
            goal.satisfy_gap(&gap_id);
            goal.record_semantic_event(
                crate::decision::goal_frontier::semantic_history::KIND_ACCEPTANCE_GAP_SATISFIED,
                None,
                &format!("acceptance gap {gap_id} satisfied"),
                ts,
            );
        }
        Event::RunRecorded { record, ts, .. } => {
            // run history itself is read from runs.jsonl; G13 ③ folds the
            // semantic summary from the event ledger.
            let summary = if record.evidence.trim().is_empty() {
                record.terminal_state.clone()
            } else {
                format!(
                    "{} — {}",
                    record.terminal_state,
                    crate::decision::truncate(&record.evidence, 120)
                )
            };
            goal.record_semantic_event(
                crate::decision::goal_frontier::semantic_history::KIND_RUN_LANDED,
                Some(&record.todo_id),
                &summary,
                ts,
            );
        }
        Event::TodoClaimed {
            todo_id,
            agent_id,
            lease_expires_at,
            holder_pid,
            ..
        } => {
            if let Some(t) = goal.todo_mut(&todo_id) {
                t.claimed_by = Some(agent_id);
                t.lease_expires_at = Some(lease_expires_at);
                t.holder_pid = holder_pid;
            }
        }
        Event::AgentRegistered {
            agent_id,
            workspaces,
            ..
        } => {
            if !goal.registered_agents.iter().any(|a| a == &agent_id) {
                goal.registered_agents.push(agent_id.clone());
            }
            goal.agent_profiles.retain(|p| p.id != agent_id);
            goal.agent_profiles.push(crate::state::AgentProfile {
                id: agent_id,
                capabilities: vec![],
                workspaces,
            });
        }
        Event::AgentOnboarded {
            agent_id,
            capabilities,
            workspaces,
            ..
        } => {
            if !goal.registered_agents.iter().any(|a| a == &agent_id) {
                goal.registered_agents.push(agent_id.clone());
            }
            goal.agent_profiles.retain(|p| p.id != agent_id);
            goal.agent_profiles.push(crate::state::AgentProfile {
                id: agent_id,
                capabilities,
                workspaces,
            });
        }
        // P0-1: advisory write-lock records are projection-only — occupancy
        // is derived from profiles + live leases; the event is the audit
        // trail (like the supervisor events below).
        Event::WorkspaceLockAcquired { .. } => {}
        Event::ReplanAcked {
            delta_kinds, ts, ..
        } => {
            goal.replan_ack = Some(crate::state::ReplanAck {
                recorded: true,
                delta_kinds: delta_kinds.clone(),
                at: ts,
            });
            // G13 ①: only frontier-changing acks reset outcome segments.
            let ack = goal.replan_ack.as_ref().expect("just recorded");
            if ack.has_frontier_delta() {
                goal.frontier_change_ts.push(ts);
            }
            goal.record_semantic_event(
                crate::decision::goal_frontier::semantic_history::KIND_REPLAN_ACKED,
                None,
                &format!("delta kinds: {}", delta_kinds.join(",")),
                ts,
            );
        }
        Event::ProfileSet {
            outcome_floor_streak_threshold,
            ..
        } => {
            goal.execution_profile.outcome_floor_streak_threshold = outcome_floor_streak_threshold;
        }
        Event::AuthoritySet {
            write_scope,
            requires_approval,
            ..
        } => {
            goal.authority.write_scope = write_scope;
            goal.authority.requires_approval = requires_approval;
        }
        Event::TodoArchived { todo_id, ts, .. } => {
            goal.archive_todo(&todo_id);
            // G13 ①: archiving removes the todo from the frontier.
            goal.frontier_change_ts.push(ts);
        }
        Event::MonitorPolled {
            todo_id,
            result,
            no_change_count,
            ts,
            ..
        } => {
            // Mirror executor::writeback monitor handling exactly, using the
            // poll timestamp so replay is deterministic (not now()-relative).
            if let Some(m) = goal.todo_mut(&todo_id) {
                if result == "changed" {
                    m.consecutive_no_change = 0;
                    m.status = crate::state::TodoStatus::Done;
                } else {
                    m.consecutive_no_change = no_change_count;
                    // P1-3②: cadence-aware reschedule (the same derivation
                    // the run path uses) with the fixed G-8 backoff as the
                    // no-cadence fallback — replay stays exact.
                    let next_due = crate::scheduler::monitor_poll::next_poll_due_epoch(
                        ts,
                        m.monitor_cadence.as_deref(),
                    );
                    m.resume_when = Some(
                        std::time::SystemTime::UNIX_EPOCH
                            + std::time::Duration::from_secs(next_due),
                    );
                }
                m.updated_at = ts;
            }
            goal.record_semantic_event(
                crate::decision::goal_frontier::semantic_history::KIND_MONITOR_POLL,
                Some(&todo_id),
                &result,
                ts,
            );
        }
        Event::QuotaSpent { slots, .. } => {
            goal.quota_spent_slots = goal.quota_spent_slots.saturating_add(slots);
        }
        Event::EvidenceAttached {
            todo_id,
            evidence,
            ts,
            ..
        } => {
            if let Some(t) = goal.todo_mut(&todo_id) {
                t.evidence = Some(evidence);
                t.updated_at = ts;
            }
        }
        Event::TodoRenewed {
            todo_id,
            agent_id,
            lease_expires_at,
            ts,
            ..
        } => {
            if let Some(t) = goal.todo_mut(&todo_id) {
                if t.claimed_by.is_none() {
                    t.claimed_by = Some(agent_id);
                }
                t.lease_expires_at = Some(lease_expires_at);
                t.updated_at = ts;
            }
        }
        Event::TodoReleased {
            todo_id,
            agent_id,
            ts,
            ..
        } => {
            if let Some(t) = goal.todo_mut(&todo_id) {
                if t.claimed_by.as_deref() == Some(agent_id.as_str()) {
                    t.claimed_by = None;
                    t.lease_expires_at = None;
                    t.updated_at = ts;
                }
            }
        }
        Event::TodoExpired { todo_id, ts, .. } => {
            if let Some(t) = goal.todo_mut(&todo_id) {
                if t.claimed_by.is_some() {
                    t.claimed_by = None;
                    t.lease_expires_at = None;
                    t.updated_at = ts;
                }
            }
        }
        // P0-2: delivery outcomes fold into the per-work-item delivery read
        // model (latest wins; transitions are validated at the command layer
        // before the event is appended).
        Event::DeliveryOutcomeRecorded {
            todo_id,
            outcome,
            note,
            delivered_turn,
            seq,
            ts,
            ..
        } => {
            goal.apply_delivery_outcome(&todo_id, &outcome, note, delivered_turn, seq, ts);
            goal.record_semantic_event(
                crate::decision::goal_frontier::semantic_history::KIND_DELIVERY_OUTCOME,
                Some(&todo_id),
                &outcome,
                ts,
            );
        }
        Event::FollowthroughCreated {
            source_todo_id,
            followup_todo_id,
            ts,
            ..
        } => goal.apply_followthrough(&source_todo_id, &followup_todo_id, ts),
        // P1-3①: liveness heartbeat / alert fold into goal state (they ARE
        // the state the liveness check + attention escalation read).
        Event::SchedulerTicked { agent_id, ts, .. } => {
            let entry = goal.scheduler_heartbeats.entry(agent_id).or_insert(ts);
            *entry = (*entry).max(ts);
        }
        Event::AutomationLivenessAlert {
            agent_id,
            elapsed_secs,
            threshold_secs,
            consecutive,
            ts,
            ..
        } => goal.liveness_alerts.push(crate::state::LivenessAlert {
            agent_id,
            elapsed_secs,
            threshold_secs,
            consecutive,
            ts,
        }),
        // O3: idle-turn no-progress records fold into goal state
        // (append-only) — visible in `status` so orchestrators can see the
        // breach without replaying the raw ledger.
        Event::TurnNoProgress {
            goal_id,
            todo_id,
            agent_id,
            idle_secs,
            tool_calls_total,
            ts,
        } => {
            let summary = format!("{}s idle after {} tool calls", idle_secs, tool_calls_total);
            goal.turn_no_progress
                .push(crate::state::TurnNoProgressRecord {
                    goal_id,
                    todo_id,
                    agent_id,
                    idle_secs,
                    tool_calls_total,
                    ts,
                });
            goal.record_semantic_event(
                crate::decision::goal_frontier::semantic_history::KIND_TURN_NO_PROGRESS,
                None,
                &summary,
                ts,
            );
        }
        // G13 ②: replan rule set update — latest wins; empty ids reset to
        // the default rule set (the folded state never stores an empty list).
        Event::ReplanRuleSetUpdated {
            rule_set_version,
            rule_ids,
            ts,
            ..
        } => {
            // Empty ids on the wire = reset to the default rule set.
            goal.replan_rule_set = if rule_ids.is_empty() {
                None
            } else {
                Some(
                    crate::decision::goal_frontier::replan_rules::ReplanRuleSet {
                        schema_version: rule_set_version,
                        rule_ids,
                    },
                )
            };
            goal.record_semantic_event(
                crate::decision::goal_frontier::semantic_history::KIND_REPLAN_ACKED,
                None,
                "replan rule set updated",
                ts,
            );
        }
        // P1-1 + G-16 + G12 projection-only events: read from
        // the event log by their read models; goal state is unchanged on
        // replay.
        Event::DecisionSummaryRecorded { .. }
        | Event::HeartbeatReceiptRecorded { .. }
        | Event::SchedulerAcked { .. }
        | Event::SupervisorProposed { .. }
        | Event::SupervisorReceiptRecorded { .. }
        | Event::ProjectionRepaired { .. }
        | Event::MultiAgentContractSet { .. }
        | Event::AgentRecipeAdded { .. } => {}
        Event::SuccessionOccurred {
            primary,
            backup,
            reason,
            ts,
            ..
        } => {
            goal.record_semantic_event(
                crate::decision::goal_frontier::semantic_history::KIND_ROLE_SUCCESSION,
                None,
                &format!("{primary}→{backup} ({reason})"),
                ts,
            );
        }
    }
}

// ── Projection gap check ───────────────────────────────────────────────────

/// reference `state_projection_gap_warning`: an executable Next Action with no
/// open agent todo (or a user-wait Next Action with no open user gate) means
/// the active-state projection drifted from the todo frontier. The kernel
/// should emit a self-repair obligation until the projection is re-synced.
pub fn projection_gap(goal: &Goal) -> Option<String> {
    let next_action = goal
        .next_action
        .as_deref()
        .filter(|na| !na.trim().is_empty())?;
    if next_action.contains("complete; no further") {
        return None;
    }
    let agent_open = goal.open_of(crate::state::TaskClass::Advancement).count();
    let user_open = goal.open_gates().count();
    if agent_open == 0 && !next_action.starts_with("[P1]") && !next_action.contains("waiting") {
        return Some(format!(
            "active-state Next Action `{next_action}` has no matching open agent todo"
        ));
    }
    if user_open == 0 && next_action.to_lowercase().contains("decide") {
        return Some(format!(
            "active-state Next Action `{next_action}` waits on a decision with no open user gate"
        ));
    }
    None
}

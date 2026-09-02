//! G12 multi_agent subdomain — the reference
//! `control_plane/agents/multi_agent/` package (contract.py, recipe.py,
//! role_successor.py, collective_round_ledger.py, visible_wake_scheduler.py),
//! natively (core subset).
//!
//! The contract is the single declarative surface for a goal's multi-agent
//! topology: peers (with backup/succession edges), handoff rules, and named
//! collectives. Everything else is a PROJECTION over the event ledger —
//! the goal state itself is untouched (same rule as the G-16 supervisor):
//!
//! - [`MultiAgentContract`] + validation → `MultiAgentContractSet` event;
//! - [`AgentRecipe`] (named capabilities/workspaces/priority) →
//!   `AgentRecipeAdded` event, applied by `agent onboard --recipe`;
//! - role succession: a primary whose live lease expired or whose scheduler
//!   heartbeat went silent past the threshold is succeeded by its declared
//!   backup → `SuccessionOccurred` event + attention hint;
//! - [`wake_roster`]: the round-robin wake order inside a collective;
//! - [`collective_turn_ledger`]: per-collective turn counts derived from
//!   claim events (a claim = one bounded turn opportunity, the same unit the
//!   reference collective-round ledger counts).

use std::collections::{BTreeMap, HashSet};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::state::{now_epoch, Goal, Priority, TodoStatus};
use crate::store::{Event, Store};
use crate::work_items::attention::AttentionItem;

pub const MULTI_AGENT_CONTRACT_SCHEMA_VERSION: &str = "multi_agent_contract_v0";
pub const MULTI_AGENT_RECIPE_SCHEMA_VERSION: &str = "multi_agent_recipe_v0";
pub const ROLE_SUCCESSOR_PROJECTION_SCHEMA_VERSION: &str = "multi_agent_role_succession_v0";
pub const WAKE_ROSTER_SCHEMA_VERSION: &str = "multi_agent_wake_roster_v0";
pub const COLLECTIVE_TURN_LEDGER_SCHEMA_VERSION: &str = "multi_agent_collective_turn_ledger_v0";

/// Succession reasons (reference role_successor trigger vocabulary).
pub const SUCCESSION_REASON_LEASE_EXPIRED: &str = "lease_expired";
pub const SUCCESSION_REASON_OFFLINE: &str = "offline";

/// Default offline threshold before a backup may succeed a primary
/// (30 minutes; the automation-liveness threshold is deliberately larger).
pub const DEFAULT_SUCCESSOR_OFFLINE_THRESHOLD_SECS: u64 = 30 * 60;

/// Effective successor offline threshold. The `FUTURE_LOOP_SUCCESSOR_OFFLINE_SECS`
/// env var shrinks it in tests (mirrors the FUTURE_LOOP_NO_PROGRESS_SECS
/// hook); invalid or non-positive values fall back to the 30-minute default.
pub fn successor_offline_threshold_secs() -> u64 {
    std::env::var("FUTURE_LOOP_SUCCESSOR_OFFLINE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(DEFAULT_SUCCESSOR_OFFLINE_THRESHOLD_SECS)
}

// ── Multi-agent contract ───────────────────────────────────────────────────

/// One peer's role inside the contract. `backup_for` names the primary agent
/// this peer succeeds when the primary dies (None = this peer is a primary
/// itself, or standalone). In this native single-process subset a role maps
/// 1:1 to an agent id, so `handoff_rules.to_role` resolves to a peer agent.
/// `capabilities` is descriptive metadata only (the capability framework
/// and its gate were removed); `workspaces` feeds the workspace guard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRole {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_for: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub workspaces: Vec<String>,
}

/// One event→role handoff rule (reference handoff_hints, compact): when
/// `from_event` occurs (e.g. `lease_expired`, `todo_completed`), the wake
/// order favors `to_role` (a contract peer agent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffRule {
    pub from_event: String,
    pub to_role: String,
}

/// The declarative multi-agent topology for one goal. `collectives` are
/// named peer groups — the scope of the wake roster and the collective turn
/// ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiAgentContract {
    pub schema_version: String,
    #[serde(default)]
    pub peers: BTreeMap<String, PeerRole>,
    #[serde(default)]
    pub handoff_rules: Vec<HandoffRule>,
    #[serde(default)]
    pub collectives: BTreeMap<String, Vec<String>>,
}

impl Default for MultiAgentContract {
    fn default() -> Self {
        Self {
            schema_version: MULTI_AGENT_CONTRACT_SCHEMA_VERSION.to_string(),
            peers: BTreeMap::new(),
            handoff_rules: vec![],
            collectives: BTreeMap::new(),
        }
    }
}

impl MultiAgentContract {
    /// The backup agent declared for `primary` (None when undeclared).
    pub fn backup_of(&self, primary: &str) -> Option<&str> {
        self.peers
            .iter()
            .find(|(_, role)| role.backup_for.as_deref() == Some(primary))
            .map(|(agent, _)| agent.as_str())
    }

    /// Members of a collective (contract order; unknown → None).
    pub fn collective_members(&self, collective: &str) -> Option<&Vec<String>> {
        self.collectives.get(collective)
    }

    /// The collective an agent belongs to (None = no collective).
    pub fn collective_of(&self, agent_id: &str) -> Option<&str> {
        self.collectives
            .iter()
            .find(|(_, members)| members.iter().any(|m| m == agent_id))
            .map(|(name, _)| name.as_str())
    }

    /// The first handoff rule matching `from_event` → its target peer.
    pub fn handoff_target(&self, from_event: &str) -> Option<&str> {
        self.handoff_rules
            .iter()
            .find(|r| r.from_event == from_event)
            .map(|r| r.to_role.as_str())
    }
}

/// Validate a contract (reference: contracts fail closed — a topology that
/// cannot be trusted must not be recorded). Returns every issue found
/// (empty = valid).
pub fn contract_issues(contract: &MultiAgentContract) -> Vec<String> {
    let mut issues = vec![];
    if contract.schema_version != MULTI_AGENT_CONTRACT_SCHEMA_VERSION {
        issues.push(format!(
            "schema_version must be `{MULTI_AGENT_CONTRACT_SCHEMA_VERSION}` (got `{}`)",
            contract.schema_version
        ));
    }
    for (id, role) in &contract.peers {
        if id.trim().is_empty() {
            issues.push("peer agent id must be non-empty".to_string());
        }
        if let Some(target) = &role.backup_for {
            if target.trim().is_empty() {
                issues.push(format!("peer `{id}`: backup_for must be non-empty"));
            } else if target == id {
                issues.push(format!("peer `{id}` cannot back up itself"));
            } else if !contract.peers.contains_key(target) {
                issues.push(format!(
                    "peer `{id}`: backup_for `{target}` is not a contract peer"
                ));
            }
        }
    }
    // Backup chains must be acyclic (a cycle would make succession
    // oscillate forever).
    for id in contract.peers.keys() {
        let mut seen: HashSet<&str> = HashSet::from([id.as_str()]);
        let mut cur = id.as_str();
        while let Some(next) = contract
            .peers
            .get(cur)
            .and_then(|role| role.backup_for.as_deref())
        {
            if !seen.insert(next) {
                issues.push(format!("backup chain cycle involving `{next}`"));
                break;
            }
            if !contract.peers.contains_key(next) {
                break; // unknown target already reported above
            }
            cur = next;
        }
    }
    let mut seen_rules: HashSet<(String, String)> = HashSet::new();
    for rule in &contract.handoff_rules {
        if rule.from_event.trim().is_empty() {
            issues.push("handoff rule: from_event must be non-empty".to_string());
        }
        if rule.to_role.trim().is_empty() {
            issues.push("handoff rule: to_role must be non-empty".to_string());
        } else if !contract.peers.contains_key(&rule.to_role) {
            issues.push(format!(
                "handoff rule: to_role `{}` is not a contract peer",
                rule.to_role
            ));
        }
        if !seen_rules.insert((rule.from_event.clone(), rule.to_role.clone())) {
            issues.push(format!(
                "duplicate handoff rule `{}` → `{}`",
                rule.from_event, rule.to_role
            ));
        }
    }
    let mut membership: HashSet<&str> = HashSet::new();
    for (name, members) in &contract.collectives {
        if name.trim().is_empty() {
            issues.push("collective name must be non-empty".to_string());
        }
        if members.is_empty() {
            issues.push(format!("collective `{name}` must have at least one member"));
        }
        let mut local: HashSet<&str> = HashSet::new();
        for m in members {
            if !contract.peers.contains_key(m) {
                issues.push(format!(
                    "collective `{name}`: member `{m}` is not a contract peer"
                ));
            } else if !local.insert(m.as_str()) {
                issues.push(format!("collective `{name}`: duplicate member `{m}`"));
            } else if !membership.insert(m.as_str()) {
                issues.push(format!("agent `{m}` appears in more than one collective"));
            }
        }
    }
    issues
}

/// Record a contract set (full replace — the latest event wins). Validation
/// fails closed: an invalid contract is never appended.
pub fn record_contract(
    store: &mut Store,
    goal_id: &str,
    contract: &MultiAgentContract,
) -> Result<String> {
    let issues = contract_issues(contract);
    if !issues.is_empty() {
        bail!("invalid multi-agent contract: {}", issues.join("; "));
    }
    store.append(Event::MultiAgentContractSet {
        goal_id: goal_id.to_string(),
        contract: contract.clone(),
        ts: now_epoch(),
    })
}

/// The latest recorded contract (None when none was ever set).
pub fn latest_contract(store: &Store, goal_id: &str) -> Result<Option<MultiAgentContract>> {
    let events = store.events(goal_id)?;
    let mut contract: Option<MultiAgentContract> = None;
    for stored in &events {
        if let Event::MultiAgentContractSet {
            goal_id: g,
            contract: c,
            ..
        } = &stored.event
        {
            if g == goal_id {
                contract = Some(c.clone());
            }
        }
    }
    Ok(contract)
}

// ── Agent recipes ──────────────────────────────────────────────────────────

/// A named onboarding recipe: capabilities + workspaces + the default
/// priority for work this agent claims (LoopX minimal-recipe idea, native:
/// the reusable part is the declarative profile, not process mechanics).
/// `capabilities` is kept as a descriptive string list — nothing consumes
/// it as a runnability gate anymore; `workspaces` feeds the workspace
/// guard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRecipe {
    pub schema_version: String,
    pub name: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub workspaces: Vec<String>,
    #[serde(default)]
    pub priority: Priority,
}

/// Validate a recipe (fail closed — a recipe is a reusable onboarding
/// profile, so it must be well-formed before it can be applied).
pub fn recipe_issues(recipe: &AgentRecipe) -> Vec<String> {
    let mut issues = vec![];
    if recipe.schema_version != MULTI_AGENT_RECIPE_SCHEMA_VERSION {
        issues.push(format!(
            "schema_version must be `{MULTI_AGENT_RECIPE_SCHEMA_VERSION}` (got `{}`)",
            recipe.schema_version
        ));
    }
    if recipe.name.trim().is_empty() {
        issues.push("recipe name must be non-empty".to_string());
    }
    issues
}

/// Record a recipe (`AgentRecipeAdded`). Re-adding a name is allowed —
/// lookups resolve the latest event (latest wins).
pub fn record_recipe(store: &mut Store, goal_id: &str, recipe: &AgentRecipe) -> Result<String> {
    let issues = recipe_issues(recipe);
    if !issues.is_empty() {
        bail!("invalid agent recipe: {}", issues.join("; "));
    }
    store.append(Event::AgentRecipeAdded {
        goal_id: goal_id.to_string(),
        recipe: recipe.clone(),
        ts: now_epoch(),
    })
}

/// All recorded recipes in ledger order.
pub fn recipes(store: &Store, goal_id: &str) -> Result<Vec<AgentRecipe>> {
    let events = store.events(goal_id)?;
    let mut out = vec![];
    for stored in &events {
        if let Event::AgentRecipeAdded {
            goal_id: g, recipe, ..
        } = &stored.event
        {
            if g == goal_id {
                out.push(recipe.clone());
            }
        }
    }
    Ok(out)
}

/// The latest recipe under `name` (None when unknown).
pub fn recipe_named(store: &Store, goal_id: &str, name: &str) -> Result<Option<AgentRecipe>> {
    Ok(recipes(store, goal_id)?
        .into_iter()
        .rfind(|r| r.name == name))
}

/// Onboard `agent_id` with a recipe (capabilities + workspaces land on the
/// same `AgentOnboarded` event the explicit onboard path uses — capabilities
/// stay descriptive; the workspace guard consumes `workspaces`).
pub fn apply_recipe_onboard(
    store: &mut Store,
    goal_id: &str,
    agent_id: &str,
    recipe: &AgentRecipe,
) -> Result<String> {
    store.append(Event::AgentOnboarded {
        goal_id: goal_id.to_string(),
        agent_id: agent_id.to_string(),
        capabilities: recipe.capabilities.clone(),
        workspaces: recipe.workspaces.clone(),
        ts: now_epoch(),
    })
}

// ── Role succession ────────────────────────────────────────────────────────

/// One succession trigger detected from current state (not yet recorded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuccessionCandidate {
    /// The role slot being succeeded (= the primary agent id).
    pub role: String,
    pub primary: String,
    pub backup: String,
    /// `lease_expired` | `offline`.
    pub reason: String,
    /// When the trigger state was observed: the expired lease timestamp for
    /// `lease_expired`, the last heartbeat for `offline`.
    pub since: u64,
}

/// One recorded succession (read back from the ledger).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuccessionRecord {
    pub primary: String,
    pub backup: String,
    pub reason: String,
    pub ts: u64,
    pub event_id: String,
}

/// Detect succession triggers against current goal state. A primary is
/// succeeded when (a) it holds a live-lease todo whose lease has expired
/// (stalled mid-slice), or (b) its scheduler heartbeat went silent past the
/// offline threshold. Heartbeat-less primaries are NOT presumed offline
/// (no "first seen" anchor) — they fall back to lease-expiry detection.
pub fn succession_candidates_with(
    goal: &Goal,
    contract: &MultiAgentContract,
    now: u64,
    offline_threshold_secs: u64,
) -> Vec<SuccessionCandidate> {
    let mut out = vec![];
    for (backup, role) in &contract.peers {
        let Some(primary) = role.backup_for.as_deref() else {
            continue;
        };
        if primary == backup || primary.trim().is_empty() {
            continue; // validation rejects these; defensive here
        }
        // The primary must actually be part of this goal's automation.
        if !goal.registered_agents.iter().any(|a| a == primary) {
            continue;
        }
        let expired_lease = goal
            .todos
            .iter()
            .filter(|t| t.claimed_by.as_deref() == Some(primary) && t.status == TodoStatus::Open)
            .filter_map(|t| t.lease_expires_at)
            .filter(|e| *e <= now)
            .max();
        if let Some(expired) = expired_lease {
            out.push(SuccessionCandidate {
                role: primary.to_string(),
                primary: primary.to_string(),
                backup: backup.clone(),
                reason: SUCCESSION_REASON_LEASE_EXPIRED.to_string(),
                since: expired,
            });
            continue;
        }
        if let Some(last_hb) = goal.scheduler_heartbeats.get(primary) {
            if now.saturating_sub(*last_hb) >= offline_threshold_secs {
                out.push(SuccessionCandidate {
                    role: primary.to_string(),
                    primary: primary.to_string(),
                    backup: backup.clone(),
                    reason: SUCCESSION_REASON_OFFLINE.to_string(),
                    since: *last_hb,
                });
            }
        }
    }
    out
}

/// [`succession_candidates_with`] using the effective threshold
/// (env-overridable, see [`successor_offline_threshold_secs`]).
pub fn succession_candidates(
    goal: &Goal,
    contract: &MultiAgentContract,
    now: u64,
) -> Vec<SuccessionCandidate> {
    succession_candidates_with(goal, contract, now, successor_offline_threshold_secs())
}

/// Record a succession (`SuccessionOccurred`). Idempotent per trigger
/// episode: re-recording the same (primary, backup, reason) returns the
/// existing event id instead of appending a duplicate.
pub fn record_succession(
    store: &mut Store,
    goal_id: &str,
    candidate: &SuccessionCandidate,
) -> Result<String> {
    if let Some(existing) = successions(store, goal_id)?.into_iter().find(|r| {
        r.primary == candidate.primary
            && r.backup == candidate.backup
            && r.reason == candidate.reason
    }) {
        return Ok(existing.event_id);
    }
    store.append(Event::SuccessionOccurred {
        goal_id: goal_id.to_string(),
        primary: candidate.primary.clone(),
        backup: candidate.backup.clone(),
        reason: candidate.reason.clone(),
        ts: now_epoch(),
    })
}

/// Recorded successions in ledger order.
pub fn successions(store: &Store, goal_id: &str) -> Result<Vec<SuccessionRecord>> {
    let events = store.events(goal_id)?;
    let mut out = vec![];
    for stored in &events {
        if let Event::SuccessionOccurred {
            goal_id: g,
            primary,
            backup,
            reason,
            ts,
        } = &stored.event
        {
            if g == goal_id {
                out.push(SuccessionRecord {
                    primary: primary.clone(),
                    backup: backup.clone(),
                    reason: reason.clone(),
                    ts: *ts,
                    event_id: stored.effective_id(),
                });
            }
        }
    }
    Ok(out)
}

/// Auto-promote every currently-met succession trigger for `goal_id` by
/// recording its `SuccessionOccurred` event (the declarative contract says a
/// primary whose lease expired or whose heartbeat went silent is succeeded
/// by its `backup_for` peer — the scheduler tick drives this periodically, so
/// promotion happens without an explicit `agent succession apply`). Idempotent
/// per trigger episode: re-recording the same (primary, backup, reason)
/// returns the existing event id (see [`record_succession`]). Returns the
/// succession records that landed (empty when no trigger is met, the goal is
/// gone, or no contract is set).
pub fn auto_promote_successions(
    store: &mut Store,
    goal_id: &str,
    now: u64,
) -> Result<Vec<SuccessionRecord>> {
    let Some(goal) = store.replay(goal_id)? else {
        return Ok(vec![]);
    };
    let Some(contract) = latest_contract(store, goal_id)? else {
        return Ok(vec![]);
    };
    let candidates = succession_candidates(&goal, &contract, now);
    for candidate in &candidates {
        record_succession(store, goal_id, candidate)?;
    }
    if candidates.is_empty() {
        return Ok(vec![]);
    }
    // Re-read the ledger and return only the episodes we just promoted, so the
    // caller sees exactly what landed (already-recorded episodes are returned
    // with their existing event id, not a duplicate).
    let promoted = successions(store, goal_id)?
        .into_iter()
        .filter(|r| {
            candidates
                .iter()
                .any(|c| c.primary == r.primary && c.backup == r.backup && c.reason == r.reason)
        })
        .collect();
    Ok(promoted)
}

/// Attention hints for recorded successions — one item per role slot
/// (latest succession per primary wins). A succession is considered
/// recovered once the primary's scheduler heartbeat lands at or after the
/// succession event (the primary proved it is alive again), which
/// suppresses the item.
pub fn succession_attention_items(store: &Store, goal: &Goal) -> Result<Vec<AttentionItem>> {
    let mut items = vec![];
    let mut latest_per_primary: BTreeMap<String, SuccessionRecord> = BTreeMap::new();
    for r in successions(store, &goal.goal_id)? {
        latest_per_primary.insert(r.primary.clone(), r);
    }
    for (primary, record) in latest_per_primary {
        let recovered = goal
            .scheduler_heartbeats
            .get(&primary)
            .is_some_and(|hb| *hb >= record.ts);
        if recovered {
            continue;
        }
        items.push(AttentionItem {
            goal_id: goal.goal_id.clone(),
            status: "role_succession".to_string(),
            waiting_on: "user_or_controller".to_string(),
            severity: "high".to_string(),
            recommended_action: format!(
                "primary `{primary}` → backup `{}` promoted ({}) — verify/restore the primary or make the promotion permanent",
                record.backup, record.reason
            ),
            source: "role_succession".to_string(),
        });
    }
    Ok(items)
}

// ── Wake roster ────────────────────────────────────────────────────────────

/// The round-robin wake order inside a collective (reference
/// visible_wake_scheduler, native projection): agents wake in contract
/// order, rotated by the collective's completed turn count. `order[0]`
/// (`current`) wakes next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WakeRoster {
    pub schema_version: String,
    pub collective: String,
    /// Completed collective turns this roster is projected for.
    pub turn: u32,
    /// Wake order for the NEXT turn (rotated).
    pub order: Vec<String>,
    /// Who wakes first (= `order[0]`).
    pub current: String,
}

/// Project the wake roster for a collective. `turn` completed turns rotate
/// the order by `turn % member_count` (deterministic — contract order is
/// BTreeMap order). None for an unknown/empty collective.
pub fn wake_roster(
    contract: &MultiAgentContract,
    collective: &str,
    turn: u32,
) -> Option<WakeRoster> {
    let members = contract.collective_members(collective)?;
    if members.is_empty() {
        return None;
    }
    let mut order = members.clone();
    let len = order.len();
    order.rotate_left((turn as usize) % len);
    Some(WakeRoster {
        schema_version: WAKE_ROSTER_SCHEMA_VERSION.to_string(),
        collective: collective.to_string(),
        turn,
        current: order[0].clone(),
        order,
    })
}

// ── Collective turn ledger ─────────────────────────────────────────────────

/// One agent's turn count inside a collective (a claim = one bounded turn
/// opportunity — the same unit the reference collective-round ledger
/// counts as an asynchronous turn).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CollectiveTurnEntry {
    pub agent_id: String,
    pub turns: u32,
    pub last_turn_ts: Option<u64>,
}

/// Per-collective turn counts, derived from `TodoClaimed` events (a claim
/// precedes every bounded turn). `full_participation_rounds` is the min
/// count across members — the asynchronous full-participation basis of the
/// reference ledger (0 until every member has claimed at least once).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CollectiveTurnLedger {
    pub schema_version: String,
    pub collective: String,
    pub agents: Vec<String>,
    pub per_agent: BTreeMap<String, CollectiveTurnEntry>,
    pub full_participation_rounds: u32,
    pub total_claims: u32,
}

/// Project the turn ledger for one collective. None for an unknown or
/// empty collective.
pub fn collective_turn_ledger(
    store: &Store,
    goal_id: &str,
    contract: &MultiAgentContract,
    collective: &str,
) -> Result<Option<CollectiveTurnLedger>> {
    let Some(members) = contract.collective_members(collective) else {
        return Ok(None);
    };
    if members.is_empty() {
        return Ok(None);
    }
    let mut per_agent: BTreeMap<String, CollectiveTurnEntry> = members
        .iter()
        .map(|a| {
            (
                a.clone(),
                CollectiveTurnEntry {
                    agent_id: a.clone(),
                    turns: 0,
                    last_turn_ts: None,
                },
            )
        })
        .collect();
    let mut total = 0u32;
    for stored in store.events(goal_id)? {
        if let Event::TodoClaimed {
            goal_id: g,
            agent_id,
            ts,
            ..
        } = &stored.event
        {
            if g == goal_id {
                if let Some(entry) = per_agent.get_mut(agent_id) {
                    entry.turns += 1;
                    entry.last_turn_ts = Some(entry.last_turn_ts.map_or(*ts, |t| t.max(*ts)));
                    total += 1;
                }
            }
        }
    }
    let full = members
        .iter()
        .map(|a| per_agent.get(a).map(|e| e.turns).unwrap_or(0))
        .min()
        .unwrap_or(0);
    Ok(Some(CollectiveTurnLedger {
        schema_version: COLLECTIVE_TURN_LEDGER_SCHEMA_VERSION.to_string(),
        collective: collective.to_string(),
        agents: members.clone(),
        per_agent,
        full_participation_rounds: full,
        total_claims: total,
    }))
}

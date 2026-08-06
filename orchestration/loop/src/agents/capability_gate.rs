//! Capability gate (G-24) — LoopX `control_plane/agents/capability_gate.py`,
//! natively. Binds agent-scope capability availability to todo runnability:
//! a todo is runnable for an agent only when every `required_capability` is
//! available (declared by the agent session or a default runtime capability).
//!
//! Missing capabilities resolve into an owner-classified action:
//! - owner-held (`credentials` / `production_access`) → `ask_owner`
//! - repair bridges (`benchmark_runner` / `network` / `external_evidence_poll`
//!   / `worker_bridge` / `cli_bridge`) → `repair_bridge`
//! - anything else → `skip` (unsupported)
//!
//! The gate never blocks the kernel decision; it projects the runnable /
//! blocked candidate split for the agent lane.

use crate::state::Todo;

pub const CAPABILITY_GATE_SCHEMA_VERSION: &str = "capability_gate_v0";

/// LoopX DEFAULT_AVAILABLE_CAPABILITIES.
pub const DEFAULT_AVAILABLE_CAPABILITIES: [&str; 3] =
    ["shell", "filesystem_read", "filesystem_write"];

/// Owner-held capability hints: resolution is a user decision.
pub const CAPABILITY_OWNER_GATE_HINTS: [&str; 2] = ["credentials", "production_access"];

/// Repair-bridge capability hints: the agent can repair the bridge.
pub const CAPABILITY_REPAIR_BRIDGE_HINTS: [&str; 5] = [
    "benchmark_runner",
    "network",
    "external_evidence_poll",
    "worker_bridge",
    "cli_bridge",
];

/// Gate resolution action (LoopX `_capability_missing_action`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityAction {
    Run,
    AskOwner,
    RepairBridge,
    Skip,
}

impl CapabilityAction {
    pub fn label(&self) -> &'static str {
        match self {
            CapabilityAction::Run => "run",
            CapabilityAction::AskOwner => "ask_owner",
            CapabilityAction::RepairBridge => "repair_bridge",
            CapabilityAction::Skip => "skip",
        }
    }
}

/// The decision owner for a resolution (LoopX `decision_owner`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityDecisionOwner {
    User,
    Agent,
    CapabilityGate,
}

impl CapabilityDecisionOwner {
    pub fn label(&self) -> &'static str {
        match self {
            CapabilityDecisionOwner::User => "user",
            CapabilityDecisionOwner::Agent => "agent",
            CapabilityDecisionOwner::CapabilityGate => "capability_gate",
        }
    }
}

/// One blocked candidate binding (LoopX `_blocked_capability_resolution_bindings`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CapabilityResolutionBinding {
    pub owner: String,
    pub action: String,
    pub capability: String,
    pub blocked_todo_ids: Vec<String>,
}

/// The runnable/blocked candidate split (LoopX build_capability_gate).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CapabilityGate {
    pub schema_version: String,
    pub source: String,
    pub available: Vec<String>,
    pub runnable_todo_ids: Vec<String>,
    pub blocked_todo_ids: Vec<String>,
    pub missing: Vec<String>,
    pub action: String,
    pub decision_owner: String,
    pub owner_missing: Vec<String>,
    pub repair_missing: Vec<String>,
    pub unsupported_missing: Vec<String>,
    pub resolution_bindings: Vec<CapabilityResolutionBinding>,
    pub blocks_delivery: bool,
    pub reason: String,
}

/// Available capabilities = runtime defaults + agent-declared capabilities
/// (LoopX available_capabilities_with_defaults).
pub fn available_capabilities_with_defaults(declared: &[String]) -> Vec<String> {
    let mut result: Vec<String> = DEFAULT_AVAILABLE_CAPABILITIES
        .iter()
        .map(|s| s.to_string())
        .collect();
    for capability in declared {
        if !result.contains(capability) {
            result.push(capability.clone());
        }
    }
    result
}

/// Missing required capabilities for one todo (target capabilities satisfy
/// requirements; LoopX missing_required_capabilities).
pub fn missing_required_capabilities(todo: &Todo, available: &[String]) -> Vec<String> {
    let Some(required) = todo.required_capability.as_deref() else {
        return vec![];
    };
    if available.iter().any(|c| c == required) {
        return vec![];
    }
    vec![required.to_string()]
}

fn classify(capability: &str) -> (CapabilityAction, CapabilityDecisionOwner) {
    if CAPABILITY_OWNER_GATE_HINTS.contains(&capability) {
        (CapabilityAction::AskOwner, CapabilityDecisionOwner::User)
    } else if CAPABILITY_REPAIR_BRIDGE_HINTS.contains(&capability) {
        (
            CapabilityAction::RepairBridge,
            CapabilityDecisionOwner::Agent,
        )
    } else {
        (
            CapabilityAction::Skip,
            CapabilityDecisionOwner::CapabilityGate,
        )
    }
}

/// Project the capability gate for one goal's open advancement todos.
/// `None` when no todo declares a required capability and nothing is blocked
/// (LoopX: gate absent when there is no capability signal).
pub fn build_capability_gate(
    todos: &[Todo],
    declared_capabilities: &[String],
) -> Option<CapabilityGate> {
    let available = available_capabilities_with_defaults(declared_capabilities);
    let candidates: Vec<&Todo> = todos
        .iter()
        .filter(|t| {
            t.class == crate::state::TaskClass::Advancement
                && t.status == crate::state::TodoStatus::Open
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let mut runnable: Vec<String> = vec![];
    let mut blocked: Vec<(String, Vec<String>)> = vec![];
    let mut saw_requirement = false;
    for todo in candidates {
        let missing = missing_required_capabilities(todo, &available);
        if todo.required_capability.is_some() {
            saw_requirement = true;
        }
        if missing.is_empty() {
            runnable.push(todo.id.clone());
        } else {
            blocked.push((todo.id.clone(), missing));
        }
    }
    if !saw_requirement && blocked.is_empty() {
        return None;
    }

    let mut missing_all: Vec<String> = vec![];
    let mut owner_missing: Vec<String> = vec![];
    let mut repair_missing: Vec<String> = vec![];
    let mut unsupported_missing: Vec<String> = vec![];
    let mut bindings: Vec<CapabilityResolutionBinding> = vec![];
    let mut actions: Vec<CapabilityAction> = vec![];
    for (todo_id, missing) in &blocked {
        for capability in missing {
            if !missing_all.contains(capability) {
                missing_all.push(capability.clone());
            }
            let (action, owner) = classify(capability);
            if !actions.contains(&action) {
                actions.push(action);
            }
            match owner {
                CapabilityDecisionOwner::User => {
                    if !owner_missing.contains(capability) {
                        owner_missing.push(capability.clone());
                    }
                }
                CapabilityDecisionOwner::Agent => {
                    if !repair_missing.contains(capability) {
                        repair_missing.push(capability.clone());
                    }
                }
                CapabilityDecisionOwner::CapabilityGate => {
                    if !unsupported_missing.contains(capability) {
                        unsupported_missing.push(capability.clone());
                    }
                }
            }
            let binding = bindings
                .iter_mut()
                .find(|b| b.capability == *capability && b.owner == owner.label());
            match binding {
                Some(b) => {
                    if !b.blocked_todo_ids.contains(todo_id) {
                        b.blocked_todo_ids.push(todo_id.clone());
                    }
                }
                None => bindings.push(CapabilityResolutionBinding {
                    owner: owner.label().to_string(),
                    action: action.label().to_string(),
                    capability: capability.clone(),
                    blocked_todo_ids: vec![todo_id.clone()],
                }),
            }
        }
    }
    let action = if runnable.is_empty() && !blocked.is_empty() {
        // All visible candidates are blocked → the gate decides the resolution
        // action from the first missing capability (LoopX resolution).
        if owner_missing
            .iter()
            .any(|c| CAPABILITY_OWNER_GATE_HINTS.contains(&c.as_str()))
        {
            CapabilityAction::AskOwner
        } else if repair_missing
            .iter()
            .any(|c| CAPABILITY_REPAIR_BRIDGE_HINTS.contains(&c.as_str()))
        {
            CapabilityAction::RepairBridge
        } else {
            CapabilityAction::Skip
        }
    } else {
        CapabilityAction::Run
    };
    let decision_owner = match action {
        CapabilityAction::AskOwner => CapabilityDecisionOwner::User,
        CapabilityAction::RepairBridge => CapabilityDecisionOwner::Agent,
        _ => CapabilityDecisionOwner::CapabilityGate,
    };
    let blocks_delivery = runnable.is_empty() && !blocked.is_empty();
    let reason = if blocked.is_empty() {
        "capability gate projected runnable candidate set; agent chooses the actual todo"
            .to_string()
    } else if runnable.is_empty() {
        "all visible executable todo candidates require unavailable capabilities".to_string()
    } else {
        "some candidates blocked by unavailable capabilities; runnable candidates remain"
            .to_string()
    };
    let runnable_todo_ids = runnable;
    let blocked_todo_ids: Vec<String> = blocked.iter().map(|(id, _)| id.clone()).collect();
    Some(CapabilityGate {
        schema_version: CAPABILITY_GATE_SCHEMA_VERSION.to_string(),
        source: "agent_todo_summary.executable_backlog_items".to_string(),
        available,
        runnable_todo_ids,
        blocked_todo_ids,
        missing: missing_all,
        action: action.label().to_string(),
        decision_owner: decision_owner.label().to_string(),
        owner_missing,
        repair_missing,
        unsupported_missing,
        resolution_bindings: bindings,
        blocks_delivery,
        reason,
    })
}

/// Gate a single todo's required capability against a declared capability
/// set (used by agent scope lane selection).
pub fn todo_is_runnable(todo: &Todo, declared_capabilities: &[String]) -> bool {
    let available = available_capabilities_with_defaults(declared_capabilities);
    missing_required_capabilities(todo, &available).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Todo;

    #[test]
    fn no_requirement_no_gate() {
        let todos = vec![Todo::advancement("t1", "work")];
        assert!(build_capability_gate(&todos, &[]).is_none());
    }

    #[test]
    fn runnable_when_capability_available() {
        let mut t = Todo::advancement("t1", "fix");
        t.required_capability = Some("github".into());
        let gate = build_capability_gate(&[t.clone()], &["github".into()]).unwrap();
        assert_eq!(gate.runnable_todo_ids, vec!["t1"]);
        assert!(gate.blocked_todo_ids.is_empty());
        assert_eq!(gate.action, "run");
    }

    #[test]
    fn owner_held_capability_asks_owner() {
        let mut t = Todo::advancement("t1", "deploy");
        t.required_capability = Some("production_access".into());
        let gate = build_capability_gate(&[t], &[]).unwrap();
        assert!(gate.runnable_todo_ids.is_empty());
        assert_eq!(gate.blocked_todo_ids, vec!["t1"]);
        assert_eq!(gate.action, "ask_owner");
        assert_eq!(gate.decision_owner, "user");
        assert!(gate.blocks_delivery);
        assert_eq!(gate.owner_missing, vec!["production_access"]);
    }

    #[test]
    fn repair_bridge_capability_routes_to_agent() {
        let mut t = Todo::advancement("t1", "poll");
        t.required_capability = Some("network".into());
        let gate = build_capability_gate(&[t], &[]).unwrap();
        assert_eq!(gate.action, "repair_bridge");
        assert_eq!(gate.decision_owner, "agent");
    }

    #[test]
    fn unknown_capability_is_unsupported() {
        let mut t = Todo::advancement("t1", "x");
        t.required_capability = Some("quantum".into());
        let gate = build_capability_gate(&[t], &[]).unwrap();
        assert_eq!(gate.action, "skip");
        assert_eq!(gate.decision_owner, "capability_gate");
        assert_eq!(gate.unsupported_missing, vec!["quantum"]);
    }

    #[test]
    fn default_capabilities_always_available() {
        let t = Todo::advancement("t1", "shell work");
        let mut with_requirement = t.clone();
        with_requirement.required_capability = Some("shell".into());
        let gate = build_capability_gate(&[with_requirement], &[]).unwrap();
        assert!(gate.runnable_todo_ids.contains(&"t1".to_string()));
        assert!(gate.missing.is_empty());
    }
}

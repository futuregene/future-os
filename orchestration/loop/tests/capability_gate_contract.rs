//! G-24 capability-gate contract tests: the gate binds agent-scope
//! capability availability to todo runnability (run / ask_owner /
//! repair_bridge / skip resolutions), and per-capability command hooks
//! register through the CLI registry with the catalog status gate.

use future_loop::agents::capability_gate::{
    build_capability_gate, missing_required_capabilities, todo_is_runnable, CapabilityAction,
};
use future_loop::capabilities::catalog::CapabilityCatalog;
use future_loop::cli::registry::CommandRegistry;
use future_loop::state::Todo;

fn todo_requiring(capability: &str) -> Todo {
    let mut t = Todo::advancement("t1", "work");
    t.required_capability = Some(capability.to_string());
    t
}

#[test]
fn gate_absent_without_capability_signal() {
    let todos = vec![Todo::advancement("t1", "plain work")];
    assert!(build_capability_gate(&todos, &[]).is_none());
}

#[test]
fn runnable_when_requirement_is_available() {
    let gate = build_capability_gate(&[todo_requiring("github")], &["github".into()]).unwrap();
    assert_eq!(gate.runnable_todo_ids, vec!["t1"]);
    assert!(gate.blocked_todo_ids.is_empty());
    assert_eq!(gate.action, CapabilityAction::Run.label());
    assert!(!gate.blocks_delivery);
    assert!(
        missing_required_capabilities(&todo_requiring("github"), &["github".into()]).is_empty()
    );
}

#[test]
fn owner_held_capability_resolves_to_ask_owner() {
    let gate = build_capability_gate(&[todo_requiring("production_access")], &[]).unwrap();
    assert_eq!(gate.action, "ask_owner");
    assert_eq!(gate.decision_owner, "user");
    assert!(gate.blocks_delivery);
    assert_eq!(gate.owner_missing, vec!["production_access"]);
    assert_eq!(gate.resolution_bindings.len(), 1);
    assert_eq!(gate.resolution_bindings[0].owner, "user");
    assert_eq!(gate.resolution_bindings[0].blocked_todo_ids, vec!["t1"]);
}

#[test]
fn repair_bridge_capability_resolves_to_agent() {
    let gate = build_capability_gate(&[todo_requiring("network")], &[]).unwrap();
    assert_eq!(gate.action, "repair_bridge");
    assert_eq!(gate.decision_owner, "agent");
    assert_eq!(gate.repair_missing, vec!["network"]);
}

#[test]
fn unsupported_capability_resolves_to_skip() {
    let gate = build_capability_gate(&[todo_requiring("quantum_teleport")], &[]).unwrap();
    assert_eq!(gate.action, "skip");
    assert_eq!(gate.decision_owner, "capability_gate");
    assert_eq!(gate.unsupported_missing, vec!["quantum_teleport"]);
}

#[test]
fn default_runtime_capabilities_always_available() {
    assert!(todo_is_runnable(&todo_requiring("shell"), &[]));
    assert!(todo_is_runnable(&todo_requiring("filesystem_read"), &[]));
    assert!(!todo_is_runnable(&todo_requiring("github"), &[]));
    assert!(todo_is_runnable(
        &todo_requiring("github"),
        &["github".into()]
    ));
}

#[test]
fn mixed_candidates_split_runnable_and_blocked() {
    let mut t2 = Todo::advancement("t2", "needs credentials");
    t2.required_capability = Some("credentials".into());
    let todos = vec![Todo::advancement("t1", "plain"), t2];
    let gate = build_capability_gate(&todos, &[]).unwrap();
    assert_eq!(gate.runnable_todo_ids, vec!["t1"]);
    assert_eq!(gate.blocked_todo_ids, vec!["t2"]);
    assert_eq!(gate.action, "run"); // runnable remain → agent proceeds
    assert!(!gate.blocks_delivery);
}

#[test]
fn per_capability_command_hooks_register_with_status_gate() {
    // G-24 + G-26: catalog command hooks register into the CLI registry;
    // experimental capabilities' hooks are hidden unless requested.
    let catalog = CapabilityCatalog::with_builtin();
    let mut registry = CommandRegistry::new();
    let group = registry.group("capability", "capability framework");
    for record in catalog.records(true) {
        for c in &record.commands {
            if record.is_experimental() {
                registry.command_experimental(
                    group,
                    &c.name,
                    &c.purpose,
                    &format!("{} --input", c.name),
                );
            } else {
                registry.command(group, &c.name, &c.purpose, &format!("{} --input", c.name));
            }
        }
    }
    // Stable hooks visible by default.
    assert!(registry.find("issue-fix", false).is_some());
    assert!(registry.find("periodic-report", false).is_some());
    // Experimental hooks hidden by default, visible with the flag.
    assert!(registry.find("auto-research", false).is_none());
    assert!(registry.find("auto-research", true).is_some());
    assert!(registry.find("context-providers", false).is_none());
    assert!(registry.find("context-providers", true).is_some());
    // pr_review_queue shipped its P2-3 rule version as active-preview → its
    // hook is stable-visible.
    assert!(registry.find("pr-review-queue", false).is_some());
    assert!(registry.find("pr-review-queue", true).is_some());
    // Hook name ↔ capability id mapping is resolvable (dispatch target).
    let snake = "auto-research".replace('-', "_");
    assert_eq!(snake, "auto_research");
    assert!(catalog.get(&snake).is_some());
}

#[test]
fn capability_gate_schema_version_is_stable() {
    assert_eq!(
        future_loop::agents::capability_gate::CAPABILITY_GATE_SCHEMA_VERSION,
        "capability_gate_v0"
    );
}

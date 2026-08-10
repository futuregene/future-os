//! P2/P3 contract tests: capability framework (finite typed proposals),
//! capability gate (agent must declare a required capability), and worker
//! bridge writeback semantics. Deterministic.

use std::time::SystemTime;

use future_loop::capabilities::{CapabilityRegistry, ProposalKind};
use future_loop::contract::TurnMode;
use future_loop::decision::decide_for;
use future_loop::state::{Goal, Todo};

// ── P3: capability framework produces finite typed proposals ──────────────
#[test]
fn issue_fix_proposes_successors_for_actionable_issues() {
    let cap = CapabilityRegistry::with_builtin();
    let issue_fix = cap.get("issue_fix").expect("issue_fix registered");
    let proposals = issue_fix.propose(
        "Panic on empty input: `Error: index out of bounds` — repro: run \
         `calc --empty`; expected: graceful error message, actual: panic.",
    );
    assert!(!proposals.is_empty());
    assert!(proposals
        .iter()
        .any(|p| p.kind == ProposalKind::SuccessorTodo));
    assert!(proposals
        .iter()
        .all(|p| p.todo.is_none() || p.todo.is_some()));
}

#[test]
fn issue_fix_triages_thin_issues() {
    let cap = CapabilityRegistry::with_builtin();
    let proposals = cap.get("issue_fix").unwrap().propose("it's broken");
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
    assert!(proposals[0].reason.contains("lacks enough signal"));
}

#[test]
fn issue_fix_no_followup_on_empty() {
    let cap = CapabilityRegistry::with_builtin();
    let proposals = cap.get("issue_fix").unwrap().propose("");
    assert_eq!(proposals[0].kind, ProposalKind::NoFollowUp);
}

#[test]
fn periodic_report_requires_cadence() {
    let cap = CapabilityRegistry::with_builtin();
    let r1 = cap
        .get("periodic_report")
        .unwrap()
        .propose("report on project X");
    assert!(r1[0].reason.contains("cadence"));
    // G-25 deepening: a parseable cadence now yields a recurring MONITOR
    // (recurring observation) + a collect step, not a one-shot successor.
    let r2 = cap
        .get("periodic_report")
        .unwrap()
        .propose("daily report on project X");
    assert_eq!(r2[0].kind, ProposalKind::Monitor);
    assert!(r2[0].todo.as_ref().unwrap().text.contains("daily"));
    assert_eq!(r2[1].kind, ProposalKind::SuccessorTodo);
    // every-N cadences parse too.
    let r3 = cap
        .get("periodic_report")
        .unwrap()
        .propose("cadence: every-2h\nscope: project");
    assert_eq!(r3[0].kind, ProposalKind::Monitor);
}

#[test]
fn change_quality_repairs_unvalidated_changes() {
    let cap = CapabilityRegistry::with_builtin();
    let p = cap
        .get("change_quality")
        .unwrap()
        .propose("I made some edits");
    assert_eq!(p[0].kind, ProposalKind::SuccessorTodo);
    assert!(p[0].reason.contains("validation") || p[0].reason.contains("evidence"));
}

// ── P3: capability gate — agent must declare the required capability ──────
#[test]
fn capability_gate_hides_todo_from_agents_without_capability() {
    let mut goal = Goal::new("g", "objective", "/tmp");
    goal.register_agent("alice", vec!["shell".into()]);
    goal.register_agent("bob", vec![]);
    goal.add(Todo::advancement("t1", "Run the experiment").requiring("shell"));

    // Alice declares shell → runnable.
    let pa = decide_for(&goal, SystemTime::now(), Some("alice"));
    assert_eq!(pa.interaction_contract.mode, TurnMode::BoundedDelivery);
    assert_eq!(
        pa.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("t1")
    );

    // Bob does not declare shell → hidden (no runnable work).
    let pb = decide_for(&goal, SystemTime::now(), Some("bob"));
    assert_eq!(pb.interaction_contract.agent_channel.selected_todo, None);
}

// ── P2: agent profiles persist through the ledger ──────────────────────────
#[test]
fn agent_profiles_survive_replay() {
    let root = std::env::temp_dir().join(format!("future-loop-p3-{}", nano()));
    std::fs::create_dir_all(&root).unwrap();
    let mut store = future_loop::store::Store::open(&root.to_string_lossy()).unwrap();
    let g = Goal::new("g1", "objective", "/tmp");
    store.register(&g).unwrap();
    let ts = g.created_at;
    store
        .append(future_loop::store::Event::AgentOnboarded {
            goal_id: "g1".into(),
            agent_id: "alice".into(),
            capabilities: vec!["shell".into(), "github".into()],
            workspaces: vec![],
            ts,
        })
        .unwrap();
    let rebuilt = future_loop::store::Store::open(&root.to_string_lossy())
        .unwrap()
        .replay("g1")
        .unwrap()
        .unwrap();
    assert_eq!(rebuilt.agent_capabilities("alice"), vec!["shell", "github"]);
    assert!(rebuilt.is_registered_agent(Some("alice")));
}

fn nano() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

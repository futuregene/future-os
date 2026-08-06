//! P0 contract tests: claim/lease, agent registration, identity-scoped
//! frontier (LoopX: task-lease + coordination.registered_agents + equal
//! peers). Deterministic — no gRPC/LLM.

use std::time::{Duration, SystemTime};

use future_loop::contract::TurnMode;
use future_loop::decision::{decide, decide_for};
use future_loop::state::{now_epoch, Goal, Todo};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("loopx-p0-{tag}-{}", nano()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn nano() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn goal_with_todo(agents: &[&str]) -> Goal {
    let mut g = Goal::new("g", "objective", "/tmp");
    g.registered_agents = agents.iter().map(|s| s.to_string()).collect();
    g.add(Todo::advancement("t1", "Do the thing"));
    g
}

// ── Contract: unregistered agent identity is fail-closed ───────────────────
// LoopX: quota --agent-id requires coordination.registered_agents.
#[test]
fn unregistered_agent_fails_closed() {
    let g = goal_with_todo(&["alice"]);
    let p = decide_for(&g, SystemTime::now(), Some("bob"));
    assert_eq!(p.decision, "skip");
    assert!(!p.should_run);
    assert_eq!(p.effective_action, "automation_prompt_upgrade_required");
    assert!(p.reason.contains("register this agent identity"));
    // Anonymous path still works.
    let p2 = decide(&g, SystemTime::now());
    assert_eq!(p2.interaction_contract.mode, TurnMode::BoundedDelivery);
}

// ── Contract: claim/lease — a live lease held by another agent hides the
//    todo from that agent's frontier but not from the owner's. ──────────────
#[test]
fn lease_blocks_other_agent_until_expiry() {
    let mut g = goal_with_todo(&["alice", "bob"]);
    let now = now_epoch();
    assert!(g.todo_mut("t1").unwrap().claim("alice", 3600, now));

    // Bob's frontier: t1 hidden (alice holds a live lease).
    let pb = decide_for(&g, SystemTime::now(), Some("bob"));
    assert_eq!(pb.interaction_contract.agent_channel.selected_todo, None);
    assert_eq!(pb.interaction_contract.agent_channel.fallback_todo, None);
    assert_eq!(g.runnable_advancement_for(Some("bob")).count(), 0);

    // Alice's frontier: t1 visible (she owns it).
    let pa = decide_for(&g, SystemTime::now(), Some("alice"));
    assert_eq!(
        pa.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("t1")
    );
}

#[test]
fn expired_lease_returns_todo_to_shared_frontier() {
    let mut g = goal_with_todo(&["alice", "bob"]);
    let now = now_epoch();
    // 1-second lease, then time passes.
    assert!(g.todo_mut("t1").unwrap().claim("alice", 1, now));
    std::thread::sleep(Duration::from_millis(1200));
    let pb = decide_for(&g, SystemTime::now(), Some("bob"));
    assert_eq!(
        pb.interaction_contract
            .agent_channel
            .selected_todo
            .as_deref(),
        Some("t1"),
        "expired lease must return the todo to every eligible peer"
    );
}

#[test]
fn claim_rejects_live_lease_from_other_agent() {
    let mut g = goal_with_todo(&["alice", "bob"]);
    let now = now_epoch();
    assert!(g.todo_mut("t1").unwrap().claim("alice", 3600, now));
    let claimed = g.todo_mut("t1").unwrap().claim("bob", 3600, now);
    assert!(!claimed, "live lease cannot be stolen");
    assert_eq!(g.todo("t1").unwrap().claimed_by.as_deref(), Some("alice"));
}

// ── Contract: claim/lease persists through the event ledger ────────────────
#[test]
fn claim_survives_replay() {
    let root = tmp_root("claim-replay");
    let mut store = Store::open(&root).unwrap();
    let g = Goal::new("g1", "objective", "/tmp");
    store.register(&g).unwrap();
    let ts = g.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts,
        })
        .unwrap();
    store
        .append(Event::AgentRegistered {
            goal_id: "g1".into(),
            agent_id: "alice".into(),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "Work"),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoClaimed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "alice".into(),
            lease_expires_at: ts + 3600,
            ts,
        })
        .unwrap();

    let rebuilt = Store::open(&root).unwrap().replay("g1").unwrap().unwrap();
    assert!(rebuilt.registered_agents.iter().any(|a| a == "alice"));
    assert_eq!(
        rebuilt.todo("t1").unwrap().claimed_by.as_deref(),
        Some("alice")
    );
    assert_eq!(
        rebuilt.todo("t1").unwrap().lease_expires_at,
        Some(ts + 3600)
    );
}

// ── Contract: registered agents see unclaimed work (equal peers) ───────────
#[test]
fn unclaimed_todo_wakes_every_registered_peer() {
    let g = goal_with_todo(&["alice", "bob", "carol"]);
    for agent in ["alice", "bob", "carol"] {
        let p = decide_for(&g, SystemTime::now(), Some(agent));
        assert_eq!(
            p.interaction_contract
                .agent_channel
                .selected_todo
                .as_deref(),
            Some("t1"),
            "{agent} must see unclaimed work"
        );
    }
}

// ── Contract: CLI-level claim refuses without registration ────────────────
#[test]
fn claim_requires_registration() {
    let g = goal_with_todo(&[]);
    let _now = now_epoch();
    // No registered agents: claim by anyone fails at the CLI guard level
    // (is_registered_agent(Some(_)) is false).
    assert!(!g.is_registered_agent(Some("alice")));
    assert!(g.is_registered_agent(None));
}

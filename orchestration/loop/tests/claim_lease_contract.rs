//! P0 contract tests: claim/lease, agent registration, identity-scoped
//! frontier (LoopX: task-lease + coordination.registered_agents + equal
//! peers). Deterministic — no gRPC/LLM.

use std::time::{Duration, SystemTime};

use future_loop::contract::TurnMode;
use future_loop::decision::{decide, decide_for};
use future_loop::state::{now_epoch, Goal, Todo};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("future-loop-p0-{tag}-{}", nano()));
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

// ── Lease liveness on the manual `todo claim` path (state::Todo::claim) ────
// A killed run leaves a live lease whose holder pid no longer exists; the
// next manual claim must reclaim it (mirrors the run-path task_lease steal),
// while a live pid keeps the hard error and a pre-liveness ledger (no pid)
// keeps the old refusal.
#[test]
fn claim_reclaims_dead_holder_lease() {
    let mut g = goal_with_todo(&["alice", "bob"]);
    let now = now_epoch();
    {
        let t = g.todo_mut("t1").unwrap();
        t.claimed_by = Some("alice".to_string());
        t.lease_expires_at = Some(now + 3600);
        t.holder_pid = Some(4_000_000_000); // far outside the pid range
    }
    let claimed = g.todo_mut("t1").unwrap().claim("bob", 3600, now);
    assert!(claimed, "dead holder's lease must be reclaimed");
    assert_eq!(g.todo("t1").unwrap().claimed_by.as_deref(), Some("bob"));
}

#[test]
fn claim_refuses_live_holder_pid() {
    let mut g = goal_with_todo(&["alice", "bob"]);
    let now = now_epoch();
    {
        let t = g.todo_mut("t1").unwrap();
        t.claimed_by = Some("alice".to_string());
        t.lease_expires_at = Some(now + 3600);
        t.holder_pid = Some(std::process::id()); // the test process is alive
    }
    let claimed = g.todo_mut("t1").unwrap().claim("bob", 3600, now);
    assert!(!claimed, "a live holder's lease must not be stolen");
    assert_eq!(g.todo("t1").unwrap().claimed_by.as_deref(), Some("alice"));
}

// ── Contract (P0-1): workspace declarations survive replay and drive the
//    guard — a peer holding a live lease in an overlapping workspace
//    conflicts; idle or disjoint peers do not. ─────────────────────────────
#[test]
fn workspace_guard_survives_replay_and_detects_conflicts() {
    use future_loop::agents::workspace_guard as wsg;
    let root = tmp_root("wsguard");
    let mut store = Store::open(&root).unwrap();
    let g = Goal::new("g1", "objective", "/tmp");
    store.register(&g).unwrap();
    let now = now_epoch();
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts: now,
        })
        .unwrap();
    store
        .append(Event::AgentOnboarded {
            goal_id: "g1".into(),
            agent_id: "alice".into(),
            capabilities: vec![],
            workspaces: vec!["/definitely/not/here/wt1".into()],
            ts: now,
        })
        .unwrap();
    store
        .append(Event::AgentOnboarded {
            goal_id: "g1".into(),
            agent_id: "bob".into(),
            capabilities: vec![],
            workspaces: vec!["/definitely/not/here/wt1".into()],
            ts: now,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "shared work"),
            ts: now,
        })
        .unwrap();
    store
        .append(Event::TodoClaimed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "alice".into(),
            holder_pid: None,
            lease_expires_at: now + 3600,
            ts: now,
        })
        .unwrap();
    store
        .append(Event::WorkspaceLockAcquired {
            goal_id: "g1".into(),
            agent_id: "alice".into(),
            todo_id: "t1".into(),
            paths: vec!["/definitely/not/here/wt1".into()],
            forced: false,
            ts: now,
        })
        .unwrap();

    // Fresh store → replay: declarations, lease and the lock audit event
    // all survive.
    let rebuilt = Store::open(&root).unwrap().replay("g1").unwrap().unwrap();
    assert_eq!(
        wsg::agent_workspaces(&rebuilt, "alice"),
        vec!["/definitely/not/here/wt1".to_string()]
    );
    let conflicts = wsg::live_workspace_conflicts(&rebuilt, "bob", now + 10);
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].holder_agent_id, "alice");
    assert_eq!(conflicts[0].holder_todo_ids, vec!["t1".to_string()]);
    // Alice never conflicts with herself; after the lease lapses the
    // workspace is free again.
    assert!(wsg::live_workspace_conflicts(&rebuilt, "alice", now + 10).is_empty());
    assert!(wsg::live_workspace_conflicts(&rebuilt, "bob", now + 7200).is_empty());
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
            workspaces: vec![],
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
            holder_pid: None,
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

// ── Contract: try_claim_todo is atomic (check+append under one lock) ───────
// Regression: concurrent `run --agent-id` processes both passed the old
// check-then-append path and the last claim won → double execution.
fn store_with_todo(tag: &str) -> (String, Store) {
    let root = tmp_root(tag);
    let mut store = Store::open(&root).unwrap();
    let g = Goal::new("g1", "objective", &root);
    store.register(&g).unwrap();
    let ts = g.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: "g1".into(),
            ts,
        })
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "work"),
            ts,
        })
        .unwrap();
    (root, store)
}

#[test]
fn atomic_claim_conflict_is_reported() {
    let (_root, store) = store_with_todo("atomic-conflict");
    assert!(
        store
            .try_claim_todo("g1", "t1", "alice", 3600)
            .unwrap()
            .claimed
    );
    // A different agent loses atomically (no overwrite in the ledger).
    assert!(
        !store
            .try_claim_todo("g1", "t1", "bob", 3600)
            .unwrap()
            .claimed
    );
    // The holder reclaiming (renewal) succeeds.
    assert!(
        store
            .try_claim_todo("g1", "t1", "alice", 3600)
            .unwrap()
            .claimed
    );
}

#[test]
fn atomic_claim_after_expiry_or_release() {
    let (_root, mut store) = store_with_todo("atomic-expiry");
    // An explicit expiry marker frees the lease for another agent.
    assert!(
        store
            .try_claim_todo("g1", "t1", "alice", 3600)
            .unwrap()
            .claimed
    );
    store
        .append(Event::TodoExpired {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            ts: now_epoch(),
        })
        .unwrap();
    assert!(
        store
            .try_claim_todo("g1", "t1", "bob", 3600)
            .unwrap()
            .claimed
    );
    // A third agent cannot claim while bob's lease is live.
    assert!(
        !store
            .try_claim_todo("g1", "t1", "carol", 3600)
            .unwrap()
            .claimed
    );
    // An explicit release frees the todo immediately.
    store
        .append(Event::TodoReleased {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "bob".into(),
            ts: now_epoch(),
        })
        .unwrap();
    assert!(
        store
            .try_claim_todo("g1", "t1", "carol", 3600)
            .unwrap()
            .claimed
    );
}

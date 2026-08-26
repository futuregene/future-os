//! G12 multi-agent contract tests — the five assertion groups for the
//! multi_agent subdomain:
//! ① contract validation (fail-closed topology checks),
//! ② recipe application (record → resolve → onboard),
//! ③ succession triggering (lease expiry / offline → promotion + attention),
//! ④ wake roster (deterministic round-robin rotation),
//! ⑤ collective turn ledger (per-agent claim counts, full-participation min).

use std::collections::BTreeMap;

use future_loop::agents::multi_agent::{
    apply_recipe_onboard, collective_turn_ledger, contract_issues, latest_contract, recipe_named,
    recipes, record_contract, record_recipe, record_succession, succession_attention_items,
    succession_candidates_with, successions, wake_roster, AgentRecipe, HandoffRule,
    MultiAgentContract, PeerRole, SuccessionCandidate, MULTI_AGENT_CONTRACT_SCHEMA_VERSION,
    MULTI_AGENT_RECIPE_SCHEMA_VERSION, SUCCESSION_REASON_LEASE_EXPIRED, SUCCESSION_REASON_OFFLINE,
};
use future_loop::state::{now_epoch, Goal, Priority, Todo};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "future-loop-g12-multi-agent-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn open_goal(store: &mut Store, goal_id: &str) {
    let goal = Goal::new(goal_id, "objective", "/tmp");
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: goal_id.into(),
            ts: goal.created_at,
        })
        .unwrap();
}

fn peer(backup_for: Option<&str>, capabilities: &[&str], workspaces: &[&str]) -> PeerRole {
    PeerRole {
        backup_for: backup_for.map(|s| s.to_string()),
        capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
        workspaces: workspaces.iter().map(|s| s.to_string()).collect(),
    }
}

/// A three-agent contract: `primary` (backed up by `backup`), `backup`, and
/// `worker`, with one collective `crew` covering all three.
fn sample_contract() -> MultiAgentContract {
    let mut contract = MultiAgentContract::default();
    contract
        .peers
        .insert("primary".into(), peer(None, &["shell"], &["/ws/p"]));
    contract.peers.insert(
        "backup".into(),
        peer(Some("primary"), &["shell"], &["/ws/b"]),
    );
    contract
        .peers
        .insert("worker".into(), peer(None, &["github"], &["/ws/w"]));
    contract.handoff_rules.push(HandoffRule {
        from_event: "lease_expired".into(),
        to_role: "backup".into(),
    });
    contract.collectives.insert(
        "crew".into(),
        vec!["primary".into(), "backup".into(), "worker".into()],
    );
    contract
}

// ── ① contract validation ─────────────────────────────────────────────────

#[test]
fn contract_validation_accepts_well_formed_topology() {
    let contract = sample_contract();
    assert!(
        contract_issues(&contract).is_empty(),
        "sample must be valid"
    );
    assert_eq!(contract.backup_of("primary"), Some("backup"));
    assert_eq!(contract.backup_of("backup"), None);
    assert_eq!(contract.handoff_target("lease_expired"), Some("backup"));
    assert_eq!(contract.collective_of("worker"), Some("crew"));
}

#[test]
fn contract_validation_rejects_unknown_and_self_backup() {
    let mut contract = MultiAgentContract::default();
    contract.peers.insert("a".into(), peer(None, &[], &[]));
    contract
        .peers
        .insert("b".into(), peer(Some("ghost"), &[], &[]));
    let issues = contract_issues(&contract);
    assert!(
        issues.iter().any(|i| i.contains("backup_for `ghost`")),
        "got: {issues:?}"
    );

    let mut self_contract = MultiAgentContract::default();
    self_contract
        .peers
        .insert("a".into(), peer(Some("a"), &[], &[]));
    let issues = contract_issues(&self_contract);
    assert!(
        issues.iter().any(|i| i.contains("cannot back up itself")),
        "got: {issues:?}"
    );
}

#[test]
fn contract_validation_rejects_backup_cycles() {
    let mut contract = MultiAgentContract::default();
    contract.peers.insert("a".into(), peer(Some("b"), &[], &[]));
    contract.peers.insert("b".into(), peer(Some("a"), &[], &[]));
    let issues = contract_issues(&contract);
    assert!(
        issues.iter().any(|i| i.contains("cycle")),
        "got: {issues:?}"
    );

    // A longer chain must also be caught.
    let mut chain = MultiAgentContract::default();
    chain.peers.insert("a".into(), peer(Some("b"), &[], &[]));
    chain.peers.insert("b".into(), peer(Some("c"), &[], &[]));
    chain.peers.insert("c".into(), peer(Some("a"), &[], &[]));
    assert!(contract_issues(&chain).iter().any(|i| i.contains("cycle")));
}

#[test]
fn contract_validation_rejects_bad_handoff_rules_and_collectives() {
    let mut contract = MultiAgentContract::default();
    contract.peers.insert("a".into(), peer(None, &[], &[]));
    contract.peers.insert("b".into(), peer(None, &[], &[]));
    // to_role must resolve to a contract peer.
    contract.handoff_rules.push(HandoffRule {
        from_event: "ev".into(),
        to_role: "ghost".into(),
    });
    let issues = contract_issues(&contract);
    assert!(
        issues.iter().any(|i| i.contains("to_role `ghost`")),
        "got: {issues:?}"
    );

    // Duplicate handoff rule.
    let mut dup = MultiAgentContract::default();
    dup.peers.insert("a".into(), peer(None, &[], &[]));
    dup.handoff_rules.push(HandoffRule {
        from_event: "ev".into(),
        to_role: "a".into(),
    });
    dup.handoff_rules.push(HandoffRule {
        from_event: "ev".into(),
        to_role: "a".into(),
    });
    assert!(contract_issues(&dup)
        .iter()
        .any(|i| i.contains("duplicate handoff rule")));

    // Unknown collective member + cross-collective membership.
    let mut coll = MultiAgentContract::default();
    coll.peers.insert("a".into(), peer(None, &[], &[]));
    coll.peers.insert("b".into(), peer(None, &[], &[]));
    coll.collectives
        .insert("c1".into(), vec!["a".into(), "ghost".into()]);
    let issues = contract_issues(&coll);
    assert!(issues.iter().any(|i| i.contains("member `ghost`")));
    coll.collectives.insert("c1".into(), vec!["a".into()]);
    coll.collectives
        .insert("c2".into(), vec!["a".into(), "b".into()]);
    let issues = contract_issues(&coll);
    assert!(
        issues
            .iter()
            .any(|i| i.contains("more than one collective")),
        "got: {issues:?}"
    );
}

#[test]
fn contract_validation_rejects_empty_identifiers() {
    // Empty peer id.
    let mut c = MultiAgentContract::default();
    c.peers.insert("".into(), peer(None, &[], &[]));
    assert!(contract_issues(&c)
        .iter()
        .any(|i| i.contains("peer agent id must be non-empty")));

    // Empty backup_for target.
    let mut c = MultiAgentContract::default();
    c.peers.insert("a".into(), peer(Some(""), &[], &[]));
    assert!(contract_issues(&c)
        .iter()
        .any(|i| i.contains("backup_for must be non-empty")));

    // Empty handoff from_event / to_role.
    let mut c = MultiAgentContract::default();
    c.peers.insert("a".into(), peer(None, &[], &[]));
    c.handoff_rules.push(HandoffRule {
        from_event: "".into(),
        to_role: "a".into(),
    });
    assert!(contract_issues(&c)
        .iter()
        .any(|i| i.contains("from_event must be non-empty")));
    c.handoff_rules.clear();
    c.handoff_rules.push(HandoffRule {
        from_event: "ev".into(),
        to_role: "".into(),
    });
    assert!(contract_issues(&c)
        .iter()
        .any(|i| i.contains("to_role must be non-empty")));

    // Empty collective name.
    let mut c = MultiAgentContract::default();
    c.peers.insert("a".into(), peer(None, &[], &[]));
    c.collectives.insert("".into(), vec!["a".into()]);
    assert!(contract_issues(&c)
        .iter()
        .any(|i| i.contains("collective name must be non-empty")));

    // Collective with no members.
    let mut c = MultiAgentContract::default();
    c.collectives.insert("c1".into(), vec![]);
    assert!(contract_issues(&c)
        .iter()
        .any(|i| i.contains("must have at least one member")));

    // Duplicate member within one collective.
    let mut c = MultiAgentContract::default();
    c.peers.insert("a".into(), peer(None, &[], &[]));
    c.collectives.insert("c1".into(), vec!["a".into(), "a".into()]);
    assert!(contract_issues(&c)
        .iter()
        .any(|i| i.contains("duplicate member `a`")));
}

#[test]
fn successor_offline_threshold_reads_env() {
    // Unset → default.
    std::env::remove_var("FUTURE_LOOP_SUCCESSOR_OFFLINE_SECS");
    assert_eq!(
        future_loop::agents::multi_agent::successor_offline_threshold_secs(),
        30 * 60
    );
    // Valid shrink.
    std::env::set_var("FUTURE_LOOP_SUCCESSOR_OFFLINE_SECS", "5");
    assert_eq!(
        future_loop::agents::multi_agent::successor_offline_threshold_secs(),
        5
    );
    // Non-numeric → default.
    std::env::set_var("FUTURE_LOOP_SUCCESSOR_OFFLINE_SECS", "abc");
    assert_eq!(
        future_loop::agents::multi_agent::successor_offline_threshold_secs(),
        30 * 60
    );
    // Non-positive → default.
    std::env::set_var("FUTURE_LOOP_SUCCESSOR_OFFLINE_SECS", "0");
    assert_eq!(
        future_loop::agents::multi_agent::successor_offline_threshold_secs(),
        30 * 60
    );
    std::env::remove_var("FUTURE_LOOP_SUCCESSOR_OFFLINE_SECS");
}

#[test]
fn recipe_rejects_bad_schema_version() {
    let root = tmp_root("recipe-schema");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");
    let bad = AgentRecipe {
        schema_version: "not-a-version".into(),
        name: "worker".into(),
        capabilities: vec![],
        workspaces: vec![],
        priority: Priority::P1,
    };
    let err = record_recipe(&mut store, "g1", &bad).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("schema_version must be"), "got: {msg}");
}

#[test]
fn record_contract_fails_closed_on_invalid_and_persists_valid() {
    let root = tmp_root("contract");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");

    let bad = MultiAgentContract {
        schema_version: "something_else_v9".into(),
        ..MultiAgentContract::default()
    };
    assert!(record_contract(&mut store, "g1", &bad).is_err());

    let contract = sample_contract();
    let event_id = record_contract(&mut store, "g1", &contract).unwrap();
    assert!(event_id.starts_with("evt-"));

    // Latest wins: a second, smaller contract replaces the first.
    let mut smaller = MultiAgentContract::default();
    smaller.peers.insert("solo".into(), peer(None, &[], &[]));
    record_contract(&mut store, "g1", &smaller).unwrap();
    let latest = latest_contract(&store, "g1").unwrap().unwrap();
    assert_eq!(latest.peers.len(), 1);
    assert!(latest.peers.contains_key("solo"));

    // Replay is untouched by the projection-only events.
    let goal = store.replay("g1").unwrap().unwrap();
    assert!(goal.todos.is_empty());
    assert!(goal.registered_agents.is_empty());
}

// ── ② recipe application ──────────────────────────────────────────────────

#[test]
fn recipe_record_resolve_and_apply() {
    let root = tmp_root("recipe");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");

    let recipe = AgentRecipe {
        schema_version: MULTI_AGENT_RECIPE_SCHEMA_VERSION.to_string(),
        name: "researcher".into(),
        capabilities: vec!["shell".into(), "github".into()],
        workspaces: vec!["/ws/r".into()],
        priority: Priority::P0,
    };
    record_recipe(&mut store, "g1", &recipe).unwrap();

    assert_eq!(recipes(&store, "g1").unwrap().len(), 1);
    let resolved = recipe_named(&store, "g1", "researcher").unwrap().unwrap();
    assert_eq!(resolved.capabilities, recipe.capabilities);
    assert_eq!(resolved.workspaces, recipe.workspaces);
    assert_eq!(resolved.priority, Priority::P0);
    assert!(recipe_named(&store, "g1", "missing").unwrap().is_none());

    // Applying the recipe onboards the agent with the recipe profile — the
    // workspace guard consumes the same AgentOnboarded event as the explicit
    // onboard path; capabilities ride along as descriptive metadata.
    apply_recipe_onboard(&mut store, "g1", "agent-r", &resolved).unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    assert!(goal.registered_agents.contains(&"agent-r".to_string()));
    let profile = goal
        .agent_profiles
        .iter()
        .find(|p| p.id == "agent-r")
        .unwrap();
    assert_eq!(profile.capabilities, vec!["shell", "github"]);
    assert_eq!(profile.workspaces, vec!["/ws/r"]);
}

#[test]
fn recipe_re_add_resolves_latest_and_invalid_recipe_fails() {
    let root = tmp_root("recipe2");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");

    let v1 = AgentRecipe {
        schema_version: MULTI_AGENT_RECIPE_SCHEMA_VERSION.to_string(),
        name: "worker".into(),
        capabilities: vec!["shell".into()],
        workspaces: vec![],
        priority: Priority::P1,
    };
    record_recipe(&mut store, "g1", &v1).unwrap();
    let v2 = AgentRecipe {
        schema_version: MULTI_AGENT_RECIPE_SCHEMA_VERSION.to_string(),
        name: "worker".into(),
        capabilities: vec!["shell".into(), "network".into()],
        workspaces: vec!["/ws/2".into()],
        priority: Priority::P2,
    };
    record_recipe(&mut store, "g1", &v2).unwrap();

    assert_eq!(recipes(&store, "g1").unwrap().len(), 2);
    let latest = recipe_named(&store, "g1", "worker").unwrap().unwrap();
    assert_eq!(latest.capabilities, v2.capabilities);
    assert_eq!(latest.priority, Priority::P2);

    // Empty name fails closed.
    let bad = AgentRecipe {
        schema_version: MULTI_AGENT_RECIPE_SCHEMA_VERSION.to_string(),
        name: "   ".into(),
        capabilities: vec![],
        workspaces: vec![],
        priority: Priority::P1,
    };
    assert!(record_recipe(&mut store, "g1", &bad).is_err());
}

// ── ③ succession triggering ───────────────────────────────────────────────

fn contract_with_backup() -> MultiAgentContract {
    let mut contract = MultiAgentContract::default();
    contract
        .peers
        .insert("primary".into(), peer(None, &[], &[]));
    contract
        .peers
        .insert("backup".into(), peer(Some("primary"), &[], &[]));
    contract
}

fn goal_with_primary(store: &mut Store, goal_id: &str) {
    open_goal(store, goal_id);
    store
        .append(Event::AgentRegistered {
            goal_id: goal_id.into(),
            agent_id: "primary".into(),
            workspaces: vec![],
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::AgentRegistered {
            goal_id: goal_id.into(),
            agent_id: "backup".into(),
            workspaces: vec![],
            ts: now_epoch(),
        })
        .unwrap();
}

#[test]
fn succession_triggers_on_expired_lease() {
    let root = tmp_root("succession-lease");
    let mut store = Store::open(&root).unwrap();
    goal_with_primary(&mut store, "g1");
    let contract = contract_with_backup();
    record_contract(&mut store, "g1", &contract).unwrap();

    let now = now_epoch();
    let mut todo = Todo::advancement("t1", "stalled work");
    todo.claimed_by = Some("primary".into());
    todo.lease_expires_at = Some(now - 10); // expired 10s ago
    let mut goal = store.replay("g1").unwrap().unwrap();
    goal.add(todo);

    let candidates = succession_candidates_with(&goal, &contract, now, 600);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].primary, "primary");
    assert_eq!(candidates[0].backup, "backup");
    assert_eq!(candidates[0].reason, SUCCESSION_REASON_LEASE_EXPIRED);

    // Recording lands one SuccessionOccurred event and is idempotent.
    let candidate = candidates[0].clone();
    let event_id = record_succession(&mut store, "g1", &candidate).unwrap();
    let recorded = successions(&store, "g1").unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].event_id, event_id);
    let again = record_succession(&mut store, "g1", &candidate).unwrap();
    assert_eq!(again, event_id);
    assert_eq!(successions(&store, "g1").unwrap().len(), 1);
}

#[test]
fn succession_triggers_on_offline_heartbeat() {
    let root = tmp_root("succession-offline");
    let mut store = Store::open(&root).unwrap();
    goal_with_primary(&mut store, "g1");
    let contract = contract_with_backup();
    record_contract(&mut store, "g1", &contract).unwrap();

    let now = now_epoch();
    // Primary heartbeated an hour ago → past the 600s threshold.
    store
        .append(Event::SchedulerTicked {
            goal_id: "g1".into(),
            agent_id: "primary".into(),
            action: "tick".into(),
            rrule: None,
            ts: now - 3600,
        })
        .unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    let candidates = succession_candidates_with(&goal, &contract, now, 600);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].reason, SUCCESSION_REASON_OFFLINE);
    assert_eq!(candidates[0].since, now - 3600);

    // A heartbeat-less primary is NOT presumed offline (no anchor).
    let root2 = tmp_root("succession-no-hb");
    let mut store2 = Store::open(&root2).unwrap();
    goal_with_primary(&mut store2, "g2");
    record_contract(&mut store2, "g2", &contract).unwrap();
    let goal2 = store2.replay("g2").unwrap().unwrap();
    assert!(succession_candidates_with(&goal2, &contract, now, 600).is_empty());

    // A fresh heartbeat suppresses the trigger.
    store
        .append(Event::SchedulerTicked {
            goal_id: "g1".into(),
            agent_id: "primary".into(),
            action: "tick".into(),
            rrule: None,
            ts: now,
        })
        .unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    assert!(succession_candidates_with(&goal, &contract, now, 600).is_empty());
}

#[test]
fn succession_attention_item_until_primary_recovers() {
    let root = tmp_root("succession-attention");
    let mut store = Store::open(&root).unwrap();
    goal_with_primary(&mut store, "g1");
    let contract = contract_with_backup();
    record_contract(&mut store, "g1", &contract).unwrap();

    let candidate = SuccessionCandidate {
        role: "primary".into(),
        primary: "primary".into(),
        backup: "backup".into(),
        reason: SUCCESSION_REASON_LEASE_EXPIRED.into(),
        since: now_epoch(),
    };
    record_succession(&mut store, "g1", &candidate).unwrap();

    let goal = store.replay("g1").unwrap().unwrap();
    let items = succession_attention_items(&store, &goal).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].status, "role_succession");
    assert_eq!(items[0].source, "role_succession");
    assert_eq!(items[0].waiting_on, "user_or_controller");
    assert!(items[0].recommended_action.contains("primary"));
    assert!(items[0].recommended_action.contains("backup"));

    // Primary heartbeats after the succession → recovered, hint suppressed.
    let succession_ts = successions(&store, "g1").unwrap()[0].ts;
    store
        .append(Event::SchedulerTicked {
            goal_id: "g1".into(),
            agent_id: "primary".into(),
            action: "tick".into(),
            rrule: None,
            ts: succession_ts + 1,
        })
        .unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    assert!(succession_attention_items(&store, &goal)
        .unwrap()
        .is_empty());
}

#[test]
fn succession_ignores_unregistered_primary() {
    let root = tmp_root("succession-unregistered");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1"); // no agents registered
    let contract = contract_with_backup();
    record_contract(&mut store, "g1", &contract).unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    // Even a stale heartbeat cannot trigger: the primary is not part of
    // this goal's automation.
    let mut g = goal.clone();
    g.scheduler_heartbeats
        .insert("primary".into(), now_epoch() - 7200);
    assert!(succession_candidates_with(&g, &contract, now_epoch(), 600).is_empty());
}

// ── ④ wake roster ─────────────────────────────────────────────────────────

#[test]
fn wake_roster_rotates_round_robin() {
    let contract = sample_contract();
    let r0 = wake_roster(&contract, "crew", 0).unwrap();
    assert_eq!(r0.order, vec!["primary", "backup", "worker"]);
    assert_eq!(r0.current, "primary");

    let r1 = wake_roster(&contract, "crew", 1).unwrap();
    assert_eq!(r1.order, vec!["backup", "worker", "primary"]);
    assert_eq!(r1.current, "backup");

    let r2 = wake_roster(&contract, "crew", 2).unwrap();
    assert_eq!(r2.order, vec!["worker", "primary", "backup"]);
    assert_eq!(r2.current, "worker");

    // Full cycle wraps deterministically.
    let r3 = wake_roster(&contract, "crew", 3).unwrap();
    assert_eq!(r3.order, r0.order);

    // Unknown collectives project to None.
    assert!(wake_roster(&contract, "nope", 0).is_none());
}

// ── ⑤ collective turn ledger ──────────────────────────────────────────────

#[test]
fn collective_turn_ledger_counts_claims_and_full_rounds() {
    let root = tmp_root("ledger");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");
    let contract = sample_contract();
    record_contract(&mut store, "g1", &contract).unwrap();

    let base = now_epoch();
    // Claims: primary ×3, backup ×2, worker ×2 (each claim = one bounded
    // turn opportunity).
    for (agent, n) in [("primary", 3), ("backup", 2), ("worker", 2)] {
        for i in 0..n {
            store
                .append(Event::TodoClaimed {
                    goal_id: "g1".into(),
                    todo_id: format!("t-{agent}-{i}"),
                    agent_id: agent.into(),
                    holder_pid: None,
                    lease_expires_at: base + 1000,
                    ts: base + i,
                })
                .unwrap();
        }
    }
    // A claim by an agent OUTSIDE the collective must not be counted.
    store
        .append(Event::TodoClaimed {
            goal_id: "g1".into(),
            todo_id: "t-outsider".into(),
            agent_id: "outsider".into(),
            holder_pid: None,
            lease_expires_at: base + 1000,
            ts: base + 99,
        })
        .unwrap();

    let ledger = collective_turn_ledger(&store, "g1", &contract, "crew")
        .unwrap()
        .unwrap();
    assert_eq!(ledger.agents, vec!["primary", "backup", "worker"]);
    assert_eq!(ledger.per_agent["primary"].turns, 3);
    assert_eq!(ledger.per_agent["backup"].turns, 2);
    assert_eq!(ledger.per_agent["worker"].turns, 2);
    assert_eq!(ledger.per_agent["primary"].last_turn_ts, Some(base + 2));
    assert_eq!(ledger.total_claims, 7);
    // Asynchronous full participation = min across members.
    assert_eq!(ledger.full_participation_rounds, 2);

    // The wake roster for the NEXT collective turn uses the completed
    // rounds as the rotation input (turn 2 → worker wakes first).
    let roster = wake_roster(&contract, "crew", ledger.full_participation_rounds).unwrap();
    assert_eq!(roster.current, "worker");

    // Unknown collective → None.
    assert!(collective_turn_ledger(&store, "g1", &contract, "nope")
        .unwrap()
        .is_none());
}

#[test]
fn collective_full_participation_is_zero_until_every_member_claims() {
    let root = tmp_root("ledger-partial");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");
    let contract = sample_contract();
    record_contract(&mut store, "g1", &contract).unwrap();

    let base = now_epoch();
    for agent in ["primary", "backup"] {
        store
            .append(Event::TodoClaimed {
                goal_id: "g1".into(),
                todo_id: format!("t-{agent}"),
                agent_id: agent.into(),
                holder_pid: None,
                lease_expires_at: base + 1000,
                ts: base,
            })
            .unwrap();
    }
    let ledger = collective_turn_ledger(&store, "g1", &contract, "crew")
        .unwrap()
        .unwrap();
    assert_eq!(ledger.total_claims, 2);
    assert_eq!(ledger.per_agent["worker"].turns, 0);
    // `worker` never claimed → no complete collective round yet.
    assert_eq!(ledger.full_participation_rounds, 0);
    // Rotation at turn 0 still starts at the contract order head.
    let roster = wake_roster(&contract, "crew", ledger.full_participation_rounds).unwrap();
    assert_eq!(roster.current, "primary");
}

#[test]
fn contract_schema_version_constant_is_stable() {
    assert_eq!(
        MULTI_AGENT_CONTRACT_SCHEMA_VERSION,
        "multi_agent_contract_v0"
    );
    // The event payload round-trips through serde (BTreeMap ordering is
    // preserved, so ledger JSON stays deterministic).
    let contract = sample_contract();
    let json = serde_json::to_string(&contract).unwrap();
    let parsed: MultiAgentContract = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, contract);
    let peers: BTreeMap<String, PeerRole> = parsed.peers;
    assert_eq!(
        peers.keys().cloned().collect::<Vec<_>>(),
        vec!["backup", "primary", "worker"]
    );
}

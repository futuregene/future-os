//! Coverage drive for `state.rs` — Todo builders, claim/lease primitives,
//! Goal query/mutation arms that no command path exercises directly.

use future_loop::state::{default_goal_status, Goal, Priority, TaskClass, Todo, TodoStatus};

#[test]
fn todo_builders() {
    let t = Todo::advancement("t1", "base")
        .at_index(3)
        .with_title("Title")
        .with_note("a note")
        .with_gate_scope(true, true)
        .with_monitor_target("file:x")
        .with_monitor_policy("exists")
        .with_monitor_cadence("15m")
        .at_priority(Priority::P0)
        .with_action_kind("deploy");
    assert_eq!(t.index, 3);
    assert_eq!(t.title, "Title");
    assert_eq!(t.note.as_deref(), Some("a note"));
    assert!(t.goal_bound && t.global_gate);
    assert_eq!(t.monitor_target.as_deref(), Some("file:x"));
    assert_eq!(t.monitor_policy.as_deref(), Some("exists"));
    assert_eq!(t.monitor_cadence.as_deref(), Some("15m"));
    assert_eq!(t.priority, Priority::P0);
    assert_eq!(t.action_kind.as_deref(), Some("deploy"));

    // monitor_with derives the first due time from a cadence (or keeps the
    // explicit delay for `once`/unknown cadences).
    let m = Todo::monitor_with(
        "m1",
        "watch",
        Some("file:x"),
        Some("exists"),
        Some("15m"),
        std::time::Duration::from_secs(5),
    );
    assert!(m.resume_when.is_some());
    assert_eq!(m.monitor_target.as_deref(), Some("file:x"));
    let m2 = Todo::monitor_with(
        "m2",
        "watch",
        None,
        None,
        Some("bogus"),
        std::time::Duration::from_secs(5),
    );
    assert!(m2.resume_when.is_some(), "explicit delay kept");
    let m3 = Todo::monitor_with(
        "m3",
        "watch",
        None,
        None,
        None,
        std::time::Duration::from_secs(5),
    );
    assert!(m3.resume_when.is_some());

    // user_gate default question / blocks; blocker; user_action.
    let g = Todo::user_gate("g1", "question?", &["a", "b"]);
    assert_eq!(g.class, TaskClass::UserGate);
    assert_eq!(g.blocked_by_gate.as_deref(), Some("a,b"));
    let b = Todo::blocker("b1", "blocked", &["x"]);
    assert_eq!(b.class, TaskClass::Blocker);
    let ua = Todo::user_action("u1", "user does it");
    assert_eq!(ua.class, TaskClass::UserAction);
}

#[test]
fn todo_claim_and_complete_primitives() {
    let mut t = Todo::advancement("t1", "x");
    // Claim on a non-open todo → false.
    t.status = TodoStatus::Done;
    assert!(!t.claim("a", 60, 100));
    // Live lease held by someone else → false.
    let mut t = Todo::advancement("t1", "x");
    assert!(t.claim("a", 60, 100));
    assert!(!t.claim("b", 60, 100));
    // Same agent re-claims (renew) → true.
    assert!(t.claim("a", 60, 100));
    // Expired lease → another agent claims.
    assert!(t.claim("b", 60, 10_000), "lease expired at 160");
    // set_evidence stamps the todo.
    t.set_evidence("proof");
    assert_eq!(t.evidence.as_deref(), Some("proof"));
}

#[test]
fn goal_query_and_mutation_arms() {
    assert_eq!(default_goal_status(), "active");
    let mut goal =
        Goal::new("g", "obj", "/tmp").with_acceptance(vec![("gap1", "d1"), ("gap2", "d2")]);
    // register_agent dedup + profile replacement.
    goal.register_agent("a1", vec!["shell".into()]);
    goal.register_agent("a1", vec!["web".into()]);
    assert_eq!(goal.registered_agents, vec!["a1".to_string()]);
    let p = goal.agent_profiles.iter().find(|p| p.id == "a1").unwrap();
    assert!(p.has("web"));
    assert!(!p.has("shell"));
    // agent_capabilities: found profile returns its declared capabilities;
    // an unregistered agent falls back to an empty list.
    assert_eq!(goal.agent_capabilities("a1"), vec!["web".to_string()]);
    assert!(goal.agent_capabilities("ghost").is_empty());
    // is_registered_agent.
    assert!(goal.is_registered_agent(Some("a1")));
    assert!(!goal.is_registered_agent(Some("ghost")));
    assert!(goal.is_registered_agent(None), "anonymous path is allowed");

    // Todos for the blocking matrix.
    goal.todos.push(Todo::advancement("pred", "predecessor"));
    goal.todos.push(Todo::user_gate("gate1", "q?", &[]));
    let mut dependent = Todo::advancement("dep", "dependent");
    dependent.blocked_by_gate = Some("pred".into());
    goal.todos.push(dependent.clone());
    let mut gated = Todo::advancement("gated", "gated");
    gated.blocked_by_gate = Some("gate1".into());
    goal.todos.push(gated);
    let mut unknown = Todo::advancement("unk", "unknown pred");
    unknown.blocked_by_gate = Some("todo_ghost".into());
    goal.todos.push(unknown);
    let mut blank = Todo::advancement("blank", "blank pred");
    blank.blocked_by_gate = Some("".into());
    goal.todos.push(blank);

    // Plain-todo predecessor still open → blocked; done → unblocked.
    assert!(goal.is_blocked(&dependent));
    goal.todo_mut("pred").unwrap().status = TodoStatus::Done;
    assert!(!goal.is_blocked(&dependent));
    // Open gate predecessor → blocked; resolved → unblocked.
    assert!(goal.is_blocked(&goal.todo("gated").unwrap().clone()));
    goal.todo_mut("gate1").unwrap().status = TodoStatus::Done;
    assert!(!goal.is_blocked(&goal.todo("gated").unwrap().clone()));
    // Superseded predecessor does not block.
    goal.todo_mut("pred").unwrap().status = TodoStatus::Superseded;
    assert!(!goal.is_blocked(&dependent));
    // Unknown / empty predecessor ids never block.
    assert!(!goal.is_blocked(&goal.todo("unk").unwrap().clone()));
    assert!(!goal.is_blocked(&goal.todo("blank").unwrap().clone()));
    // No blocked_by_gate at all → false.
    assert!(!goal.is_blocked(&Todo::advancement("free", "free")));

    // open_blocking_sources: gates + blockers that are open.
    goal.todos.push(Todo::blocker("blk", "ext", &[]));
    let sources: Vec<_> = goal.open_blocking_sources().collect();
    assert!(sources.iter().any(|t| t.id == "blk"));

    // completed_without_closure_intent.
    let mut done_open = Todo::advancement("d1", "done silently");
    done_open.status = TodoStatus::Done;
    goal.todos.push(done_open);
    let mut done_closed = Todo::advancement("d2", "done with intent");
    done_closed.status = TodoStatus::Done;
    done_closed.no_follow_up = true;
    goal.todos.push(done_closed);
    let unclosed: Vec<_> = goal.completed_without_closure_intent();
    assert_eq!(unclosed.len(), 1);
    assert_eq!(unclosed[0].id, "d1");

    // Gaps.
    assert_eq!(goal.unsatisfied_gaps().len(), 2);
    goal.satisfy_gap("gap1");
    goal.satisfy_gap("gap_ghost");
    assert_eq!(goal.unsatisfied_gaps().len(), 1);

    // supersede / archive on present + missing todos.
    goal.supersede("d1");
    goal.supersede("todo_ghost");
    assert_eq!(goal.todo("d1").unwrap().status, TodoStatus::Superseded);
    goal.archive_todo("d2");
    goal.archive_todo("todo_ghost");
    assert_eq!(goal.todo("d2").unwrap().archive_state, "archived");

    // runnable_advancement_for: an agent's frontier excludes todos claimed by
    // others (live lease) but includes its own.
    let mut goal2 = Goal::new("g2", "obj", "/tmp");
    let mut mine = Todo::advancement("t1", "mine");
    mine.claim("alice", 3600, now_for_test());
    let mut theirs = Todo::advancement("t2", "theirs");
    theirs.claim("bob", 3600, now_for_test());
    goal2.todos.push(theirs);
    goal2.todos.push(mine);
    goal2.todos.push(Todo::advancement("t3", "free"));
    let alice_frontier: Vec<_> = goal2
        .runnable_advancement_for(Some("alice"))
        .map(|t| t.id.clone())
        .collect();
    assert!(alice_frontier.contains(&"t1".to_string()));
    assert!(alice_frontier.contains(&"t3".to_string()));
    assert!(!alice_frontier.contains(&"t2".to_string()));
    // Anonymous frontier excludes all claimed todos.
    let anon: Vec<_> = goal2.runnable_advancement().map(|t| t.id.clone()).collect();
    assert_eq!(anon, vec!["t3".to_string()]);
}

#[test]
fn owner_scope_limits_the_frontier_and_survives_lease_expiry() {
    let now = now_for_test();
    // A coordination todo is NOT advancement — it never enters the runnable
    // advancement frontier regardless of owner.
    let mut g = Goal::new("g", "obj", "/tmp");
    g.add(Todo::coordination("summary", "final validation"));
    g.add(Todo::advancement("shared", "shared work"));
    g.add(Todo::advancement("alice-work", "for alice").owned_by("alice"));
    let mut expired = Todo::advancement("expired", "leased then lapsed");
    // Simulate a claim that has since lapsed: claimed_by is stale but the
    // lease expiry is in the past.
    expired.claim("alice", 1, now.saturating_sub(2));
    g.add(expired);

    // Alice sees shared + her owned todo + the lapsed-lease todo (lease
    // lapsed, owner is None so it's back in the shared pool).
    let alice: Vec<_> = g
        .runnable_advancement_for(Some("alice"))
        .map(|t| t.id.clone())
        .collect();
    assert!(alice.contains(&"shared".to_string()));
    assert!(alice.contains(&"alice-work".to_string()));
    assert!(alice.contains(&"expired".to_string()));
    assert!(
        !alice.contains(&"summary".to_string()),
        "coordination never runnable"
    );

    // Bob does NOT see alice's owner-scoped todo, even though it has no live
    // lease — the owner assignment survives the absence of a lease.
    let bob: Vec<_> = g
        .runnable_advancement_for(Some("bob"))
        .map(|t| t.id.clone())
        .collect();
    assert!(bob.contains(&"shared".to_string()));
    assert!(
        !bob.contains(&"alice-work".to_string()),
        "owner scope must hold"
    );
    assert!(
        bob.contains(&"expired".to_string()),
        "no-owner lapsed lease returns to pool"
    );
}

#[test]
fn owner_scope_blocks_other_agents_even_with_a_live_lease_elsewhere() {
    // The owner filter is independent of the lease filter: a todo owned by
    // alice is invisible to bob whether or not someone holds a live lease.
    let now = now_for_test();
    let mut g = Goal::new("g", "obj", "/tmp");
    let mut owned = Todo::advancement("owned", "for alice").owned_by("alice");
    owned.claim("alice", 3600, now);
    g.add(owned);

    // Alice (owner + lease holder) sees it.
    assert_eq!(g.runnable_advancement_for(Some("alice")).count(), 1);
    // Bob sees nothing: owner scope hides it even though the lease is alice's.
    assert_eq!(g.runnable_advancement_for(Some("bob")).count(), 0);
    // Anonymous (no agent id) only sees shared-pool (owner None) todos.
    assert_eq!(g.runnable_advancement().count(), 0);
}

fn now_for_test() -> u64 {
    future_loop::state::now_epoch()
}

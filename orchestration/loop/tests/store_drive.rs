//! Coverage drive for `store.rs`: ledger read/write edge paths, the
//! registry dual-format loader, backup/restore/delete arms, schema-version
//! normalization, try_claim_todo lease reconstruction, and the event-apply
//! matrix (via append + replay assertions).

mod common;

use common::run_record;
use future_loop::state::{now_epoch, Goal, Todo, TodoStatus};
use future_loop::store::{Event, Store};
use std::io::Write;

fn fresh_store(tag: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(tag);
    std::fs::create_dir_all(&root).unwrap();
    (dir, root.to_string_lossy().into_owned())
}

fn registered_goal(store: &mut Store, gid: &str) {
    let goal = Goal::new(gid, "store drive", "/tmp");
    store.register(&goal).unwrap();
    store
        .append(Event::GoalStarted {
            goal_id: gid.to_string(),
            ts: now_epoch(),
        })
        .unwrap();
}

fn add(store: &mut Store, gid: &str, id: &str) {
    store
        .append(Event::TodoAdded {
            goal_id: gid.to_string(),
            todo: Todo::advancement(id, &format!("task {id}")),
            ts: now_epoch(),
        })
        .unwrap();
}

// ── append / dedupe / conflicts ────────────────────────────────────────────

#[test]
fn append_requires_registration_and_dedupes() {
    let (_d, root) = fresh_store("s1");
    let mut store = Store::open(&root).unwrap();
    // Unregistered goal → bail.
    assert!(store
        .append(Event::GoalStarted {
            goal_id: "goal_ghost".into(),
            ts: 1,
        })
        .is_err());
    registered_goal(&mut store, "g1");
    // Explicit event id appended twice with identical content → idempotent.
    let mk = || Event::TodoAdded {
        goal_id: "g1".into(),
        todo: Todo::advancement("t1", "x"),
        ts: 1,
    };
    store
        .append_with_meta(mk(), Some("evt-fixed".into()), None, None, None, None, None)
        .unwrap();
    store
        .append_with_meta(mk(), Some("evt-fixed".into()), None, None, None, None, None)
        .unwrap();
    let lines = store.raw_ledger_lines("g1").unwrap();
    assert_eq!(lines.len(), 2, "identical duplicate is not re-appended");
    // Same id, different content → conflict error.
    let conflicting = Event::TodoAdded {
        goal_id: "g1".into(),
        todo: Todo::advancement("t1", "DIFFERENT"),
        ts: 2,
    };
    assert!(store
        .append_with_meta(
            conflicting,
            Some("evt-fixed".into()),
            None,
            None,
            None,
            None,
            None
        )
        .is_err());
    // raw_ledger_lines on a goal with no ledger file → empty.
    let store2 = Store::open(&root).unwrap();
    assert!(store2.raw_ledger_lines("goal_nofile").unwrap().is_empty());
}

// ── try_claim_todo ─────────────────────────────────────────────────────────

#[test]
fn double_register_is_a_noop() {
    let (_d, root) = fresh_store("s-dup-reg");
    let mut store = Store::open(&root).unwrap();
    let goal = Goal::new("g1", "store drive", "/tmp");
    store.register(&goal).unwrap();
    // Re-registering an already-registered goal keeps the registry as-is.
    store.register(&goal).unwrap();
    assert_eq!(store.registry().len(), 1);
}

#[test]
fn try_claim_ignores_non_lease_events_and_p9_normalizes_to_p1() {
    let (_d, root) = fresh_store("s-claim-misc");
    let mut store = Store::open(&root).unwrap();
    registered_goal(&mut store, "g1");
    add(&mut store, "g1", "t1");
    // A todo_updated line for the same todo hits the non-lease match arm
    // during lease reconstruction; priority "P9" normalizes to P1 on apply.
    store
        .append(Event::TodoUpdated {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            text: None,
            status: None,
            evidence: None,
            note: Some("n".into()),
            priority: Some("P9".into()),
            resume_when: None,
            blocks: None,
            ts: now_epoch(),
        })
        .unwrap();
    assert!(store.try_claim_todo("g1", "t1", "alice", 3600).unwrap());
    let g = store.replay("g1").unwrap().unwrap();
    assert_eq!(
        g.todo("t1").unwrap().priority,
        future_loop::state::Priority::P1
    );
}

#[test]
fn renew_and_release_on_unknown_todo_are_noops() {
    let (_d, root) = fresh_store("s-lease-ghost");
    let mut store = Store::open(&root).unwrap();
    registered_goal(&mut store, "g1");
    store
        .append(Event::TodoRenewed {
            goal_id: "g1".into(),
            todo_id: "ghost".into(),
            agent_id: "a".into(),
            lease_expires_at: 42,
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::TodoReleased {
            goal_id: "g1".into(),
            todo_id: "ghost".into(),
            agent_id: "a".into(),
            ts: now_epoch(),
        })
        .unwrap();
    let g = store.replay("g1").unwrap().unwrap();
    assert!(g.todo("ghost").is_none());
}

#[test]
fn fingerprint_of_non_object_value_is_the_empty_object() {
    assert_eq!(
        future_loop::store::event_fingerprint(&serde_json::json!("x")),
        "{}"
    );
}

#[test]
fn try_claim_todo_lease_reconstruction() {
    let (_d, root) = fresh_store("s2");
    let mut store = Store::open(&root).unwrap();
    registered_goal(&mut store, "g1");
    add(&mut store, "g1", "t1");
    // Free todo → claim succeeds.
    assert!(store.try_claim_todo("g1", "t1", "alice", 3600).unwrap());
    // Live lease held by alice → bob loses the race (Ok(false)).
    assert!(!store.try_claim_todo("g1", "t1", "bob", 3600).unwrap());
    // Alice re-claims (same holder) → succeeds.
    assert!(store.try_claim_todo("g1", "t1", "alice", 3600).unwrap());
    // Release → free → bob claims.
    store
        .append(Event::TodoReleased {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "alice".into(),
            ts: now_epoch(),
        })
        .unwrap();
    assert!(store.try_claim_todo("g1", "t1", "bob", 1).unwrap());
    // Let bob's 1s lease lapse → carol steals (expired arm).
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert!(store.try_claim_todo("g1", "t1", "carol", 3600).unwrap());
    // Expire event clears the lease → free.
    store
        .append(Event::TodoExpired {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            ts: now_epoch(),
        })
        .unwrap();
    assert!(store.try_claim_todo("g1", "t1", "dave", 3600).unwrap());
    // A garbage line in the ledger is skipped during reconstruction.
    let events_path = store.goal_dir("g1").join("events.jsonl");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&events_path)
        .unwrap()
        .write_all(b"{not json\n")
        .unwrap();
    assert!(store.try_claim_todo("g1", "t1", "dave", 3600).is_ok());
}

// ── schema version normalization ───────────────────────────────────────────

#[test]
fn goal_schema_version_variants() {
    let (_d, root) = fresh_store("s3");
    let mut store = Store::open(&root).unwrap();
    registered_goal(&mut store, "g1");
    // The stamp written at first append is the current schema.
    assert!(store.goal_schema_version("g1").is_some());
    let dir = store.goal_dir("g1");
    // Missing file → None.
    std::fs::remove_file(dir.join("schema.json")).unwrap();
    assert!(store.goal_schema_version("g1").is_none());
    // Invalid JSON → None.
    std::fs::write(dir.join("schema.json"), "{nope").unwrap();
    assert!(store.goal_schema_version("g1").is_none());
    // Missing field → None.
    std::fs::write(dir.join("schema.json"), "{}").unwrap();
    assert!(store.goal_schema_version("g1").is_none());
    // Legacy tokens normalize.
    std::fs::write(
        dir.join("schema.json"),
        "{\"event_store_schema_version\":\"loopx_event_store_v1\"}",
    )
    .unwrap();
    assert_ne!(
        store.goal_schema_version("g1").as_deref(),
        Some("loopx_event_store_v1")
    );
    std::fs::write(
        dir.join("schema.json"),
        "{\"event_store_schema_version\":\"loopx_event_store_v0\"}",
    )
    .unwrap();
    let v0 = store.goal_schema_version("g1").unwrap();
    assert!(v0.contains("legacy") || v0.contains("v0"), "{v0}");
}

// ── replay edge paths ──────────────────────────────────────────────────────

#[test]
fn replay_edge_paths() {
    let (_d, root) = fresh_store("s4");
    let mut store = Store::open(&root).unwrap();
    // Registry entry without an events file → replay None.
    store.register(&Goal::new("g_empty", "x", "/tmp")).unwrap();
    assert!(store.replay("g_empty").unwrap().is_none());
    // Not in the registry at all → None.
    assert!(store.replay("goal_nope").unwrap().is_none());
    // Malformed event line → read_ledger errors with line context.
    registered_goal(&mut store, "g1");
    let events_path = store.goal_dir("g1").join("events.jsonl");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&events_path)
        .unwrap()
        .write_all(b"{broken\n")
        .unwrap();
    assert!(store.replay("g1").is_err());
    // Identical duplicate lines collapse on read; conflicting ids fail closed.
    std::fs::write(&events_path, "").unwrap();
    let line = {
        let se = serde_json::json!({
            "event_id": "evt-dup",
            "kind": "goal_started",
            "goal_id": "g1",
            "ts": 1,
        });
        format!("{}\n", se)
    };
    std::fs::write(&events_path, format!("{line}{line}")).unwrap();
    let goal = store.replay("g1").unwrap();
    assert!(goal.is_some(), "identical dups collapse");
    // Conflicting: same id, different payload.
    let a = serde_json::json!({"event_id":"evt-c","kind":"goal_started","goal_id":"g1","ts":1});
    let b = serde_json::json!({"event_id":"evt-c","kind":"goal_started","goal_id":"g1","ts":2});
    std::fs::write(&events_path, format!("{}\n{}\n", a, b)).unwrap();
    assert!(store.replay("g1").is_err(), "conflicting ids fail closed");
}

#[test]
fn replay_skips_malformed_run_lines() {
    let (_d, root) = fresh_store("s5");
    let mut store = Store::open(&root).unwrap();
    registered_goal(&mut store, "g1");
    let runs = store.goal_dir("g1").join("runs.jsonl");
    std::fs::write(&runs, "{broken\n").unwrap();
    store
        .append_run("g1", &run_record("t1", "completed", now_epoch()))
        .unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    assert_eq!(goal.history.len(), 1, "malformed line skipped");
}

// ── registry dual-format loader ────────────────────────────────────────────

#[test]
fn registry_loader_formats() {
    // Map form with id/repo aliasing.
    let (_d, root) = fresh_store("s6");
    std::fs::write(
        std::path::Path::new(&root).join("registry.json"),
        serde_json::to_string(&serde_json::json!({
            "goals": [{"id": "g_legacy", "objective": "obj", "repo": "/tmp/repo", "status": "active", "created_at": 1}]
        }))
        .unwrap(),
    )
    .unwrap();
    let store = Store::open(&root).unwrap();
    assert!(
        store.registered("g_legacy"),
        "id→goal_id, repo→cwd aliasing"
    );
    // Neither array nor {goals} → bail.
    let (_d2, root2) = fresh_store("s7");
    std::fs::write(
        std::path::Path::new(&root2).join("registry.json"),
        "\"just a string\"",
    )
    .unwrap();
    assert!(Store::open(&root2).is_err());
    // Non-object entry → error.
    let (_d3, root3) = fresh_store("s8");
    std::fs::write(
        std::path::Path::new(&root3).join("registry.json"),
        "[\"nope\"]",
    )
    .unwrap();
    assert!(Store::open(&root3).is_err());
    // Invalid JSON → error.
    let (_d4, root4) = fresh_store("s9");
    std::fs::write(std::path::Path::new(&root4).join("registry.json"), "{bad").unwrap();
    assert!(Store::open(&root4).is_err());
}

// ── backup / restore / delete arms ─────────────────────────────────────────

#[test]
fn backup_restore_delete_arms() {
    let (_d, root) = fresh_store("s10");
    let mut store = Store::open(&root).unwrap();
    // Backup a goal with no state → bail.
    store
        .register(&Goal::new("g_nostate", "x", "/tmp"))
        .unwrap();
    assert!(store.backup_goal("g_nostate").is_err());
    // Full backup including scheduler-state + registry entry.
    registered_goal(&mut store, "g1");
    let sched = store.goal_dir("g1").join("scheduler-state");
    std::fs::create_dir_all(&sched).unwrap();
    std::fs::write(sched.join("state.json"), "{}").unwrap();
    let dest = store.backup_goal("g1").unwrap();
    assert!(std::path::Path::new(&dest)
        .join("scheduler-state/state.json")
        .exists());
    assert!(std::path::Path::new(&dest)
        .join("registry-entry.json")
        .exists());
    // Restore from a dir without events.jsonl → bail; from the real backup → ok.
    assert!(store.restore_goal("g1", "/nonexistent-backup").is_err());
    store.restore_goal("g1", &dest).unwrap();
    // Delete: unknown goal bails; registered goal disappears (registry + dir).
    assert!(store.delete_goal("goal_nope").is_err());
    store.delete_goal("g1").unwrap();
    assert!(!store.registered("g1"));
    assert!(!store.goal_dir("g1").exists());
}

// ── verify ─────────────────────────────────────────────────────────────────

#[test]
fn verify_ledger_reports() {
    let (_d, root) = fresh_store("s11");
    let mut store = Store::open(&root).unwrap();
    registered_goal(&mut store, "g1");
    let report = store.verify("g1").unwrap();
    assert!(report.ok);
    // Legacy line without event_id counts; duplicates and conflicts show up.
    let events_path = store.goal_dir("g1").join("events.jsonl");
    let legacy = serde_json::json!({"kind":"goal_started","goal_id":"g1","ts":1});
    let dup = serde_json::json!({"event_id":"e1","kind":"goal_started","goal_id":"g1","ts":1});
    let conflict_a =
        serde_json::json!({"event_id":"e2","kind":"goal_started","goal_id":"g1","ts":1});
    let conflict_b =
        serde_json::json!({"event_id":"e2","kind":"goal_started","goal_id":"g1","ts":9});
    std::fs::write(
        &events_path,
        format!(
            "{}\n{}\n{}\n{}\n{}\n",
            legacy, dup, dup, conflict_a, conflict_b,
        ),
    )
    .unwrap();
    let report = store.verify("g1").unwrap();
    assert_eq!(report.legacy_lines_without_id, 1);
    assert_eq!(report.idempotent_duplicates, 1);
    assert!(report.conflicts.contains(&"e2".to_string()));
    assert!(!report.ok);
    // No ledger file → empty report.
    let report = store.verify("goal_nofile").unwrap();
    assert_eq!(report.total_events, 0);
}

#[test]
fn delete_dirless_goal_and_append_skip_arms() {
    let (_d, root) = fresh_store("s13");
    let mut store = Store::open(&root).unwrap();
    // Registered but no goal dir on disk → delete skips the remove.
    store
        .register(&Goal::new("g_dirless", "x", "/tmp"))
        .unwrap();
    store.delete_goal("g_dirless").unwrap();
    assert!(!store.registered("g_dirless"));
    // A garbage ledger line is skipped by the append dedup scan.
    registered_goal(&mut store, "g1");
    let events_path = store.goal_dir("g1").join("events.jsonl");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&events_path)
        .unwrap()
        .write_all(b"{broken\n")
        .unwrap();
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "x"),
            ts: now_epoch(),
        })
        .unwrap();
}

#[test]
fn verify_conflict_dedup_arm() {
    let (_d, root) = fresh_store("s14");
    let mut store = Store::open(&root).unwrap();
    registered_goal(&mut store, "g1");
    // Three lines sharing an id with TWO distinct payloads: the conflict is
    // recorded once even though the mismatch is seen twice.
    let events_path = store.goal_dir("g1").join("events.jsonl");
    let a = serde_json::json!({"event_id":"e3","kind":"goal_started","goal_id":"g1","ts":1});
    let b = serde_json::json!({"event_id":"e3","kind":"goal_started","goal_id":"g1","ts":2});
    let c = serde_json::json!({"event_id":"e3","kind":"goal_started","goal_id":"g1","ts":3});
    std::fs::write(&events_path, format!("{}\n{}\n{}\n", a, b, c)).unwrap();
    let report = store.verify("g1").unwrap();
    assert_eq!(report.conflicts.len(), 1, "{report:?}");
}

#[test]
fn apply_renew_and_priority_arms() {
    let (_d, root) = fresh_store("s15");
    let mut store = Store::open(&root).unwrap();
    registered_goal(&mut store, "g1");
    add(&mut store, "g1", "t1");
    // Claim, then renew: the claim-fill arm is skipped (already claimed) but
    // the lease updates.
    store
        .append(Event::TodoClaimed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "a".into(),
            lease_expires_at: 100,
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::TodoRenewed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "a".into(),
            lease_expires_at: 200,
            ts: now_epoch(),
        })
        .unwrap();
    // Release by the owner clears both fields.
    store
        .append(Event::TodoReleased {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "a".into(),
            ts: now_epoch(),
        })
        .unwrap();
    // Expiry on a CLAIMED todo clears it (the had-claim arm).
    store
        .append(Event::TodoClaimed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "b".into(),
            lease_expires_at: 50,
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::TodoExpired {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            ts: now_epoch(),
        })
        .unwrap();
    // TodoUpdated priority P0 arm (P2 covered elsewhere).
    store
        .append(Event::TodoUpdated {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            text: None,
            status: None,
            evidence: None,
            note: None,
            priority: Some("P0".into()),
            resume_when: None,
            blocks: None,
            ts: now_epoch(),
        })
        .unwrap();
    let goal = store.replay("g1").unwrap().unwrap();
    let t = goal.todo("t1").unwrap();
    assert_eq!(t.claimed_by, None, "expired claim cleared");
    assert_eq!(t.lease_expires_at, None);
    assert_eq!(t.priority, future_loop::state::Priority::P0);
}

// ── apply matrix (append + replay state assertions) ────────────────────────

#[test]
fn apply_matrix() {
    let (_d, root) = fresh_store("s12");
    let mut store = Store::open(&root).unwrap();
    registered_goal(&mut store, "g1");
    add(&mut store, "g1", "t1");
    // TodoAdded with a preset index skips the index assignment.
    let mut preset = Todo::advancement("t_preset", "preset index");
    preset.index = 7;
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: preset,
            ts: now_epoch(),
        })
        .unwrap();
    // TodoUpdated: every status arm + bogus status + missing todo.
    for status in ["open", "blocked", "deferred", "superseded", "bogus"] {
        store
            .append(Event::TodoUpdated {
                goal_id: "g1".into(),
                todo_id: "t1".into(),
                text: None,
                status: Some(status.into()),
                evidence: None,
                note: None,
                priority: None,
                resume_when: None,
                blocks: None,
                ts: now_epoch(),
            })
            .unwrap();
    }
    store
        .append(Event::TodoUpdated {
            goal_id: "g1".into(),
            todo_id: "todo_ghost".into(),
            text: Some("x".into()),
            status: None,
            evidence: None,
            note: None,
            priority: Some("P2".into()),
            resume_when: Some("defer:5".into()),
            blocks: Some(vec!["a".into()]),
            ts: now_epoch(),
        })
        .unwrap();
    // GateResolved with note on a real todo; on a missing todo.
    store
        .append(Event::GateResolved {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            decision: "d".into(),
            note: Some("n".into()),
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::GateResolved {
            goal_id: "g1".into(),
            todo_id: "todo_ghost".into(),
            decision: "d".into(),
            note: None,
            ts: now_epoch(),
        })
        .unwrap();
    // Claim on missing todo (skip arm); renew with/without prior claim.
    store
        .append(Event::TodoClaimed {
            goal_id: "g1".into(),
            todo_id: "todo_ghost".into(),
            agent_id: "a".into(),
            lease_expires_at: 9,
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::TodoRenewed {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "a".into(),
            lease_expires_at: 42,
            ts: now_epoch(),
        })
        .unwrap();
    // AgentRegistered twice (dedup arm), re-onboard replaces the profile.
    for _ in 0..2 {
        store
            .append(Event::AgentRegistered {
                goal_id: "g1".into(),
                agent_id: "a".into(),
                workspaces: vec![],
                ts: now_epoch(),
            })
            .unwrap();
    }
    store
        .append(Event::AgentOnboarded {
            goal_id: "g1".into(),
            agent_id: "a".into(),
            capabilities: vec!["shell".into()],
            workspaces: vec![],
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::AgentOnboarded {
            goal_id: "g1".into(),
            agent_id: "a".into(),
            capabilities: vec!["web".into()],
            workspaces: vec![],
            ts: now_epoch(),
        })
        .unwrap();
    // EvidenceAttached on live + missing todos.
    store
        .append(Event::EvidenceAttached {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            evidence: "e".into(),
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::EvidenceAttached {
            goal_id: "g1".into(),
            todo_id: "todo_ghost".into(),
            evidence: "e".into(),
            ts: now_epoch(),
        })
        .unwrap();
    // MonitorPolled changed + no_change; QuotaSpent.
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::monitor("m1", "watch", std::time::Duration::from_secs(60)),
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::MonitorPolled {
            goal_id: "g1".into(),
            todo_id: "m1".into(),
            result: "changed".into(),
            no_change_count: 0,
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::MonitorPolled {
            goal_id: "g1".into(),
            todo_id: "todo_ghost".into(),
            result: "no_change".into(),
            no_change_count: 3,
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::QuotaSpent {
            goal_id: "g1".into(),
            run_id: "r".into(),
            todo_id: "t1".into(),
            source: "run".into(),
            slots: 2,
            ts: now_epoch(),
        })
        .unwrap();
    // TodoCompleted on a missing todo (skip arm); supersede; release; expire.
    store
        .append(Event::TodoCompleted {
            goal_id: "g1".into(),
            todo_id: "todo_ghost".into(),
            no_follow_up: true,
            successor_ids: vec![],
            evidence: Some("e".into()),
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::TodoSuperseded {
            goal_id: "g1".into(),
            todo_id: "t_preset".into(),
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::TodoReleased {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            agent_id: "a".into(),
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::TodoExpired {
            goal_id: "g1".into(),
            todo_id: "todo_ghost".into(),
            ts: now_epoch(),
        })
        .unwrap();
    // Supervisor events (apply arms).
    store
        .append(Event::SupervisorProposed {
            goal_id: "g1".into(),
            supervisor_agent_id: "sup".into(),
            decision_id: "d1".into(),
            decision_kind: "observe".into(),
            target_agent_id: "a".into(),
            required_host_capabilities: vec![],
            decision: "watch".into(),
            ts: now_epoch(),
        })
        .unwrap();
    store
        .append(Event::SupervisorReceiptRecorded {
            goal_id: "g1".into(),
            decision_id: "d1".into(),
            receipt_id: "r1".into(),
            adapter_id: "ad".into(),
            outcome: "rejected".into(),
            authority_ref: None,
            rollback_ref: None,
            ts: now_epoch(),
        })
        .unwrap();
    // GoalCancelled + GapSatisfied.
    store
        .append(Event::GapSatisfied {
            goal_id: "g1".into(),
            gap_id: "gap1".into(),
            ts: now_epoch(),
        })
        .unwrap();

    let goal = store.replay("g1").unwrap().unwrap();
    let t1 = goal.todo("t1").unwrap();
    assert_eq!(t1.evidence.as_deref(), Some("e"));
    // The TodoReleased apply cleared the earlier claim/renew.
    assert_eq!(t1.claimed_by, None);
    assert_eq!(t1.lease_expires_at, None);
    assert_eq!(goal.registered_agents, vec!["a".to_string()]);
    let profile = goal.agent_profiles.iter().find(|p| p.id == "a").unwrap();
    assert_eq!(profile.capabilities, vec!["web".to_string()]);
    assert_eq!(goal.quota_spent_slots, 2);
    assert_eq!(
        goal.todo("t_preset").unwrap().status,
        TodoStatus::Superseded
    );
    assert_eq!(goal.todo("t_preset").unwrap().index, 7);
    assert_eq!(goal.todo("m1").unwrap().status, TodoStatus::Done);
}

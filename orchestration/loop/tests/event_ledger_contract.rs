//! G-3 event-ledger contract tests: content-derived event ids, idempotent
//! re-append, conflict detection (StateEventConflictError), the new
//! QuotaSpent / EvidenceAttached events, and idempotent markdown backfill
//! through the store (with source provenance).

use future_loop::backfill::backfill_todo_events;
use future_loop::projection::privacy::PrivacyLevel;
use future_loop::state::{Goal, Todo};
use future_loop::store::{Event, Store};

fn tmp_root(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "loopx-p2-events-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

fn open_goal(store: &mut Store, goal_id: &str) -> u64 {
    let goal = Goal::new(goal_id, "objective", "/tmp");
    store.register(&goal).unwrap();
    let ts = goal.created_at;
    store
        .append(Event::GoalStarted {
            goal_id: goal_id.into(),
            ts,
        })
        .unwrap();
    ts
}

/// ── Event ids are content-derived and stable ──────────────────────────────
#[test]
fn event_id_is_content_derived_and_stable() {
    let event = Event::TodoAdded {
        goal_id: "g".into(),
        todo: Todo::advancement("t1", "work"),
        ts: 1_000,
    };
    let id1 = future_loop::store::derive_event_id(&event);
    let id2 = future_loop::store::derive_event_id(&event);
    assert_eq!(id1, id2);
    assert!(id1.starts_with("evt-"));
    assert_eq!(id1.len(), 4 + 16);
    // Different content → different id.
    let other = Event::TodoAdded {
        goal_id: "g".into(),
        todo: Todo::advancement("t1", "other work"),
        ts: 1_000,
    };
    assert_ne!(id1, future_loop::store::derive_event_id(&other));
}

/// ── Idempotent re-append: same content is a no-op ─────────────────────────
#[test]
fn append_is_idempotent_for_identical_content() {
    let root = tmp_root("idempotent");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");
    let event = Event::TodoAdded {
        goal_id: "g1".into(),
        todo: Todo::advancement("t1", "work"),
        ts: 1_000,
    };
    let id = store.append(event.clone()).unwrap();
    let again = store.append(event).unwrap();
    assert_eq!(id, again);
    // Only ONE ledger line exists.
    let lines = store.raw_ledger_lines("g1").unwrap();
    assert_eq!(lines.len(), 2, "goal_started + one todo_added");
    let report = store.verify("g1").unwrap();
    assert!(report.ok);
    assert_eq!(report.total_events, 2);
    assert_eq!(report.idempotent_duplicates, 0);
    assert_eq!(report.unique_events, 2);
    // Replay sees exactly one todo.
    let goal = store.replay("g1").unwrap().unwrap();
    assert_eq!(goal.todos.len(), 1);
}

/// ── Conflict: same id, different content fails closed ─────────────────────
#[test]
fn conflicting_event_id_fails_closed() {
    let root = tmp_root("conflict");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");
    // Explicit id (backfill-style) with content A.
    store
        .append_with_meta(
            Event::TodoAdded {
                goal_id: "g1".into(),
                todo: Todo::advancement("t1", "work"),
                ts: 1_000,
            },
            Some("backfill-add-deadbeef".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    // Same id, DIFFERENT content → StateEventConflictError.
    let err = store.append_with_meta(
        Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "different work"),
            ts: 1_000,
        },
        Some("backfill-add-deadbeef".into()),
        None,
        None,
        None,
        None,
        None,
    );
    assert!(err.is_err(), "conflicting event id must fail closed");
    let msg = format!("{:?}", err.err().unwrap());
    assert!(msg.contains("conflicting event_id"), "got: {msg}");
}

/// ── Backfill through the store is idempotent and carries provenance ──────
#[test]
fn backfill_append_is_idempotent_with_provenance() {
    let root = tmp_root("backfill");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");

    let md = "## Agent Todo\n\n\
        - [ ] [P1] Run the check\n  <!-- loopx:todo todo_id=todo_abc status=open updated_at=2026-08-05T12:00:00+00:00 -->\n\
        - [x] Ship it\n  <!-- loopx:todo todo_id=todo_def status=done no_followup=true evidence=ok completed_at=2026-08-05T13:00:00+00:00 updated_at=2026-08-05T13:00:00+00:00 -->\n";
    let outcome = backfill_todo_events(md, "g1", PrivacyLevel::LocalPrivate).unwrap();
    assert_eq!(outcome.todo_count, 2);

    for event in &outcome.events {
        store
            .append_with_meta(
                event.event.clone(),
                Some(event.event_id.clone()),
                Some(future_loop::backfill::MARKDOWN_BACKFILL_PRODUCER.into()),
                Some(event.source_ref.clone()),
                Some(event.source_section.clone()),
                Some(event.source_line),
                Some(event.privacy.as_str().into()),
            )
            .unwrap();
    }
    // Re-run the backfill → idempotent (no new lines).
    for event in &outcome.events {
        store
            .append_with_meta(
                event.event.clone(),
                Some(event.event_id.clone()),
                Some(future_loop::backfill::MARKDOWN_BACKFILL_PRODUCER.into()),
                Some(event.source_ref.clone()),
                Some(event.source_section.clone()),
                Some(event.source_line),
                Some(event.privacy.as_str().into()),
            )
            .unwrap();
    }
    let report = store.verify("g1").unwrap();
    assert!(report.ok);
    assert_eq!(
        report.idempotent_duplicates, 0,
        "idempotent re-append adds no lines"
    );

    // Provenance survives on the ledger.
    let events = store.events("g1").unwrap();
    let backfilled: Vec<_> = events
        .iter()
        .filter(|e| {
            e.producer.as_deref() == Some(future_loop::backfill::MARKDOWN_BACKFILL_PRODUCER)
        })
        .collect();
    assert_eq!(backfilled.len(), 3, "2 adds + 1 complete");
    let add = backfilled
        .iter()
        .find(|e| e.event_id.starts_with("backfill-add-"))
        .unwrap();
    assert_eq!(add.source_ref.as_deref(), Some("ACTIVE_GOAL_STATE.md"));
    assert_eq!(add.source_section.as_deref(), Some("Agent Todo"));
    assert!(add.source_line.is_some());

    // Replay rebuilds the two todos (one done with evidence).
    let goal = store.replay("g1").unwrap().unwrap();
    assert_eq!(goal.todos.len(), 2);
    let done = goal.todo("todo_def").unwrap();
    assert_eq!(done.status, future_loop::state::TodoStatus::Done);
    assert_eq!(done.evidence.as_deref(), Some("ok"));
}

/// ── QuotaSpent + EvidenceAttached replay ──────────────────────────────────
#[test]
fn quota_spent_and_evidence_events_replay() {
    let root = tmp_root("spend-evidence");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "work"),
            ts: 1_000,
        })
        .unwrap();

    store
        .append(Event::QuotaSpent {
            goal_id: "g1".into(),
            run_id: "run-1".into(),
            todo_id: "t1".into(),
            source: "run".into(),
            slots: 1,
            ts: 1_010,
        })
        .unwrap();
    store
        .append(Event::QuotaSpent {
            goal_id: "g1".into(),
            run_id: "run-2".into(),
            todo_id: "t1".into(),
            source: "agent".into(),
            slots: 1,
            ts: 1_020,
        })
        .unwrap();
    store
        .append(Event::EvidenceAttached {
            goal_id: "g1".into(),
            todo_id: "t1".into(),
            evidence: "validated artifact".into(),
            ts: 1_030,
        })
        .unwrap();

    // Fresh store replay rebuilds the projections.
    let store2 = Store::open(&root).unwrap();
    let goal = store2.replay("g1").unwrap().unwrap();
    assert_eq!(goal.quota_spent_slots, 2, "QuotaSpent events accumulate");
    assert_eq!(
        goal.todo("t1").unwrap().evidence.as_deref(),
        Some("validated artifact")
    );
    let report = store2.verify("g1").unwrap();
    assert!(report.ok);
    assert_eq!(
        report.total_events, 5,
        "started + added + 2 spent + evidence"
    );
}

/// ── Unregistered goal still fails closed ──────────────────────────────────
#[test]
fn append_with_meta_requires_registered_goal() {
    let root = tmp_root("unregistered-meta");
    let mut store = Store::open(&root).unwrap();
    let err = store.append_with_meta(
        Event::GoalStarted {
            goal_id: "ghost".into(),
            ts: 0,
        },
        None,
        None,
        None,
        None,
        None,
        None,
    );
    assert!(err.is_err());
}

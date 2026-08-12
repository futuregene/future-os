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
        "future-loop-p2-events-{tag}-{}",
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
        - [ ] [P1] Run the check\n  <!-- future-loop:todo todo_id=todo_abc status=open updated_at=2026-08-05T12:00:00+00:00 -->\n\
        - [x] Ship it\n  <!-- future-loop:todo todo_id=todo_def status=done no_followup=true evidence=ok completed_at=2026-08-05T13:00:00+00:00 updated_at=2026-08-05T13:00:00+00:00 -->\n";
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

/// ── Fencing token (schema reservation): optional, backward compatible ─────
#[test]
fn fencing_token_is_reserved_and_never_populated_by_kernel_appends() {
    let root = tmp_root("fencing-reserved");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");
    store
        .append(Event::TodoAdded {
            goal_id: "g1".into(),
            todo: Todo::advancement("t1", "work"),
            ts: 1_000,
        })
        .unwrap();
    // Kernel appends do not populate the reserved token and the on-disk line
    // shape is unchanged (no `fencing_token` key) — pre-reservation readers
    // tolerate new lines and content-derived ids stay stable.
    for line in store.raw_ledger_lines("g1").unwrap() {
        assert!(
            !line.contains("fencing_token"),
            "reserved field stays absent: {line}"
        );
    }
    // ...and reads back as None.
    let events = store.events("g1").unwrap();
    assert!(events.iter().all(|e| e.fencing_token.is_none()));
}

/// ── Fencing token: lines carrying a token parse and round-trip ────────────
#[test]
fn fencing_token_round_trips_when_present() {
    use future_loop::store::StoredEvent;
    // A future-producer line carrying a token deserializes (old ledgers
    // without the key deserialize as None — covered by every other test).
    let line = r#"{"event_id":"evt-0123456789abcdef","fencing_token":7,"kind":"goal_started","goal_id":"g1","ts":1000}"#;
    let stored: StoredEvent = serde_json::from_str(line).unwrap();
    assert_eq!(stored.fencing_token, Some(7));
    assert_eq!(stored.event_id, "evt-0123456789abcdef");
    // Re-serializing preserves the token.
    let value = serde_json::to_value(&stored).unwrap();
    assert_eq!(value["fencing_token"], serde_json::json!(7));
    // None is omitted from serialization entirely.
    let without: StoredEvent =
        serde_json::from_str(r#"{"kind":"goal_started","goal_id":"g1","ts":1000}"#).unwrap();
    assert_eq!(without.fencing_token, None);
    assert!(!serde_json::to_string(&without)
        .unwrap()
        .contains("fencing_token"));
}

/// ── Fencing token: writer metadata, not content (idempotent re-append) ────
#[test]
fn fencing_token_does_not_break_idempotent_reappend() {
    // Two raw lines with identical event content and ids but different
    // fencing tokens have equal fingerprints (token is envelope metadata,
    // stripped like `producer` / `privacy`).
    let base: serde_json::Value = serde_json::from_str(
        r#"{"event_id":"evt-abc","kind":"goal_started","goal_id":"g1","ts":1000}"#,
    )
    .unwrap();
    let mut fenced = base.clone();
    fenced["fencing_token"] = serde_json::json!(9);
    assert_eq!(
        future_loop::store::event_fingerprint(&base),
        future_loop::store::event_fingerprint(&fenced)
    );

    // End-to-end: a ledger line that differs from a pending append only in
    // its fencing token is an idempotent no-op, NOT a conflict.
    let root = tmp_root("fencing-idempotent");
    let mut store = Store::open(&root).unwrap();
    open_goal(&mut store, "g1");
    let event = Event::TodoAdded {
        goal_id: "g1".into(),
        todo: Todo::advancement("t1", "work"),
        ts: 1_000,
    };
    let id = store.append(event.clone()).unwrap();
    // Rewrite the ledger line as a future producer would: same id/content
    // plus a fencing token.
    let dir = store.goal_dir("g1");
    let path = dir.join("events.jsonl");
    let mut lines = store.raw_ledger_lines("g1").unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&lines.pop().unwrap()).unwrap();
    value["fencing_token"] = serde_json::json!(42);
    lines.push(serde_json::to_string(&value).unwrap());
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
    // Re-append of the same event: no-op, no conflict, no duplicate line.
    let again = store.append(event).unwrap();
    assert_eq!(id, again);
    assert_eq!(store.raw_ledger_lines("g1").unwrap().len(), 2);
    // Read path collapses the token-carrying duplicate and keeps the token.
    let events = store.events("g1").unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].fencing_token, Some(42));
    assert!(store.verify("g1").unwrap().ok);
}

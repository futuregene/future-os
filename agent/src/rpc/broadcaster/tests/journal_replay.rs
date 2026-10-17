use super::*;

#[test]
fn journal_reads_flush_buffered_deltas_and_enqueue_failure_interrupts() {
    let directory = tempfile::tempdir().unwrap();
    let b = SseBroadcaster::new();
    b.configure_journal(
        "session-1",
        &crate::session::Manager::new(directory.path().to_path_buf()),
    )
    .unwrap();
    b.start_run("run-1".to_string(), 3);
    let interrupt = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    b.set_persistence_interrupt(interrupt.clone());
    let mut receiver = b.subscribe();

    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text":"ok"}),
    ));
    let event = receiver.try_recv().unwrap();
    assert_eq!(event.session_id, "session-1");
    assert_eq!(event.event_id, "session-1:run-1:3:0");
    let stored = b.read_journal("run-1").unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].event_id, event.event_id);

    b.fail_next_append();
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text":"lost"}),
    ));
    assert!(
        receiver.try_recv().is_err(),
        "rejected event must not be visible"
    );
    assert!(interrupt.load(std::sync::atomic::Ordering::SeqCst));
    assert!(b.persistence_error().is_some());
    assert_eq!(b.last_idx(), 0, "failed append must not advance the cursor");
}

#[test]
fn envelope_carries_run_sequence_and_session_replay_has_own_cursor() {
    let directory = tempfile::tempdir().unwrap();
    let b = SseBroadcaster::new();
    b.configure_journal(
        "session-1",
        &crate::session::Manager::new(directory.path().to_path_buf()),
    )
    .unwrap();
    b.start_run_with_sequence("run-1".to_string(), 3, Some(17));
    b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
    let (_, events, _, _) = b.events_since("run-1", -1).unwrap();
    assert_eq!(events[0].run_sequence, 17);
    b.broadcast(SseEvent::new(
        "model_changed",
        serde_json::json!({"model":"x"}),
    ));
    let session_events = b.session_events_since(-1).unwrap();
    assert_eq!(session_events.len(), 1);
    assert_eq!(session_events[0].session_idx, 0);
    assert_eq!(session_events[0].run_sequence, -1);
}

#[test]
fn atomic_attach_replays_disk_when_memory_ring_is_truncated() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let store = manager.storage().unwrap();
    store.bind_events("session-1").unwrap();
    let mut events = Vec::new();
    for idx in 0..(MAX_RUN_EVENTS as i64 + 5) {
        let event = SseEvent {
            event_type: "usage".to_string(),
            data: serde_json::json!({"n": idx}).to_string(),
            run_id: "run-1".to_string(),
            epoch: 2,
            idx,
            session_id: "session-1".to_string(),
            event_id: format!("session-1:run-1:2:{idx}"),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            session_idx: -1,
            run_sequence: 9,
        };
        events.push(serde_json::to_value(event).unwrap());
    }
    store
        .db
        .call(move |db| {
            let tx = db.transaction()?;
            for event in events {
                crate::session::sqlite_store::insert_event_record(&tx, "session-1", event)?;
            }
            tx.commit()?;
            Ok(())
        })
        .unwrap();

    let b = SseBroadcaster::new();
    b.configure_journal(
        "session-1",
        &crate::session::Manager::new(directory.path().to_path_buf()),
    )
    .unwrap();
    b.start_run("run-1".to_string(), 2);
    let attachment = b.attach("run-1", -1).unwrap();
    assert!(attachment.truncated);
    assert!(attachment.projection.is_none());
    assert_eq!(attachment.events.len(), MAX_RUN_EVENTS + 5);
    assert_eq!(attachment.events.first().unwrap().idx, 0);
    assert_eq!(
        attachment.events.last().unwrap().idx,
        MAX_RUN_EVENTS as i64 + 4
    );
}

#[test]
fn events_since_reads_a_settled_run_from_its_durable_journal() {
    let directory = tempfile::tempdir().unwrap();
    let broadcaster = SseBroadcaster::new();
    broadcaster
        .configure_journal(
            "session-1",
            &crate::session::Manager::new(directory.path().to_path_buf()),
        )
        .unwrap();
    broadcaster.start_run("run-a".to_string(), 1);
    broadcaster.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
    broadcaster.broadcast(SseEvent::new("agent_end", serde_json::json!({})));
    broadcaster.start_run("run-b".to_string(), 2);

    let (run_id, events, _, projection) = broadcaster.events_since("run-a", -1).unwrap();
    assert_eq!(run_id, "run-a");
    assert_eq!(events.len(), 2);
    assert!(projection.is_none());
    let (_, tail, _, _) = broadcaster.events_since("run-a", 0).unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].idx, 1);
    let (_, exhausted, _, _) = broadcaster.events_since("run-a", 1).unwrap();
    assert!(exhausted.is_empty());
    assert!(broadcaster.events_since("unknown", -1).is_err());
}

/// A settled run that has been **compacted** is read from its published
/// projection snapshot. Two entry points do that along two different paths, and
/// neither is reachable without a snapshot in the store: `run_snapshot` for a
/// *historical* run, and `events_page` for the **current** run once its
/// in-memory ring has dropped events the client still asks for.
#[test]
fn a_compacted_run_is_served_from_its_published_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let broadcaster = SseBroadcaster::new();
    broadcaster
        .configure_journal("session-snapshot", &manager)
        .unwrap();
    broadcaster.start_run("run-settled".to_string(), 1);
    broadcaster.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
    broadcaster.broadcast(SseEvent::new("agent_end", serde_json::json!({})));
    // Move on, so `run-settled` is historical from here.
    broadcaster.start_run("run-live".to_string(), 2);

    // Compaction only retires a run the store has settled: seed that row first.
    manager.test_execute(
        "INSERT INTO runs(session_id,run_id,status,completed_at_ms) \
         VALUES ('session-snapshot','run-settled','completed',1)",
    );
    manager
        .storage()
        .unwrap()
        .compact_run("session-snapshot", "run-settled")
        .unwrap();

    // A historical cursor below the snapshot's returns the folded projection and
    // replays no raw events. The tuple's third value is the minimum index of
    // those (empty) events, not the cursor, so it reads 0.
    let (run_id, events, min_idx, projection) =
        broadcaster.events_since("run-settled", -1).unwrap();
    assert_eq!(run_id, "run-settled");
    assert!(events.is_empty(), "a snapshot replaces the raw replay");
    assert_eq!(min_idx, 0, "no raw events were returned");
    let projection = projection.expect("the published snapshot must be returned");
    assert_eq!(projection.run_id, "run-settled");
    // Two events were broadcast (idx 0 and 1), so the snapshot's cursor is 1.
    assert_eq!(projection.cursor, 1, "the snapshot's own cursor");
    assert!(
        !projection.events.is_empty(),
        "the snapshot carries the folded projection: {projection:?}"
    );

    // `run_snapshot` for the same historical run takes its own snapshot branch.
    let direct = broadcaster.run_snapshot("run-settled").unwrap();
    assert_eq!(direct.run_id, "run-settled");
    assert_eq!(direct.cursor, projection.cursor);
    assert_eq!(
        serde_json::to_value(&direct.events).unwrap(),
        serde_json::to_value(&projection.events).unwrap()
    );
}

/// The same store preference, taken by the **current** run: once its in-memory
/// ring has dropped events a client still asks for, `events_page` reads the
/// journal instead of answering from memory, and prefers a published snapshot
/// over replaying raw rows.
#[test]
fn a_truncated_current_run_prefers_its_published_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let broadcaster = SseBroadcaster::new();
    broadcaster
        .configure_journal("session-ring", &manager)
        .unwrap();
    // Stay in ONE run and overflow the ring (MAX_RUN_EVENTS = 2000), so the
    // oldest events are gone while the run is still the current one.
    broadcaster.start_run("run-ring".to_string(), 1);
    for index in 0..2_100 {
        broadcaster.broadcast(SseEvent::new("text_chunk", serde_json::json!({"i": index})));
    }
    // Compaction only retires a run whose LAST raw event is `agent_end`
    // (`journal_compaction.rs` returns Ok(0) otherwise), so the run has to be
    // closed properly even though the broadcaster keeps it as its current run.
    broadcaster.broadcast(SseEvent::new("agent_end", serde_json::json!({})));
    broadcaster.flush_journal().unwrap();

    // Settle and compact it: the snapshot lives in the store, while the ring
    // does not know the run has been retired.
    manager.test_execute(
        "INSERT INTO runs(session_id,run_id,status,completed_at_ms) \
         VALUES ('session-ring','run-ring','completed',1)",
    );
    let retired = manager
        .storage()
        .unwrap()
        .compact_run("session-ring", "run-ring")
        .unwrap();
    assert!(
        retired > 0,
        "the run must have raw rows to retire (its last event is agent_end)"
    );

    // The client's cursor is below what the ring still holds, so the answer can
    // only come from the store — and the store has a snapshot.
    let (run_id, events, cursor, projection) =
        broadcaster.events_page("run-ring", -1, None).unwrap();
    assert_eq!(run_id, "run-ring");
    assert!(events.is_empty(), "the snapshot replaces the raw replay");
    let projection = projection.expect("the published snapshot must be preferred");
    assert_eq!(projection.run_id, "run-ring");
    assert_eq!(projection.cursor, cursor);
}

#[test]
fn pre_run_compaction_events_do_not_collide_with_session_events_on_restart() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let first = SseBroadcaster::new();
    first.configure_journal("s", &manager).unwrap();
    first.broadcast(SseEvent::new("model_changed", serde_json::json!({})));
    first.broadcast(SseEvent::new("compaction_started", serde_json::json!({})));
    assert!(first.persistence_error().is_none());
    let second = SseBroadcaster::new();
    second.configure_journal("s", &manager).unwrap();
    second.broadcast(SseEvent::new("compaction_committed", serde_json::json!({})));
    assert!(second.persistence_error().is_none());
    let events = second.read_journal("").unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].session_idx, 0);
    assert_eq!(events[1].idx, 0);
    assert_eq!(events[2].idx, 1);
    assert_eq!(second.session_events_since(-1).unwrap().len(), 1);
    assert!(second.session_events_since(0).unwrap().is_empty());
    assert_eq!(second.session_events_since(-2).unwrap().len(), 3);
}

#[test]
fn events_since_beyond_ring_reads_disk_journal() {
    let dir = tempfile::tempdir().unwrap();
    let broadcaster = SseBroadcaster::new();
    broadcaster
        .configure_journal(
            "s1".to_string(),
            &crate::session::Manager::new(dir.path().to_path_buf()),
        )
        .unwrap();
    broadcaster.start_run("run-long".to_string(), 1);
    for i in 0..2100 {
        broadcaster.broadcast(SseEvent::new("text_chunk", serde_json::json!({"i": i})));
    }
    // The in-memory ring holds 2000; idx 0 is truncated out.
    let (_, events, _, projection) = broadcaster.events_since("run-long", 0).unwrap();
    assert!(
        projection.is_none(),
        "journal present → disk replay, not projection"
    );
    let event_count = events.len();
    assert!(event_count > 2000, "full history from disk: {event_count}");
}

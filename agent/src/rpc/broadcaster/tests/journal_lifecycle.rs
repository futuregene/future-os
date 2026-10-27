use super::*;

/// A store that cannot accept the batch must fail the durable caller, mark
/// the journal unhealthy, and make the read path report it rather than
/// return a silently short history.
#[test]
fn a_failed_batch_commit_is_reported_and_blocks_the_read_path() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let b = SseBroadcaster::new();
    b.configure_legacy_journal("session-broken", &manager)
        .unwrap();
    b.start_run("run-broken".to_string(), 1);
    b.broadcast(SseEvent::new("agent_end", serde_json::json!({})));
    assert_eq!(b.journal_commit_count(), 1, "the healthy commit must land");
    assert_eq!(b.read_journal("run-broken").unwrap().len(), 1);

    // The table the event batch writes to is gone, so the next commit fails
    // exactly as it would on a corrupt store.
    manager.test_execute("DROP TABLE run_events");
    b.broadcast(SseEvent::new("agent_end", serde_json::json!({})));

    let error = b
        .persistence_error()
        .expect("a failed commit must be recorded as a persistence error");
    assert!(
        error.contains("event batch commit failed") || error.contains("persistence"),
        "{error}"
    );
    assert!(
        b.read_journal("run-broken").is_err(),
        "the read path must surface the failure, not a truncated history"
    );
    // The snapshot resume path reads the same journal, so it must refuse too: a
    // client resuming from an unhealthy journal would otherwise be handed a
    // prefix the store never durably held.
    let snapshot = b.run_snapshot("run-broken").unwrap_err();
    assert!(
        snapshot
            .to_string()
            .contains("run snapshot persistence is unhealthy"),
        "{snapshot}"
    );
}

#[test]
fn closed_journal_rejects_late_broadcast_without_recreating_files() {
    let directory = tempfile::tempdir().unwrap();
    let b = SseBroadcaster::new();
    b.configure_legacy_journal(
        "session-delete",
        &crate::session::Manager::new(directory.path().to_path_buf()),
    )
    .unwrap();
    b.start_run("run-delete".to_string(), 1);
    let mut receiver = b.subscribe();

    b.close_journal();
    std::fs::remove_dir_all(directory.path()).unwrap();
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text":"too late"}),
    ));

    assert!(!directory.path().exists());
    assert!(receiver.try_recv().is_err());
}

#[test]
fn closing_journal_flushes_pending_deltas() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let b = SseBroadcaster::new();
    b.configure_legacy_journal("session-close", &manager)
        .unwrap();
    b.start_run("run-close".to_string(), 1);
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text":"pending"}),
    ));

    b.close_journal();

    let events = manager
        .storage()
        .unwrap()
        .events("session-close", "run-close")
        .unwrap();
    assert_eq!(events.len(), 1);
}

#[test]
fn journal_failpoints_cover_append_flush_and_sync_boundaries() {
    for point in [
        JournalFailPoint::Append,
        JournalFailPoint::Flush,
        JournalFailPoint::Sync,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let b = SseBroadcaster::new();
        b.configure_legacy_journal(
            "session-1",
            &crate::session::Manager::new(directory.path().to_path_buf()),
        )
        .unwrap();
        b.start_run("run-1".to_string(), 1);
        let mut receiver = b.subscribe();
        b.fail_at(point);
        b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        assert!(receiver.try_recv().is_err());
        assert!(b.persistence_error().is_some());
        assert_eq!(b.last_idx(), -1);
    }
}

// ─── coverage batch: journal resume/recovery/projection arms ───────────

#[test]
fn configure_journal_resumes_session_idx_sequence() {
    let dir = tempfile::tempdir().unwrap();
    // Pre-existing session journal with events at session_idx 0 and 1.
    let first = SseBroadcaster::new();
    first
        .configure_legacy_journal(
            "s1".to_string(),
            &crate::session::Manager::new(dir.path().to_path_buf()),
        )
        .unwrap();
    first.broadcast(SseEvent::new("model_changed", serde_json::json!({"m": 1})));
    first.broadcast(SseEvent::new("model_changed", serde_json::json!({"m": 2})));

    let second = SseBroadcaster::new();
    second
        .configure_legacy_journal(
            "s1".to_string(),
            &crate::session::Manager::new(dir.path().to_path_buf()),
        )
        .unwrap();
    second.broadcast(SseEvent::new("model_changed", serde_json::json!({"m": 3})));
    let events = second.session_events_since(-1).unwrap();
    let idxs: Vec<i64> = events.iter().map(|e| e.session_idx).collect();
    assert_eq!(idxs, vec![0, 1, 2], "resumed after the on-disk sequence");
}

#[test]
fn configured_journal_cannot_be_rebound_to_another_session() {
    let dir = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(dir.path().to_path_buf());
    let broadcaster = SseBroadcaster::new();
    broadcaster
        .configure_legacy_journal("parent", &manager)
        .unwrap();

    let error = broadcaster
        .configure_legacy_journal("child", &manager)
        .expect_err("journal ownership must be immutable");
    assert!(error.to_string().contains("cannot rebind"));
    broadcaster.broadcast(SseEvent::new("model_changed", serde_json::json!({})));
    let event = broadcaster.session_events_since(-1).unwrap().pop().unwrap();
    assert_eq!(event.session_id, "parent");
}

#[test]
fn set_persistence_interrupt_flags_when_journal_failed() {
    let dir = tempfile::tempdir().unwrap();
    // A file where the journal directory belongs → configure fails.
    let blocker = dir.path().join("blocked");
    std::fs::write(&blocker, "x").unwrap();
    let broadcaster = SseBroadcaster::new();
    assert!(broadcaster
        .configure_journal(
            "s1".to_string(),
            &crate::session::Manager::new(blocker.join("sub"))
        )
        .is_err());
    assert!(broadcaster.persistence_error().is_some());
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    broadcaster.set_persistence_interrupt(flag.clone());
    assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn start_run_recovers_from_corrupt_journal() {
    let dir = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(dir.path().to_path_buf());
    let broadcaster = SseBroadcaster::new();
    broadcaster
        .configure_legacy_journal("s1".to_string(), &manager)
        .unwrap();
    manager.test_execute("DROP TABLE run_events");
    broadcaster.start_run("run-x".to_string(), 1);
    assert!(broadcaster.persistence_error().is_some());
}

#[test]
fn broadcast_without_configured_journal_is_in_memory_only() {
    let broadcaster = SseBroadcaster::new();
    broadcaster.start_run("run-mem".to_string(), 1);
    broadcaster.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "x"}),
    ));
    assert!(broadcaster.persistence_error().is_none());
    let (_, events, _, _) = broadcaster.events_since("run-mem", -1).unwrap();
    assert_eq!(events.len(), 1);
}

/// A settled run's raw rows are retired behind a snapshot, so a delta still
/// queued for that run can no longer be appended. That refusal is permanent:
/// the event used to stay in the batch and fail every later batch with it, so
/// one late delta silently took the whole session's persistence down until the
/// agent restarted.
#[test]
fn an_event_the_journal_refuses_is_dropped_without_taking_the_session_down() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let b = SseBroadcaster::new();
    b.configure_legacy_journal("session-refused", &manager)
        .unwrap();

    b.start_run("run-settled".to_string(), 1);
    b.broadcast(SseEvent::new("agent_end", serde_json::json!({"n": 1})));
    assert_eq!(b.journal_commit_count(), 1, "the settled run must commit");

    // What run completion does afterwards, on its own thread: publish the
    // snapshot and retire the run's raw rows. Compaction only retires a run
    // the store has settled, so seed that row and compact it.
    manager.test_execute(
        "INSERT INTO runs(session_id,run_id,status,completed_at_ms) \
         VALUES ('session-refused','run-settled','completed',1)",
    );
    manager
        .storage()
        .unwrap()
        .compact_run("session-refused", "run-settled")
        .unwrap();

    // The late delta this guards against. It is buffered (no reply), so a
    // failed batch keeps it and retries it forever.
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "late"}),
    ));
    // A session-scoped event is durable, so it flushes the batch holding the
    // refusal — the exact shape the incident's journal showed.
    b.broadcast(SseEvent::new(
        "permission_level_changed",
        serde_json::json!({"level": "workspace"}),
    ));

    assert_eq!(
        b.persistence_error(),
        None,
        "a permanently refused event must not mark the journal unhealthy"
    );
    assert_eq!(
        b.session_events_since(-1).unwrap().len(),
        1,
        "the event after the refusal must still land"
    );
    assert_eq!(
        b.read_journal("run-settled").unwrap().len(),
        1,
        "the refused late delta must be dropped, not appended"
    );
}

/// A refused event is not a session-wide outage. The writer already drops the
/// one event the journal refuses, but `broadcast` recorded that refusal as a
/// journal health failure — and health is only cleared by a successful
/// `start_run` journal read, which the failure itself blocks. So a single
/// unappendable *durable* event (a manual compaction's lifecycle signal stamped
/// with the run that just settled, whose journal was already snapshotted) left
/// every later prompt failing at the flush barrier until the agent restarted:
/// the conversation was permanently unwritable while its status read healthy.
#[test]
fn a_refused_durable_event_does_not_wedge_the_session() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let b = SseBroadcaster::new();
    b.configure_legacy_journal("session-refused-durable", &manager)
        .unwrap();

    b.start_run("run-settled".to_string(), 1);
    b.broadcast(SseEvent::new("agent_end", serde_json::json!({"n": 1})));
    assert_eq!(b.journal_commit_count(), 1);
    manager.test_execute(
        "INSERT INTO runs(session_id,run_id,status,completed_at_ms) \
         VALUES ('session-refused-durable','run-settled','completed',1)",
    );
    manager
        .storage()
        .unwrap()
        .compact_run("session-refused-durable", "run-settled")
        .unwrap();

    // A durable event stamped with the retired run: the store refuses it for
    // good, so it must be dropped — but only the event, not the session.
    b.broadcast(SseEvent::new("agent_end", serde_json::json!({"n": 2})));
    assert_eq!(
        b.persistence_error(),
        None,
        "a refused event must not be reported as a session-wide journal failure"
    );

    // The session still accepts the next run's journal writes.
    b.start_run("run-next".to_string(), 2);
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "next"}),
    ));
    assert_eq!(
        b.read_journal("run-next").unwrap().len(),
        1,
        "the next run must journal normally"
    );
}

/// `recover_storage` drains the writer before it can clear the health flag, so
/// a refusal left in the batch used to make recovery fail too — leaving no way
/// back but a restart.
#[test]
fn the_recover_path_is_not_blocked_by_a_refused_event() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let b = SseBroadcaster::new();
    b.configure_legacy_journal("session-recover", &manager)
        .unwrap();

    b.start_run("run-settled".to_string(), 1);
    b.broadcast(SseEvent::new("agent_end", serde_json::json!({"n": 1})));
    manager.test_execute(
        "INSERT INTO runs(session_id,run_id,status,completed_at_ms) \
         VALUES ('session-recover','run-settled','completed',1)",
    );
    manager
        .storage()
        .unwrap()
        .compact_run("session-recover", "run-settled")
        .unwrap();
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "late"}),
    ));

    b.recover_storage()
        .expect("a refusal in the batch must not block recovery");
    assert!(b.persistence_error().is_none());
}

/// `flush_journal` is what run completion waits on before it asks for
/// compaction: it must put the buffered batch in the store, so the snapshot
/// published next cannot race a delta still in memory.
#[test]
fn flush_journal_puts_the_buffered_batch_in_the_store() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let b = SseBroadcaster::new();
    b.configure_legacy_journal("session-flush", &manager)
        .unwrap();
    b.start_run("run-flush".to_string(), 1);

    // A delta is buffered, not durable: nothing is committed yet.
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "buffered"}),
    ));
    assert_eq!(b.journal_commit_count(), 0, "deltas stay in the batch");

    b.flush_journal().unwrap();
    assert_eq!(b.journal_commit_count(), 1, "flushing commits the batch");
    assert_eq!(b.read_journal("run-flush").unwrap().len(), 1);
}

/// A broadcaster that never bound a journal is a supported state — the doc on
/// `configure_journal` says tests and short-lived utility broadcasters may stay
/// memory-only on purpose. Both journal lifecycle calls a host makes anyway
/// (the per-run completion flush and the drain's shutdown) must therefore be
/// no-ops that succeed, rather than reporting a failure the caller cannot fix.
#[test]
fn journal_lifecycle_calls_are_no_ops_without_a_configured_journal() {
    let broadcaster = SseBroadcaster::new();
    assert!(
        broadcaster.flush_journal().is_ok(),
        "flushing a memory-only broadcaster must succeed"
    );
    assert!(
        broadcaster.shutdown_journal().is_ok(),
        "shutting down a memory-only broadcaster must succeed"
    );
    // Shutdown is terminal and still idempotent: a second call, and a flush
    // afterwards, must not resurrect a journal or report an error.
    assert!(broadcaster.shutdown_journal().is_ok());
    assert!(broadcaster.flush_journal().is_ok());
    assert_eq!(broadcaster.journal_commit_count(), 0);
}

#[test]
fn attach_beyond_ring_without_journal_returns_projection() {
    let broadcaster = SseBroadcaster::new();
    broadcaster.start_run("run-ring".to_string(), 1);
    for i in 0..2100 {
        broadcaster.broadcast(SseEvent::new("text_chunk", serde_json::json!({"i": i})));
    }
    let attachment = broadcaster.attach("run-ring", 0).unwrap();
    let projection = attachment
        .projection
        .expect("projection over the truncated ring");
    assert_eq!(projection.run_id, "run-ring");
    assert!(!projection.events.is_empty());
}

use super::*;

#[test]
fn transient_full_event_queue_backpressures_without_failing_health() {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let health = std::sync::Arc::new(EventJournalHealth::default());
    let writer = std::sync::Arc::new(EventBatchWriter {
        sender,
        thread: None,
        health: health.clone(),
        commits: Default::default(),
    });
    writer
        .sender
        .send(EventWriteCommand::Append {
            event: serde_json::json!({"n":1}),
            reply: None,
        })
        .unwrap();
    let copy = writer.clone();
    let (done, completed) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        done.send(copy.append(serde_json::json!({"n":2}), false))
            .unwrap();
    });
    let early = completed.recv_timeout(std::time::Duration::from_millis(50));
    let first = receiver.recv().unwrap();
    let result = match early {
        Ok(result) => result,
        Err(_) => completed
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap(),
    };
    thread.join().unwrap();
    let second = receiver.try_recv();
    drop(receiver); // make the test writer's Drop nonblocking
    assert!(result.is_ok());
    assert!(health.error().is_none());
    assert!(matches!(first, EventWriteCommand::Append { event, .. } if event["n"] == 1));
    assert!(matches!(second, Ok(EventWriteCommand::Append { event, .. }) if event["n"] == 2));
}

/// The batch writer's own contract, driven directly: it commits when the
/// journal is healthy, refuses with the recorded error once the journal is
/// unhealthy (without touching the store), and detects a dead worker.
#[test]
fn event_batch_writer_refuses_appends_when_unhealthy_or_when_its_worker_is_gone() {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::session::sqlite_store::SqliteStore::open(&directory.path().join("agent.db"))
        .unwrap();
    store.bind_events("session-direct").unwrap();
    let health = std::sync::Arc::new(EventJournalHealth::default());
    let mut writer =
        EventBatchWriter::new(store, "session-direct".to_string(), health.clone()).unwrap();
    let event = serde_json::to_value(SseEvent::new("agent_end", serde_json::json!({}))).unwrap();

    // Healthy: a durable append is committed and acknowledged.
    writer.append(event.clone(), true).unwrap();
    assert_eq!(writer.commit_count(), 1);

    // Unhealthy: the recorded failure is the error, and nothing is written.
    health.fail("injected journal failure".to_string());
    let error = writer.append(event.clone(), true).unwrap_err();
    assert!(
        error.to_string().contains("injected journal failure"),
        "{error}"
    );
    assert_eq!(
        writer.commit_count(),
        1,
        "a refused append must not reach the store"
    );

    // Worker gone: the enqueue fails and the writer records the loss so
    // later callers get the same error instead of silently dropping events.
    // The retirement is joined, not slept on: after `stop_worker_for_test`
    // the thread has provably dropped its receiver, so the failure below is
    // deterministic on any machine speed.
    health.clear();
    writer.stop_worker_for_test();
    let error = writer.append(event, true).unwrap_err();
    assert!(
        error.to_string().contains("worker is unavailable"),
        "{error}"
    );
    assert!(
        health.error().is_some(),
        "losing the worker must mark the journal unhealthy"
    );

    // Retirement is idempotent: a shutdown *after* the worker is already gone
    // succeeds, rather than reporting a failure the caller cannot act on. This
    // is the path a recovered or already-retired writer takes at drain time.
    writer
        .shutdown()
        .expect("shutting down a writer whose worker is gone must be a no-op");
}

/// Once a batch commit has failed, the worker itself must refuse later
/// durable writes — and an explicit flush must report the failure — even if
/// the shared health flag was cleared in the meantime. Reporting success
/// there would acknowledge durability that did not happen.
#[test]
fn event_batch_writer_refuses_durable_writes_after_a_failed_batch() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let store = manager.storage().unwrap().clone();
    store.bind_events("session-failed-batch").unwrap();
    let health = std::sync::Arc::new(EventJournalHealth::default());
    let writer =
        EventBatchWriter::new(store, "session-failed-batch".to_string(), health.clone()).unwrap();
    let event = serde_json::to_value(SseEvent::new("agent_end", serde_json::json!({}))).unwrap();
    writer.append(event.clone(), true).unwrap();
    assert_eq!(writer.commit_count(), 1);

    // The table the batch writes to is gone: the durable append reports the
    // store failure, records it, and the batch is now failed.
    manager.test_execute("DROP TABLE run_events");
    let error = writer.append(event.clone(), true).unwrap_err();
    assert!(
        error.to_string().contains("event batch commit failed"),
        "{error}"
    );
    assert_eq!(
        writer.commit_count(),
        1,
        "a failed batch must not count as a commit"
    );

    // Clearing the shared health flag must not make the worker accept a
    // durable append it cannot commit: it answers with the failure instead.
    health.clear();
    let error = writer.append(event.clone(), true).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("event persistence is unavailable"),
        "a rejected durable append must say why: {error}"
    );
    assert_eq!(writer.commit_count(), 1);

    // An explicit flush with a still-pending event must fail the same way
    // rather than answer Ok. The worker is held first so the queued event
    // is provably still pending when the flush runs (no batch-window race).
    // The error is the store failure: the flush records the reason it could
    // not answer before reporting it.
    health.clear();
    let gate = writer.hold_worker_for_test();
    gate.wait(); // worker is parked inside the command loop
    writer.append(event.clone(), false).unwrap();
    std::thread::scope(|scope| {
        let gate = gate.clone();
        scope.spawn(move || gate.wait());
        let error = writer.flush().unwrap_err();
        assert!(
            error.to_string().contains("no such table: run_events"),
            "a flush the store could not satisfy must report why: {error}"
        );
    });
    assert_eq!(writer.commit_count(), 1);
    assert!(
        health
            .error()
            .is_some_and(|error| error.contains("no such table: run_events")),
        "the failed flush must leave the journal unhealthy"
    );
}

/// Both ways a worker can find its command channel gone: parked in `recv()`
/// with an empty batch, and parked in `recv_timeout` with a batch still
/// pending. The second must drop that batch instead of committing it late,
/// and both must leave the writer failing closed rather than hanging.
/// The narrowest window in the batch writer, and the only one that needs a
/// hand-made channel: the append was **accepted** (`send` returned), the caller
/// is blocked waiting for its acknowledgement, and the receiving end goes away
/// before that acknowledgement can be sent. Nothing in the queue is lost — the
/// queued `Append` carries the reply sender, so dropping the queue drops it.
///
/// Why the receiving end must be held by the test: with a live worker, both of
/// the ways it can stop — the `Shutdown` handshake and `Drop` — flush pending
/// events and answer them first, so a reply is never abandoned, and every
/// remaining append path fails at the *send* (the arm covered by
/// `event_batch_writer_refuses_appends_when_unhealthy_or_when_its_worker_is_gone`).
/// Reaching this arm therefore needs a worker that dies with an accepted command
/// in hand, which is exactly what holding and dropping the queue models.
#[test]
fn an_accepted_durable_append_reports_unavailable_when_the_worker_dies_before_answering() {
    let (sender, receiver) = std::sync::mpsc::sync_channel(4);
    let health = std::sync::Arc::new(EventJournalHealth::default());
    let writer = std::sync::Arc::new(EventBatchWriter {
        sender,
        thread: None,
        health: health.clone(),
        commits: Default::default(),
    });

    let copy = writer.clone();
    let (done, completed) = std::sync::mpsc::channel();
    let appender = std::thread::spawn(move || {
        let _ = done.send(copy.append(serde_json::json!({"n": 1}), true));
    });

    // Receiving the command proves the append was accepted: the caller has passed
    // the `send` and is now waiting on the reply channel. Dropping the command and
    // the queue is what a worker dying with it in hand does — the reply sender
    // lives inside the command, so its acknowledgement can never be sent.
    let accepted = receiver.recv().expect("the append must be accepted");
    assert!(
        matches!(accepted, EventWriteCommand::Append { .. }),
        "a durable append queues an Append command"
    );
    drop(accepted);
    drop(receiver);

    let outcome = completed
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("the appender must not block forever");
    appender.join().unwrap();
    assert!(
        matches!(
            outcome,
            Err(AppendFailure::Unavailable(ref message))
                if message.contains("event persistence worker is unavailable")
        ),
        "an accepted append whose worker died must be reported as unavailable: {outcome:?}"
    );
    assert!(
        health.error().is_none(),
        "losing the reply is not a storage failure, so it must not mark the journal unhealthy"
    );
}

#[test]
fn event_batch_worker_exits_when_its_last_sender_disappears() {
    let event = serde_json::to_value(SseEvent::new("agent_end", serde_json::json!({}))).unwrap();

    // (a) Empty batch: the worker is blocked in `recv()`.
    {
        let directory = tempfile::tempdir().unwrap();
        let store =
            crate::session::sqlite_store::SqliteStore::open(&directory.path().join("agent.db"))
                .unwrap();
        store.bind_events("session-empty-batch").unwrap();
        let health = std::sync::Arc::new(EventJournalHealth::default());
        let mut writer =
            EventBatchWriter::new(store, "session-empty-batch".to_string(), health.clone())
                .unwrap();
        let gate = writer.hold_worker_for_test();
        gate.wait();
        writer.disconnect_worker_for_test();
        gate.wait();
        // The worker left through the disconnected `recv()`; nothing else
        // can have consumed the command, and the writer now fails closed.
        let error = writer.append(event.clone(), true).unwrap_err();
        assert!(
            error.to_string().contains("worker is unavailable"),
            "{error}"
        );
        assert!(health.error().is_some());
    }

    // (b) Pending batch: the worker is blocked in `recv_timeout()`.
    {
        let directory = tempfile::tempdir().unwrap();
        let manager = crate::session::Manager::new(directory.path().to_path_buf());
        let store = manager.storage().unwrap().clone();
        store.bind_events("session-pending-batch").unwrap();
        let before = store.event_cursors("session-pending-batch").unwrap();
        let health = std::sync::Arc::new(EventJournalHealth::default());
        let mut writer =
            EventBatchWriter::new(store.clone(), "session-pending-batch".to_string(), health)
                .unwrap();
        writer.append(event.clone(), false).unwrap();
        let gate = writer.hold_worker_for_test();
        gate.wait();
        writer.disconnect_worker_for_test();
        gate.wait();
        // Dropping the writer joins the worker, so the "nothing was
        // committed" claim below is about a thread that has provably
        // finished, not about a sleep that happened to be long enough.
        drop(writer);
        assert_eq!(
            store.event_cursors("session-pending-batch").unwrap(),
            before,
            "a worker that died with a pending batch must not commit it late"
        );
    }
}

#[test]
fn delta_burst_commits_as_one_batch_at_a_durable_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let b = SseBroadcaster::new();
    b.configure_journal(
        "session-batch",
        &crate::session::Manager::new(directory.path().to_path_buf()),
    )
    .unwrap();
    b.start_run("run-batch".to_string(), 1);

    for index in 0..32 {
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": index.to_string()}),
        ));
    }
    b.broadcast(SseEvent::new("agent_end", serde_json::json!({})));

    assert_eq!(b.journal_commit_count(), 1);
    assert_eq!(b.read_journal("run-batch").unwrap().len(), 33);
}

#[test]
fn lone_delta_is_flushed_after_the_batch_window() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let b = SseBroadcaster::new();
    b.configure_journal("session-timer", &manager).unwrap();
    b.start_run("run-timer".to_string(), 1);
    b.broadcast(SseEvent::new(
        "thinking_delta",
        serde_json::json!({"text":"safe"}),
    ));

    // Start observing the batch window only after the worker accepted the
    // delta. Thread startup under CI load is not part of the flush deadline.
    let gate = b
        .journal
        .lock()
        .writer
        .as_ref()
        .unwrap()
        .hold_worker_for_test();
    gate.wait();
    gate.wait();

    std::thread::sleep(EVENT_BATCH_WINDOW + std::time::Duration::from_millis(100));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while b.journal_commit_count() == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let count = manager
        .storage()
        .unwrap()
        .db
        .call(|db| {
            Ok(db.query_row("SELECT count(*) FROM run_events", [], |row| {
                row.get::<_, i64>(0)
            })?)
        })
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(b.journal_commit_count(), 1);
}

#[test]
fn asynchronous_delta_failure_interrupts_the_run() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let b = SseBroadcaster::new();
    b.configure_journal("session-failure", &manager).unwrap();
    b.start_run("run-failure".to_string(), 1);
    let interrupt = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    b.set_persistence_interrupt(interrupt.clone());
    let mut receiver = b.subscribe();
    manager.test_execute("DROP TABLE run_events");

    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text":"visible-before-flush"}),
    ));
    assert!(receiver.try_recv().is_ok());
    // Start observing the batch window only after the worker accepted the
    // delta. Thread startup under CI load is not part of the flush deadline.
    let gate = b
        .journal
        .lock()
        .writer
        .as_ref()
        .unwrap()
        .hold_worker_for_test();
    gate.wait();
    gate.wait();
    // The journal's failure is eventual, not synchronous: the worker flushes on
    // its batch window, and its thread competes with the rest of a fully
    // parallel suite. Five seconds still fails a journal that never reports,
    // while not measuring how fast a loaded runner schedules the worker (the
    // previous 500 ms bound failed once on `main`, run 39767494038).
    for _ in 0..500 {
        if b.persistence_error().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(b.persistence_error().is_some());
    assert!(interrupt.load(std::sync::atomic::Ordering::SeqCst));
}

/// The batch-retry loop's two remaining arms, driven by one carefully shaped
/// batch: a first event this journal *refuses* (so the whole batch fails as a
/// refusal and the writer retries event by event), and later events whose single
/// append hits a **storage** problem instead.
///
/// Dropping `storage_meta` is what makes that split possible: the refusal is
/// decided from the event's own shape before any storage is touched, while the
/// insert needs `storage_meta` for its sequence. So the batch fails as a refusal
/// (first event), and the retry of the *valid* events fails as storage — which is
/// the distinction the writer must keep, because one is permanent for that event
/// and the other is not.
#[test]
fn a_refused_batch_retries_singly_and_separates_refusal_from_storage_failure() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let store = manager.storage().unwrap().clone();
    store.bind_events("session-mixed-batch").unwrap();
    let health = std::sync::Arc::new(EventJournalHealth::default());
    let writer = EventBatchWriter::new(
        store.clone(),
        "session-mixed-batch".to_string(),
        health.clone(),
    )
    .unwrap();

    // An event the journal refuses on its own shape: no epoch to key it by.
    let refused = serde_json::json!({
        "event_type": "agent_end", "run_id": "run-1", "idx": 0, "session_id": "session-mixed-batch",
    });
    let valid_a = serde_json::json!({
        "event_type": "agent_end", "run_id": "run-1", "epoch": 0, "idx": 1,
        "session_id": "session-mixed-batch",
    });
    let valid_b = serde_json::json!({
        "event_type": "agent_end", "run_id": "run-1", "epoch": 0, "idx": 2,
        "session_id": "session-mixed-batch",
    });

    // The insert cannot run: the sequence source is gone. The refusal above is
    // decided before this table is consulted, which is what splits the two.
    manager.test_execute("DROP TABLE storage_meta");

    // Queue the refused event and one valid event without a reply, then make the
    // next append durable so the batch is committed (and fails) right away.
    writer.append(refused, false).unwrap();
    writer.append(valid_b, false).unwrap();
    let error = writer
        .append(valid_a.clone(), true)
        .expect_err("the batch cannot be committed");
    // The durable caller is told the storage problem, not the refusal: its own
    // event was fine.
    assert!(
        matches!(error, AppendFailure::Unavailable(_)),
        "a durable append whose own event is valid must hear the storage failure: {error:?}"
    );
    assert!(
        error.to_string().contains("event batch commit failed"),
        "{error}"
    );
    assert!(
        health
            .error()
            .is_some_and(|error| error.contains("event batch commit failed")),
        "the journal is now unhealthy, with the refusing batch's error: {:?}",
        health.error()
    );
    // Nothing landed, and the batch is still pending behind the failure.
    assert_eq!(store.event_cursors("session-mixed-batch").unwrap().1, 0);

    // The event with no reply is quietly requeued for the next attempt rather
    // than dropped, so a later healthy commit can still deliver it.
    let error = writer.append(valid_a, true).expect_err("still unavailable");
    assert!(matches!(error, AppendFailure::Unavailable(_)), "{error:?}");
}

/// Durable failures must publish unhealthy state even when there is no
/// unacknowledged event to retain. The real queue is parked so both shapes
/// reach one batch without depending on the timer or runner scheduling.
#[test]
fn refused_batch_with_only_durable_storage_failures_stays_unhealthy() {
    let directory = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(directory.path().to_path_buf());
    let store = manager.storage().unwrap().clone();
    let session = "session-durable-refused-batch";
    store.bind_events(session).unwrap();
    let health = std::sync::Arc::new(EventJournalHealth::default());
    let writer = EventBatchWriter::new(store.clone(), session.into(), health.clone()).unwrap();
    manager.test_execute("DROP TABLE storage_meta");

    let gate = writer.hold_worker_for_test();
    gate.wait();
    writer
        .append(
            serde_json::json!({
                "event_type":"agent_end", "run_id":"run", "idx":0, "session_id":session
            }),
            false,
        )
        .unwrap();
    let valid = serde_json::json!({
        "event_type":"agent_end", "run_id":"run", "epoch":0, "idx":1, "session_id":session
    });
    let (reply, result) = std::sync::mpsc::sync_channel(1);
    writer
        .sender
        .send(EventWriteCommand::Append {
            event: valid.clone(),
            reply: Some(reply),
        })
        .unwrap();
    gate.wait();

    let failure = result
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(failure, AppendFailure::Unavailable(_)),
        "{failure:?}"
    );
    assert!(
        health.error().is_some(),
        "health must precede the durable failure reply"
    );
    assert_eq!(store.event_cursors(session).unwrap().1, 0);

    // Even if another caller clears the health report, the worker must retain
    // its failed state after a storage failure with no fire-and-forget retry.
    health.clear();
    let failure = writer.append(valid, true).unwrap_err();
    assert!(
        failure
            .to_string()
            .contains("event persistence is unavailable"),
        "{failure}"
    );
    assert_eq!(writer.commit_count(), 0);
}

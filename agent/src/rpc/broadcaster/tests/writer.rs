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

    std::thread::sleep(EVENT_BATCH_WINDOW + std::time::Duration::from_millis(100));
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
    for _ in 0..50 {
        if b.persistence_error().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(b.persistence_error().is_some());
    assert!(interrupt.load(std::sync::atomic::Ordering::SeqCst));
}

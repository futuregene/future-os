use super::*;

// ─── SseBroadcaster (P1) ────────────────────────────────────────────────

#[test]
fn stamps_run_id_idx_and_backfills() {
    let b = SseBroadcaster::new();
    b.start_run("run1".to_string(), 1);
    b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "a"}),
    ));
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "b"}),
    ));

    // Backfill from idx 0 → the two events after idx 0 (idx 1, 2), in order.
    let (rid, evs, min_idx, projection) = b.events_since("run1", 0).unwrap();
    assert_eq!(rid, "run1");
    assert_eq!(evs.len(), 2);
    assert_eq!((evs[0].idx, evs[1].idx), (1, 2));
    assert_eq!(evs[0].run_id, "run1");
    // Nothing dropped yet → earliest buffered idx is still 0 (no gap).
    assert_eq!(min_idx, 0);
    assert!(projection.is_none());

    // From -1 → all three (idx 0,1,2).
    let (_, all, _, _) = b.events_since("run1", -1).unwrap();
    assert_eq!(all.iter().map(|e| e.idx).collect::<Vec<_>>(), vec![0, 1, 2]);

    // New run resets idx + clears buffer.
    b.start_run("run2".to_string(), 1);
    b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
    let (rid2, evs2, _, _) = b.events_since("run2", -1).unwrap();
    assert_eq!(rid2, "run2");
    assert_eq!(evs2.len(), 1);
    assert_eq!((evs2[0].idx, evs2[0].run_id.as_str()), (0, "run2"));

    assert!(b.events_since("run1", 100).is_err());
}

#[test]
fn session_scoped_events_have_independent_durable_identity() {
    let directory = tempfile::tempdir().unwrap();
    let b = SseBroadcaster::new();
    b.configure_legacy_journal(
        "session-1",
        &crate::session::Manager::new(directory.path().to_path_buf()),
    )
    .unwrap();
    b.start_run("run-1".to_string(), 1);
    b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
    b.broadcast(SseEvent::new(
        "model_changed",
        serde_json::json!({"model":"x"}),
    ));
    b.broadcast(SseEvent::new(
        "cwd_changed",
        serde_json::json!({"cwd":"/tmp"}),
    ));
    let events = b.read_journal("").unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].run_id, "");
    assert_eq!(events[0].idx, -1);
    assert_eq!(events[0].session_idx, 0);
    assert_eq!(events[1].session_idx, 1);
    assert_eq!(events[1].event_id, "session-1:session:1");
}

#[test]
fn attach_has_no_snapshot_subscribe_window() {
    let b = SseBroadcaster::new();
    b.start_run("run1".to_string(), 1);
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "a"}),
    ));
    let mut attachment = b.attach("run1", -1).unwrap();
    assert_eq!(attachment.events.len(), 1);

    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "b"}),
    ));
    let live = attachment.receiver.try_recv().unwrap();
    assert_eq!(live.idx, 1);
    assert!(!attachment.truncated);
}

#[test]
fn attach_reports_truncated_ring_and_rejects_other_run() {
    let b = SseBroadcaster::new();
    b.start_run("run1".to_string(), 1);
    for idx in 0..=MAX_RUN_EVENTS {
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": idx.to_string()}),
        ));
    }
    let mut attachment = b.attach("run1", -1).unwrap();
    assert!(attachment.truncated);
    assert!(attachment.events.is_empty());
    let snapshot = attachment.projection.take().unwrap();
    assert_eq!(snapshot.run_id, "run1");
    assert_eq!(snapshot.cursor, MAX_RUN_EVENTS as i64);
    assert_eq!(snapshot.events.len(), 1);
    assert_eq!(snapshot.events[0].idx, MAX_RUN_EVENTS as i64);
    let projected_data: serde_json::Value = serde_json::from_str(&snapshot.events[0].data).unwrap();
    assert!(projected_data["text"]
        .as_str()
        .is_some_and(|text| text.starts_with('0') && text.ends_with("2000")));

    // Receiver registration and snapshot capture share the run lock: the
    // first live event starts exactly after the snapshot cursor.
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "live"}),
    ));
    let live = attachment.receiver.try_recv().unwrap();
    assert_eq!(live.idx, snapshot.cursor + 1);

    let (_, replay, _, replay_projection) = b.events_since("run1", -1).unwrap();
    assert!(replay.is_empty());
    assert_eq!(
        replay_projection.as_ref().map(|value| value.cursor),
        Some(live.idx)
    );
    assert!(b.attach("run2", -1).is_err());
}

#[test]
fn truncation_counter_tracks_ring_overflow_resyncs() {
    let b = SseBroadcaster::new();
    b.start_run("run1".to_string(), 1);
    assert_eq!(b.truncation_count(), 0);

    // Within the ring: a full backfill is NOT a truncation.
    for idx in 0..10 {
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": idx.to_string()}),
        ));
    }
    let _ = b.events_since("run1", -1).unwrap();
    assert_eq!(
        b.truncation_count(),
        0,
        "in-ring backfill is not a truncation"
    );

    // Overflow the ring; now a backfill whose cursor predates the ring is a
    // truncation, and each such resync is counted (attach + events_since).
    for idx in 0..=MAX_RUN_EVENTS {
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": idx.to_string()}),
        ));
    }
    let attachment = b.attach("run1", -1).unwrap();
    assert!(attachment.truncated);
    assert_eq!(b.truncation_count(), 1);
    let _ = b.events_since("run1", -1).unwrap();
    assert_eq!(
        b.truncation_count(),
        2,
        "events_since truncation is counted too"
    );
}

#[test]
fn lag_counter_is_observable_and_starts_at_zero() {
    let b = SseBroadcaster::new();
    assert_eq!(b.lag_count(), 0);
    b.record_lag();
    b.record_lag();
    assert_eq!(b.lag_count(), 2);
    // The counter is shared across clones (the gRPC layer holds a clone).
    let clone = b.clone();
    clone.record_lag();
    assert_eq!(b.lag_count(), 3);
}

#[test]
fn concurrent_broadcasts_to_one_broadcaster_yield_contiguous_idx() {
    use std::sync::Arc;
    use std::thread;
    const THREADS: usize = 8;
    const PER_THREAD: usize = 250; // 8 * 250 = 2000 == MAX_RUN_EVENTS (no overflow)
    let b = Arc::new(SseBroadcaster::new());
    b.start_run("run1".to_string(), 1);
    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let b = b.clone();
            thread::spawn(move || {
                for n in 0..PER_THREAD {
                    b.broadcast(SseEvent::new("text_chunk", serde_json::json!({"text": n})));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    // The single stamping lock serializes concurrent broadcasts: every event
    // got a unique, contiguous idx (no gaps, no duplicates) under contention.
    let total = (THREADS * PER_THREAD) as i64;
    assert_eq!(b.last_idx(), total - 1);
    assert_eq!(b.truncation_count(), 0);
    let (run_id, events, min_idx, projection) = b.events_since("run1", total - 1 - 100).unwrap();
    assert_eq!(run_id, "run1");
    assert_eq!(min_idx, 0);
    assert!(projection.is_none());
    let expected_start = total - 100; // first idx > (total - 1 - 100)
    assert_eq!(events.len(), 100);
    for (i, event) in events.iter().enumerate() {
        assert_eq!(event.idx, expected_start + i as i64, "contiguous, no gaps");
        assert_eq!(event.run_id, "run1");
        assert_eq!(event.epoch, 1);
    }
}

use super::*;
use serde_json::json;

fn canonical_store() -> (
    tempfile::TempDir,
    crate::session::Manager,
    crate::session::sqlite_store::SqliteStore,
) {
    let root = tempfile::tempdir().unwrap();
    let manager = crate::session::Manager::new(root.path().to_path_buf());
    let store = manager.storage().unwrap().clone();
    store.reserve_segmented("s").unwrap();
    store.replace("s", vec![json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"cwd":".","model":"mock"}})]).unwrap();
    (root, manager, store)
}

#[test]
fn ephemeral_delivery_keeps_canonical_bodies_out_of_the_database() {
    let (_root, manager, store) = canonical_store();
    let before: (i64, i64, i64) = store.db.read(|db| {
        Ok(db.query_row("SELECT (SELECT COUNT(*) FROM stream_runs), (SELECT COUNT(*) FROM content_objects), (SELECT COUNT(*) FROM run_events)", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?)
    }).unwrap();
    let broadcaster = SseBroadcaster::new();
    broadcaster.configure_journal("s", &manager).unwrap();
    broadcaster.use_memory_journal().unwrap();
    assert!(!broadcaster.uses_segmented_storage());
    let mut receiver = broadcaster.subscribe();
    broadcaster.start_run("ephemeral".into(), 1);
    broadcaster.broadcast(SseEvent::new("agent_start", json!({})));
    broadcaster.broadcast(SseEvent::new("text_chunk", json!({"text":"volatile body"})));
    broadcaster.broadcast(SseEvent::new("agent_end", json!({"state":"completed"})));
    broadcaster.flush_journal().unwrap();
    let events: Vec<_> = std::iter::from_fn(|| receiver.try_recv().ok()).collect();
    assert_eq!(events.len(), 3);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&events[1].data).unwrap()["text"],
        "volatile body"
    );
    store.db.read(move |db| {
        let counts: (i64, i64, i64) = db.query_row("SELECT (SELECT COUNT(*) FROM stream_runs), (SELECT COUNT(*) FROM content_objects), (SELECT COUNT(*) FROM run_events)", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        assert_eq!(counts, before);
        Ok(())
    }).unwrap();
    assert!(broadcaster.persistence_error().is_none());
    assert!(broadcaster.configure_journal("s", &manager).is_err());
    assert!(!broadcaster.uses_segmented_storage());
}

#[test]
fn persistence_can_be_reselected_only_before_memory_events_exist() {
    let (_root, manager, _store) = canonical_store();
    let broadcaster = SseBroadcaster::new();
    broadcaster.configure_journal("s", &manager).unwrap();
    broadcaster.use_memory_journal().unwrap();
    broadcaster.configure_journal("s", &manager).unwrap();
    assert!(broadcaster.uses_segmented_storage());
    broadcaster.use_memory_journal().unwrap();
    broadcaster.broadcast_session(SseEvent::new("model_changed", json!({"model":"volatile"})));
    assert!(broadcaster.configure_journal("s", &manager).is_err());
    assert!(!broadcaster.uses_segmented_storage());
}

#[tokio::test]
async fn canonical_barrier_yields_and_has_capacity_when_the_body_lane_is_full() {
    let (_root, manager, store) = canonical_store();
    let broadcaster = SseBroadcaster::new();
    broadcaster.configure_journal("s", &manager).unwrap();
    broadcaster.start_run("r".into(), 1);
    let gate = broadcaster
        .journal
        .lock()
        .writer
        .as_ref()
        .unwrap()
        .hold_worker_for_test();
    gate.wait();
    for _ in 0..BROADCAST_RING_CAPACITY {
        broadcaster.broadcast_deferred(SseEvent::new("usage", json!({"input_tokens":1})));
    }
    assert!(broadcaster.persistence_error().is_none());
    let mut fence = Box::pin(broadcaster.flush_journal_async());
    // Poll directly: this current-thread runtime cannot progress if the fence
    // blocks on recv(). No sleeps or scheduling guesses establish readiness.
    assert!(futures::poll!(&mut fence).is_pending());
    {
        let journal = broadcaster.journal.lock();
        let writer = journal.writer.as_ref().unwrap();
        assert_eq!(writer.sender.len(), BROADCAST_RING_CAPACITY);
        assert_eq!(writer.control.sender.len(), 1);
    }
    gate.wait();
    fence.await.unwrap();
    let page = store.segmented_run_tail("s", "r", 1).unwrap();
    assert_eq!(page.last().unwrap().idx, BROADCAST_RING_CAPACITY as i64 - 1);
}

#[tokio::test]
async fn dropping_an_async_fence_does_not_drop_an_admitted_body() {
    let (_root, manager, store) = canonical_store();
    let broadcaster = SseBroadcaster::new();
    broadcaster.configure_journal("s", &manager).unwrap();
    broadcaster.start_run("r".into(), 1);
    let gate = broadcaster
        .journal
        .lock()
        .writer
        .as_ref()
        .unwrap()
        .hold_worker_for_test();
    gate.wait();
    broadcaster.broadcast_deferred(SseEvent::new("usage", json!({"input_tokens":7})));
    let mut fence = Box::pin(broadcaster.flush_journal_async());
    assert!(futures::poll!(&mut fence).is_pending());
    drop(fence);
    gate.wait();
    broadcaster.flush_journal_async().await.unwrap();
    let page = store
        .segmented_replay("s", "r", -1, false, Some(10))
        .unwrap();
    assert_eq!(page.events.len(), 1);
    assert!(page.events[0].data.contains("7"));
}

#[tokio::test]
async fn live_terminal_uses_the_record_already_committed_with_the_transcript() {
    let (_root, manager, store) = canonical_store();
    store.append("s",vec![json!({"id":"start","type":"run_started","timestamp":"2026-10-26T00:00:00Z","content":{"run_id":"r","epoch":1}})]).unwrap();
    let broadcaster = SseBroadcaster::new();
    broadcaster.configure_journal("s", &manager).unwrap();
    broadcaster.start_run("r".into(), 1);
    broadcaster.broadcast_deferred(SseEvent::new("usage", json!({"input_tokens":1})));
    broadcaster.flush_journal_async().await.unwrap();
    let commits = broadcaster.journal_commit_count();
    let mut receiver = broadcaster.subscribe();
    store.commit_terminal_entries("s",vec![json!({"id":"end","type":"run_terminal","timestamp":"2026-10-26T00:00:01Z","content":{"run_id":"r","state":"completed","run_tokens":4}})]).unwrap();
    broadcaster.publish_committed_terminal().await.unwrap();
    let live = receiver.try_recv().unwrap();
    assert_eq!(live.event_type, "agent_end");
    assert_eq!(live.idx, 1);
    assert_eq!(broadcaster.last_idx(), 1);
    broadcaster.flush_journal_async().await.unwrap();
    assert_eq!(
        broadcaster.journal_commit_count(),
        commits,
        "no second terminal is enqueued"
    );
    let replay = store
        .segmented_replay("s", "r", 0, false, Some(10))
        .unwrap();
    assert_eq!(replay.events.len(), 1);
    assert_eq!(replay.events[0].event_id, live.event_id);
    assert_eq!(replay.events[0].data, live.data);
}
fn fragment(idx: i64, offset: i64, text: &str) -> serde_json::Value {
    json!({"session_id":"s","run_id":"r","epoch":1,"idx":idx,"session_idx":-1,"run_sequence":4,
        "event_type":"text_chunk","timestamp":"2026-10-26T00:00:00Z","data":json!({"text":text,"content_key":"b","content_offset":offset,"content_bytes":text.len(),"content_first_idx":idx}).to_string()})
}

#[test]
fn canonical_start_reads_a_short_tail_and_attachment_walks_the_full_prefix_in_pages() {
    let (_root, manager, store) = canonical_store();
    let mut writer = crate::session::content::journal::Writer::default();
    let text = "x".repeat(crate::session::content::WRITE_BYTES);
    for idx in 0..140 {
        writer
            .persist(
                &store,
                "s",
                &[fragment(idx, idx * text.len() as i64, &text)],
            )
            .unwrap();
    }
    let broadcaster = SseBroadcaster::new();
    broadcaster.configure_journal("s", &manager).unwrap();
    broadcaster.start_run_with_sequence("r".into(), 1, Some(4));
    assert!(broadcaster.persistence_error().is_none());
    assert_eq!(broadcaster.last_idx(), 139);
    assert_eq!(broadcaster.run.lock().events.len(), 8);
    assert!(!broadcaster.run.lock().projection_complete);
    let mut attachment = broadcaster.attach("r", -1).unwrap();
    assert!(attachment.events.is_empty());
    assert!(attachment.projection.is_none());
    let boundary = attachment.backfill.take().expect("paged backfill");
    assert_eq!(boundary.until_idx, 139);
    // Registration preceded every disk read. A later event remains in the
    // live receiver while the immutable old prefix is consumed page by page.
    broadcaster.broadcast(SseEvent::new("usage", json!({"input_tokens":1})));
    let mut cursor = -1;
    let mut events = 0;
    while cursor < boundary.until_idx {
        let page = broadcaster.attachment_page(&boundary, cursor).unwrap();
        assert!(page.len() <= 64);
        assert_eq!(page[0].idx, cursor + 1);
        cursor = page.last().unwrap().idx;
        events += page.len();
    }
    assert_eq!(events, 140);
    assert_eq!(cursor, 139);
    assert_eq!(attachment.receiver.try_recv().unwrap().idx, 140);
}

#[test]
fn an_attachment_boundary_inside_a_coalesced_commit_returns_the_complete_range() {
    let (_root, manager, store) = canonical_store();
    let mut writer = crate::session::content::journal::Writer::default();
    writer
        .persist(
            &store,
            "s",
            &[
                fragment(0, 0, "你"),
                fragment(1, 3, "好"),
                fragment(2, 6, "啊"),
            ],
        )
        .unwrap();
    let broadcaster = SseBroadcaster::new();
    broadcaster.configure_journal("s", &manager).unwrap();
    {
        // The subscription observed two produced fragments, while the writer
        // committed a batch also containing the next fragment before readback.
        let mut run = broadcaster.run.lock();
        run.run_id = "r".into();
        run.epoch = 1;
        run.idx = 2;
    }
    let attachment = broadcaster.attach("r", -1).unwrap();
    let boundary = attachment.backfill.unwrap();
    assert_eq!(boundary.until_idx, 1);
    let page = broadcaster.attachment_page(&boundary, -1).unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(
        page[0].idx, 2,
        "transport must advance the live cutoff to this range end"
    );
    let data: serde_json::Value = serde_json::from_str(&page[0].data).unwrap();
    assert_eq!(data["content_first_idx"], 0);
    assert_eq!(data["text"], "你好啊");
}

#[test]
fn canonical_resume_refuses_to_reuse_a_run_identity_in_a_different_epoch() {
    let (_root, manager, store) = canonical_store();
    let mut writer = crate::session::content::journal::Writer::default();
    writer
        .persist(&store, "s", &[fragment(0, 0, "preserved")])
        .unwrap();
    let broadcaster = SseBroadcaster::new();
    broadcaster.configure_journal("s", &manager).unwrap();
    broadcaster.start_run("r".into(), 2);
    assert!(broadcaster
        .persistence_error()
        .unwrap()
        .contains("epoch differs"));
    let mut receiver = broadcaster.subscribe();
    broadcaster.broadcast(SseEvent::new(
        "text_chunk",
        json!({"text":"must not replace the old run"}),
    ));
    assert!(receiver.try_recv().is_err());
    let old = store
        .segmented_replay("s", "r", -1, false, Some(10))
        .unwrap();
    assert_eq!(old.events.len(), 1);
    assert!(old.events[0].data.contains("preserved"));
}

/// A live delta larger than the delivery budget is streamed as several bounded
/// pieces that stay one canonical body: the pieces tile the text exactly, keep
/// contiguous byte offsets, and only the first may be a replacement snapshot.
#[test]
fn a_live_delta_over_the_text_budget_is_delivered_as_bounded_pieces() {
    let (_root, manager, _store) = canonical_store();
    let broadcaster = SseBroadcaster::new();
    broadcaster.configure_journal("s", &manager).unwrap();
    assert!(broadcaster.uses_segmented_storage());
    let mut receiver = broadcaster.subscribe();
    broadcaster.start_run("run".into(), 1);
    broadcaster.broadcast(SseEvent::new("agent_start", json!({})));
    broadcaster.broadcast(SseEvent::new("message_start", json!({"entry_id": "m"})));
    let text = "思考🙂".repeat(2_000);
    assert!(text.len() > crate::session::content::transport::LIVE_TEXT_BYTES);
    broadcaster.broadcast_deferred(SseEvent::new(
        "text_chunk",
        json!({"text": text, "snapshot": true}),
    ));
    broadcaster.broadcast(SseEvent::new("text_end", json!({})));
    broadcaster.broadcast(SseEvent::new("agent_end", json!({"state": "completed"})));
    broadcaster.flush_journal().unwrap();
    assert!(
        broadcaster.persistence_error().is_none(),
        "{:?}",
        broadcaster.persistence_error()
    );

    let delivered: Vec<serde_json::Value> = std::iter::from_fn(|| receiver.try_recv().ok())
        .filter(|event| event.event_type == "text_chunk")
        .map(|event| serde_json::from_str(&event.data).unwrap())
        .collect();
    assert!(delivered.len() > 1, "the delta was not split");
    let mut assembled = String::new();
    let mut expected_offset = 0;
    for (index, piece) in delivered.iter().enumerate() {
        let piece_text = piece["text"].as_str().unwrap();
        assert!(!piece_text.is_empty());
        assert!(
            piece_text.len() <= crate::session::content::transport::LIVE_TEXT_BYTES,
            "piece {index} exceeds the live text budget"
        );
        // Each piece continues the same canonical body at the byte it starts.
        assert_eq!(piece["content_offset"], json!(expected_offset));
        assert_eq!(piece["content_bytes"], json!(piece_text.len()));
        assert_eq!(piece["snapshot"], json!(index == 0));
        expected_offset += piece_text.len() as i64;
        assembled.push_str(piece_text);
    }
    assert_eq!(assembled, text, "the pieces must tile the original text");
}

/// A canonical event whose payload is not JSON never reaches the store *or* a
/// subscriber: the payload is parsed before the stamping lock is taken, so the
/// refusal is immediate and recorded on the journal, and no frame the store
/// could never hold is delivered to a live client.
#[test]
fn an_unparseable_canonical_event_fails_the_journal_before_delivery() {
    let (_root, manager, _store) = canonical_store();
    let broadcaster = SseBroadcaster::new();
    broadcaster.configure_journal("s", &manager).unwrap();
    assert!(
        broadcaster.uses_segmented_storage(),
        "this refusal only exists on the canonical path"
    );
    let mut receiver = broadcaster.subscribe();
    broadcaster.start_run("run-bad-json".into(), 1);
    broadcaster.broadcast(SseEvent::new("agent_start", json!({})));

    let mut truncated = SseEvent::new("text_chunk", json!({"text": "x"}));
    truncated.data = "{\"text\": \"unterminated".to_string();
    broadcaster.broadcast(truncated);

    let error = broadcaster
        .persistence_error()
        .expect("the refusal is recorded on the journal");
    assert!(error.contains("invalid event data"), "{error}");

    let delivered: Vec<String> = std::iter::from_fn(|| receiver.try_recv().ok())
        .map(|event| event.event_type)
        .collect();
    assert_eq!(
        delivered,
        vec!["agent_start".to_string()],
        "the frame the store could not hold must not be delivered"
    );
}

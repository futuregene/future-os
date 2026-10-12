use super::*;

#[test]
fn completed_journal_is_replayable_after_reopen_and_old_cursor_resync() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agent.db");
    let store = fixture(&path, "completed", 1000);
    assert_eq!(store.compact_run("s", "run").unwrap(), 1002);
    assert!(store.has_events("s", "run").unwrap());
    let page = store.replay_page("s", "run", 123, false, Some(1)).unwrap();
    assert!(page.known && page.events.is_empty());
    let snapshot = page.snapshot.unwrap();
    assert_eq!(snapshot.cursor, 1001);
    assert_eq!(text(&snapshot), "中文𠮷".repeat(1000));
    assert!(snapshot.events.len() < 10);
    drop(store);
    let manager = crate::session::Manager::new(dir.path().to_path_buf());
    let broadcaster = SseBroadcaster::new();
    broadcaster.configure_journal("s", &manager).unwrap();
    let historical = broadcaster.events_since("run", 123).unwrap();
    assert!(historical.1.is_empty());
    assert_eq!(historical.3.unwrap().cursor, 1001);
    assert!(broadcaster.events_since("run", 1001).unwrap().3.is_none());
    broadcaster.start_run_with_sequence("run".into(), 0, Some(1));
    let reopened = broadcaster.run_snapshot("run").unwrap();
    assert_eq!(reopened.cursor, 1001);
    assert_eq!(
        broadcaster
            .events_since("run", 123)
            .unwrap()
            .3
            .unwrap()
            .cursor,
        1001
    );
    assert_eq!(
        broadcaster
            .attach("run", 123)
            .unwrap()
            .projection
            .unwrap()
            .cursor,
        1001
    );
    assert!(broadcaster
        .attach("run", 1001)
        .unwrap()
        .projection
        .is_none());
    assert!(broadcaster.events_since("unknown", -1).is_err());
}

#[test]
fn pricing_and_session_events_survive_and_explicit_prune_deletes_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("agent.db"), "completed", 1000);
    store.db.call(|db| {
        db.execute("UPDATE run_events SET payload=json_set(payload,'$.event_type','usage','$.data',?1) WHERE idx=500",[json!({"output_tokens":7}).to_string()])?;
        Ok(())
    }).unwrap();
    store.append_event("s",json!({"run_id":"","session_idx":0,"idx":-1,"event_type":"model_changed","data":json!({"model":"test"}).to_string()})).unwrap();
    let pricing = store.pricing_event_payloads("s").unwrap();
    store.compact_run("s", "run").unwrap();
    assert_eq!(store.pricing_event_payloads("s").unwrap(), pricing);
    assert_eq!(store.events("s", "").unwrap().len(), 1);
    store.prune_events("s", "run").unwrap();
    assert!(!store.has_events("s", "run").unwrap());
    assert_eq!(store.pricing_event_payloads("s").unwrap().len(), 1);
}

#[test]
fn folder_preserves_reasoning_blocks_and_replacement_tool_fragments() {
    let mut folder = DeltaFolder::bounded();
    for (idx, (kind, data)) in [
        ("thinking_delta", json!({"text":"one","block_id":"a"})),
        ("thinking_delta", json!({"text":"two","block_id":"b"})),
        (
            "tool_delta",
            json!({"text":"old","tool_id":"t","snapshot":true}),
        ),
        (
            "tool_delta",
            json!({"text":"new","tool_id":"t","snapshot":true}),
        ),
        ("tool_end", json!({"tool_id":"t","text":"result"})),
    ]
    .into_iter()
    .enumerate()
    {
        folder.push(SseEvent {
            run_id: "r".into(),
            idx: idx as i64,
            ..SseEvent::new(kind, data)
        });
    }
    let events = folder.finish();
    assert_eq!(events.len(), 5);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&events[3].data).unwrap()["text"],
        "new"
    );
}

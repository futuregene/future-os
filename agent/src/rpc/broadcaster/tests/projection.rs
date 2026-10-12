use super::*;

#[test]
fn projection_preserves_semantic_order_while_coalescing_deltas() {
    let b = SseBroadcaster::new();
    b.start_run("run1".to_string(), 1);
    b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
    b.broadcast(SseEvent::new(
        "thinking_delta",
        serde_json::json!({"text": "a"}),
    ));
    b.broadcast(SseEvent::new(
        "thinking_delta",
        serde_json::json!({"text": "b"}),
    ));
    b.broadcast(SseEvent::new(
        "tool_start",
        serde_json::json!({"tool_id": "t1", "tool_name": "read"}),
    ));
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "hello"}),
    ));
    b.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": " world"}),
    ));
    for idx in 0..MAX_RUN_EVENTS {
        b.broadcast(SseEvent::new(
            "usage",
            serde_json::json!({"usage": {"output_tokens": idx}}),
        ));
    }

    let snapshot = b.attach("run1", -1).unwrap().projection.unwrap();
    assert_eq!(
        snapshot
            .events
            .iter()
            .take(4)
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["agent_start", "thinking_delta", "tool_start", "text_chunk"]
    );
    let thinking: serde_json::Value = serde_json::from_str(&snapshot.events[1].data).unwrap();
    let text: serde_json::Value = serde_json::from_str(&snapshot.events[3].data).unwrap();
    assert_eq!(thinking["text"], "ab");
    assert_eq!(text["text"], "hello world");
    for event in [&snapshot.events[1], &snapshot.events[3]] {
        assert!(
            event.event_id.ends_with(&format!(":{}", event.idx)),
            "{event:?}"
        );
    }
}

#[test]
fn projection_preserves_reconnect_lifecycle_beyond_replay_ring() {
    let broadcaster = SseBroadcaster::new();
    broadcaster.start_run("run-retry".to_string(), 1);
    broadcaster.broadcast(SseEvent::new(
        "stream_retry",
        serde_json::json!({
            "type": "stream_retry", "attempt": 1, "maxRetries": 5, "delayMs": 2000
        }),
    ));
    broadcaster.broadcast(SseEvent::new(
        "stream_resumed",
        serde_json::json!({"type": "stream_resumed"}),
    ));
    for _ in 0..2100 {
        broadcaster.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "a"}),
        ));
    }
    broadcaster.broadcast(SseEvent::new(
        "stream_retry",
        serde_json::json!({
            "type": "stream_retry", "attempt": 2, "maxRetries": 5, "delayMs": 4000
        }),
    ));
    let projection = broadcaster
        .attach("run-retry", 0)
        .unwrap()
        .projection
        .unwrap();
    let lifecycle: Vec<_> = projection
        .events
        .iter()
        .filter(|e| e.event_type.starts_with("stream_"))
        .collect();
    assert_eq!(lifecycle.len(), 3);
    assert_eq!(lifecycle[0].event_type, "stream_retry");
    assert_eq!(lifecycle[1].event_type, "stream_resumed");
    assert_eq!(lifecycle[2].event_type, "stream_retry");
    let payload: serde_json::Value = serde_json::from_str(&lifecycle[2].data).unwrap();
    assert_eq!(payload["attempt"], 2);
}

#[test]
fn projection_skips_raw_text_deltas() {
    let mut projection = Vec::new();
    apply_to_projection(
        &mut projection,
        &SseEvent::new("text_delta", serde_json::json!({"text": "raw"})),
    );
    assert!(projection.is_empty());
    apply_to_projection(
        &mut projection,
        &SseEvent::new("text_chunk", serde_json::json!({"text": "kept"})),
    );
    assert_eq!(projection.len(), 1);
    // A second text_chunk replaces the projection's previous text entry.
    apply_to_projection(
        &mut projection,
        &SseEvent::new("text_chunk", serde_json::json!({"text": "kept v2"})),
    );
    assert_eq!(projection.len(), 1);
}

#[test]
fn projection_folds_tool_deltas_of_the_same_tool_stream() {
    let mut projection = Vec::new();
    apply_to_projection(
        &mut projection,
        &SseEvent::new(
            "toolcall_delta",
            serde_json::json!({"tool_id": "t1", "text": "a"}),
        ),
    );
    apply_to_projection(
        &mut projection,
        &SseEvent::new(
            "toolcall_delta",
            serde_json::json!({"tool_id": "t1", "text": "b"}),
        ),
    );
    assert_eq!(projection.len(), 1);
    let data: serde_json::Value = serde_json::from_str(&projection[0].data).unwrap();
    assert_eq!(data["text"], "ab");
}

#[test]
fn projection_keeps_tool_deltas_of_different_tool_streams() {
    let mut projection = Vec::new();
    apply_to_projection(
        &mut projection,
        &SseEvent::new(
            "toolcall_delta",
            serde_json::json!({"tool_id": "t1", "text": "a"}),
        ),
    );
    apply_to_projection(
        &mut projection,
        &SseEvent::new(
            "toolcall_delta",
            serde_json::json!({"tool_id": "t2", "text": "b"}),
        ),
    );
    assert_eq!(projection.len(), 2);
}

#[test]
fn projection_pushes_when_event_data_is_not_foldable() {
    // Previous event's data is not parseable JSON → no fold, plain push.
    let mut projection = Vec::new();
    let mut broken = SseEvent::new("text_chunk", serde_json::json!({"text": "a"}));
    broken.data = "{not json".to_string();
    apply_to_projection(&mut projection, &broken);
    apply_to_projection(
        &mut projection,
        &SseEvent::new("text_chunk", serde_json::json!({"text": "b"})),
    );
    assert_eq!(projection.len(), 2);
    // Valid JSON but no "text" field on the previous event → no fold.
    let mut projection = Vec::new();
    apply_to_projection(
        &mut projection,
        &SseEvent::new("text_chunk", serde_json::json!({"other": 1})),
    );
    apply_to_projection(
        &mut projection,
        &SseEvent::new("text_chunk", serde_json::json!({"text": "b"})),
    );
    assert_eq!(projection.len(), 2);
}

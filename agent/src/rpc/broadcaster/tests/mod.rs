use super::*;
mod delivery;
mod journal_lifecycle;
mod journal_replay;
mod projection;
mod writer;

mod canonical_attach;

mod storage_acceptance;

#[test]
fn semantic_progress_counts_only_nonempty_model_content_for_its_run() {
    let broadcaster = SseBroadcaster::new();
    broadcaster.start_run("first".into(), 1);
    for (kind, data) in [
        ("thinking_start", serde_json::json!({})),
        ("thinking_delta", serde_json::json!({"text":""})),
        ("heartbeat", serde_json::json!({"text":"alive"})),
        ("usage", serde_json::json!({"output_tokens":100})),
    ] {
        broadcaster.broadcast(SseEvent::new(kind, data));
    }
    assert_eq!(broadcaster.model_output_progress()["bytes"], 0);
    broadcaster.broadcast(SseEvent::new(
        "thinking_delta",
        serde_json::json!({"text":"思考🙂"}),
    ));
    broadcaster.broadcast(SseEvent::new(
        "text_chunk",
        serde_json::json!({"text":"answer"}),
    ));
    assert_eq!(
        broadcaster.model_output_progress(),
        serde_json::json!({"runId":"first","bytes":"思考🙂answer".len()})
    );
    broadcaster.start_run("second".into(), 2);
    broadcaster.broadcast(SseEvent::new("text_chunk", serde_json::json!({"text":"x"})));
    assert_eq!(
        broadcaster.model_output_progress(),
        serde_json::json!({"runId":"second","bytes":1})
    );
}

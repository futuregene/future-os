//! Host event outlet. Agent observers know only this sink, not remote transport.
use std::sync::OnceLock;
pub(crate) struct AgentEvent<'a> {
    pub session_id: &'a str,
    pub event_type: &'a str,
    pub data: &'a str,
    pub run_id: &'a str,
    pub idx: i64,
    pub epoch: i64,
    pub event_id: &'a str,
    pub timestamp: &'a str,
    pub session_idx: i64,
    pub run_sequence: i64,
}
static SINK: OnceLock<fn(AgentEvent<'_>)> = OnceLock::new();
pub(crate) fn attach(sink: fn(AgentEvent<'_>)) {
    let _ = SINK.set(sink);
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn publish_event(
    session_id: &str,
    event_type: &str,
    data: &str,
    run_id: &str,
    idx: i64,
    epoch: i64,
    event_id: &str,
    timestamp: &str,
    session_idx: i64,
    run_sequence: i64,
) {
    if let Some(sink) = SINK.get() {
        sink(AgentEvent {
            session_id,
            event_type,
            data,
            run_id,
            idx,
            epoch,
            event_id,
            timestamp,
            session_idx,
            run_sequence,
        });
    }
}
pub(crate) fn publish_snapshot(
    session_id: &str,
    run_id: &str,
    snapshot_cursor: i64,
    events: &[crate::agent_proto::ProjectedRunEvent],
    run_sequence: i64,
) {
    let events: Vec<_> = events.iter().map(|event| serde_json::json!({
        "type": event.r#type, "data": future_rpc::decode::projected_event_data(event), "idx": event.idx
    })).collect();
    let data = serde_json::json!({"snapshotEvents": events, "snapshotCursor": snapshot_cursor})
        .to_string();
    publish_event(
        session_id,
        "run_snapshot",
        &data,
        run_id,
        snapshot_cursor,
        0,
        &format!("{session_id}:{run_id}:snapshot:{snapshot_cursor}"),
        "",
        -1,
        run_sequence,
    );
}

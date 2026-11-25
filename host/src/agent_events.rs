//! Host event outlet. Agent observers know only this sink, not remote transport.
use std::sync::OnceLock;
pub struct AgentEvent<'a> {
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
pub fn attach(sink: fn(AgentEvent<'_>)) {
    let _ = SINK.set(sink);
}

/// Everything published in this process, drained by a test.
///
/// Independent of [`SINK`] on purpose: a test asserting that an event did *not*
/// reach remote clients must not depend on which sink happened to be installed
/// first (the e2e fixtures install the production one), and the filter being
/// asserted is decided before the sink is chosen.
#[cfg(test)]
static RECORDED: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// The `(session id, event type)` pairs published for one session since the
/// last drain.
#[cfg(test)]
pub(crate) fn recorded(session_id: &str) -> Vec<String> {
    let mut recorded = RECORDED.lock().unwrap_or_else(|error| error.into_inner());
    // Take only this session's rows: tests run in one process, and removing
    // everything would empty another session's assertion mid-flight.
    let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut *recorded)
        .into_iter()
        .partition(|(session, _)| session == session_id);
    *recorded = others;
    mine.into_iter().map(|(_, event_type)| event_type).collect()
}
#[allow(clippy::too_many_arguments)]
pub fn publish_event(
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
    #[cfg(test)]
    RECORDED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push((session_id.to_string(), event_type.to_string()));
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
/// Invalidate desktop and phone views after a committed settings/skills change.
/// No preferences or credentials are carried by this notification: both clients
/// reread the desktop's authoritative state, including after reconnect/resume.
pub fn publish_invalidation(event_type: &str) {
    crate::events::notify(event_type, serde_json::Value::Null);
    publish_event(
        "_global",
        event_type,
        "{}",
        "",
        -1,
        0,
        &format!("{event_type}-{:016x}", rand::random::<u64>()),
        "",
        -1,
        -1,
    );
}

pub fn publish_snapshot(
    session_id: &str,
    run_id: &str,
    snapshot_cursor: i64,
    events: &[future_rpc::proto::ProjectedRunEvent],
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

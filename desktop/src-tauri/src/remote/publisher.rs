use super::*;
use std::sync::atomic::AtomicBool;

/// Capability a client declares on `secure_ready` to receive the coalesced
/// event lane. Named so an older desktop ignores it and an older client never
/// asks for it.
#[cfg(test)]
pub(super) use future_remote_protocol::EVENT_COALESCING_FEATURE;

/// Capability a client declares on `secure_ready` to be told a catalog row's
/// new recency in a few dozen bytes instead of being sent the whole session
/// list again.
///
/// A streaming run rewrites its thread's activity timestamp about once a
/// second, and that timestamp is the list's ordering key — so before this
/// capability every second of every run re-sent every session the host owns.
/// Measured on the six heaviest completed runs, that lane cost 101.7–178.8 MB
/// per run against a live event lane of 0.26–1.11 MB; the session list itself
/// is ~95 KB for 232 conversations, and one row of it moved.
///
/// A client that does not declare it keeps receiving whole snapshots on exactly
/// the old condition (`scripts/measure/measure-catalog-activity.py` measures
/// both — 0.36–0.55 MB per run for one that does, −99.6%), so an older client is
/// never silently given a list it cannot keep current.
#[cfg(test)]
pub(super) use future_remote_protocol::CATALOG_ACTIVITY_FEATURE;

pub(super) use future_remote_server::events::EventPublish;
#[cfg(test)]
pub(super) use future_remote_server::events::{build_event_body, MAX_EVENT_BYTES};

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
    SUPERVISOR.publish_event(
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
    );
}

/// Mirror a run's projection snapshot as a wholesale-replacement signal. The
/// snapshot's folded events cannot be applied incrementally — a coalesced
/// chunk's payload spans idx values the client already applied — so this goes
/// out as a single `run_snapshot` event and the client heals by resyncing
/// rather than folding. The folded events ride along in `data` for consumers
/// that can apply a snapshot directly.
#[cfg(test)]
pub fn publish_snapshot(
    session_id: &str,
    run_id: &str,
    snapshot_cursor: i64,
    snapshot_events: &[crate::agent_proto::ProjectedRunEvent],
    run_sequence: i64,
) {
    let events: Vec<serde_json::Value> = snapshot_events
        .iter()
        .map(|event| {
            json!({
                "type": event.r#type,
                "data": future_rpc::decode::projected_event_data(event),
                "idx": event.idx,
            })
        })
        .collect();
    let data = json!({ "snapshotEvents": events, "snapshotCursor": snapshot_cursor }).to_string();
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

/// Return `data` unchanged when it fits the payload budget, else a well-formed
/// JSON placeholder that keeps the event renderable and tells the client where
/// the full content lives (the persisted run history via `get_messages`). The
/// placeholder retains tool identity, outcome and a bounded output tail so a
/// completed call cannot get stuck running when its result exceeds the cap.
#[cfg(test)]
pub(super) use future_remote_server::events::cap_event_data;

/// Serially publishes queued events on one connection, preserving agent
/// emission order. Exits when every sender is dropped (stop, or a credential
/// refresh that swapped in a new queue): on refresh the old drain is NOT
/// aborted — it keeps its client clone alive until its backlog is flushed,
/// avoiding a mid-stream gap at the swap point.
///
/// Drop/backlog reporting happens in [`publish_event`], event-driven, since
/// this loop can be blocked on `publish().await` (queue-backed NATS) and
/// wouldn't reach its own timer while the backlog persists.
#[cfg(test)]
pub(super) fn spawn_event_publisher(
    client: async_nats::Client,
    rx: tokio::sync::mpsc::Receiver<EventPublish>,
) -> tokio::task::JoinHandle<()> {
    spawn_secure_event_publisher(
        client,
        rx,
        secure::Transport::legacy_fixture(),
        Arc::new(AtomicBool::new(false)),
    )
}

/// Drain the ordered event queue, optionally merging a run's text fragments.
///
/// The queue is the single ordered lane to this connection, so coalescing here
/// (rather than per publisher) is what makes "any non-fragment flushes first" a
/// property of the code instead of a hope. A client that never declares the
/// capability takes the pass-through path unchanged.
pub(super) fn spawn_secure_event_publisher(
    client: async_nats::Client,
    rx: tokio::sync::mpsc::Receiver<EventPublish>,
    security: secure::Transport,
    coalesce: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(future_remote_server::events::run_event_publisher(
        client, rx, security, coalesce,
    ))
}

/// Heartbeat cadence for the liveness packet (`p.{pair}.presence`).
///
/// This is deliberately slower than the catalog tick. Liveness needs only to
/// beat inside the client's receipt window (15s) to keep the link readable, so
/// 3s keeps a 5x margin while cutting the idle packet rate to a third. Tests
/// shrink it to milliseconds so publish patterns can be observed without a
/// wall-clock wait.
pub(super) fn presence_tick() -> std::time::Duration {
    #[cfg(test)]
    const TICK: std::time::Duration = std::time::Duration::from_millis(10);
    #[cfg(not(test))]
    const TICK: std::time::Duration = std::time::Duration::from_secs(3);
    TICK
}

/// Catalog change-detection cadence. Kept at one second so a session created by
/// another surface (TUI, `future` CLI, loop) reaches the phone as fast as it
/// did before — only the presence packet got slower. Each tick recomputes the
/// snapshot signature from the store, so this cadence is the detection latency
/// for any change; the snapshots themselves are published only on a change.
pub(super) fn catalog_tick() -> std::time::Duration {
    #[cfg(test)]
    const TICK: std::time::Duration = std::time::Duration::from_millis(10);
    #[cfg(not(test))]
    const TICK: std::time::Duration = std::time::Duration::from_secs(1);
    TICK
}

/// Credential-refresh / health-check cadence. Tests shrink it to milliseconds
/// so the generation swap can run without a 15s wall-clock wait per tick.
pub(super) fn refresh_tick() -> std::time::Duration {
    #[cfg(test)]
    const TICK: std::time::Duration = std::time::Duration::from_millis(10);
    #[cfg(not(test))]
    const TICK: std::time::Duration = std::time::Duration::from_secs(15);
    TICK
}

#[cfg(test)]
pub(super) fn spawn_presence_heartbeat(
    client: async_nats::Client,
    pair_id: String,
    bridge_instance_id: String,
) -> tokio::task::JoinHandle<()> {
    spawn_secure_presence_heartbeat(
        client,
        pair_id,
        bridge_instance_id,
        secure::Transport::legacy_fixture(),
    )
}

pub(super) fn spawn_secure_presence_heartbeat(
    client: async_nats::Client,
    pair_id: String,
    bridge_instance_id: String,
    security: secure::Transport,
) -> tokio::task::JoinHandle<()> {
    let supervisor = SUPERVISOR.clone();
    let timing =
        future_remote_server::supervisor::PublisherTiming::new(presence_tick(), catalog_tick())
            .expect("desktop publisher intervals are nonzero");
    tokio::spawn(supervisor.run_presence_heartbeat(
        client,
        pair_id,
        bridge_instance_id,
        security,
        timing,
    ))
}

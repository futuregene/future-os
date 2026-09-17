use super::*;

/// Cap on a single event's serialized size. A huge event (e.g. a large tool
/// result) would otherwise exceed the NATS 1MB user-JWT payload limit and be
/// rejected by the broker — silently leaving a permanent gap in the client's
/// event stream. Over-limit events keep their type/runId/idx (so ordering and
/// dedup still work) but ship a truncated `data` marker instead.
pub(super) const MAX_EVENT_BYTES: usize = 900 * 1024;

/// One agent event queued for publishing, in agent-emission order.
pub(super) struct EventPublish {
    pub(super) subject: String,
    pub(super) payload: Vec<u8>,
    pub(super) status_subject: Option<String>,
}

pub(super) fn is_catalog_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "agent_start"
            | "agent_end"
            | "approval_request"
            | "approval_decision"
            | "session_name_changed"
            | "provider_config_changed"
            | "model_visibility_changed"
            | "app_settings_changed"
            | "skills_changed"
            | "run_snapshot"
            | "error"
    )
}

/// If remote is running, queue an agent event for mirroring to
/// `p.{pairId}.evt.{session}`. Returns immediately when not connected — never
/// blocks GUI event consumption.
///
/// Events go through a bounded FIFO queue drained by a single task per
/// connection, so publish order matches agent emission order (the previous
/// per-event `tokio::spawn` could interleave two publishes and deliver idx
/// N+1 before idx N under load — the client dedups by (runId,idx) but renders
/// in arrival order, so reordering garbled streamed text).
///
/// The drain publishes via core NATS (fire-and-forget). Completeness is
/// guaranteed at the application layer: the client recovers gaps via
/// `get_events_since` backfill on reattach or jitter-gap detection.
///
/// On queue overflow the newest event is dropped and logged; the client heals
/// the gap via `get_events_since` backfill on its next reattach.
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
    let Some((tx, pair_id, connected, drop_counters)) = ({
        let guard = SUPERVISOR.state.lock().unwrap();
        guard.as_ref().map(|s| {
            (
                s.event_tx.clone(),
                s.pair_id.clone(),
                s.client.connection_state() == async_nats::connection::State::Connected,
                s.drop_counters.clone(),
            )
        })
    }) else {
        return;
    };
    // NATS offline (server down / network out): publish would block until
    // reconnect and queue events that can't be sent. Skip them here — the
    // client recovers any gap via `get_events_since` backfill on its next
    // reattach, same as a dropped event.
    if !connected {
        if let Some(line) = drop_counters.record_drop(
            "NATS not connected",
            event_type,
            session_id,
            unix_timestamp_ms(),
        ) {
            eprintln!("{line}");
        }
        return;
    }
    // Guard the NATS payload cap: an oversized event is published with a
    // truncated `data` marker (type/runId/idx preserved) rather than dropped,
    // so the client's dedup cursor doesn't get a permanent hole.
    let body = build_event_body(
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
    // A serde_json::Value always serializes, so this cannot fail.
    let payload = serde_json::to_vec(&body).expect("an event Value always serializes");
    let event = EventPublish {
        subject: format!("p.{pair_id}.evt.{session_id}"),
        payload,
        // Existing state wildcard permissions cover this low-rate lane. Old
        // clients ignore the new suffix and keep their full legacy event feed.
        status_subject: is_catalog_event(event_type).then(|| format!("p.{pair_id}.state.events")),
    };
    if tx.try_send(event).is_err() {
        if let Some(line) = drop_counters.record_drop(
            "event publish queue full",
            event_type,
            session_id,
            unix_timestamp_ms(),
        ) {
            eprintln!("{line}");
        }
        return;
    }
    // A drop episode (queue full or NATS offline) has recovered: this event
    // enqueued normally. Report once, with the episode's total. Best-effort —
    // a burst racing across threads may miscount a line, never the flood.
    if let Some(line) = drop_counters.report_recovery() {
        eprintln!("{line}");
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_event_body(
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
) -> serde_json::Value {
    let data = cap_event_data(data);
    json!({
        "schemaVersion": 2,
        "sessionId": session_id,
        "type": event_type,
        "data": data,
        "runId": run_id,
        "idx": idx,
        "epoch": epoch,
        "eventId": event_id,
        "timestamp": timestamp,
        "sessionIdx": session_idx,
        "runSequence": run_sequence,
    })
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
/// placeholder has no `type`-specific fields, so it's a harmless no-op in the
/// client's renderer while still advancing the (runId,idx) dedup cursor.
pub(super) fn cap_event_data(data: &str) -> std::borrow::Cow<'_, str> {
    if data.len() <= MAX_EVENT_BYTES {
        return std::borrow::Cow::Borrowed(data);
    }
    std::borrow::Cow::Owned(format!(
        r#"{{"_truncated":true,"bytes":{},"note":"event exceeded the relay payload limit and was truncated; full content is available via get_messages"}}"#,
        data.len()
    ))
}

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
    spawn_secure_event_publisher(client, rx, secure::Transport::legacy_fixture())
}

pub(super) fn spawn_secure_event_publisher(
    client: async_nats::Client,
    mut rx: tokio::sync::mpsc::Receiver<EventPublish>,
    security: secure::Transport,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let sent = async {
                if let Some(subject) = event.status_subject {
                    secure::publish(&client, &security, subject, event.payload.clone()).await?;
                }
                secure::publish(&client, &security, event.subject, event.payload).await
            }
            .await;
            if let Err(error) = sent {
                if let Some(line) = EVENT_PUBLISH_EPISODE.record("event_publish", error) {
                    eprintln!("{line}");
                }
                break;
            } else if let Some(line) = EVENT_PUBLISH_EPISODE.recovered() {
                eprintln!("{line}");
            }
        }
    })
}

/// Heartbeat cadence. Tests shrink it to milliseconds so the publish pattern
/// (baseline → signature change → self-heal) can be observed without a
/// multi-second wall-clock wait.
pub(super) fn presence_tick() -> std::time::Duration {
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
    tokio::spawn(async move {
        let health = tokio::spawn(host().monitor_agent());
        let mut catalog =
            spawn_catalog_publisher(client.clone(), pair_id.clone(), security.clone());
        let mut tasks = lifecycle::CandidateTasks::default();
        tasks.track(&catalog);
        tasks.track(&health);
        let mut interval = tokio::time::interval(presence_tick());
        loop {
            tokio::select! {
                _ = &mut catalog => return,
                _ = interval.tick() => {}
            }
            let bytes = serde_json::to_vec(&light_presence_payload(&pair_id, &bridge_instance_id))
                .expect("a presence Value always serializes");
            if let Err(error) =
                secure::publish(&client, &security, format!("p.{pair_id}.presence"), bytes).await
            {
                if let Some(line) = HEARTBEAT_PUBLISH_EPISODE.record("heartbeat_publish", error) {
                    eprintln!("{line}");
                }
                return;
            }
            if let Some(line) = HEARTBEAT_PUBLISH_EPISODE.recovered() {
                eprintln!("{line}");
            }
        }
    })
}

pub(super) fn spawn_catalog_publisher(
    client: async_nats::Client,
    pair_id: String,
    security: secure::Transport,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Three independent publish channels:
        //   p.{pair}.presence          — liveness micro-packet every 1s
        //   p.{pair}.state.sessions    — session list on signature change + 20s self-heal
        //   p.{pair}.state.workspaces  — workspace list on dirty + 20s self-heal
        let mut interval = tokio::time::interval(presence_tick());
        let mut last_sessions_sig = String::new();
        let mut last_workspaces_sig = String::new();
        let mut secs_since_sessions: u8 = 20; // first tick publishes a baseline
        let mut secs_since_workspaces: u8 = 20;
        loop {
            interval.tick().await;

            // 2. Sessions snapshot (signature change or 20s self-heal).
            let snapshot_pair = pair_id.clone();
            let Ok((dirty, sessions, workspaces)) = tokio::task::spawn_blocking(move || {
                (
                    host().catalog_dirty(),
                    build_sessions_snapshot(&snapshot_pair),
                    build_workspaces_snapshot(),
                )
            })
            .await
            else {
                return;
            };
            // A prolonged store read failure must not overflow and panic the
            // heartbeat task in debug/dev builds; the task supervisor would
            // reconnect it, but the deterministic panic would simply repeat.
            secs_since_sessions = secs_since_sessions.saturating_add(1);
            secs_since_workspaces = secs_since_workspaces.saturating_add(1);
            if let Some((sessions_payload, sessions_sig)) = sessions {
                if sessions_sig != last_sessions_sig || secs_since_sessions >= 20 {
                    let bytes = serde_json::to_vec(&sessions_payload)
                        .expect("a sessions Value always serializes");
                    if let Err(e) = secure::publish(
                        &client,
                        &security,
                        format!("p.{pair_id}.state.sessions"),
                        bytes,
                    )
                    .await
                    {
                        if let Some(line) = HEARTBEAT_PUBLISH_EPISODE.record("state_publish", e) {
                            eprintln!("{line}");
                        }
                        return;
                    }
                    last_sessions_sig = sessions_sig;
                    secs_since_sessions = 0;
                }
            }

            // 3. Workspaces snapshot (dirty flag or 20s self-heal).
            let Some((workspaces_payload, workspaces_sig)) = workspaces else {
                continue;
            };
            if dirty || workspaces_sig != last_workspaces_sig || secs_since_workspaces >= 20 {
                let bytes = serde_json::to_vec(&workspaces_payload)
                    .expect("a workspaces Value always serializes");
                if let Err(e) = secure::publish(
                    &client,
                    &security,
                    format!("p.{pair_id}.state.workspaces"),
                    bytes,
                )
                .await
                {
                    if let Some(line) = HEARTBEAT_PUBLISH_EPISODE.record("state_publish", e) {
                        eprintln!("{line}");
                    }
                    return;
                }
                last_workspaces_sig = workspaces_sig;
                secs_since_workspaces = 0;
            }
        }
    })
}

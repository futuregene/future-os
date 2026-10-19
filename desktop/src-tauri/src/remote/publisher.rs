use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Capability a client declares on `secure_ready` to receive the coalesced
/// event lane. Named so an older desktop ignores it and an older client never
/// asks for it.
pub(super) const EVENT_COALESCING_FEATURE: &str = "event_coalescing_v1";

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
pub(super) const CATALOG_ACTIVITY_FEATURE: &str = "catalog_activity_v1";

/// Where a recency packet goes.
///
/// A separate subject rather than a second shape on `state.sessions`: an older
/// client applies every `state.sessions` payload as the *whole* list (an empty
/// one reads as "your conversations are gone"), so a packet there would either
/// blank its list or need the capability check to be duplicated in every
/// reader. Under `state.` it is covered by the platform's existing wildcard
/// permission, and a client that does not know the suffix ignores it.
fn catalog_activity_subject(pair_id: &str) -> String {
    format!("p.{pair_id}.state.activity")
}

/// Cap on a single event's serialized size. A huge event (e.g. a large tool
/// result) would otherwise exceed the NATS 1MB user-JWT payload limit and be
/// rejected by the broker — silently leaving a permanent gap in the client's
/// event stream. Over-limit events keep their type/runId/idx (so ordering and
/// dedup still work) but ship a truncated `data` marker instead.
pub(super) use future_remote_server::events::EventPublish;
#[cfg(test)]
pub(super) use future_remote_server::events::MAX_EVENT_BYTES;

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
        // Events go to the device in control, and nowhere else. A standby
        // device is still paired and still connected — that is what lets it
        // take back over instantly — but it is not watching this machine's
        // conversations. Publishing to every slot would make "superseded" look
        // like a label rather than a fact.
        let target = SUPERVISOR.controlling_pair();
        let mut guard = SUPERVISOR.state.lock().unwrap();
        target
            .as_ref()
            .and_then(|pair_id| guard.get_mut(pair_id))
            .map(|s| {
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
    // Lean feed: reasoning and streamed tool arguments never reach the lane.
    // Dropping here (rather than later in the drain) keeps the queue and the
    // coalescer from ever holding them.
    // The controlling connection's declaration. Events go to that device only,
    // so its own declaration is the one that decides the trimming.
    let lean = SUPERVISOR.lean_events(&pair_id).load(Ordering::Acquire);
    let data = if lean {
        match crate::remote_host::lean::lean_event_data(event_type, data) {
            Some(data) => data,
            None => return,
        }
    } else {
        std::borrow::Cow::Borrowed(data)
    };
    // Guard the NATS payload cap: an oversized event is published with a
    // truncated `data` marker (type/runId/idx preserved) rather than dropped,
    // so the client's dedup cursor doesn't get a permanent hole.
    let mut body = build_event_body(
        session_id,
        event_type,
        &data,
        run_id,
        idx,
        epoch,
        event_id,
        timestamp,
        session_idx,
        run_sequence,
    );
    // The envelope is the lean lane's largest remaining cost, and most of it is
    // fields no subscriber reads. Trimmed under the same gate as the payload, so
    // a client that did not declare lean still gets the byte-identical legacy
    // body it always did.
    if lean {
        crate::remote_host::lean::lean_event_body(&mut body);
    }
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

pub(super) use future_remote_server::events::build_event_body;

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
        client,
        rx,
        security.inner,
        coalesce,
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
            let bytes =
                serde_json::to_vec(&presence_heartbeat_payload(&pair_id, &bridge_instance_id))
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
        // Three independent publish channels, all change-driven. There is no
        // periodic re-send: a dropped packet is reported through the presence
        // packet's catalog revision (the heartbeat task owns that), which heals
        // in one heartbeat instead of a 20s timer.
        //   p.{pair}.state.sessions    — whole session list, when its
        //                                *structural* signature changes (and on
        //                                any change at all for a client that did
        //                                not declare `catalog_activity_v1`)
        //   p.{pair}.state.activity    — the recency rows that moved, when only
        //                                those moved
        //   p.{pair}.state.workspaces  — workspace list on dirty or signature change
        let mut interval = tokio::time::interval(catalog_tick());
        let mut last_sessions_sig = String::new();
        let mut last_structural_sig = String::new();
        // What this connection has actually been told, per session. The wire
        // carries absolute values, so this is the filter that keeps a packet to
        // the rows that moved — not a ledger a lost packet could corrupt. An
        // empty base (a fresh connection) simply means the first packet carries
        // every row.
        let mut delivered_activity: std::collections::HashMap<String, i64> =
            std::collections::HashMap::new();
        let mut last_workspaces_sig = String::new();
        loop {
            interval.tick().await;
            // A standby device is served liveness only (its presence packet says
            // so). Clearing the recorded signatures is what makes the first tick
            // after it regains control publish a whole snapshot instead of an
            // empty diff against a catalogue it never received.
            if !SUPERVISOR.in_control(&pair_id) {
                last_sessions_sig.clear();
                last_structural_sig.clear();
                last_workspaces_sig.clear();
                delivered_activity.clear();
                continue;
            }
            // Read once per tick: the declaration belongs to the connection, and
            // a client that never made it is served the whole snapshot on every
            // change exactly as before.
            let activity_declared = SUPERVISOR
                .catalog_activity(&pair_id)
                .load(Ordering::Acquire);

            // 2. Sessions: the whole list when its structure moved, the moved
            //    rows alone when only their recency did.
            let snapshot_pair = pair_id.clone();
            let Ok((dirty, sessions, workspaces)) = tokio::task::spawn_blocking(move || {
                (
                    host().catalog_dirty(),
                    host().sessions_catalog(&snapshot_pair),
                    build_workspaces_snapshot(),
                )
            })
            .await
            else {
                return;
            };
            // A store read that fails returns None and this tick is skipped; the
            // next tick recomputes the signature from the store, so a transient
            // failure delays a change by one tick rather than losing it.
            if let Some(catalog) = sessions {
                // Sending the whole list is the *only* thing that can restore a
                // structure the client cannot patch, so it is what a client
                // without the declaration keeps getting.
                let full = !activity_declared || catalog.structural != last_structural_sig;
                if full {
                    if catalog.signature != last_sessions_sig {
                        let bytes = serde_json::to_vec(&catalog.payload)
                            .expect("a sessions Value always serializes");
                        match secure::publish(
                            &client,
                            &security,
                            format!("p.{pair_id}.state.sessions"),
                            bytes,
                        )
                        .await
                        {
                            // Record the signature only once the snapshot actually
                            // left the endpoint. `Ok(false)` means the secure
                            // channel had no key yet, so the payload was dropped —
                            // the next tick retries, which is what makes this a
                            // "retry until delivered" publisher instead of a
                            // "send once and hope" one.
                            Ok(true) => {
                                last_sessions_sig = catalog.signature.clone();
                                last_structural_sig = catalog.structural.clone();
                                delivered_activity = catalog.activity.iter().cloned().collect();
                            }
                            Ok(false) => {}
                            Err(e) => {
                                if let Some(line) =
                                    HEARTBEAT_PUBLISH_EPISODE.record("state_publish", e)
                                {
                                    eprintln!("{line}");
                                }
                                return;
                            }
                        }
                    }
                } else {
                    let updates: Vec<serde_json::Value> = catalog
                        .activity
                        .iter()
                        .filter(|(session, at)| delivered_activity.get(session) != Some(at))
                        .map(|(session, at)| {
                            serde_json::json!({ "sessionId": session, "lastMessageAt": at })
                        })
                        .collect();
                    if !updates.is_empty() {
                        // The revision rides along for the same reason it rides
                        // on a snapshot: it is what lets the client's gate tell a
                        // stale or repeated packet from a newer one, and what the
                        // heartbeat compares when one is lost.
                        let body = serde_json::json!({
                            "pairId": pair_id,
                            "activity": updates,
                            "version": catalog.payload["version"].clone(),
                        });
                        let bytes =
                            serde_json::to_vec(&body).expect("an activity Value always serializes");
                        match secure::publish(
                            &client,
                            &security,
                            catalog_activity_subject(&pair_id),
                            bytes,
                        )
                        .await
                        {
                            // Same retry rule as the snapshot: the base advances
                            // only for what the endpoint actually sent, so a
                            // packet the secure channel could not seal is rebuilt
                            // on the next tick.
                            Ok(true) => {
                                delivered_activity = catalog.activity.iter().cloned().collect()
                            }
                            Ok(false) => {}
                            Err(e) => {
                                if let Some(line) =
                                    HEARTBEAT_PUBLISH_EPISODE.record("state_publish", e)
                                {
                                    eprintln!("{line}");
                                }
                                return;
                            }
                        }
                    }
                }
            }

            // 3. Workspaces snapshot (dirty flag or signature change).
            let Some((workspaces_payload, workspaces_sig)) = workspaces else {
                continue;
            };
            if dirty || workspaces_sig != last_workspaces_sig {
                let bytes = serde_json::to_vec(&workspaces_payload)
                    .expect("a workspaces Value always serializes");
                match secure::publish(
                    &client,
                    &security,
                    format!("p.{pair_id}.state.workspaces"),
                    bytes,
                )
                .await
                {
                    Ok(true) => last_workspaces_sig = workspaces_sig,
                    Ok(false) => {}
                    Err(e) => {
                        if let Some(line) = HEARTBEAT_PUBLISH_EPISODE.record("state_publish", e) {
                            eprintln!("{line}");
                        }
                        return;
                    }
                }
            }
        }
    })
}

//! Instance-owned event admission, liveness and change-driven catalogs.
use super::*;
use crate::{
    events::{is_catalog_event, EventPublish},
    secure,
    services::SessionsCatalog,
};
use std::time::Duration;

/// Host-supplied scheduling. Production defaults preserve the existing cadence;
/// deterministic test drivers can supply their own without application cfg flags.
#[derive(Clone, Copy)]
pub struct PublisherTiming {
    pub(super) presence: Duration,
    pub(super) catalog: Duration,
}
impl Default for PublisherTiming {
    fn default() -> Self {
        Self {
            presence: Duration::from_secs(3),
            catalog: Duration::from_secs(1),
        }
    }
}
impl PublisherTiming {
    pub fn new(presence: Duration, catalog: Duration) -> Result<Self, crate::Error> {
        if presence.is_zero() || catalog.is_zero() {
            return Err(crate::Error::Message(
                "remote_publisher_interval_zero".into(),
            ));
        }
        Ok(Self { presence, catalog })
    }
}

fn catalog_activity_subject(pair: &str) -> String {
    format!("p.{pair}.state.activity")
}
type CatalogRead = (
    bool,
    Option<SessionsCatalog>,
    Option<(serde_json::Value, String)>,
);

impl Supervisor {
    #[allow(clippy::too_many_arguments)]
    pub fn publish_event(
        &self,
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
            let target = self.controlling_pair();
            let mut guard = self.state.lock().unwrap();
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
        // A legacy peer must receive an explicit upgrade outcome, never
        // incomplete referenced bodies. Suppress later frames for this run.
        // The cache is bounded and is reset with the connection declaration.
        let unsupported = !self.segmented_content(&pair_id).load(Ordering::Acquire)
            && crate::content_capability::canonical_data(data);
        let refusal;
        let (event_type, data) = if unsupported {
            let shared = self.shared_or_insert(&pair_id);
            let key = serde_json::json!([session_id, run_id, epoch]).to_string();
            let mut refused = shared.content_refusals.lock().unwrap();
            if refused.contains(&key) {
                return;
            }
            if refused.len() >= 64 {
                refused.pop_front();
            }
            refused.push_back(key);
            refusal = serde_json::json!({"type":"agent_end","state":"error",
                "error":crate::content_capability::UPGRADE_REQUIRED,
                "error_code":"client_upgrade_required"})
            .to_string();
            ("agent_end", refusal.as_str())
        } else {
            (event_type, data)
        };
        // Lean feed: reasoning and streamed tool arguments never reach the lane.
        // Dropping here (rather than later in the drain) keeps the queue and the
        // coalescer from ever holding them.
        // The controlling connection's declaration. Events go to that device only,
        // so its own declaration is the one that decides the trimming.
        let lean = self.lean_events(&pair_id).load(Ordering::Acquire);
        let data = if lean {
            match crate::lean::lean_event_data(event_type, data) {
                Some(data) => data,
                None => return,
            }
        } else {
            std::borrow::Cow::Borrowed(data)
        };
        // Guard the NATS payload cap: an oversized event is published with a
        // truncated `data` marker (type/runId/idx preserved) rather than dropped,
        // so the client's dedup cursor doesn't get a permanent hole.
        let mut body = crate::events::build_event_body(
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
            crate::lean::lean_event_body(&mut body);
        }
        // A serde_json::Value always serializes, so this cannot fail.
        let payload = serde_json::to_vec(&body).expect("an event Value always serializes");
        let event = EventPublish {
            subject: format!("p.{pair_id}.evt.{session_id}"),
            payload,
            // Existing state wildcard permissions cover this low-rate lane. Old
            // clients ignore the new suffix and keep their full legacy event feed.
            status_subject: is_catalog_event(event_type)
                .then(|| format!("p.{pair_id}.state.events")),
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

    /// Poll the heartbeat and its child work together. Dropping this future
    /// cancels the monitor and catalog future before the caller observes exit.
    pub fn run_presence_heartbeat(
        self: &Arc<Self>,
        client: async_nats::Client,
        pair_id: String,
        bridge_instance_id: String,
        security: secure::Transport,
        timing: PublisherTiming,
    ) -> BoxFuture<'static, ()> {
        let epoch = self.access.current();
        let server = self.clone();
        Box::pin(async move {
            tokio::select! {
                biased;
                _ = server.access.cancelled(epoch) => {},
                _ = server.presence_loop(client, pair_id, bridge_instance_id, security, timing, epoch) => {},
            }
        })
    }
    async fn presence_loop(
        &self,
        client: async_nats::Client,
        pair_id: String,
        bridge_instance_id: String,
        security: secure::Transport,
        timing: PublisherTiming,
        epoch: u64,
    ) {
        let health = self.host.monitor_agent();
        let catalog = self.catalog_loop(
            epoch,
            timing.catalog,
            client.clone(),
            pair_id.clone(),
            security.clone(),
        );
        let heartbeat = self.heartbeat_loop(
            client,
            &pair_id,
            &bridge_instance_id,
            security,
            timing.presence,
        );
        tokio::pin!(health, catalog, heartbeat);
        let mut health_done = false;
        loop {
            tokio::select! {
                _ = &mut catalog => return,
                _ = &mut heartbeat => return,
                _ = &mut health, if !health_done => health_done = true,
            }
        }
    }

    async fn heartbeat_loop(
        &self,
        client: async_nats::Client,
        pair_id: &str,
        bridge_instance_id: &str,
        security: secure::Transport,
        tick: Duration,
    ) {
        let mut interval = tokio::time::interval(tick);
        loop {
            interval.tick().await;
            let bytes =
                serde_json::to_vec(&self.presence_heartbeat_payload(pair_id, bridge_instance_id))
                    .expect("a presence Value always serializes");
            if let Err(error) =
                secure::publish(&client, &security, format!("p.{pair_id}.presence"), bytes).await
            {
                if let Some(line) = self.publisher_failures.record("heartbeat_publish", error) {
                    eprintln!("{line}");
                }
                return;
            }
            if let Some(line) = self.publisher_failures.recovered() {
                eprintln!("{line}");
            }
        }
    }

    async fn catalog_snapshot(&self, epoch: u64, pair: &str) -> Option<CatalogRead> {
        let host = self.host.clone();
        let access = self.access.clone();
        let pair = pair.to_owned();
        let (tx, rx) = tokio::sync::oneshot::channel();
        // Register atomically with invalidation. Already-running native reads
        // cannot be cancelled; wait_tasks observes their real completion.
        self.access.commit(epoch, || {
            let (handle, job) = tasks::native(Box::new(move || {
                if access.current() == epoch && !tx.is_closed() {
                    let snapshot = (
                        host.catalog_dirty(),
                        host.sessions_catalog(&pair),
                        host.workspaces(),
                    );
                    let _ = tx.send(snapshot);
                }
            }));
            let mut native = self.native_tasks.lock().unwrap();
            native.retain(|task| !task.is_finished());
            native.push(handle);
            self.executor.spawn_native(job);
        })?;
        tokio::select! {
            biased;
            _ = self.access.cancelled(epoch) => None,
            result = rx => result.ok(),
        }
    }
    async fn catalog_loop(
        &self,
        epoch: u64,
        tick: Duration,
        client: async_nats::Client,
        pair_id: String,
        security: secure::Transport,
    ) {
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
        let mut interval = tokio::time::interval(tick);
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
            if !self.in_control(&pair_id) {
                last_sessions_sig.clear();
                last_structural_sig.clear();
                last_workspaces_sig.clear();
                delivered_activity.clear();
                continue;
            }
            // Read once per tick: the declaration belongs to the connection, and
            // a client that never made it is served the whole snapshot on every
            // change exactly as before.
            let activity_declared = self.catalog_activity(&pair_id).load(Ordering::Acquire);

            // 2. Sessions: the whole list when its structure moved, the moved
            //    rows alone when only their recency did.
            let Some((dirty, sessions, workspaces)) = self.catalog_snapshot(epoch, &pair_id).await
            else {
                return;
            };
            // Native reads can finish after a stop or takeover. Such a result
            // must not disclose a catalog to a device that has lost control.
            if self.access.current() != epoch || !self.in_control(&pair_id) {
                continue;
            }
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
                                    self.publisher_failures.record("state_publish", e)
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
                                    self.publisher_failures.record("state_publish", e)
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
                        if let Some(line) = self.publisher_failures.record("state_publish", e) {
                            eprintln!("{line}");
                        }
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::{ownership_tests::QueueExecutor, test_support::MemoryHost};

    #[tokio::test]
    async fn catalogue_reads_skip_invalidated_epochs_before_registration() {
        let host = Arc::new(MemoryHost::default());
        let executor = Arc::new(QueueExecutor::default());
        let server = Supervisor::new(host.clone(), executor.clone());
        let epoch = server.access.current();
        server.access.invalidate(|| {});
        assert!(server.catalog_snapshot(epoch, "pair_old").await.is_none());
        assert!(executor.native.lock().unwrap().is_empty());
        assert!(host.catalog_reads.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancelled_catalogue_reader_keeps_native_completion_owned() {
        let host = Arc::new(MemoryHost::default());
        let executor = Arc::new(QueueExecutor::default());
        let server = Supervisor::new(host.clone(), executor.clone());
        let mut read = Box::pin(server.catalog_snapshot(server.access.current(), "pair_first"));
        assert!(futures::poll!(&mut read).is_pending());
        assert_eq!(executor.native.lock().unwrap().len(), 1);
        drop(read);
        assert!(server.wait_tasks(Duration::from_millis(1)).await.is_err());
        executor.run_native_reverse();
        server.wait_tasks(Duration::from_secs(1)).await.unwrap();
        assert!(
            host.catalog_reads.lock().unwrap().is_empty(),
            "a cancelled queued read must not touch storage"
        );
    }

    #[tokio::test]
    async fn delayed_catalogue_read_cannot_cross_stop_or_instance_boundaries() {
        let first_host = Arc::new(MemoryHost::default());
        let second_host = Arc::new(MemoryHost::default());
        let executor = Arc::new(QueueExecutor::default());
        let first = Supervisor::new(first_host.clone(), executor.clone());
        let second = Supervisor::new(second_host.clone(), executor.clone());
        let mut stale = Box::pin(first.catalog_snapshot(first.access.current(), "pair_first"));
        let mut current = Box::pin(second.catalog_snapshot(second.access.current(), "pair_second"));
        assert!(futures::poll!(&mut stale).is_pending());
        assert!(futures::poll!(&mut current).is_pending());
        first.access.invalidate(|| {});
        executor.run_native_reverse();
        assert!(stale.await.is_none());
        assert!(current.await.is_some());
        first.wait_tasks(Duration::from_secs(1)).await.unwrap();
        second.wait_tasks(Duration::from_secs(1)).await.unwrap();
        assert!(first_host.catalog_reads.lock().unwrap().is_empty());
        assert_eq!(*second_host.catalog_reads.lock().unwrap(), ["pair_second"]);
    }
    #[tokio::test]
    async fn running_catalogue_read_survives_cancellation_until_native_work_finishes() {
        let host = Arc::new(MemoryHost::default());
        let (entered, observed) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        *host.catalog_pause.lock().unwrap() = Some((entered, released));
        let server = Supervisor::new(
            host.clone(),
            Arc::new(crate::supervisor::test_support::TokioExecutor),
        );
        let mut read = Box::pin(server.catalog_snapshot(server.access.current(), "pair_running"));
        assert!(futures::poll!(&mut read).is_pending());
        observed.await.unwrap();
        server.access.invalidate(|| {});
        assert!(
            read.await.is_none(),
            "invalidation drops the async receiver immediately"
        );
        assert!(
            server.wait_tasks(Duration::from_millis(1)).await.is_err(),
            "actual native work is still running"
        );
        release.send(()).unwrap();
        server.wait_tasks(Duration::from_secs(1)).await.unwrap();
        assert_eq!(*host.catalog_reads.lock().unwrap(), ["pair_running"]);
    }
}

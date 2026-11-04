//! Atomic attachment, replay cursors and run snapshots.
use super::{RunAttachment, RunProjectionSnapshot, SseBroadcaster, SseEvent};
use serde_json::Value;

/// Snapshot capture copies shared references and a bounded tail under the lock.
/// Body cloning/JSON materialization and argument folding happen after release.
pub(super) struct CapturedSnapshot {
    run_id: String,
    epoch: i64,
    run_sequence: i64,
    cursor: i64,
    view: super::projection::ProjectionView,
}
impl CapturedSnapshot {
    pub(super) fn capture(run: &super::RunState) -> Self {
        Self {
            run_id: run.run_id.clone(),
            epoch: run.epoch,
            run_sequence: run.run_sequence,
            cursor: run.idx.saturating_sub(1),
            view: run.projection_events.view(),
        }
    }
    pub(super) fn materialize(self) -> RunProjectionSnapshot {
        RunProjectionSnapshot {
            run_id: self.run_id,
            epoch: self.epoch,
            run_sequence: self.run_sequence,
            cursor: self.cursor,
            events: self.view.materialize(),
        }
    }
}

impl SseBroadcaster {
    /// Explicit history reset invalidates every cached old run. The session
    /// maintenance fence guarantees no execution owns this broadcaster now.
    pub(crate) fn forget_idle_replay(&self) {
        let mut run = self.run.lock();
        run.run_id.clear();
        run.epoch = 0;
        run.idx = 0;
        run.run_sequence = -1;
        run.events.clear();
        run.event_bytes = 0;
        run.projection_events.clear();
        run.projection_complete = true;
        run.compacted = false;
    }

    /// Register a receiver and capture its replay boundary under the stamping
    /// lock. Raw disk backfill stops at that cursor; subsequent events arrive
    /// through the receiver while disk IO runs without broadcaster locks.
    pub fn attach(&self, run_id: &str, after_idx: i64) -> anyhow::Result<RunAttachment> {
        anyhow::ensure!(after_idx >= -1, "invalid attachment cursor");
        let run = self.run.lock();
        if run.run_id != run_id {
            anyhow::bail!("run `{run_id}` is not the active run");
        }
        let receiver = self.tx.subscribe();
        let cursor = run.idx.saturating_sub(1);
        let min_idx = run.events.front().map(|event| event.idx).unwrap_or(run.idx);
        let truncated = after_idx.saturating_add(1) < min_idx;
        let (has_store, canonical) = {
            let journal = self.journal.lock();
            (truncated && journal.store.is_some(), journal.segmented)
        };
        anyhow::ensure!(
            !canonical || after_idx <= cursor,
            "attachment cursor exceeds the run's produced prefix"
        );
        if has_store && canonical {
            let backfill = super::RunBackfill {
                run_id: run_id.to_owned(),
                epoch: run.epoch,
                after_idx,
                until_idx: cursor,
            };
            return Ok(RunAttachment {
                receiver,
                events: Vec::new(),
                truncated: true,
                projection: None,
                backfill: Some(backfill),
            });
        }
        let projection =
            (truncated && (run.compacted || !has_store)).then(|| CapturedSnapshot::capture(&run));
        let events = run
            .events
            .iter()
            .filter(|event| !truncated && event.idx > after_idx)
            .cloned()
            .collect();
        drop(run);
        let projection = projection.map(CapturedSnapshot::materialize);
        if truncated {
            let count = self
                .truncation_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            if !has_store {
                tracing::warn!(
                    run_id,
                    requested_after_idx = after_idx,
                    min_available_idx = min_idx,
                    truncation_count = count,
                    "run replay ring truncated; returning projection snapshot"
                );
            }
        }
        if has_store {
            let (known, mut events, snapshot) =
                self.read_journal_page(run_id, after_idx, false, None)?;
            anyhow::ensure!(known, "run `{run_id}` is not known by this session");
            if let Some(snapshot) = snapshot {
                // A complete persisted replacement also covers any events
                // queued on the receiver through its cursor; clients resync
                // from that cursor just as in the existing snapshot protocol.
                return Ok(RunAttachment {
                    receiver,
                    events: Vec::new(),
                    truncated: true,
                    projection: Some(snapshot),
                    backfill: None,
                });
            }
            events.retain(|event| event.idx <= cursor);
            return Ok(RunAttachment {
                receiver,
                events,
                truncated,
                projection,
                backfill: None,
            });
        }
        Ok(RunAttachment {
            receiver,
            events,
            truncated,
            projection,
            backfill: None,
        })
    }

    /// Called by the transport on a blocking worker. A stored range may cover
    /// the captured boundary plus newer fragments in the same commit. Return
    /// that complete immutable range; the transport advances its live cutoff
    /// to its last cursor so the receiver's overlapping fragments are skipped.
    pub(crate) fn attachment_page(
        &self,
        backfill: &super::RunBackfill,
        since: i64,
    ) -> anyhow::Result<Vec<SseEvent>> {
        if since == backfill.after_idx {
            self.flush_journal()?;
        }
        let (store, session) = {
            let journal = self.journal.lock();
            (
                journal
                    .store
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("attachment storage is unavailable"))?,
                journal.session_id.clone(),
            )
        };
        let events = store.segmented_stream_page(
            &crate::session::content::stream::StreamIdentity {
                session_id: session,
                run_id: backfill.run_id.clone(),
                epoch: backfill.epoch,
            },
            since,
            64,
        )?;
        let mut result = Vec::new();
        let mut expected = since.saturating_add(1);
        for event in events {
            anyhow::ensure!(
                event.epoch == backfill.epoch && event.run_id == backfill.run_id,
                "attachment run epoch changed"
            );
            let data = serde_json::from_str(&event.data)?;
            anyhow::ensure!(
                future_rpc::content_range::covers_next(&data, expected, event.idx),
                "attachment durable prefix has a gap"
            );
            expected = event.idx.saturating_add(1);
            let finished = event.idx >= backfill.until_idx;
            result.push(event);
            if finished {
                break;
            }
        }
        anyhow::ensure!(
            !result.is_empty(),
            "attachment durable prefix is incomplete"
        );
        Ok(result)
    }

    /// Capture a finite read window before decoding its first page. Canonical
    /// readers fence the already-admitted queue outside every broadcaster lock.
    pub(crate) fn replay_watermark(&self, run_id: &str) -> anyhow::Result<Option<i64>> {
        let (active, store, session, canonical) = {
            let run = self.run.lock();
            let journal = self.journal.lock();
            (
                (run.run_id == run_id).then(|| run.idx.saturating_sub(1)),
                journal.store.clone(),
                journal.session_id.clone(),
                journal.segmented,
            )
        };
        if let Some(store) = store {
            if canonical {
                self.flush_journal()?;
            }
            if canonical || active.is_none() {
                return store.run_replay_watermark(&session, run_id);
            }
        }
        Ok(active)
    }

    /// The earliest cursor this Agent can still serve for `run_id`, or `None`
    /// when it holds nothing for that run.
    ///
    /// The answer is the union of the two replay sources: the live ring's front
    /// and the journal's first stored change. A reader whose cursor sits below it
    /// asked for events that are gone, and the replay must say so — otherwise a
    /// consumer folding the surviving tail into its own projection skips whatever
    /// fell outside the window without ever learning it did.
    ///
    /// The journal is only asked when the ring cannot answer for `since`: while
    /// the ring still covers the requested window the replay is served from it and
    /// the head is, by definition, the ring's front.
    pub(crate) fn earliest_cursor(&self, run_id: &str, since: i64) -> anyhow::Result<Option<i64>> {
        let (ring_front, store, session) = {
            let run = self.run.lock();
            let journal = self.journal.lock();
            let ring_front = (run.run_id == run_id)
                .then(|| run.events.front().map(|event| event.idx))
                .flatten()
                .or_else(|| {
                    // A compacted run keeps no raw ring; its projection starts at
                    // the oldest event it folded, which is what it can serve.
                    (run.run_id == run_id && run.compacted)
                        .then(|| run.projection_events.first_idx())
                        .flatten()
                });
            (
                ring_front,
                journal.store.clone(),
                journal.session_id.clone(),
            )
        };
        if ring_front.is_some_and(|front| since.saturating_add(1) >= front) {
            return Ok(ring_front);
        }
        let stored = match store {
            Some(store) => {
                // The ring could not answer for `since`, so the journal has to:
                // fence the buffered appends first, exactly like the watermark
                // probe, or a delta still in flight would read as a lost head.
                self.flush_journal()?;
                store.run_first_cursor(&session, run_id)?
            }
            None => None,
        };
        Ok(match (ring_front, stored) {
            (Some(ring), Some(stored)) => Some(ring.min(stored)),
            (ring, stored) => ring.or(stored),
        })
    }

    /// Current-run events with `idx > since_idx`, plus the earliest idx still in
    /// the buffer (`min_idx`, 0 if empty). A stale run id is an explicit error;
    /// it must never silently return another run's events. A
    /// full backfill (`since_idx < 0`) whose result starts above `min_idx == 0`
    /// — i.e. `min_idx > 0` — means the run's prefix was dropped on overflow, so
    /// the caller can surface the gap instead of silently reconstructing a
    /// truncated message.
    pub fn events_since(
        &self,
        run_id: &str,
        since_idx: i64,
    ) -> anyhow::Result<(String, Vec<SseEvent>, i64, Option<RunProjectionSnapshot>)> {
        self.events_page(run_id, since_idx, None)
    }

    /// Fetch at most `limit` events at the source, including disk backfill.
    /// Callers request one lookahead row to determine whether a page follows.
    pub fn events_page(
        &self,
        run_id: &str,
        since_idx: i64,
        limit: Option<usize>,
    ) -> anyhow::Result<(String, Vec<SseEvent>, i64, Option<RunProjectionSnapshot>)> {
        let run = self.run.lock();
        if run.run_id != run_id {
            // Historical IO must not hold the current run's stamping lock.
            drop(run);
            let (known, events, snapshot) =
                self.read_journal_page(run_id, since_idx, false, limit)?;
            if !known {
                anyhow::bail!("run `{run_id}` is not known by this session");
            }
            let min_idx = events.first().map(|event| event.idx).unwrap_or(0);
            let projection = snapshot.filter(|snapshot| since_idx < snapshot.cursor);
            return Ok((run_id.to_string(), events, min_idx, projection));
        }
        if run.compacted {
            let cursor = run.idx.saturating_sub(1);
            let projection = (since_idx < cursor).then(|| CapturedSnapshot::capture(&run));
            drop(run);
            return Ok((
                run_id.to_string(),
                Vec::new(),
                cursor,
                projection.map(CapturedSnapshot::materialize),
            ));
        }
        let min_idx = run.events.front().map(|event| event.idx).unwrap_or(0);
        let truncated = since_idx.saturating_add(1) < min_idx;
        let has_store = truncated && self.journal.lock().store.is_some();
        let events = run
            .events
            .iter()
            .filter(|event| !truncated && event.idx > since_idx)
            .take(limit.unwrap_or(usize::MAX))
            .cloned()
            .collect();
        let projection = (truncated && !has_store).then(|| CapturedSnapshot::capture(&run));
        // Capture the memory answer once. Disk backfill may wait for the DB
        // worker; later broadcasts and run transitions can proceed meanwhile.
        let active_run_id = run.run_id.clone();
        drop(run);
        let projection = projection.map(CapturedSnapshot::materialize);
        if truncated {
            let count = self
                .truncation_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            if !has_store {
                tracing::warn!(
                    run_id,
                    requested_after_idx = since_idx,
                    min_available_idx = min_idx,
                    truncation_count = count,
                    "run event query crossed replay-ring boundary; returning projection snapshot"
                );
            }
        }
        if has_store {
            let (known, events, snapshot) =
                self.read_journal_page(run_id, since_idx, false, limit)?;
            anyhow::ensure!(known, "run `{run_id}` is not known by this session");
            if let Some(snapshot) = snapshot {
                let projection = (since_idx < snapshot.cursor).then(|| snapshot.clone());
                return Ok((active_run_id, Vec::new(), snapshot.cursor, projection));
            }
            return Ok((active_run_id, events, min_idx, None));
        }
        Ok((active_run_id, events, min_idx, projection))
    }

    /// Capture a resumable semantic snapshot, not the raw token journal. The
    /// active projection and cursor are cloned under the stamping lock. For an
    /// older run, one SQLite transaction supplies a consistent immutable prefix.
    pub(crate) fn run_snapshot(&self, run_id: &str) -> anyhow::Result<RunProjectionSnapshot> {
        let mut snapshot = self.uncaptured_run_snapshot(run_id)?;
        trim_projection(&mut snapshot.events);
        Ok(snapshot)
    }

    fn uncaptured_run_snapshot(&self, run_id: &str) -> anyhow::Result<RunProjectionSnapshot> {
        anyhow::ensure!(!run_id.is_empty(), "snapshot requires a run id");
        let run = self.run.lock();
        if run.run_id == run_id {
            anyhow::ensure!(
                !run.projection_events.over_budget(),
                "run snapshot exceeds memory budget"
            );
            anyhow::ensure!(run.projection_complete, "run snapshot prefix is incomplete");
            if let Some(error) = self.journal_health.error() {
                anyhow::bail!("run snapshot persistence is unhealthy: {error}");
            }
            let captured = CapturedSnapshot::capture(&run);
            drop(run);
            let mut snapshot = captured.materialize();
            snapshot.events =
                crate::rpc::run_snapshot::strip_repeated_tool_arguments(snapshot.events);
            return Ok(snapshot);
        }
        // Historical reads must not block active broadcasts while decoding or
        // folding a long journal. The DB read itself is transaction-scoped.
        drop(run);
        let (store, session_id) = {
            let journal = self.journal.lock();
            (journal.store.clone(), journal.session_id.clone())
        };
        // A fold kept beside the journal saves the whole replay. It is keyed by
        // (session, run) and versioned by ([`SNAPSHOT_CACHE_VERSION`], epoch), so
        // a run that restarted (new epoch) or a fold algorithm that changed
        // (new version) is a miss — never a stale projection handed to a client.
        // A miss costs exactly what this read cost before the cache existed.
        if let Some(store) = store.as_ref() {
            if let Some(epoch) = store.run_epoch(&session_id, run_id)? {
                if let Some(cached) = store.cached_run_snapshot(&session_id, run_id, epoch)? {
                    return Ok(RunProjectionSnapshot {
                        run_id: cached.run_id,
                        epoch: cached.epoch,
                        run_sequence: cached.run_sequence,
                        cursor: cached.cursor,
                        events: cached.events,
                    });
                }
            }
        }
        let (known, events, snapshot) = self.read_journal_page(run_id, -1, false, None)?;
        anyhow::ensure!(known, "run `{run_id}` is not known by this session");
        if let Some(snapshot) = snapshot {
            return Ok(snapshot);
        }
        anyhow::ensure!(
            covers_prefix(&events, run_id),
            "run snapshot prefix is incomplete"
        );
        let last = events
            .last()
            .ok_or_else(|| anyhow::anyhow!("run snapshot is empty"))?;
        let mut snapshot = RunProjectionSnapshot {
            run_id: run_id.to_owned(),
            epoch: last.epoch,
            run_sequence: last.run_sequence,
            cursor: last.idx,
            events: crate::rpc::run_snapshot::fold_events(events),
        };
        // Trimmed before it is kept, so the cache holds what a reply serves
        // rather than a fatter copy the next open would parse and re-trim.
        trim_projection(&mut snapshot.events);
        // Keep the fold for the next open, off this request: the write is a
        // ~260 KB row and the reply must not wait for it.
        if let Some(store) = store {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let keep = crate::session::journal_compaction::RunJournalSnapshot {
                    run_id: snapshot.run_id.clone(),
                    epoch: snapshot.epoch,
                    run_sequence: snapshot.run_sequence,
                    cursor: snapshot.cursor,
                    events: snapshot.events.clone(),
                };
                let session = session_id.clone();
                handle.spawn_blocking(move || {
                    if let Err(error) = store.store_run_snapshot_cache(&session, &keep) {
                        tracing::debug!(%error, run = %keep.run_id, "run snapshot cache write failed");
                    }
                });
            }
        }
        Ok(snapshot)
    }
}

pub(super) fn covers_prefix(events: &[SseEvent], run: &str) -> bool {
    let mut next = 0;
    for event in events {
        let data = serde_json::from_str::<serde_json::Value>(&event.data).unwrap_or_default();
        if event.run_id != run || !future_rpc::content_range::covers_next(&data, next, event.idx) {
            return false;
        }
        next = event.idx.saturating_add(1);
    }
    true
}

/// Drop what no replay reader uses from a run's folded projection.
///
/// Two bodies ride it as dead weight. A tool result's captured text is never
/// rendered: readers take the row's label from `tool_start`'s arguments and its
/// outcome from the structured fields (`exit_code`, `is_soft_fail`, `error`,
/// `shell_result`). And reasoning text is already blanked for every remote
/// connection by the lean feed, so keeping it here only costs storage and a
/// parse. Measured on a 136k-event run: 449 KiB and 269 KiB of a 1.35 MB
/// payload.
///
/// The one payload that keeps its text is the legacy tool result: without an
/// outcome field the client derives pass/fail by parsing an `[exit: N]` footer
/// out of that text, so removing it would invent a result.
pub(crate) fn trim_projection(events: &mut [SseEvent]) {
    for event in events {
        match event.event_type.as_str() {
            "tool_end" | "tool_result" => {
                let Ok(Value::Object(mut fields)) = serde_json::from_str::<Value>(&event.data)
                else {
                    continue;
                };
                let has_outcome = ["exit_code", "exitCode", "is_soft_fail", "is_error", "error"]
                    .iter()
                    .any(|key| fields.contains_key(*key));
                if !has_outcome {
                    continue;
                }
                if fields.remove("text").is_some() | fields.remove("result").is_some() {
                    event.data = Value::Object(fields).to_string();
                }
            }
            "thinking_delta" => {
                let Ok(Value::Object(mut fields)) = serde_json::from_str::<Value>(&event.data)
                else {
                    continue;
                };
                if let Some(text) = fields.get_mut("text") {
                    if text.as_str().is_some_and(|text| !text.is_empty()) {
                        *text = Value::String(String::new());
                        event.data = Value::Object(fields).to_string();
                    }
                }
            }
            _ => {}
        }
    }
}

//! Atomic attachment, replay cursors and run snapshots.
use super::{RunAttachment, RunProjectionSnapshot, SseBroadcaster, SseEvent};

impl SseBroadcaster {
    /// Register a receiver and capture its replay boundary under the stamping
    /// lock. Raw disk backfill stops at that cursor; subsequent events arrive
    /// through the receiver while disk IO runs without broadcaster locks.
    pub fn attach(&self, run_id: &str, after_idx: i64) -> anyhow::Result<RunAttachment> {
        let run = self.run.lock();
        if run.run_id != run_id {
            anyhow::bail!("run `{run_id}` is not the active run");
        }
        let receiver = self.tx.subscribe();
        let cursor = run.idx.saturating_sub(1);
        let min_idx = run.events.first().map(|event| event.idx).unwrap_or(run.idx);
        let truncated = after_idx.saturating_add(1) < min_idx;
        let has_store = truncated && self.journal.lock().store.is_some();
        let projection =
            (truncated && (run.compacted || !has_store)).then(|| RunProjectionSnapshot {
                run_id: run.run_id.clone(),
                epoch: run.epoch,
                run_sequence: run.run_sequence,
                cursor,
                events: run.projection_events.clone(),
            });
        let events = run
            .events
            .iter()
            .filter(|event| !truncated && event.idx > after_idx)
            .cloned()
            .collect();
        drop(run);
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
                });
            }
            events.retain(|event| event.idx <= cursor);
            return Ok(RunAttachment {
                receiver,
                events,
                truncated,
                projection,
            });
        }
        Ok(RunAttachment {
            receiver,
            events,
            truncated,
            projection,
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
            let projection =
                (since_idx < run.idx.saturating_sub(1)).then(|| RunProjectionSnapshot {
                    run_id: run.run_id.clone(),
                    epoch: run.epoch,
                    run_sequence: run.run_sequence,
                    cursor: run.idx.saturating_sub(1),
                    events: run.projection_events.clone(),
                });
            return Ok((
                run_id.to_string(),
                Vec::new(),
                run.idx.saturating_sub(1),
                projection,
            ));
        }
        let min_idx = run.events.first().map(|event| event.idx).unwrap_or(0);
        let truncated = since_idx.saturating_add(1) < min_idx;
        let has_store = truncated && self.journal.lock().store.is_some();
        let events = run
            .events
            .iter()
            .filter(|event| !truncated && event.idx > since_idx)
            .take(limit.unwrap_or(usize::MAX))
            .cloned()
            .collect();
        let projection = (truncated && !has_store).then(|| RunProjectionSnapshot {
            run_id: run.run_id.clone(),
            epoch: run.epoch,
            run_sequence: run.run_sequence,
            cursor: run.idx.saturating_sub(1),
            events: run.projection_events.clone(),
        });
        // Capture the memory answer once. Disk backfill may wait for the DB
        // worker; later broadcasts and run transitions can proceed meanwhile.
        let active_run_id = run.run_id.clone();
        drop(run);
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
        anyhow::ensure!(!run_id.is_empty(), "snapshot requires a run id");
        let run = self.run.lock();
        if run.run_id == run_id {
            anyhow::ensure!(run.projection_complete, "run snapshot prefix is incomplete");
            if let Some(error) = self.journal_health.error() {
                anyhow::bail!("run snapshot persistence is unhealthy: {error}");
            }
            return Ok(RunProjectionSnapshot {
                run_id: run.run_id.clone(),
                epoch: run.epoch,
                run_sequence: run.run_sequence,
                cursor: run.idx.saturating_sub(1),
                // The live projection and the historical fold both pass through
                // the same argument de-duplication, so a client that joins a
                // running session and one that opens it later receive the same
                // events rather than two dialect of the same run.
                events: crate::rpc::run_snapshot::strip_repeated_tool_arguments(
                    run.projection_events.clone(),
                ),
            });
        }
        // Historical reads must not block active broadcasts while decoding or
        // folding a long journal. The DB read itself is transaction-scoped.
        drop(run);
        let (known, events, snapshot) = self.read_journal_page(run_id, -1, false, None)?;
        anyhow::ensure!(known, "run `{run_id}` is not known by this session");
        if let Some(snapshot) = snapshot {
            return Ok(snapshot);
        }
        anyhow::ensure!(
            events
                .iter()
                .enumerate()
                .all(|(index, event)| event.run_id == run_id && event.idx == index as i64),
            "run snapshot prefix is incomplete"
        );
        let last = events
            .last()
            .ok_or_else(|| anyhow::anyhow!("run snapshot is empty"))?;
        Ok(RunProjectionSnapshot {
            run_id: run_id.to_owned(),
            epoch: last.epoch,
            run_sequence: last.run_sequence,
            cursor: last.idx,
            events: crate::rpc::run_snapshot::fold_events(events),
        })
    }
}

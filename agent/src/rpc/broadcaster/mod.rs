//! Live delivery, reconnect projection and durable journal backfill.
use super::protocol::{is_session_scoped_event, SseEvent};
use journal_writer::{AppendFailure, EventBatchWriter, EventJournalHealth};
#[cfg(test)]
use journal_writer::{EventWriteCommand, EVENT_BATCH_WINDOW};
use tokio::sync::broadcast;
mod admission;
mod content_sequence;
mod journal;
mod journal_control;
mod journal_metrics;
mod journal_writer;
mod projection;
mod replay;
#[cfg(test)]
pub(in crate::rpc) use projection::apply_to_projection;
use projection::is_buffered_delta;

/// Max buffered events per run (for `events_since` backfill). Oldest dropped.
/// Only the *current* run is buffered (cleared on `start_run`), so this is a
/// per-session ceiling, not cumulative. Sized to comfortably hold a long
/// generation's per-token `text_chunk` stream; on overflow the oldest are
/// dropped and `events_since` reports the resulting gap via `min_idx`.
/// Max events buffered per run for `events_since` resync.
/// 2000 is sufficient — a client that falls behind 2000 events
/// is effectively disconnected and should reconnect.
const MAX_RUN_EVENTS: usize = 2_000;

/// Live-subscriber broadcast ring capacity. Sized for high-reasoning model
/// bursts (measured ~3.4k `thinking_delta` events/sec at xhigh), not the
/// nominal ~15-30 events/sec of a normal turn. A receiver falling further
/// than this behind gets `RecvError::Lagged`, which the gRPC layer surfaces
/// as a `DataLoss` "event stream gap".
pub const BROADCAST_RING_CAPACITY: usize = 4_096;

struct RunState {
    run_id: String,
    epoch: i64,
    idx: i64,
    run_sequence: i64,
    events: std::collections::VecDeque<SseEvent>,
    event_bytes: usize,
    projection_events: projection::Projection,
    projection_complete: bool,
    compacted: bool,
}

impl RunState {
    fn record_event(&mut self, event: &SseEvent, segmented: bool) {
        if segmented {
            self.projection_events.limit(8 * 1024 * 1024);
        }
        self.projection_events.push(event);
        self.event_bytes = self.event_bytes.saturating_add(event.data.len() + 512);
        self.events.push_back(event.clone());
        while self.events.len() > MAX_RUN_EVENTS
            || (segmented && self.event_bytes > 4 * 1024 * 1024)
        {
            let Some(old) = self.events.pop_front() else {
                break;
            };
            self.event_bytes = self.event_bytes.saturating_sub(old.data.len() + 512);
        }
    }
}

#[derive(Default)]
struct EventJournalState {
    session_id: String,
    session_idx: i64,
    store: Option<crate::session::sqlite_store::SqliteStore>,
    writer: Option<EventBatchWriter>,
    closed: bool,
    segmented: bool,
    storage_identity: Option<future_rpc::content_range::StorageIdentity>,
    /// Volatile cursors at the mode boundary. Rebinding is safe only before
    /// a memory-only event has been admitted or a run has started.
    memory_boundary: Option<(i64, i64)>,
    content_sequence: content_sequence::ContentSequence,
    #[cfg(test)]
    fail_at: Option<JournalFailPoint>,
    /// A drain failure injected where the guard that decides whether to ask for
    /// compaction cannot see it. Appends report their own failures and mark the
    /// journal unhealthy, which takes that guard's other branch, so a drain is
    /// the one stage whose failure has to be armed directly. Deliberately **not**
    /// `cfg(test)`-gated, so the injection check stays a single line whose
    /// production half is not an uncoverable `cfg(not(test))` branch (the shape
    /// `persistence.rs` uses for its injected failures).
    fail_flush: bool,
    #[cfg(test)]
    read_flush_queued: Option<std::sync::mpsc::Sender<()>>,
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum JournalFailPoint {
    Append,
    Flush,
    Sync,
}

pub struct RunAttachment {
    pub receiver: broadcast::Receiver<SseEvent>,
    pub events: Vec<SseEvent>,
    pub truncated: bool,
    pub projection: Option<RunProjectionSnapshot>,
    pub(crate) backfill: Option<RunBackfill>,
}

/// Captured under the same lock as receiver registration. The transport walks
/// this finite prefix in bounded pages before polling the registered receiver.
#[derive(Clone)]
pub(crate) struct RunBackfill {
    pub run_id: String,
    pub epoch: i64,
    pub after_idx: i64,
    pub until_idx: i64,
}

#[derive(Debug, Clone)]
pub struct RunProjectionSnapshot {
    pub run_id: String,
    pub epoch: i64,
    pub run_sequence: i64,
    pub cursor: i64,
    pub events: Vec<SseEvent>,
}

/// Per-session SSE broadcaster. Also the **single stamping point** (P1): it
/// assigns each event's `run_id` + monotonic `idx` and buffers the current run
/// for `events_since` — all under one lock, so broadcast order matches idx order.
#[derive(Clone)]
pub struct SseBroadcaster {
    tx: broadcast::Sender<SseEvent>,
    run: std::sync::Arc<parking_lot::Mutex<RunState>>,
    /// Number of times a consumer's cursor predates the replay ring (ring
    /// truncation / idx gap), forcing a resync via the projection snapshot.
    /// Observability metric for the "ring truncation must be explicitly
    /// visible" acceptance criterion; expected to stay 0 in healthy runs.
    truncation_count: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Number of times a live subscriber fell behind the broadcast channel
    /// (tokio `RecvError::Lagged`) and the gRPC stream was terminated for cursor
    /// resume. Observability metric for the "broadcast lag" criterion; a spike
    /// means a client couldn't keep up with the event rate.
    lag_count: std::sync::Arc<std::sync::atomic::AtomicU64>,
    journal: std::sync::Arc<parking_lot::Mutex<EventJournalState>>,
    journal_health: std::sync::Arc<EventJournalHealth>,
}

impl SseBroadcaster {
    pub fn new() -> Self {
        // Ring sized for high-reasoning model bursts — see BROADCAST_RING_CAPACITY.
        let (tx, _) = broadcast::channel(BROADCAST_RING_CAPACITY);
        Self {
            tx,
            run: std::sync::Arc::new(parking_lot::Mutex::new(RunState {
                run_id: String::new(),
                epoch: 0,
                idx: 0,
                run_sequence: -1,
                events: std::collections::VecDeque::new(),
                event_bytes: 0,
                projection_events: projection::Projection::default(),
                projection_complete: true,
                compacted: false,
            })),
            truncation_count: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            lag_count: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            journal: std::sync::Arc::new(parking_lot::Mutex::new(EventJournalState::default())),
            journal_health: std::sync::Arc::new(EventJournalHealth::default()),
        }
    }

    /// Subscribe to SSE events
    pub fn subscribe(&self) -> broadcast::Receiver<SseEvent> {
        self.tx.subscribe()
    }

    /// Whether a live consumer currently holds a receiver. Idle cleanup uses
    /// this as a fence: a subscribed client may still need the run projection.
    pub fn has_subscribers(&self) -> bool {
        self.tx.receiver_count() > 0
    }

    pub fn last_idx(&self) -> i64 {
        self.run.lock().idx.saturating_sub(1)
    }

    pub fn current_run_id(&self) -> String {
        self.run.lock().run_id.clone()
    }

    /// Count of ring-truncation resyncs: times a consumer's cursor fell behind
    /// the replay ring and had to recover via the projection snapshot.
    /// Observability metric; expected to stay 0 in healthy runs (a non-zero
    /// value means a client lagged far enough to lose the incremental tail).
    pub fn truncation_count(&self) -> u64 {
        self.truncation_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Record that a live subscriber lagged behind the broadcast channel (the
    /// gRPC layer calls this when it observes `RecvError::Lagged`).
    pub fn record_lag(&self) -> u64 {
        self.lag_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    }

    /// Count of live-subscriber lag events (see `record_lag`). Observability
    /// metric; expected to stay 0 unless a client can't keep up with the rate.
    pub fn lag_count(&self) -> u64 {
        self.lag_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Stamp `run_id` + `epoch` + monotonic `idx`, buffer the event, and
    /// broadcast — all under one lock so stream order matches idx order (no
    /// reordering race).
    pub fn broadcast(&self, event: SseEvent) {
        self.broadcast_with_ack(event, true);
    }

    /// The model producer must never wait synchronously for the writer. Its
    /// semantic boundaries explicitly await `flush_journal_async` instead.
    pub(crate) fn broadcast_deferred(&self, event: SseEvent) {
        self.broadcast_with_ack(event, false);
    }

    fn broadcast_with_ack(&self, event: SseEvent, wait: bool) {
        if self.uses_segmented_storage()
            && is_buffered_delta(&event.event_type)
            && event.data.len() > crate::session::content::transport::LIVE_TEXT_BYTES
        {
            if let Ok(mut data) = serde_json::from_str::<serde_json::Value>(&event.data) {
                if let Some(text) = data["text"]
                    .as_str()
                    .filter(|text| text.len() > crate::session::content::transport::LIVE_TEXT_BYTES)
                    .map(str::to_owned)
                {
                    let mut header = event;
                    header.data.clear();
                    let mut offset = 0;
                    while offset < text.len() {
                        let end = text.floor_char_boundary(
                            (offset + crate::session::content::transport::LIVE_TEXT_BYTES)
                                .min(text.len()),
                        );
                        data["text"] = serde_json::json!(&text[offset..end]);
                        // A replacement snapshot replaces only once; subsequent
                        // pieces extend that same new content generation.
                        if offset > 0 && data["snapshot"] == true {
                            data["snapshot"] = serde_json::json!(false);
                        }
                        let mut piece = header.clone();
                        piece.data = data.to_string();
                        self.broadcast_stamped(piece, false, wait);
                        if self.persistence_error().is_some() {
                            break;
                        }
                        offset = end;
                    }
                    return;
                }
            }
        }
        let session_scoped = is_session_scoped_event(&event.event_type);
        self.broadcast_stamped(event, session_scoped, wait);
    }

    /// Broadcast a signal about the **session** rather than about a run.
    ///
    /// Use it for what a session does between runs. `broadcast` stamps an event
    /// with the current run, and the broadcaster keeps the last settled run's
    /// identity until the next `start_run` — but that run's journal is already
    /// retired: run completion hands it to the journal compactor, which
    /// publishes a snapshot and deletes its raw rows. A run-stamped append there
    /// can never succeed (`cannot append to a compacted settled run`), and the
    /// refusal used to be recorded as a session-wide journal failure. The
    /// session scope is never compacted, is durable, and is the path clients
    /// already replay for the events that have no run (`model_changed`,
    /// `cwd_changed`, …).
    pub fn broadcast_session(&self, event: SseEvent) {
        self.broadcast_stamped(event, true, true);
    }

    fn broadcast_stamped(&self, mut event: SseEvent, session_scoped: bool, wait: bool) {
        // Parse/preview large tool facts before acquiring the stamping lock.
        // The writer receives the parsed immutable data, so this critical
        // section never serializes or clones a multi-megabyte body.
        let segmented = self.uses_segmented_storage();
        let bytes = event.data.len().saturating_add(2048);
        let mut source = if segmented {
            match serde_json::from_str::<serde_json::Value>(&event.data) {
                Ok(data) => Some(data),
                Err(error) => {
                    self.journal_health
                        .fail(format!("invalid event data: {error}"));
                    return;
                }
            }
        } else {
            None
        };
        let mut wire = source.as_ref().map(|data| {
            crate::session::content::transport::prepare_wire_data(data, &event.event_type)
        });
        let initial_tool_bytes = source
            .as_ref()
            .filter(|_| event.event_type == "tool_start")
            .map(|data| {
                data["tool_args"]
                    .as_str()
                    .map(str::len)
                    .unwrap_or_else(|| data["tool_args"].to_string().len())
            })
            .unwrap_or(0);
        if segmented {
            event.data.clear();
        }
        let mut run = self.run.lock();
        let mut journal = self.journal.lock();
        if journal.closed {
            return;
        }
        event.session_id = journal.session_id.clone();
        if session_scoped {
            event.run_id.clear();
            event.epoch = 0;
            event.idx = -1;
            event.session_idx = journal.session_idx;
            event.run_sequence = -1;
        } else {
            event.run_id = run.run_id.clone();
            event.epoch = run.epoch;
            event.idx = run.idx;
            event.session_idx = -1;
            event.run_sequence = run.run_sequence;
        }
        event.timestamp = chrono::Utc::now().to_rfc3339();
        event.event_id = if session_scoped {
            format!("{}:session:{}", event.session_id, event.session_idx)
        } else {
            format!(
                "{}:{}:{}:{}",
                event.session_id, event.run_id, event.epoch, event.idx
            )
        };
        let mut content_sequence = journal.content_sequence.clone();
        if journal.segmented && !session_scoped {
            if let Err(error) = content_sequence.stamp(
                &event,
                source.as_mut().expect("canonical data prepared"),
                initial_tool_bytes,
            ) {
                self.journal_health
                    .fail(format!("content sequencing failed: {error:#}"));
                return;
            }
        }
        if let Some(wire) = wire.as_mut() {
            let identity = journal
                .storage_identity
                .as_ref()
                .expect("configured canonical identity");
            crate::session::content::transport::stamp_wire_data(
                wire,
                source.as_ref().expect("prepared source"),
                identity,
                if session_scoped { -1 } else { event.epoch },
                if session_scoped {
                    event.session_idx
                } else {
                    event.idx
                },
            );
            event.data = wire.to_string();
            if event.data.len() > crate::session::content::transport::INLINE_EVENT_BYTES {
                self.journal_health
                    .fail("event metadata exceeds the bounded live frame budget".into());
                return;
            }
        }
        let receipt = match Self::append_journal(&mut journal, &event, source, bytes) {
            Ok(receipt) => receipt,
            Err(failure) => {
                match failure {
                    // The journal refuses this event for good, so retrying could
                    // never store it — and reporting it as a session-wide journal
                    // failure is what turned one misplaced event into a session no
                    // prompt could be sent to. Name it, drop it, and keep the rest
                    // of the session writable.
                    AppendFailure::Rejected(reason) => {
                        tracing::warn!(
                            session = %event.session_id,
                            run_id = %event.run_id,
                            epoch = event.epoch,
                            idx = event.idx,
                            event_id = %event.event_id,
                            event_type = %event.event_type,
                            %reason,
                            "dropping an event this journal refuses; the session's journal stays live"
                        );
                        return;
                    }
                    // A storage problem: fail closed and keep the journal blocked
                    // until it is recovered, as before.
                    AppendFailure::Unavailable(message) => {
                        let message = format!("event journal append failed: {message}");
                        self.journal_health.fail(message.clone());
                        tracing::error!(run_id = %event.run_id, idx = event.idx, "{message}");
                        return;
                    }
                }
            }
        };
        if session_scoped {
            journal.session_idx += 1;
        }
        journal.content_sequence = content_sequence;
        let bounded = journal.segmented || journal.memory_boundary.is_some();
        drop(journal);
        if !session_scoped {
            run.idx += 1;
            run.record_event(&event, bounded);
        }
        // tokio broadcast semantics: send() only fails when there are NO
        // active receivers — normal for ephemeral sessions before a client
        // subscribes, so the error is ignored.  When the ring buffer is
        // full, send() does NOT fail; it drops the oldest events and slow
        // receivers observe RecvError::Lagged, then resync via
        // `events_since` (which reports the gap via min_idx).
        let _ = self.tx.send(event);
        // Publication remains ordered, while the durable acknowledgment may
        // wait without holding either lock. User input and terminal transcript
        // markers are already committed by their owning async operation.
        drop(run);
        if let Some(receipt) = receipt.filter(|_| wait) {
            match receipt.recv() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => self
                    .journal_health
                    .fail(format!("event commit failed: {error}")),
                Err(_) => self
                    .journal_health
                    .fail("event commit acknowledgment lost".into()),
            }
        }
    }

    /// Begin a new user run: set `run_id` + `epoch`, reset `idx`, clear the
    /// buffer. `epoch` is the run's monotonic generation within the session
    /// (from the runtime lease), stamped on every event of this run.
    pub fn start_run(&self, run_id: String, epoch: i64) {
        self.start_run_with_sequence(run_id, epoch, None);
    }

    pub fn start_run_with_sequence(&self, run_id: String, epoch: i64, run_sequence: Option<u64>) {
        let canonical = {
            let journal = self.journal.lock();
            journal
                .segmented
                .then(|| (journal.store.clone(), journal.session_id.clone()))
        };
        let recovered_page = if let Some((Some(store), session)) = canonical {
            self.flush_journal()
                .and_then(|()| store.segmented_run_tail(&session, &run_id, epoch))
                .map(|events| (true, events, None))
        } else {
            self.read_journal_page(&run_id, -1, false, None)
        };
        let (recovered, snapshot, recovery_failed) = match recovered_page {
            Ok((_, events, snapshot)) => (events, snapshot, false),
            Err(error) => {
                self.journal_health
                    .fail(format!("event journal recovery failed: {error:#}"));
                (Vec::new(), None, true)
            }
        };
        let mut run = self.run.lock();
        run.run_id = run_id;
        run.epoch = epoch;
        run.run_sequence = run_sequence
            .and_then(|value| i64::try_from(value).ok())
            .unwrap_or(-1);
        run.compacted = snapshot.is_some();
        if let Some(snapshot) = snapshot {
            run.epoch = snapshot.epoch;
            run.run_sequence = snapshot.run_sequence;
            run.idx = snapshot.cursor.saturating_add(1);
            run.events.clear();
            run.event_bytes = 0;
            run.projection_events = projection::Projection::from_events(snapshot.events);
            run.projection_complete = true;
            self.journal_health.clear();
            self.journal_health.clear_interrupt();
            return;
        }
        run.idx = recovered
            .last()
            .map_or(0, |event| event.idx.saturating_add(1));
        run.events = recovered
            .iter()
            .skip(recovered.len().saturating_sub(MAX_RUN_EVENTS))
            .cloned()
            .collect();
        run.event_bytes = run.events.iter().map(|event| event.data.len() + 512).sum();
        run.projection_events.clear();
        if self.uses_segmented_storage() {
            run.projection_events.limit(8 * 1024 * 1024);
        }
        run.projection_complete =
            !recovery_failed && replay::covers_prefix(&recovered, &run.run_id);
        for event in &recovered {
            run.projection_events.push(event);
        }
        if !recovery_failed {
            self.journal_health.clear();
        }
        self.journal_health.clear_interrupt();
    }
}

impl Default for SseBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "tests/read_lock.rs"]
mod read_lock_tests;
#[cfg(test)]
mod tests;

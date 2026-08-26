use serde::{Deserialize, Serialize};
use std::io::Write;
use tokio::sync::broadcast;

// ─── RPC Command (stdin) ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcCommand {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type")]
    pub cmd_type: String,

    // Prompting
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub model_context: String,
    #[serde(default)]
    pub images: Vec<crate::types::ImageContent>,
    #[serde(default)]
    pub attachments: Vec<crate::types::Attachment>,
    #[serde(default)]
    pub parent_session: String,

    // set_model
    #[serde(default)]
    pub model_id: String,

    // set_thinking_level
    #[serde(default)]
    pub level: String,

    // Generic decision/rule mode (approval_result, add_session_rule).
    #[serde(default)]
    pub mode: String,

    // compact
    #[serde(default)]
    pub custom_instructions: String,

    // new_session — typed provenance (see proto). Legacy clients smuggle the
    // same info as JSON in custom_instructions.
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub source_meta: String,

    // set_auto_compaction / set_auto_retry
    #[serde(default)]
    pub enabled: bool,

    // shell
    #[serde(default)]
    pub command: String,

    // Session
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub entry_id: String,
    /// Optional forward cursor for get_session_entries. Absent preserves the
    /// released all-at-once response; remote clients already send offset=0 and
    /// therefore opt into bounded pages.
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cwd: String,

    // set_system_prompt
    #[serde(default)]
    pub system_prompt: String,

    // set_tools / disable_tools
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    // set_ephemeral
    pub ephemeral: bool,

    // set_enabled_models
    #[serde(default)]
    pub enabled_models: Option<Vec<String>>,

    // get_events_since (P1)
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub since_idx: i64,
    /// Page size for get_events_since; 0 = unlimited (legacy behavior).
    /// See proto RpcCommand.max_events.
    #[serde(default)]
    pub max_events: i64,
    #[serde(default)]
    pub requested_run_id: String,
    #[serde(default)]
    pub client_request_id: String,
    #[serde(default)]
    pub busy_policy: String,

    // list_models: also carry a summary of the built-in provider catalog
    // (`builtinProviders`) in the response, so clients can render the full
    // catalog at runtime instead of compiling it in.
    #[serde(default)]
    pub include_builtin_providers: bool,

    // set_sandbox_policy — populated from the typed proto sub-message by the
    // gRPC layer (not part of the JSON command surface).
    #[serde(skip)]
    pub sandbox_policy: Option<crate::sandbox::SandboxPolicy>,

    // set_auth / upsert_provider / delete_provider — typed config writes
    // (audit item 2). Populated from the proto sub-messages by the gRPC
    // layer; the agent applies them to its own auth.json/models.json and
    // refreshes live sessions, replacing out-of-band file edits + reload_auth.
    #[serde(skip)]
    pub auth_update: Option<crate::config::providers::AuthMutation>,
    #[serde(skip)]
    pub provider_config: Option<crate::config::providers::ProviderUpsertSpec>,
}

// ─── RPC Response (stdout) ───────────────────────────────────────────────

// ─── RPC Response (stdout) ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct RpcResponse {
    #[serde(rename = "type")]
    pub resp_type: String,
    #[serde(default)]
    pub id: String,
    pub command: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_data: Option<serde_json::Value>,
}

impl RpcResponse {
    pub(super) fn ok(id: &str, command: &str, data: impl Into<serde_json::Value>) -> String {
        let resp = Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: true,
            data: Some(data.into()),
            error: None,
            error_code: None,
            error_data: None,
        };
        serde_json::to_string(&resp).unwrap_or_default()
    }

    pub fn build_fail(id: &str, command: &str, err: &str) -> String {
        let resp = Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: false,
            data: None,
            error: Some(err.to_string()),
            error_code: None,
            error_data: None,
        };
        serde_json::to_string(&resp).unwrap_or_default()
    }

    pub fn build_fail_code(
        id: &str,
        command: &str,
        code: &str,
        err: &str,
        details: impl Into<serde_json::Value>,
    ) -> String {
        let resp = Self {
            resp_type: "response".to_string(),
            id: id.to_string(),
            command: command.to_string(),
            success: false,
            data: None,
            error: Some(err.to_string()),
            error_code: Some(code.to_string()),
            error_data: Some(details.into()),
        };
        serde_json::to_string(&resp).unwrap_or_default()
    }
}

// ─── SSE Event Broadcaster ──────────────────────────────────────────────

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
    events: Vec<SseEvent>,
    projection_events: Vec<SseEvent>,
}

#[derive(Default)]
struct EventJournalState {
    session_id: String,
    session_idx: i64,
    directory: Option<std::path::PathBuf>,
    closed: bool,
    last_error: Option<String>,
    interrupt_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    #[cfg(test)]
    fail_at: Option<JournalFailPoint>,
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
                events: Vec::new(),
                projection_events: Vec::new(),
            })),
            truncation_count: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            lag_count: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            journal: std::sync::Arc::new(parking_lot::Mutex::new(EventJournalState::default())),
        }
    }

    /// Bind this session's broadcaster to its Agent-owned event directory.
    /// Tests and short-lived utility broadcasters may intentionally remain
    /// memory-only by never calling this method.
    pub fn configure_journal(
        &self,
        session_id: impl Into<String>,
        directory: std::path::PathBuf,
    ) -> anyhow::Result<()> {
        let session_id = session_id.into();
        if let Err(error) = std::fs::create_dir_all(&directory) {
            let mut journal = self.journal.lock();
            journal.session_id = session_id;
            journal.directory = Some(directory);
            journal.last_error = Some(format!("event journal directory unavailable: {error}"));
            return Err(error.into());
        }
        let mut journal = self.journal.lock();
        journal.session_id = session_id;
        // Session-scoped events have their own durable sequence. Resume after
        // an Agent restart so their event ids cannot collide with prior
        // model/name/cwd events for this session.
        journal.session_idx = std::fs::read_to_string(directory.join("_session.jsonl"))
            .ok()
            .map(|contents| {
                contents
                    .lines()
                    .filter_map(|line| serde_json::from_str::<SseEvent>(line).ok())
                    .filter_map(|event| (event.session_idx >= 0).then_some(event.session_idx))
                    .max()
                    .map_or(0, |idx| idx.saturating_add(1))
            })
            .unwrap_or(0);
        journal.directory = Some(directory);
        journal.closed = false;
        journal.last_error = None;
        Ok(())
    }

    pub fn set_persistence_interrupt(
        &self,
        interrupt_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        let mut journal = self.journal.lock();
        if journal.last_error.is_some() {
            interrupt_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        journal.interrupt_flag = Some(interrupt_flag);
    }

    pub fn persistence_error(&self) -> Option<String> {
        self.journal.lock().last_error.clone()
    }

    /// Fence the event journal before session deletion. The journal mutex is
    /// also held by append for the complete file operation, so when this
    /// returns no append can still own a path or recreate the deleted tree.
    pub fn close_journal(&self) {
        let mut journal = self.journal.lock();
        journal.closed = true;
        journal.directory = None;
        journal.interrupt_flag = None;
    }

    pub fn recover_storage(&self) -> anyhow::Result<()> {
        let directory = self
            .journal
            .lock()
            .directory
            .clone()
            .ok_or_else(|| anyhow::anyhow!("event journal is not configured"))?;
        std::fs::create_dir_all(&directory)?;
        let probe = directory.join(".health-probe.tmp");
        {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&probe)?;
            file.write_all(b"ok")?;
            file.sync_data()?;
        }
        std::fs::remove_file(probe)?;
        self.journal.lock().last_error = None;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fail_next_append(&self) {
        self.journal.lock().fail_at = Some(JournalFailPoint::Append);
    }

    #[cfg(test)]
    fn fail_at(&self, point: JournalFailPoint) {
        self.journal.lock().fail_at = Some(point);
    }

    /// Subscribe to SSE events
    pub fn subscribe(&self) -> broadcast::Receiver<SseEvent> {
        self.tx.subscribe()
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

    /// Atomically register a receiver and snapshot the requested run tail.
    /// `broadcast` uses the same run lock, so no event can land in the window
    /// between the snapshot and subscription.
    pub fn attach(&self, run_id: &str, after_idx: i64) -> anyhow::Result<RunAttachment> {
        let run = self.run.lock();
        if run.run_id != run_id {
            anyhow::bail!("run `{run_id}` is not the active run");
        }
        let receiver = self.tx.subscribe();
        let min_idx = run.events.first().map(|event| event.idx).unwrap_or(run.idx);
        let truncated = after_idx.saturating_add(1) < min_idx;
        if truncated {
            let count = self
                .truncation_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            if self.journal.lock().directory.is_some() {
                // With a durable journal the client still gets the full history
                // (disk replay), so this is a benign resync, not a data gap.
                tracing::debug!(
                    run_id,
                    requested_after_idx = after_idx,
                    min_available_idx = min_idx,
                    truncation_count = count,
                    "run replay ring truncated; falling back to disk journal replay"
                );
            } else {
                tracing::warn!(
                    run_id,
                    requested_after_idx = after_idx,
                    min_available_idx = min_idx,
                    truncation_count = count,
                    "run replay ring truncated; returning projection snapshot"
                );
            }
        }
        let disk_events = if truncated && self.journal.lock().directory.is_some() {
            Some(
                self.read_journal(run_id)?
                    .into_iter()
                    .filter(|event| event.idx > after_idx)
                    .collect(),
            )
        } else {
            None
        };
        let events = disk_events.unwrap_or_else(|| {
            run.events
                .iter()
                .filter(|event| !truncated && event.idx > after_idx)
                .cloned()
                .collect()
        });
        let projection =
            (truncated && self.journal.lock().directory.is_none()).then(|| RunProjectionSnapshot {
                run_id: run.run_id.clone(),
                epoch: run.epoch,
                run_sequence: run.run_sequence,
                cursor: run.idx.saturating_sub(1),
                events: run.projection_events.clone(),
            });
        Ok(RunAttachment {
            receiver,
            events,
            truncated,
            projection,
        })
    }

    /// Stamp `run_id` + `epoch` + monotonic `idx`, buffer the event, and
    /// broadcast — all under one lock so stream order matches idx order (no
    /// reordering race).
    pub fn broadcast(&self, mut event: SseEvent) {
        let mut run = self.run.lock();
        let session_scoped = is_session_scoped_event(&event.event_type);
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
            journal.session_idx += 1;
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
        if let Err(error) = Self::append_journal(&mut journal, &event) {
            let message = format!("event journal append failed: {error:#}");
            journal.last_error = Some(message.clone());
            if let Some(flag) = &journal.interrupt_flag {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            tracing::error!(run_id = %event.run_id, idx = event.idx, "{message}");
            return;
        }
        drop(journal);
        if !session_scoped {
            run.idx += 1;
            apply_to_projection(&mut run.projection_events, &event);
            run.events.push(event.clone());
            if run.events.len() > MAX_RUN_EVENTS {
                let overflow = run.events.len() - MAX_RUN_EVENTS;
                run.events.drain(0..overflow);
            }
        }
        // tokio broadcast semantics: send() only fails when there are NO
        // active receivers — normal for ephemeral sessions before a client
        // subscribes, so the error is ignored.  When the ring buffer is
        // full, send() does NOT fail; it drops the oldest events and slow
        // receivers observe RecvError::Lagged, then resync via
        // `events_since` (which reports the gap via min_idx).
        let _ = self.tx.send(event);
    }

    /// Begin a new user run: set `run_id` + `epoch`, reset `idx`, clear the
    /// buffer. `epoch` is the run's monotonic generation within the session
    /// (from the runtime lease), stamped on every event of this run.
    pub fn start_run(&self, run_id: String, epoch: i64) {
        self.start_run_with_sequence(run_id, epoch, None);
    }

    pub fn start_run_with_sequence(&self, run_id: String, epoch: i64, run_sequence: Option<u64>) {
        let (recovered, recovery_failed) = match self.read_journal(&run_id) {
            Ok(events) => (events, false),
            Err(error) => {
                self.journal.lock().last_error =
                    Some(format!("event journal recovery failed: {error:#}"));
                (Vec::new(), true)
            }
        };
        let mut run = self.run.lock();
        run.run_id = run_id;
        run.epoch = epoch;
        run.run_sequence = run_sequence
            .and_then(|value| i64::try_from(value).ok())
            .unwrap_or(-1);
        run.idx = recovered
            .last()
            .map_or(0, |event| event.idx.saturating_add(1));
        run.events = recovered
            .iter()
            .rev()
            .take(MAX_RUN_EVENTS)
            .cloned()
            .collect::<Vec<_>>();
        run.events.reverse();
        run.projection_events.clear();
        for event in &recovered {
            apply_to_projection(&mut run.projection_events, event);
        }
        let mut journal = self.journal.lock();
        if !recovery_failed {
            journal.last_error = None;
        }
        journal.interrupt_flag = None;
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
        let run = self.run.lock();
        if run.run_id != run_id {
            // A completed run is no longer in the live ring, but its durable
            // journal remains the canonical history.  GUI/TUI inspectors and
            // reconnect backfill must not lose that history merely because a
            // later run became active.
            let path = self
                .journal_path(run_id)
                .ok_or_else(|| anyhow::anyhow!("event journal is not configured"))?;
            if !path.exists() {
                anyhow::bail!("run `{run_id}` is not known by this session");
            }
            let events = self
                .read_journal(run_id)?
                .into_iter()
                .filter(|event| event.idx > since_idx)
                .collect::<Vec<_>>();
            let min_idx = events.first().map(|event| event.idx).unwrap_or(0);
            return Ok((run_id.to_string(), events, min_idx, None));
        }
        let min_idx = run.events.first().map(|e| e.idx).unwrap_or(0);
        let truncated = since_idx.saturating_add(1) < min_idx;
        if truncated {
            let count = self
                .truncation_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1;
            if self.journal.lock().directory.is_some() {
                // With a durable journal the client still gets the full history
                // (disk replay), so this is a benign resync, not a data gap.
                tracing::debug!(
                    run_id,
                    requested_after_idx = since_idx,
                    min_available_idx = min_idx,
                    truncation_count = count,
                    "run event query crossed replay-ring boundary; falling back to disk journal replay"
                );
            } else {
                tracing::warn!(
                    run_id,
                    requested_after_idx = since_idx,
                    min_available_idx = min_idx,
                    truncation_count = count,
                    "run event query crossed replay-ring boundary; returning projection snapshot"
                );
            }
        }
        let disk_events = if truncated && self.journal.lock().directory.is_some() {
            Some(
                self.read_journal(run_id)?
                    .into_iter()
                    .filter(|event| event.idx > since_idx)
                    .collect(),
            )
        } else {
            None
        };
        let events = disk_events.unwrap_or_else(|| {
            run.events
                .iter()
                .filter(|event| !truncated && event.idx > since_idx)
                .cloned()
                .collect()
        });
        let projection =
            (truncated && self.journal.lock().directory.is_none()).then(|| RunProjectionSnapshot {
                run_id: run.run_id.clone(),
                epoch: run.epoch,
                run_sequence: run.run_sequence,
                cursor: run.idx.saturating_sub(1),
                events: run.projection_events.clone(),
            });
        Ok((run.run_id.clone(), events, min_idx, projection))
    }

    pub fn session_events_since(&self, since_idx: i64) -> anyhow::Result<Vec<SseEvent>> {
        self.read_journal("").map(|events| {
            events
                .into_iter()
                .filter(|event| event.session_idx > since_idx)
                .collect()
        })
    }

    fn journal_path(&self, run_id: &str) -> Option<std::path::PathBuf> {
        self.journal.lock().directory.as_ref().map(|directory| {
            if run_id.is_empty() {
                directory.join("_session.jsonl")
            } else {
                directory.join(format!("{run_id}.jsonl"))
            }
        })
    }

    fn append_journal(journal: &mut EventJournalState, event: &SseEvent) -> anyhow::Result<()> {
        #[cfg(test)]
        {
            if matches!(journal.fail_at, Some(JournalFailPoint::Append)) {
                journal.fail_at = None;
                anyhow::bail!("injected append failure");
            }
        }
        // The only caller (broadcast) holds the journal lock across its own
        // closed check, so a closed journal can never reach this point.
        debug_assert!(!journal.closed);
        let Some(directory) = journal.directory.as_ref() else {
            return Ok(());
        };
        let path = if event.run_id.is_empty() {
            directory.join("_session.jsonl")
        } else {
            directory.join(format!("{}.jsonl", event.run_id))
        };
        let mut bytes = serde_json::to_vec(event)?;
        bytes.push(b'\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        file.write_all(&bytes)?;
        #[cfg(test)]
        {
            if matches!(journal.fail_at, Some(JournalFailPoint::Flush)) {
                journal.fail_at = None;
                anyhow::bail!("injected flush failure");
            }
        }
        file.flush()?;
        #[cfg(test)]
        {
            if matches!(journal.fail_at, Some(JournalFailPoint::Sync)) {
                journal.fail_at = None;
                anyhow::bail!("injected sync failure");
            }
        }
        file.sync_data()?;
        Ok(())
    }

    /// Read only complete JSONL records. A process crash can leave one partial
    /// tail record; it is ignored and truncated before the next append.
    fn read_journal(&self, run_id: &str) -> anyhow::Result<Vec<SseEvent>> {
        let Some(path) = self.journal_path(run_id) else {
            return Ok(Vec::new());
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut events = Vec::new();
        let mut valid_bytes = 0_u64;
        let ends_with_newline = bytes.ends_with(b"\n");
        let parts = bytes.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        for (index, line) in parts.iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            match serde_json::from_slice::<SseEvent>(line) {
                Ok(event) => {
                    valid_bytes += line.len() as u64 + 1;
                    events.push(event);
                }
                Err(_error) if index + 1 == parts.len() && !ends_with_newline => {
                    let writable = std::fs::OpenOptions::new().write(true).open(&path)?;
                    writable.set_len(valid_bytes)?;
                    break;
                }
                Err(error) => anyhow::bail!(
                    "event journal corruption at byte {valid_bytes} in {}: {error}",
                    path.display()
                ),
            }
        }
        Ok(events)
    }
}

/// Fold a run event into the durable-in-memory semantic projection.
///
/// The replay ring is intentionally bounded, while the projection must retain
/// enough information to rebuild the visible run after that ring truncates.
/// High-frequency deltas are coalesced into their preceding semantic segment;
/// lifecycle, tool terminal, approval, usage, error, and terminal events keep
/// their original ordering and cursor.
fn apply_to_projection(projection: &mut Vec<SseEvent>, event: &SseEvent) {
    // `text_delta` (raw provider-stream token) duplicates `text_chunk` (the
    // on_text-derived token); consumers project the latter, so retaining both
    // would duplicate assistant output.
    if event.event_type == "text_delta" {
        return;
    }

    let coalescible = matches!(
        event.event_type.as_str(),
        "text_chunk" | "thinking_delta" | "toolcall_delta" | "tool_delta"
    );
    if coalescible {
        if let Some(previous) = projection
            .last_mut()
            .filter(|previous| previous.event_type == event.event_type)
        {
            if let (Ok(mut previous_data), Ok(next_data)) = (
                serde_json::from_str::<serde_json::Value>(&previous.data),
                serde_json::from_str::<serde_json::Value>(&event.data),
            ) {
                let same_tool_stream =
                    !matches!(event.event_type.as_str(), "toolcall_delta" | "tool_delta")
                        || ["tool_id", "tc_index"]
                            .iter()
                            .all(|key| previous_data.get(key) == next_data.get(key));
                if let (Some(previous_text), Some(next_text)) = (
                    previous_data.get("text").and_then(|value| value.as_str()),
                    next_data.get("text").and_then(|value| value.as_str()),
                ) {
                    if !same_tool_stream {
                        projection.push(event.clone());
                        return;
                    }
                    let combined = format!("{previous_text}{next_text}");
                    previous_data["text"] = serde_json::Value::String(combined);
                    previous.data = serde_json::to_string(&previous_data).unwrap_or_default();
                    // The folded segment represents every source event through
                    // this cursor, so live resume starts strictly after it.
                    previous.idx = event.idx;
                    return;
                }
            }
        }
    }

    projection.push(event.clone());
}

impl Default for SseBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

/// SSE Event structure
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SseEvent {
    pub event_type: String,
    pub data: String,
    /// P1: stamped by `SseBroadcaster::broadcast` (callers leave default).
    /// `run_id` + `epoch` + `idx` are the run-scoped identity; `session_id` is
    /// added at the gRPC wire boundary (the stream is session-scoped).
    pub run_id: String,
    pub epoch: i64,
    pub idx: i64,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub event_id: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default = "default_session_idx")]
    pub session_idx: i64,
    #[serde(default = "default_session_idx")]
    pub run_sequence: i64,
}

impl SseEvent {
    pub fn new(event_type: &str, data: serde_json::Value) -> Self {
        Self {
            event_type: event_type.to_string(),
            data: serde_json::to_string(&data).unwrap_or_default(),
            run_id: String::new(),
            epoch: 0,
            idx: 0,
            session_id: String::new(),
            event_id: String::new(),
            timestamp: String::new(),
            session_idx: -1,
            run_sequence: -1,
        }
    }
}

fn default_session_idx() -> i64 {
    -1
}

fn is_session_scoped_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "model_changed"
            | "thinking_level_changed"
            | "session_name_changed"
            | "cwd_changed"
            | "permission_level_changed"
            | "sandbox_policy_changed"
            | "auto_compaction_changed"
            | "tools_changed"
            | "config_reloaded"
            | "skills_reloaded"
            | "provider_config_changed"
    )
}

// ─── Approval Gate ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── RpcCommand deserialization ──────────────────────────────────────────

    #[test]
    fn rpc_command_minimal() {
        let json = r#"{"id":"cmd1","type":"get_state","sessionId":"s1"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.id, "cmd1");
        assert_eq!(cmd.cmd_type, "get_state");
        assert_eq!(cmd.session_id, "s1");
        assert!(cmd.message.is_empty());
    }

    #[test]
    fn rpc_command_prompt() {
        let json = r#"{
            "id": "cmd2",
            "type": "prompt",
            "sessionId": "s1",
            "message": "hello",
            "modelContext": "reference summary"
        }"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.cmd_type, "prompt");
        assert_eq!(cmd.message, "hello");
        assert_eq!(cmd.model_context, "reference summary");
        assert!(cmd.busy_policy.is_empty());
    }

    #[test]
    fn rpc_command_prompt_busy_policy_uses_camel_case_wire_name() {
        let json = r#"{
            "id": "cmd2b",
            "type": "prompt",
            "sessionId": "s1",
            "message": "hello",
            "busyPolicy": "enqueue_if_busy"
        }"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.busy_policy, "enqueue_if_busy");
    }

    #[test]
    fn rpc_command_set_model() {
        let json = r#"{"id":"cmd3","type":"set_model","sessionId":"s1","modelId":"openai/gpt-4o"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.model_id, "openai/gpt-4o");
    }

    #[test]
    fn rpc_command_thinking_level() {
        let json = r#"{"id":"cmd4","type":"set_thinking_level","sessionId":"s1","level":"high"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.level, "high");
    }

    #[test]
    fn rpc_command_mode_field() {
        let json = r#"{"id":"cmd5","type":"approval_result","sessionId":"s1","mode":"approved"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.mode, "approved");
    }

    #[test]
    fn rpc_command_shell() {
        let json = r#"{"id":"cmd6","type":"shell","sessionId":"s1","command":"ls -la"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.command, "ls -la");
    }

    #[test]
    fn rpc_command_cwd() {
        let json = r#"{"id":"cmd7","type":"set_cwd","sessionId":"s1","cwd":"/tmp/project"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.cwd, "/tmp/project");
    }

    #[test]
    fn rpc_command_enabled_flag() {
        let json = r#"{"id":"cmd8","type":"set_auto_compaction","sessionId":"s1","enabled":true}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert!(cmd.enabled);
    }

    #[test]
    fn rpc_command_disabled_flag() {
        let json =
            r#"{"id":"cmd8b","type":"set_auto_compaction","sessionId":"s1","enabled":false}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert!(!cmd.enabled);
    }

    #[test]
    fn rpc_command_new_session_defaults() {
        let json = r#"{"id":"cmd9","type":"new_session"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert!(cmd.session_id.is_empty());
        assert!(cmd.cwd.is_empty());
        assert!(cmd.model_id.is_empty());
        assert!(cmd.custom_instructions.is_empty());
    }

    #[test]
    fn rpc_command_system_prompt() {
        let json = r#"{"id":"cmd10","type":"set_system_prompt","sessionId":"s1","systemPrompt":"You are helpful"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.system_prompt, "You are helpful");
    }

    #[test]
    fn rpc_command_tools_list() {
        let json = r#"{"id":"cmd11","type":"set_tools","sessionId":"s1","tools":["shell","read","write"]}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.tools, vec!["shell", "read", "write"]);
    }

    #[test]
    fn rpc_command_entry_id() {
        let json = r#"{"id":"cmd12","type":"fork","sessionId":"s1","entryId":"entry_abc"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.entry_id, "entry_abc");
    }

    #[test]
    fn rpc_command_name() {
        let json =
            r#"{"id":"cmd13","type":"set_session_name","sessionId":"s1","name":"My Session"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.name, "My Session");
    }

    #[test]
    fn rpc_command_ephemeral() {
        let json = r#"{"id":"cmd14","type":"set_ephemeral","sessionId":"s1","ephemeral":true}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert!(cmd.ephemeral);
    }

    #[test]
    fn rpc_command_events_since() {
        let json = r#"{"id":"cmd15","type":"get_events_since","sessionId":"s1","runId":"run_1","sinceIdx":5}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.run_id, "run_1");
        assert_eq!(cmd.since_idx, 5);
    }

    #[test]
    fn rpc_command_parent_session() {
        let json = r#"{"id":"cmd16","type":"fork","sessionId":"s1","parentSession":"parent_1","entryId":"e1"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.parent_session, "parent_1");
    }

    #[test]
    fn rpc_command_approval_mode() {
        let json = r#"{"id":"cmd17","type":"approval_decision","sessionId":"s1","entryId":"req_1","mode":"approved","message":"looks safe"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.mode, "approved");
        assert_eq!(cmd.entry_id, "req_1");
        assert_eq!(cmd.message, "looks safe");
    }

    #[test]
    fn rpc_command_sandbox_policy_skipped() {
        // sandbox_policy is #[serde(skip)] — should not appear in JSON
        let json = r#"{"id":"cmd18","type":"set_sandbox_policy","sessionId":"s1"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert!(cmd.sandbox_policy.is_none());
    }

    #[test]
    fn rpc_command_compact_with_instructions() {
        let json = r#"{"id":"cmd19","type":"compact","sessionId":"s1","customInstructions":"summarize in detail"}"#;
        let cmd: RpcCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.custom_instructions, "summarize in detail");
    }

    // ─── RpcResponse serialization ───────────────────────────────────────────

    #[test]
    fn rpc_response_ok_format() {
        let json_str = RpcResponse::ok("id1", "get_state", serde_json::json!({"model": "gpt-4o"}));
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["type"], "response");
        assert_eq!(parsed["id"], "id1");
        assert_eq!(parsed["command"], "get_state");
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["data"]["model"], "gpt-4o");
        assert!(parsed.get("error").is_none());
    }

    #[test]
    fn rpc_response_fail_format() {
        let json_str = RpcResponse::build_fail("id2", "prompt", "session not found");
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["type"], "response");
        assert_eq!(parsed["id"], "id2");
        assert_eq!(parsed["command"], "prompt");
        assert_eq!(parsed["success"], false);
        assert_eq!(parsed["error"], "session not found");
        assert!(parsed.get("data").is_none());
        assert!(parsed.get("error_code").is_none());
        assert!(parsed.get("error_data").is_none());
    }

    #[test]
    fn rpc_response_structured_failure_keeps_human_message() {
        let json_str = RpcResponse::build_fail_code(
            "id2b",
            "prompt",
            "busy",
            "session already has an active run",
            serde_json::json!({"active_run_id": "run-a"}),
        );
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["success"], false);
        assert_eq!(parsed["error"], "session already has an active run");
        assert_eq!(parsed["error_code"], "busy");
        assert_eq!(parsed["error_data"]["active_run_id"], "run-a");
    }

    #[test]
    fn rpc_response_ok_null_data() {
        let json_str = RpcResponse::ok("id3", "abort", serde_json::json!({}));
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["success"], true);
        assert!(parsed["data"].is_object());
    }

    #[test]
    fn rpc_response_ok_with_complex_data() {
        let data = serde_json::json!({
            "sessions": [{"id": "s1", "name": "test"}],
            "count": 1,
            "nested": {"deep": [1, 2, 3]}
        });
        let json_str = RpcResponse::ok("id4", "list_sessions", data.clone());
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["data"]["count"], 1);
        assert_eq!(
            parsed["data"]["nested"]["deep"],
            serde_json::json!([1, 2, 3])
        );
    }

    // ─── SseEvent ────────────────────────────────────────────────────────────

    #[test]
    fn sse_event_new_sets_type_and_data() {
        let event = SseEvent::new("text_chunk", serde_json::json!({"text": "hello"}));
        assert_eq!(event.event_type, "text_chunk");
        let parsed: serde_json::Value = serde_json::from_str(&event.data).unwrap();
        assert_eq!(parsed["text"], "hello");
        assert!(event.run_id.is_empty());
        assert_eq!(event.idx, 0);
    }

    #[test]
    fn sse_event_default() {
        let event = SseEvent::default();
        assert!(event.event_type.is_empty());
        assert!(event.data.is_empty());
    }

    // ─── SseBroadcaster (P1) ────────────────────────────────────────────────

    #[test]
    fn stamps_run_id_idx_and_backfills() {
        let b = SseBroadcaster::new();
        b.start_run("run1".to_string(), 1);
        b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "a"}),
        ));
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "b"}),
        ));

        // Backfill from idx 0 → the two events after idx 0 (idx 1, 2), in order.
        let (rid, evs, min_idx, projection) = b.events_since("run1", 0).unwrap();
        assert_eq!(rid, "run1");
        assert_eq!(evs.len(), 2);
        assert_eq!((evs[0].idx, evs[1].idx), (1, 2));
        assert_eq!(evs[0].run_id, "run1");
        // Nothing dropped yet → earliest buffered idx is still 0 (no gap).
        assert_eq!(min_idx, 0);
        assert!(projection.is_none());

        // From -1 → all three (idx 0,1,2).
        let (_, all, _, _) = b.events_since("run1", -1).unwrap();
        assert_eq!(all.iter().map(|e| e.idx).collect::<Vec<_>>(), vec![0, 1, 2]);

        // New run resets idx + clears buffer.
        b.start_run("run2".to_string(), 1);
        b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        let (rid2, evs2, _, _) = b.events_since("run2", -1).unwrap();
        assert_eq!(rid2, "run2");
        assert_eq!(evs2.len(), 1);
        assert_eq!((evs2[0].idx, evs2[0].run_id.as_str()), (0, "run2"));

        assert!(b.events_since("run1", 100).is_err());
    }

    #[test]
    fn session_scoped_events_have_independent_durable_identity() {
        let directory = tempfile::tempdir().unwrap();
        let b = SseBroadcaster::new();
        b.configure_journal("session-1", directory.path().to_path_buf())
            .unwrap();
        b.start_run("run-1".to_string(), 1);
        b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        b.broadcast(SseEvent::new(
            "model_changed",
            serde_json::json!({"model":"x"}),
        ));
        b.broadcast(SseEvent::new(
            "cwd_changed",
            serde_json::json!({"cwd":"/tmp"}),
        ));
        let contents = std::fs::read_to_string(directory.path().join("_session.jsonl")).unwrap();
        let events: Vec<SseEvent> = contents
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].run_id, "");
        assert_eq!(events[0].idx, -1);
        assert_eq!(events[0].session_idx, 0);
        assert_eq!(events[1].session_idx, 1);
        assert_eq!(events[1].event_id, "session-1:session:1");
    }

    #[test]
    fn attach_has_no_snapshot_subscribe_window() {
        let b = SseBroadcaster::new();
        b.start_run("run1".to_string(), 1);
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "a"}),
        ));
        let mut attachment = b.attach("run1", -1).unwrap();
        assert_eq!(attachment.events.len(), 1);

        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "b"}),
        ));
        let live = attachment.receiver.try_recv().unwrap();
        assert_eq!(live.idx, 1);
        assert!(!attachment.truncated);
    }

    #[test]
    fn attach_reports_truncated_ring_and_rejects_other_run() {
        let b = SseBroadcaster::new();
        b.start_run("run1".to_string(), 1);
        for idx in 0..=MAX_RUN_EVENTS {
            b.broadcast(SseEvent::new(
                "text_chunk",
                serde_json::json!({"text": idx.to_string()}),
            ));
        }
        let mut attachment = b.attach("run1", -1).unwrap();
        assert!(attachment.truncated);
        assert!(attachment.events.is_empty());
        let snapshot = attachment.projection.take().unwrap();
        assert_eq!(snapshot.run_id, "run1");
        assert_eq!(snapshot.cursor, MAX_RUN_EVENTS as i64);
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].idx, MAX_RUN_EVENTS as i64);
        let projected_data: serde_json::Value =
            serde_json::from_str(&snapshot.events[0].data).unwrap();
        assert!(projected_data["text"]
            .as_str()
            .is_some_and(|text| text.starts_with('0') && text.ends_with("2000")));

        // Receiver registration and snapshot capture share the run lock: the
        // first live event starts exactly after the snapshot cursor.
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "live"}),
        ));
        let live = attachment.receiver.try_recv().unwrap();
        assert_eq!(live.idx, snapshot.cursor + 1);

        let (_, replay, _, replay_projection) = b.events_since("run1", -1).unwrap();
        assert!(replay.is_empty());
        assert_eq!(
            replay_projection.as_ref().map(|value| value.cursor),
            Some(live.idx)
        );
        assert!(b.attach("run2", -1).is_err());
    }

    #[test]
    fn truncation_counter_tracks_ring_overflow_resyncs() {
        let b = SseBroadcaster::new();
        b.start_run("run1".to_string(), 1);
        assert_eq!(b.truncation_count(), 0);

        // Within the ring: a full backfill is NOT a truncation.
        for idx in 0..10 {
            b.broadcast(SseEvent::new(
                "text_chunk",
                serde_json::json!({"text": idx.to_string()}),
            ));
        }
        let _ = b.events_since("run1", -1).unwrap();
        assert_eq!(
            b.truncation_count(),
            0,
            "in-ring backfill is not a truncation"
        );

        // Overflow the ring; now a backfill whose cursor predates the ring is a
        // truncation, and each such resync is counted (attach + events_since).
        for idx in 0..=MAX_RUN_EVENTS {
            b.broadcast(SseEvent::new(
                "text_chunk",
                serde_json::json!({"text": idx.to_string()}),
            ));
        }
        let attachment = b.attach("run1", -1).unwrap();
        assert!(attachment.truncated);
        assert_eq!(b.truncation_count(), 1);
        let _ = b.events_since("run1", -1).unwrap();
        assert_eq!(
            b.truncation_count(),
            2,
            "events_since truncation is counted too"
        );
    }

    #[test]
    fn lag_counter_is_observable_and_starts_at_zero() {
        let b = SseBroadcaster::new();
        assert_eq!(b.lag_count(), 0);
        b.record_lag();
        b.record_lag();
        assert_eq!(b.lag_count(), 2);
        // The counter is shared across clones (the gRPC layer holds a clone).
        let clone = b.clone();
        clone.record_lag();
        assert_eq!(b.lag_count(), 3);
    }

    #[test]
    fn concurrent_broadcasts_to_one_broadcaster_yield_contiguous_idx() {
        use std::sync::Arc;
        use std::thread;
        const THREADS: usize = 8;
        const PER_THREAD: usize = 250; // 8 * 250 = 2000 == MAX_RUN_EVENTS (no overflow)
        let b = Arc::new(SseBroadcaster::new());
        b.start_run("run1".to_string(), 1);
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let b = b.clone();
                thread::spawn(move || {
                    for n in 0..PER_THREAD {
                        b.broadcast(SseEvent::new("text_chunk", serde_json::json!({"text": n})));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        // The single stamping lock serializes concurrent broadcasts: every event
        // got a unique, contiguous idx (no gaps, no duplicates) under contention.
        let total = (THREADS * PER_THREAD) as i64;
        assert_eq!(b.last_idx(), total - 1);
        assert_eq!(b.truncation_count(), 0);
        let (run_id, events, min_idx, projection) =
            b.events_since("run1", total - 1 - 100).unwrap();
        assert_eq!(run_id, "run1");
        assert_eq!(min_idx, 0);
        assert!(projection.is_none());
        let expected_start = total - 100; // first idx > (total - 1 - 100)
        assert_eq!(events.len(), 100);
        for (i, event) in events.iter().enumerate() {
            assert_eq!(event.idx, expected_start + i as i64, "contiguous, no gaps");
            assert_eq!(event.run_id, "run1");
            assert_eq!(event.epoch, 1);
        }
    }

    #[test]
    fn projection_preserves_semantic_order_while_coalescing_deltas() {
        let b = SseBroadcaster::new();
        b.start_run("run1".to_string(), 1);
        b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        b.broadcast(SseEvent::new(
            "thinking_delta",
            serde_json::json!({"text": "a"}),
        ));
        b.broadcast(SseEvent::new(
            "thinking_delta",
            serde_json::json!({"text": "b"}),
        ));
        b.broadcast(SseEvent::new(
            "tool_start",
            serde_json::json!({"tool_id": "t1", "tool_name": "read"}),
        ));
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "hello"}),
        ));
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": " world"}),
        ));
        for idx in 0..MAX_RUN_EVENTS {
            b.broadcast(SseEvent::new(
                "usage",
                serde_json::json!({"usage": {"output_tokens": idx}}),
            ));
        }

        let snapshot = b.attach("run1", -1).unwrap().projection.unwrap();
        assert_eq!(
            snapshot
                .events
                .iter()
                .take(4)
                .map(|event| event.event_type.as_str())
                .collect::<Vec<_>>(),
            vec!["agent_start", "thinking_delta", "tool_start", "text_chunk"]
        );
        let thinking: serde_json::Value = serde_json::from_str(&snapshot.events[1].data).unwrap();
        let text: serde_json::Value = serde_json::from_str(&snapshot.events[3].data).unwrap();
        assert_eq!(thinking["text"], "ab");
        assert_eq!(text["text"], "hello world");
    }

    #[test]
    fn journal_is_committed_before_broadcast_and_failure_interrupts() {
        let directory = tempfile::tempdir().unwrap();
        let b = SseBroadcaster::new();
        b.configure_journal("session-1", directory.path().to_path_buf())
            .unwrap();
        b.start_run("run-1".to_string(), 3);
        let interrupt = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        b.set_persistence_interrupt(interrupt.clone());
        let mut receiver = b.subscribe();

        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text":"ok"}),
        ));
        let event = receiver.try_recv().unwrap();
        assert_eq!(event.session_id, "session-1");
        assert_eq!(event.event_id, "session-1:run-1:3:0");
        let stored = b.read_journal("run-1").unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].event_id, event.event_id);

        b.fail_next_append();
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text":"lost"}),
        ));
        assert!(
            receiver.try_recv().is_err(),
            "uncommitted event must not be visible"
        );
        assert!(interrupt.load(std::sync::atomic::Ordering::SeqCst));
        assert!(b.persistence_error().is_some());
        assert_eq!(b.last_idx(), 0, "failed append must not advance the cursor");
    }

    #[test]
    fn closed_journal_rejects_late_broadcast_without_recreating_files() {
        let directory = tempfile::tempdir().unwrap();
        let b = SseBroadcaster::new();
        b.configure_journal("session-delete", directory.path().to_path_buf())
            .unwrap();
        b.start_run("run-delete".to_string(), 1);
        let mut receiver = b.subscribe();

        b.close_journal();
        std::fs::remove_dir_all(directory.path()).unwrap();
        b.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text":"too late"}),
        ));

        assert!(!directory.path().exists());
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn journal_failpoints_cover_append_flush_and_sync_boundaries() {
        for point in [
            JournalFailPoint::Append,
            JournalFailPoint::Flush,
            JournalFailPoint::Sync,
        ] {
            let directory = tempfile::tempdir().unwrap();
            let b = SseBroadcaster::new();
            b.configure_journal("session-1", directory.path().to_path_buf())
                .unwrap();
            b.start_run("run-1".to_string(), 1);
            let mut receiver = b.subscribe();
            b.fail_at(point);
            b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
            assert!(receiver.try_recv().is_err());
            assert!(b.persistence_error().is_some());
            assert_eq!(b.last_idx(), -1);
        }
    }

    #[test]
    fn journal_recovery_truncates_a_partial_tail() {
        let directory = tempfile::tempdir().unwrap();
        let b = SseBroadcaster::new();
        b.configure_journal("session-1", directory.path().to_path_buf())
            .unwrap();
        b.start_run("run-1".to_string(), 1);
        b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        let path = directory.path().join("run-1.jsonl");
        let valid_len = std::fs::metadata(&path).unwrap().len();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(br#"{"event_type":"partial""#).unwrap();
        file.sync_all().unwrap();

        let recovered = b.read_journal("run-1").unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(std::fs::metadata(path).unwrap().len(), valid_len);
    }

    #[test]
    fn journal_recovery_refuses_middle_corruption_without_truncating() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("run-1.jsonl");
        let first = serde_json::to_vec(&SseEvent::new("ok", serde_json::json!({}))).unwrap();
        let later = serde_json::to_vec(&SseEvent::new("later", serde_json::json!({}))).unwrap();
        let mut bytes = first;
        bytes.extend_from_slice(b"\nnot-json\n");
        bytes.extend_from_slice(&later);
        bytes.push(b'\n');
        std::fs::write(&path, bytes).unwrap();
        let original = std::fs::read(&path).unwrap();
        let b = SseBroadcaster::new();
        b.configure_journal("session-1", directory.path().to_path_buf())
            .unwrap();
        assert!(b.read_journal("run-1").is_err());
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn envelope_carries_run_sequence_and_session_replay_has_own_cursor() {
        let directory = tempfile::tempdir().unwrap();
        let b = SseBroadcaster::new();
        b.configure_journal("session-1", directory.path().to_path_buf())
            .unwrap();
        b.start_run_with_sequence("run-1".to_string(), 3, Some(17));
        b.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        let (_, events, _, _) = b.events_since("run-1", -1).unwrap();
        assert_eq!(events[0].run_sequence, 17);
        b.broadcast(SseEvent::new(
            "model_changed",
            serde_json::json!({"model":"x"}),
        ));
        let session_events = b.session_events_since(-1).unwrap();
        assert_eq!(session_events.len(), 1);
        assert_eq!(session_events[0].session_idx, 0);
        assert_eq!(session_events[0].run_sequence, -1);
    }

    #[test]
    fn atomic_attach_replays_disk_when_memory_ring_is_truncated() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("run-1.jsonl");
        let mut file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        for idx in 0..(MAX_RUN_EVENTS as i64 + 5) {
            let event = SseEvent {
                event_type: "usage".to_string(),
                data: serde_json::json!({"n": idx}).to_string(),
                run_id: "run-1".to_string(),
                epoch: 2,
                idx,
                session_id: "session-1".to_string(),
                event_id: format!("session-1:run-1:2:{idx}"),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                session_idx: -1,
                run_sequence: 9,
            };
            serde_json::to_writer(&mut file, &event).unwrap();
            file.write_all(b"\n").unwrap();
        }
        file.flush().unwrap();

        let b = SseBroadcaster::new();
        b.configure_journal("session-1", directory.path().to_path_buf())
            .unwrap();
        b.start_run("run-1".to_string(), 2);
        let attachment = b.attach("run-1", -1).unwrap();
        assert!(attachment.truncated);
        assert!(attachment.projection.is_none());
        assert_eq!(attachment.events.len(), MAX_RUN_EVENTS + 5);
        assert_eq!(attachment.events.first().unwrap().idx, 0);
        assert_eq!(
            attachment.events.last().unwrap().idx,
            MAX_RUN_EVENTS as i64 + 4
        );
    }

    #[test]
    fn events_since_reads_a_settled_run_from_its_durable_journal() {
        let directory = tempfile::tempdir().unwrap();
        let broadcaster = SseBroadcaster::new();
        broadcaster
            .configure_journal("session-1", directory.path().to_path_buf())
            .unwrap();
        broadcaster.start_run("run-a".to_string(), 1);
        broadcaster.broadcast(SseEvent::new("agent_start", serde_json::json!({})));
        broadcaster.broadcast(SseEvent::new("agent_end", serde_json::json!({})));
        broadcaster.start_run("run-b".to_string(), 2);

        let (run_id, events, _, projection) = broadcaster.events_since("run-a", -1).unwrap();
        assert_eq!(run_id, "run-a");
        assert_eq!(events.len(), 2);
        assert!(projection.is_none());
    }

    // ─── coverage batch: journal resume/recovery/projection arms ───────────

    #[test]
    fn configure_journal_resumes_session_idx_sequence() {
        let dir = tempfile::tempdir().unwrap();
        // Pre-existing session journal with events at session_idx 0 and 1.
        let first = SseBroadcaster::new();
        first
            .configure_journal("s1".to_string(), dir.path().to_path_buf())
            .unwrap();
        first.broadcast(SseEvent::new("model_changed", serde_json::json!({"m": 1})));
        first.broadcast(SseEvent::new("model_changed", serde_json::json!({"m": 2})));

        let second = SseBroadcaster::new();
        second
            .configure_journal("s1".to_string(), dir.path().to_path_buf())
            .unwrap();
        second.broadcast(SseEvent::new("model_changed", serde_json::json!({"m": 3})));
        let events = second.session_events_since(-1).unwrap();
        let idxs: Vec<i64> = events.iter().map(|e| e.session_idx).collect();
        assert_eq!(idxs, vec![0, 1, 2], "resumed after the on-disk sequence");
    }

    #[test]
    fn set_persistence_interrupt_flags_when_journal_failed() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the journal directory belongs → configure fails.
        let blocker = dir.path().join("blocked");
        std::fs::write(&blocker, "x").unwrap();
        let broadcaster = SseBroadcaster::new();
        assert!(broadcaster
            .configure_journal("s1".to_string(), blocker.join("sub"))
            .is_err());
        assert!(broadcaster.persistence_error().is_some());
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        broadcaster.set_persistence_interrupt(flag.clone());
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn start_run_recovers_from_corrupt_journal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("run-x.jsonl"), "{corrupt\n{also corrupt\n").unwrap();
        let broadcaster = SseBroadcaster::new();
        broadcaster
            .configure_journal("s1".to_string(), dir.path().to_path_buf())
            .unwrap();
        broadcaster.start_run("run-x".to_string(), 1);
        assert!(broadcaster.persistence_error().is_some());
    }

    #[test]
    fn broadcast_without_configured_journal_is_in_memory_only() {
        let broadcaster = SseBroadcaster::new();
        broadcaster.start_run("run-mem".to_string(), 1);
        broadcaster.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": "x"}),
        ));
        assert!(broadcaster.persistence_error().is_none());
        let (_, events, _, _) = broadcaster.events_since("run-mem", -1).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn events_since_beyond_ring_reads_disk_journal() {
        let dir = tempfile::tempdir().unwrap();
        let broadcaster = SseBroadcaster::new();
        broadcaster
            .configure_journal("s1".to_string(), dir.path().to_path_buf())
            .unwrap();
        broadcaster.start_run("run-long".to_string(), 1);
        for i in 0..2100 {
            broadcaster.broadcast(SseEvent::new("text_chunk", serde_json::json!({"i": i})));
        }
        // The in-memory ring holds 2000; idx 0 is truncated out.
        let (_, events, _, projection) = broadcaster.events_since("run-long", 0).unwrap();
        assert!(
            projection.is_none(),
            "journal present → disk replay, not projection"
        );
        let event_count = events.len();
        assert!(event_count > 2000, "full history from disk: {event_count}");
    }

    #[test]
    fn attach_beyond_ring_without_journal_returns_projection() {
        let broadcaster = SseBroadcaster::new();
        broadcaster.start_run("run-ring".to_string(), 1);
        for i in 0..2100 {
            broadcaster.broadcast(SseEvent::new("text_chunk", serde_json::json!({"i": i})));
        }
        let attachment = broadcaster.attach("run-ring", 0).unwrap();
        let projection = attachment
            .projection
            .expect("projection over the truncated ring");
        assert_eq!(projection.run_id, "run-ring");
        assert!(!projection.events.is_empty());
    }

    #[test]
    fn projection_skips_raw_text_deltas() {
        let mut projection = Vec::new();
        apply_to_projection(
            &mut projection,
            &SseEvent::new("text_delta", serde_json::json!({"text": "raw"})),
        );
        assert!(projection.is_empty());
        apply_to_projection(
            &mut projection,
            &SseEvent::new("text_chunk", serde_json::json!({"text": "kept"})),
        );
        assert_eq!(projection.len(), 1);
        // A second text_chunk replaces the projection's previous text entry.
        apply_to_projection(
            &mut projection,
            &SseEvent::new("text_chunk", serde_json::json!({"text": "kept v2"})),
        );
        assert_eq!(projection.len(), 1);
    }

    #[test]
    fn projection_folds_tool_deltas_of_the_same_tool_stream() {
        let mut projection = Vec::new();
        apply_to_projection(
            &mut projection,
            &SseEvent::new(
                "toolcall_delta",
                serde_json::json!({"tool_id": "t1", "text": "a"}),
            ),
        );
        apply_to_projection(
            &mut projection,
            &SseEvent::new(
                "toolcall_delta",
                serde_json::json!({"tool_id": "t1", "text": "b"}),
            ),
        );
        assert_eq!(projection.len(), 1);
        let data: serde_json::Value = serde_json::from_str(&projection[0].data).unwrap();
        assert_eq!(data["text"], "ab");
    }

    #[test]
    fn projection_keeps_tool_deltas_of_different_tool_streams() {
        let mut projection = Vec::new();
        apply_to_projection(
            &mut projection,
            &SseEvent::new(
                "toolcall_delta",
                serde_json::json!({"tool_id": "t1", "text": "a"}),
            ),
        );
        apply_to_projection(
            &mut projection,
            &SseEvent::new(
                "toolcall_delta",
                serde_json::json!({"tool_id": "t2", "text": "b"}),
            ),
        );
        assert_eq!(projection.len(), 2);
    }

    #[test]
    fn projection_pushes_when_event_data_is_not_foldable() {
        // Previous event's data is not parseable JSON → no fold, plain push.
        let mut projection = Vec::new();
        let mut broken = SseEvent::new("text_chunk", serde_json::json!({"text": "a"}));
        broken.data = "{not json".to_string();
        apply_to_projection(&mut projection, &broken);
        apply_to_projection(
            &mut projection,
            &SseEvent::new("text_chunk", serde_json::json!({"text": "b"})),
        );
        assert_eq!(projection.len(), 2);
        // Valid JSON but no "text" field on the previous event → no fold.
        let mut projection = Vec::new();
        apply_to_projection(
            &mut projection,
            &SseEvent::new("text_chunk", serde_json::json!({"other": 1})),
        );
        apply_to_projection(
            &mut projection,
            &SseEvent::new("text_chunk", serde_json::json!({"text": "b"})),
        );
        assert_eq!(projection.len(), 2);
    }

    #[test]
    fn sse_event_default_session_idx_is_negative() {
        let event: SseEvent = serde_json::from_str(
            r#"{"type":"x","data":"{}","event_type":"x","run_id":"","epoch":0,"idx":0,"timestamp":""}"#,
        )
        .unwrap();
        assert_eq!(event.session_idx, -1);
        let _default = SseBroadcaster::default();
    }
}

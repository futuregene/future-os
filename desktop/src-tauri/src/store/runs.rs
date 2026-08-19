use std::collections::HashMap;
#[cfg(test)]
use std::io::Write;
use std::path::PathBuf;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::db::*;
use super::records::*;
use super::status::{TERMINAL_RUN_STATUSES, TERMINAL_RUN_STATUSES_SQL};
use super::util::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: String,
    pub thread_id: String,
    pub trigger_message_id: Option<String>,
    pub status: String,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub error_message: Option<String>,
    /// Structured error classification. One of:
    /// 'stream_disconnected', 'command_failed', 'model_failed',
    /// 'abort_requested', 'timeout', 'unknown'. NULL when the run did not
    /// fail or the error type is unknown.
    pub error_type: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEventRecord {
    pub id: String,
    pub run_id: String,
    pub event_type: String,
    pub payload: Option<String>,
    pub sequence: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRecord {
    pub id: String,
    pub run_id: String,
    pub name: String,
    pub kind: String,
    pub input: Option<String>,
    pub status: String,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutputRecord {
    pub id: String,
    pub tool_call_id: String,
    pub kind: String,
    pub content: Option<String>,
    pub created_at: i64,
}

sql_record!(pub(super) RUN_COLUMNS, run_from_row -> RunRecord {
    id, thread_id, trigger_message_id, status, model_provider, model_id,
    started_at, ended_at, error_message, error_type, created_at, updated_at,
});

// TOOL_CALL_COLUMNS & tool_call_from_row removed — table dropped; ToolCallRecord
// is now reconstructed from run events (`project_tool_calls`).
// TOOL_OUTPUT_COLUMNS & tool_output_from_row removed — table dropped

pub fn create_run(input: CreateRunInput) -> Result<RunRecord, crate::AppError> {
    let id = input
        .id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| create_id("run"));
    let now = now_millis();
    let conn = connect()?;
    conn.execute(
        "INSERT INTO runs (
             id, thread_id, trigger_message_id, status, model_provider, model_id,
             started_at, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?6, ?6)",
        params![
            id,
            input.thread_id,
            input.trigger_message_id,
            input.model_provider,
            input.model_id,
            now
        ],
    )?;
    let run = loaded(get_run(&id)?, "Created run")?;
    mark_catalog_dirty();
    Ok(run)
}

/// Resolved agent session ids of every run that is not yet terminal — i.e. the
/// conversations the user still sees as "generating". Each id is the thread's
/// `agent_session_id` when set (trimmed, non-empty), else the thread id, mirroring
/// the GUI's own session-id resolution (see `useAgentThreadState` /
/// `cleanup::orphan_thread_ids`). Deduplicated. Powers the quit guard: whether to
/// warn before exit, and which sessions to abort on force-quit. On startup the
/// Agent watchdog reconciles these rows rather than guessing that they died.
pub fn active_run_sessions() -> Result<Vec<String>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT COALESCE(NULLIF(TRIM(t.agent_session_id), ''), t.id)
             FROM runs r
             JOIN threads t ON t.id = r.thread_id
             WHERE r.status NOT IN ({TERMINAL_RUN_STATUSES_SQL})"
    ))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

pub fn list_runs(thread_id: &str) -> Result<Vec<RunRecord>, crate::AppError> {
    let conn = connect()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS}
             FROM runs
             WHERE thread_id = ?1
             ORDER BY created_at DESC"
    ))?;
    let rows = stmt.query_map(params![thread_id], run_from_row)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

/// Find the run created for a durable remote command id. Mobile persists this
/// id before sending a prompt, so a retry after either process restarts can
/// recover the original acknowledgement instead of creating a duplicate run.
pub fn find_run_by_trigger_message_id(
    trigger_message_id: &str,
) -> Result<Option<RunRecord>, crate::AppError> {
    if trigger_message_id.trim().is_empty() {
        return Ok(None);
    }
    let conn = connect()?;
    conn.query_row(
        &format!(
            "SELECT {RUN_COLUMNS}
                 FROM runs
                 WHERE trigger_message_id = ?1
                 ORDER BY created_at DESC, id DESC
                 LIMIT 1"
        ),
        params![trigger_message_id],
        run_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

/// The thread's single most recent run (same ordering/tiebreak as
/// [`latest_run_infos`]). Used by initial loads and pushed terminal
/// reconciliation without transferring the thread's entire run history.
pub fn latest_run(thread_id: &str) -> Result<Option<RunRecord>, crate::AppError> {
    let conn = connect()?;
    conn.query_row(
        &format!(
            "SELECT {RUN_COLUMNS}
                 FROM runs
                 WHERE thread_id = ?1
                 ORDER BY created_at DESC, id DESC
                 LIMIT 1"
        ),
        params![thread_id],
        run_from_row,
    )
    .optional()
    .map_err(crate::AppError::from)
}

/// The single latest run's identity and status for each of `thread_ids`.
/// Powers low-frequency thread-list reconciliation in one connection/query.
/// Threads with no runs are omitted; callers treat missing as "no run yet".
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestRunInfo {
    pub thread_id: String,
    pub run_id: String,
    pub status: String,
    pub ended_at: Option<i64>,
}

pub fn latest_run_infos(thread_ids: &[String]) -> Result<Vec<LatestRunInfo>, crate::AppError> {
    if thread_ids.is_empty() {
        return Ok(vec![]);
    }
    let conn = connect()?;
    let placeholders: Vec<String> = (0..thread_ids.len())
        .map(|i| format!("?{}", i + 1))
        .collect();
    let sql = format!(
        "SELECT thread_id, id, status, ended_at FROM (
             SELECT thread_id, id, status, ended_at,
                    ROW_NUMBER() OVER (PARTITION BY thread_id ORDER BY created_at DESC, id DESC) AS rn
             FROM runs
             WHERE thread_id IN ({})
         ) WHERE rn = 1",
        placeholders.join(",")
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = thread_ids
        .iter()
        .map(|id| id as &dyn rusqlite::types::ToSql)
        .collect();
    let rows = stmt.query_map(params.as_slice(), |row| {
        Ok(LatestRunInfo {
            thread_id: row.get(0)?,
            run_id: row.get(1)?,
            status: row.get(2)?,
            ended_at: row.get(3)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(crate::AppError::from)
}

/// Cancel the still-open approvals of a single run, stamping them with `note`.
/// Called from [`update_run_status_if_active`] when a run transitions to
/// `cancelled`, so a pending approval never outlives its owning run on the
/// single-run (user abort) path. Startup convergence deliberately does NOT use
/// this — pending approvals survive a GUI restart and are reconciled against
/// the Agent's `get_state.pendingApprovals` instead (the Agent may still be
/// parked on exactly that request).
pub(super) fn cancel_children_of_runs(
    tx: &rusqlite::Transaction<'_>,
    run_id: &str,
    note: &str,
    now: i64,
) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE approval_requests
             SET status = 'cancelled',
                 decision_note = COALESCE(decision_note, ?2),
                 decided_at = COALESCE(decided_at, ?1),
                 updated_at = ?1
             WHERE status = 'pending' AND run_id = ?3",
        params![now, note, run_id],
    )?;
    Ok(())
}

/// Like [`update_run_status`], but only transitions a run that is *not already
/// terminal* — the guard is part of the `UPDATE`'s `WHERE`, so a concurrent
/// `abort_run`/`fail_run_if_active` (which sets `cancelled`/`failed`) is never
/// clobbered by a late read-then-write. Returns whether a row changed; the
/// `cancelled` cascade runs only when it did.
pub fn update_run_status_if_active(input: UpdateRunStatusInput) -> Result<bool, crate::AppError> {
    let now = now_millis();
    let mut conn = connect()?;
    let tx = conn.transaction()?;
    let changed = update_run_status_if_active_tx(&tx, &input, now)?;
    tx.commit()?;
    if changed {
        mark_catalog_dirty();
        emit_run_status_update(&input.run_id, &input.status);
    }
    Ok(changed)
}

fn update_run_status_if_active_tx(
    tx: &rusqlite::Transaction<'_>,
    input: &UpdateRunStatusInput,
    now: i64,
) -> rusqlite::Result<bool> {
    let ended_at = if TERMINAL_RUN_STATUSES.contains(&input.status.as_str()) {
        Some(now)
    } else {
        None
    };
    let affected = tx.execute(
        &format!(
            "UPDATE runs
         SET status = ?1,
             error_message = ?2,
             error_type = COALESCE(?3, error_type),
             ended_at = COALESCE(?4, ended_at),
             updated_at = ?5
         WHERE id = ?6
           AND status NOT IN ({TERMINAL_RUN_STATUSES_SQL})"
        ),
        params![
            input.status,
            input.error_message,
            input.error_type,
            ended_at,
            now,
            input.run_id
        ],
    )?;
    if affected > 0 && input.status == "cancelled" {
        cancel_children_of_runs(
            tx,
            &input.run_id,
            "Cancelled because the run was terminated.",
            now,
        )?;
    }
    Ok(affected > 0)
}

/// Transition a run to `failed` only if it is not already in a terminal state,
/// in a single atomic statement. Returns `true` if a row was updated. This is a
/// compare-and-set so a concurrent abort (which sets `cancelled`) is never
/// clobbered by a late failure projection.
pub fn fail_run_if_active(
    run_id: &str,
    error_message: &str,
    error_type: &str,
) -> Result<bool, crate::AppError> {
    let now = now_millis();
    let conn = connect()?;
    let affected = conn.execute(
        &format!(
            "UPDATE runs
         SET status = 'failed',
             error_message = ?1,
             error_type = ?2,
             ended_at = COALESCE(ended_at, ?3),
             updated_at = ?3
         WHERE id = ?4
           AND status NOT IN ({TERMINAL_RUN_STATUSES_SQL})"
        ),
        params![error_message, error_type, now, run_id],
    )?;
    let changed = affected > 0;
    if changed {
        mark_catalog_dirty();
        emit_run_status_update(run_id, "failed");
    }
    Ok(changed)
}

fn emit_run_status_update(run_id: &str, status: &str) {
    if let Ok(Some(run)) = get_run(run_id) {
        crate::emit_thread_runtime_updated(
            run.thread_id,
            run_id.to_string(),
            status.to_string(),
            false,
        );
    }
}

pub fn list_run_events(run_id: &str) -> Result<Vec<RunEventRecord>, crate::AppError> {
    Ok(read_run_events(run_id))
}

/// The tail of a run's events with `sequence > since_sequence`, in append
/// order. Legacy compatibility reader over the pre-journal GUI JSONL logs;
/// live reads go through the Agent journal (see `commands::runs`), which
/// falls back here only while the Agent is unreachable. `since_sequence < 0`
/// returns the full log (same as [`list_run_events`]).
pub fn list_run_events_since(
    run_id: &str,
    since_sequence: i64,
) -> Result<Vec<RunEventRecord>, crate::AppError> {
    if since_sequence < 0 {
        return Ok(read_run_events(run_id));
    }
    #[cfg(test)]
    {
        let buf = RUN_EVENT_BUFFER.lock().unwrap_or_else(unpoison);
        if let Some(events) = buf.get(run_id) {
            return Ok(events
                .iter()
                .filter(|event| event.sequence > since_sequence)
                .cloned()
                .collect());
        }
    }
    // This is deliberately a legacy compatibility reader only. New events
    // live in the Agent journal; GUI never buffers or writes a second source.
    Ok(read_events_from_disk(run_id)
        .into_iter()
        .filter(|event| event.sequence > since_sequence)
        .collect())
}

/// Test-only in-memory event buffer. Production keeps no GUI-side event copy —
/// the Agent journal is the source of truth — so this buffer exists purely as
/// a fixture for the compatibility-reader tests below.
#[cfg(test)]
static RUN_EVENT_BUFFER: std::sync::LazyLock<
    std::sync::Mutex<HashMap<String, Vec<RunEventRecord>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

// ── Test fixture: async disk writer ─────────────────────────────────────
// The compatibility reader must stay covered against real on-disk logs, so
// tests retain a writer that produces them: a single background thread
// holding one open BufWriter per run, coalescing bursts into one flush.
#[cfg(test)]
enum WriterMsg {
    Event(RunEventRecord),
    /// Flush and close the run's writer, then ack. Sent before the run's log
    /// is read from disk (settle) or deleted: the ack guarantees the file is
    /// complete and closed before the caller proceeds (Windows also refuses
    /// to delete an open file).
    Close {
        run_id: String,
        ack: std::sync::mpsc::Sender<()>,
    },
}

#[cfg(test)]
static DISK_WRITER: std::sync::LazyLock<std::sync::mpsc::Sender<WriterMsg>> =
    std::sync::LazyLock::new(spawn_disk_writer);

#[cfg(test)]
fn spawn_disk_writer() -> std::sync::mpsc::Sender<WriterMsg> {
    let (tx, rx) = std::sync::mpsc::channel::<WriterMsg>();
    std::thread::Builder::new()
        .name("run-event-writer".to_string())
        .spawn(move || {
            let mut writers: HashMap<String, std::io::BufWriter<std::fs::File>> = HashMap::new();
            while let Ok(msg) = rx.recv() {
                match msg {
                    WriterMsg::Event(record) => write_event(&mut writers, &record),
                    WriterMsg::Close { run_id, ack } => {
                        if let Some(mut writer) = writers.remove(&run_id) {
                            let _ = writer.flush();
                        }
                        let _ = ack.send(());
                    }
                }
            }
        })
        .expect("spawn run-event writer thread");
    tx
}

#[cfg(test)]
fn write_event(
    writers: &mut HashMap<String, std::io::BufWriter<std::fs::File>>,
    record: &RunEventRecord,
) {
    let writer = match writers.entry(record.run_id.clone()) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::hash_map::Entry::Vacant(entry) => {
            let Some(path) = run_events_path(&record.run_id) else {
                return;
            };
            let Ok(file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            else {
                return;
            };
            entry.insert(std::io::BufWriter::new(file))
        }
    };
    if let Ok(line) = serde_json::to_string(record) {
        let _ = writeln!(writer, "{line}");
    }
}

/// Flush + close the run's writer on the disk-writer thread and wait for the
/// ack (bounded), so a following disk read sees the complete log and a
/// following delete doesn't hit an open handle.
#[cfg(test)]
fn close_disk_writer(run_id: &str) {
    let (ack_tx, ack_rx) = std::sync::mpsc::channel();
    if DISK_WRITER
        .send(WriterMsg::Close {
            run_id: run_id.to_string(),
            ack: ack_tx,
        })
        .is_ok()
    {
        let _ = ack_rx.recv_timeout(std::time::Duration::from_secs(2));
    }
}

/// Force the async disk writer to flush a run's log — synchronization point
/// for tests that assert on the log file's existence.
#[cfg(test)]
pub(crate) fn flush_run_event_log_for_test(run_id: &str) {
    close_disk_writer(run_id);
}

/// Directory holding per-run event logs: `~/.future/app/run_events/`.
fn run_events_dir() -> Option<PathBuf> {
    let dir = app_dir().ok()?.join("run_events");
    // Production never writes this legacy directory. Unit tests retain a
    // fixture writer so pre-journal logs remain covered by the fallback reader.
    #[cfg(test)]
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Per-run event log path, or None if `run_id` isn't a safe filename slug
/// (defends against path traversal from an unexpected id).
fn run_events_path(run_id: &str) -> Option<PathBuf> {
    if run_id.is_empty()
        || !run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    Some(run_events_dir()?.join(format!("{run_id}.jsonl")))
}

/// Queue one event for the run's JSONL log (best-effort; a dead writer thread
/// just means that event won't survive a restart — same contract as the old
/// synchronous write, whose errors were also dropped).
#[cfg(test)]
fn persist_event_to_disk(record: RunEventRecord) {
    let _ = DISK_WRITER.send(WriterMsg::Event(record));
}

/// Read a run's events from the persisted log (one JSON object per line).
fn read_events_from_disk(run_id: &str) -> Vec<RunEventRecord> {
    let Some(path) = run_events_path(run_id) else {
        return vec![];
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return vec![];
    };
    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<RunEventRecord>(line).ok())
        .collect()
}

/// A run's events, in append order: the in-memory buffer while the run is
/// active, else the persisted log (survives restart / post-settle eviction).
fn read_run_events(run_id: &str) -> Vec<RunEventRecord> {
    #[cfg(test)]
    {
        let buf = RUN_EVENT_BUFFER.lock().unwrap_or_else(unpoison);
        if let Some(events) = buf.get(run_id) {
            return events.clone();
        }
    }
    read_events_from_disk(run_id)
}

#[cfg(test)]
pub fn append_run_event(input: AppendRunEventInput) -> Result<RunEventRecord, crate::AppError> {
    // Re-delivery guard: a run's events arrive in strictly increasing sequence
    // from its single writer, so a sequence at/below the buffered high-water
    // mark is a replay overlap or a cross-writer race duplicate — return the
    // already-stored record instead of appending a second copy. (The
    // projection-snapshot replace path clears the buffer first, so it
    // re-appends from an empty slate and is unaffected.)
    {
        let buf = RUN_EVENT_BUFFER.lock().unwrap_or_else(unpoison);
        if let Some(events) = buf.get(&input.run_id) {
            if let Some(last) = events.last() {
                if input.sequence <= last.sequence {
                    if let Some(existing) =
                        events.iter().find(|event| event.sequence == input.sequence)
                    {
                        return Ok(existing.clone());
                    }
                }
            }
        }
    }
    let id = create_id("event");
    let now = now_millis();
    let record = RunEventRecord {
        id,
        run_id: input.run_id.clone(),
        event_type: input.event_type,
        payload: input.payload,
        sequence: input.sequence,
        created_at: now,
    };
    if let Ok(mut buf) = RUN_EVENT_BUFFER.lock() {
        buf.entry(input.run_id.clone())
            .or_default()
            .push(record.clone());
    }
    persist_event_to_disk(record.clone());
    Ok(record)
}

/// Drop a settled run's in-memory events (called on `agent_end`). The
/// canonical Agent journal stays; this only bounds GUI-side memory. The tool
/// projection cache survives (settled events are immutable, so later polls
/// reuse it). The buffer itself is test-only; in production this is a no-op.
pub fn clear_run_event_buffer(_run_id: &str) {
    #[cfg(test)]
    if let Ok(mut buf) = RUN_EVENT_BUFFER.lock() {
        buf.remove(_run_id);
    }
}

/// Delete a run's persisted event log (called when the run/thread is deleted).
/// The writer is closed first — Windows refuses to delete an open file.
pub fn delete_run_events_file(run_id: &str) {
    if let Some(path) = run_events_path(run_id) {
        let _ = std::fs::remove_file(path);
    }
    let mut cache = TOOL_PROJECTION_CACHE.lock().unwrap_or_else(unpoison);
    cache.remove(run_id);
}

/// Remove the whole run-events directory (called by `clear_all_data`).
pub fn clear_all_run_events_files() {
    if let Ok(dir) = app_dir() {
        let _ = std::fs::remove_dir_all(dir.join("run_events"));
    }
    let mut cache = TOOL_PROJECTION_CACHE.lock().unwrap_or_else(unpoison);
    cache.clear();
}

/// Per-run incremental tool-call projection. The context panel polls tool
/// calls for every run; rebuilding from the full event log each time costs
/// O(events) per run per poll (clone + JSON parse of every event). Events are
/// append-only, so the projection advances over just the new tail instead. The
/// caller feeds the tail (fetched from the Agent journal, the canonical event
/// source — see `commands::runs`); this module only folds and caches it. State
/// survives the run settling (the journal is then immutable) and is dropped
/// only when the run's data is deleted or the cache overflows.
struct ToolProjectionState {
    tools: Vec<ToolCallRecord>,
    index_by_id: HashMap<String, usize>,
    /// Highest event sequence folded into `tools` so far.
    last_sequence: i64,
    /// Recency stamp for LRU eviction (see `PROJECTION_CLOCK`).
    last_used: u64,
}

static TOOL_PROJECTION_CACHE: std::sync::LazyLock<
    std::sync::Mutex<HashMap<String, ToolProjectionState>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Monotonic recency clock: every cache touch takes the next stamp. HashMap
/// iteration order is arbitrary, so LRU needs an explicit stamp.
static PROJECTION_CLOCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_projection_stamp() -> u64 {
    PROJECTION_CLOCK.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

/// Bound the cache (entries are small: one record per tool call).
const TOOL_PROJECTION_CACHE_MAX: usize = 64;

/// The highest event sequence already folded into the run's cached tool
/// projection, or -1 when nothing is cached. Callers fetch the journal tail
/// after this cursor so [`advance_tool_projection`] only sees new events.
pub fn tool_projection_cursor(run_id: &str) -> i64 {
    TOOL_PROJECTION_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(run_id).map(|state| state.last_sequence))
        .unwrap_or(-1)
}

/// True when the projection holds a tool call that started but never ended.
/// Such an entry carries the only in-memory copy of that call's tool_start
/// input, which tool_end persistence still needs (artifact path extraction),
/// so eviction prefers entries without one.
fn has_open_tool_call(state: &ToolProjectionState) -> bool {
    state.tools.iter().any(|tool| tool.status == "running")
}

/// Fold `events` into the run's cached tool projection and return the full
/// list. Empty `events` just returns the cached state — and creates NO entry
/// for a run with no cached state, so a bulk sweep over history runs without
/// tool activity doesn't churn the cache. Holding the cache mutex across the
/// fold prevents two concurrent pollers from double-applying the same tail.
pub fn advance_tool_projection(run_id: &str, events: &[RunEventRecord]) -> Vec<ToolCallRecord> {
    // Poison recovery keeps the derived cache usable (see util::unpoison).
    let mut cache = TOOL_PROJECTION_CACHE.lock().unwrap_or_else(unpoison);
    if events.is_empty() {
        // Read-only advance: refresh recency and return what's folded.
        if let Some(state) = cache.get_mut(run_id) {
            state.last_used = next_projection_stamp();
            return state.tools.clone();
        }
        return Vec::new();
    }
    // Make room one entry at a time. The victim is the least recently used
    // entry WITHOUT an open tool call: bulk polls advance the newest (active)
    // run first and then 64+ history runs (list_runs is created_at DESC), so
    // plain recency would leave the active run as the OLDEST entry mid-sweep
    // and evict it — dropping the tool_start input its tool_end persistence
    // still needs. A bulk `clear()` (the historical behavior) was worse.
    while cache.len() >= TOOL_PROJECTION_CACHE_MAX && !cache.contains_key(run_id) {
        // The cache is provably non-empty here (the loop condition), and the
        // or_else arm yields the oldest entry when every entry has an open
        // tool call.
        let victim = cache
            .iter()
            .filter(|(_, state)| !has_open_tool_call(state))
            .min_by_key(|(_, state)| state.last_used)
            .or_else(|| cache.iter().min_by_key(|(_, state)| state.last_used))
            .map(|(id, _)| id.clone())
            .expect("cache is non-empty while evicting");
        cache.remove(&victim);
    }
    let mut state = cache.remove(run_id).unwrap_or_else(|| ToolProjectionState {
        tools: Vec::new(),
        index_by_id: HashMap::new(),
        last_sequence: -1,
        last_used: 0,
    });
    for event in events {
        if event.sequence <= state.last_sequence {
            continue; // replay overlap — already folded
        }
        apply_tool_event(&mut state, event);
        if event.sequence > state.last_sequence {
            state.last_sequence = event.sequence;
        }
    }
    state.last_used = next_projection_stamp();
    let tools = state.tools.clone();
    cache.insert(run_id.to_string(), state);
    tools
}

/// One-shot tool-call projection over an event log (no cache). Reconstructs
/// each tool call from its tool_start / tool_end events (the tool_calls table
/// was dropped). Both events carry the agent's stable tool id, so pair by id —
/// a single "current" slot would mispair overlapping (parallel) tool calls.
#[cfg(test)]
pub fn project_tool_calls(events: &[RunEventRecord]) -> Vec<ToolCallRecord> {
    let mut state = ToolProjectionState {
        tools: Vec::new(),
        index_by_id: HashMap::new(),
        last_sequence: -1,
        last_used: 0,
    };
    for event in events {
        apply_tool_event(&mut state, event);
    }
    state.tools
}

/// Fold one event into the tool-call projection (see [`project_tool_calls`]).
fn apply_tool_event(state: &mut ToolProjectionState, event: &RunEventRecord) {
    let tools = &mut state.tools;
    let index_by_id = &mut state.index_by_id;
    match event.event_type.as_str() {
        "tool_start" | "toolcall_start" => {
            // Use the same fallback as the frontend (agentActivity.ts:445):
            // `${toolName}_${sequence}`.  When the payload lacks an explicit
            // tool_id, the front-end generates this synthetic id — we must
            // match it so the tool detail lookup succeeds.
            let id = event_tool_id(event).unwrap_or_else(|| {
                let (name, _, _) = parse_tool_start_payload(event.payload.as_deref());
                let seq = event.sequence;
                if name.is_empty() {
                    event.id.clone()
                } else {
                    format!("{name}_{seq}")
                }
            });
            let (name, kind, input) = parse_tool_start_payload(event.payload.as_deref());
            if let Some(&idx) = index_by_id.get(&id) {
                // The same call announced twice: `toolcall_start` fires first
                // with empty args (they stream in via toolcall_delta), then
                // the execution `tool_start` carries the complete args. Enrich
                // the existing record rather than adding an empty duplicate.
                if input.as_deref().is_some_and(|s| !s.is_empty()) {
                    tools[idx].input = input;
                }
                if !name.is_empty() {
                    tools[idx].name = name;
                    tools[idx].kind = kind;
                }
            } else {
                index_by_id.insert(id.clone(), tools.len());
                tools.push(ToolCallRecord {
                    id,
                    run_id: event.run_id.clone(),
                    name,
                    kind,
                    input,
                    status: "running".to_string(),
                    started_at: Some(event.created_at),
                    ended_at: None,
                    created_at: event.created_at,
                });
            }
        }
        "tool_end" | "tool_result" => {
            let idx = event_tool_id(event)
                .as_deref()
                .and_then(|id| index_by_id.get(id).copied());
            if let Some(idx) = idx {
                let command = shell_command_from_input(tools[idx].input.as_deref());
                tools[idx].status = tool_end_status(event.payload.as_deref(), command.as_deref());
                tools[idx].ended_at = Some(event.created_at);
            }
        }
        _ => {}
    }
}

/// The agent's stable tool-call id from a buffered tool event payload
/// (`tool_id`/`toolID`/`tool_call_id`). Both `tool_start` and `tool_end` carry
/// it (agent/mod.rs broadcasts `tc.id` on each), so it's how a tool call and
/// its output are correlated across the two events.
fn event_tool_id(event: &RunEventRecord) -> Option<String> {
    let payload = event.payload.as_deref()?;
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    ["tool_id", "toolID", "tool_call_id"].iter().find_map(|k| {
        v.get(*k)
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

fn parse_tool_start_payload(payload: Option<&str>) -> (String, String, Option<String>) {
    let default = (String::new(), String::new(), None);
    let Some(payload) = payload else {
        return default;
    };
    let Ok(v): Result<serde_json::Value, _> = serde_json::from_str(payload) else {
        return default;
    };
    // Match the frontend's toolFromPayload precedence (agentActivity.ts:435-437)
    let name = v
        .get("tool_name")
        .or_else(|| v.get("toolName"))
        .or_else(|| v.get("name"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let kind = name.clone(); // tool_name doubles as kind (shell, write, edit, read)
    let input = v
        .get("tool_args")
        .or(v.get("input"))
        .and_then(normalize_tool_input);
    (name, kind, input)
}

/// Preserve the historical string representation consumed by the desktop UI
/// while accepting the protocol-neutral agent's native JSON arguments. Older
/// OpenAI Chat events commonly carry a JSON-encoded string; Responses and
/// Anthropic carry an object. Keeping both forms here also lets the later,
/// complete tool_start enrich an earlier streaming start with empty args.
fn normalize_tool_input(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) => Some(value.clone()),
        value => serde_json::to_string(value).ok(),
    }
}

/// Tool-call status from its `tool_end` payload. An explicit `error` is a
/// failure; so is a shell command that exits non-zero — the agent returns that
/// as a *successful* result (no error field). The agent reports the conclusion
/// structured (`exit_code` + `is_soft_fail`) on the event; events without
/// those (older agents, journal-synthesized imports) fall back to parsing the
/// `[exit: N]` footer on the last line of the output. A bare grep/diff/test
/// exiting 1 is a normal "no match / differs" signal, not a failure.
fn tool_end_status(payload: Option<&str>, command: Option<&str>) -> String {
    let Some(payload) = payload else {
        return "completed".to_string();
    };
    let Ok(v): Result<serde_json::Value, _> = serde_json::from_str(payload) else {
        return "completed".to_string();
    };
    let has_error = v
        .get("error")
        .or_else(|| v.get("errorText"))
        .and_then(|s| s.as_str())
        .is_some_and(|s| !s.is_empty());
    if has_error {
        return "failed".to_string();
    }
    if let Some(code) = v
        .get("exit_code")
        .or_else(|| v.get("exitCode"))
        .and_then(|code| code.as_i64())
    {
        if code == 0 {
            return "completed".to_string();
        }
        let soft_fail = v
            .get("is_soft_fail")
            .or_else(|| v.get("isSoftFail"))
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false);
        return if soft_fail {
            "completed".to_string()
        } else {
            "failed".to_string()
        };
    }
    let output = v
        .get("text")
        .or_else(|| v.get("result"))
        .and_then(|s| s.as_str());
    match nonzero_exit_code(output) {
        Some(1) if is_soft_fail_command(command) => "completed".to_string(),
        Some(_) => "failed".to_string(),
        None => "completed".to_string(),
    }
}

/// The non-zero code from the `[exit: N]` footer line, or None (exit 0 / not a
/// shell result). Mirrors the agent-bridge persist logic.
fn nonzero_exit_code(output: Option<&str>) -> Option<i64> {
    let line = output?.trim_end().lines().last()?;
    let code = line.strip_prefix("[exit: ")?.strip_suffix(']')?;
    code.trim().parse::<i64>().ok().filter(|code| *code != 0)
}

/// A bare grep/diff/cmp/test exiting 1 is a normal signal, not an error. Any
/// shell operator makes the exit ambiguous (pipeline/list), so those stay
/// failures.
fn is_soft_fail_command(command: Option<&str>) -> bool {
    let Some(command) = command else {
        return false;
    };
    if command.contains(['|', '&', ';', '\n', '`', '<', '>']) || command.contains("$(") {
        return false;
    }
    let Some(first) = command.split_whitespace().next() else {
        return false;
    };
    let base = first
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(first)
        .to_ascii_lowercase();
    let program = base.strip_suffix(".exe").unwrap_or(base.as_str());
    matches!(
        program,
        "grep" | "egrep" | "fgrep" | "rg" | "findstr" | "diff" | "cmp" | "test" | "["
    )
}

/// Extract the shell `command` from a tool call's persisted input JSON (used to
/// exempt soft-fail commands). Handles a doubly-encoded JSON string input.
fn shell_command_from_input(input: Option<&str>) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(input?).ok()?;
    if let serde_json::Value::String(inner) = &value {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(inner) {
            value = parsed;
        }
    }
    value
        .get("command")
        .and_then(|c| c.as_str())
        .map(str::to_string)
}

/// The structured `tool_args` recorded for a tool call, read from the run's
/// tool projection (the persist path folds every live event into it as it
/// lands, journal-tail polls and fork/import synthesis do the same). Falls
/// back to the legacy GUI JSONL for pre-journal builds. This is what
/// `tool_end` persistence uses for artifact path extraction — the structured
/// input beats parsing the tool's output prose.
pub fn get_tool_call_input(
    run_id: &str,
    tool_call_id: &str,
) -> Result<Option<String>, crate::AppError> {
    {
        let mut cache = TOOL_PROJECTION_CACHE.lock().unwrap_or_else(unpoison);
        if let Some(mut state) = cache.remove(run_id) {
            let input = state
                .index_by_id
                .get(tool_call_id)
                .and_then(|&idx| state.tools[idx].input.clone());
            // The read counts as a use — refresh recency so a run whose tools
            // are still being persisted isn't evicted mid-flight.
            state.last_used = next_projection_stamp();
            cache.insert(run_id.to_string(), state);
            if input.is_some() {
                return Ok(input);
            }
        }
    }
    // Compatibility fallback: pre-journal builds persisted raw events to the
    // legacy GUI JSONL log. Scan it for the matching tool_start.
    let events = read_run_events(run_id);
    for event in events.iter().rev() {
        if event.event_type != "tool_start" || event_tool_id(event).as_deref() != Some(tool_call_id)
        {
            continue;
        }
        // event_tool_id just parsed this payload successfully (and found the
        // id), so both unwraps are invariant-backed.
        let payload = event.payload.as_deref().expect("tool_id implies payload");
        let v: serde_json::Value =
            serde_json::from_str(payload).expect("payload parsed by event_tool_id");
        return Ok(v
            .get("tool_args")
            .or(v.get("input"))
            .and_then(normalize_tool_input));
    }
    Ok(None)
}

/// Reconstruct a tool call's output from the run's `tool_end` event that
/// carries the same stable tool id. `tool_end` carries the result text and any
/// error (agent/mod.rs broadcasts `text`/`error`). The caller supplies the
/// run's event log (Agent journal, canonical); the inspector's stdout/stderr
/// panes thus survive an app restart.
pub fn project_tool_outputs(
    events: &[RunEventRecord],
    tool_call_id: &str,
) -> Vec<ToolOutputRecord> {
    for event in events {
        if !matches!(event.event_type.as_str(), "tool_end" | "tool_result") {
            continue;
        }
        let resolved_id = event_tool_id(event).unwrap_or_else(|| {
            let (name, _, _) = parse_tool_start_payload(event.payload.as_deref());
            let seq = event.sequence;
            if name.is_empty() {
                event.id.clone()
            } else {
                format!("{name}_{seq}")
            }
        });
        if resolved_id != tool_call_id {
            continue;
        }
        let v: serde_json::Value = event
            .payload
            .as_deref()
            .and_then(|p| serde_json::from_str(p).ok())
            .unwrap_or(serde_json::Value::Null);
        let text = v
            .get("text")
            .or_else(|| v.get("result"))
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty());
        let error = v
            .get("error")
            .or_else(|| v.get("errorText"))
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty());

        // Wrap into a JSON object: the inspector runs the content through
        // `parseJsonish` and keeps only object results, reading stdout from
        // `text` and stderr from `error`. A bare string would be dropped.
        let mut obj = serde_json::Map::new();
        if let Some(text) = text {
            obj.insert(
                "text".to_string(),
                serde_json::Value::String(text.to_string()),
            );
        }
        if let Some(error) = error {
            obj.insert(
                "error".to_string(),
                serde_json::Value::String(error.to_string()),
            );
        }
        let content = if obj.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(obj).to_string())
        };

        return vec![ToolOutputRecord {
            id: event.id.clone(),
            tool_call_id: tool_call_id.to_string(),
            kind: if error.is_some() { "error" } else { "text" }.to_string(),
            content,
            created_at: event.created_at,
        }];
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use rusqlite::{params, Connection};

    use super::*;
    use crate::store::schema::SCHEMA;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(SCHEMA).expect("initialize test schema");
        // These tests exercise the run-status CAS in isolation, so insert run
        // rows directly without their thread/workspace parents.
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("disable foreign keys");
        conn
    }

    fn insert_run(conn: &Connection, id: &str, status: &str) {
        conn.execute(
            "INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES (?1, 'thread', ?2, 1, 1)",
            params![id, status],
        )
        .expect("insert run");
    }

    #[test]
    fn tool_end_status_prefers_structured_exit_fields() {
        // Structured fields from the agent win...
        assert_eq!(
            tool_end_status(Some(r#"{"exit_code": 0, "text": "[exit: 127]"}"#), None),
            "completed"
        );
        assert_eq!(
            tool_end_status(Some(r#"{"exit_code": 127, "text": "not found"}"#), None),
            "failed"
        );
        // ...including the soft-fail conclusion for a bare grep exit 1.
        assert_eq!(
            tool_end_status(Some(r#"{"exit_code": 1, "is_soft_fail": true}"#), None),
            "completed"
        );
        // An explicit error field still fails regardless of exit fields.
        assert_eq!(
            tool_end_status(Some(r#"{"exit_code": 0, "error": "boom"}"#), None),
            "failed"
        );
        // Legacy events (no structured fields) fall back to the prose footer.
        assert_eq!(
            tool_end_status(
                Some(r#"{"text": "no match\n[exit: 1]"}"#),
                Some("grep foo bar")
            ),
            "completed"
        );
        assert_eq!(
            tool_end_status(
                Some(r#"{"text": "no match\n[exit: 1]"}"#),
                Some("cargo build")
            ),
            "failed"
        );
    }

    /// Seed the process-global event buffer directly (no disk writes) with
    /// `count` events sequenced 0..count, returning the unique run id used.
    /// `tag` keeps concurrently-running tests off each other's buffer entry.
    fn seed_event_buffer(tag: &str, count: i64) -> String {
        let run_id = format!("test_since_{tag}_{}", std::process::id());
        let mut buf = RUN_EVENT_BUFFER.lock().expect("lock event buffer");
        let events = buf.entry(run_id.clone()).or_default();
        events.clear();
        for sequence in 0..count {
            events.push(RunEventRecord {
                id: format!("e{sequence}"),
                run_id: run_id.clone(),
                event_type: "text_chunk".to_string(),
                payload: None,
                sequence,
                created_at: sequence,
            });
        }
        run_id
    }

    #[test]
    fn list_run_events_since_returns_only_the_unseen_tail() {
        let run_id = seed_event_buffer("tail", 10);

        let tail = list_run_events_since(&run_id, 4).expect("tail read");
        assert_eq!(tail.len(), 5, "sequences 5..=9");
        assert_eq!(tail[0].sequence, 5);
        assert_eq!(tail[8 - 5].sequence, 8);

        // At the watermark: nothing new.
        assert!(list_run_events_since(&run_id, 9)
            .expect("no new")
            .is_empty());
        // Beyond the watermark (caller ahead of the log): still nothing.
        assert!(list_run_events_since(&run_id, 100)
            .expect("ahead")
            .is_empty());
        // Negative watermark: the full log, matching list_run_events.
        assert_eq!(list_run_events_since(&run_id, -1).expect("full").len(), 10);

        clear_run_event_buffer(&run_id);
    }

    /// Build one run event record for tool-projection tests.
    fn tool_event(run_id: &str, event_type: &str, payload: &str, sequence: i64) -> RunEventRecord {
        RunEventRecord {
            id: format!("e{sequence}"),
            run_id: run_id.to_string(),
            event_type: event_type.to_string(),
            payload: Some(payload.to_string()),
            sequence,
            created_at: sequence,
        }
    }

    #[test]
    fn tool_projection_advances_incrementally_without_duplicates() {
        let run_id = format!("test_tools_{}", std::process::id());

        let first = advance_tool_projection(
            &run_id,
            &[
                tool_event(
                    &run_id,
                    "tool_start",
                    r#"{"tool_id":"t1","tool_name":"read","tool_args":"{\"path\":\"/a.ts\"}"}"#,
                    0,
                ),
                tool_event(&run_id, "tool_end", r#"{"tool_id":"t1","text":"ok"}"#, 1),
            ],
        );
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].status, "completed");
        assert_eq!(tool_projection_cursor(&run_id), 1);

        // Re-feeding the same tail (replay overlap) must not duplicate.
        let second = advance_tool_projection(
            &run_id,
            &[tool_event(
                &run_id,
                "tool_end",
                r#"{"tool_id":"t1","text":"ok"}"#,
                1,
            )],
        );
        assert_eq!(second.len(), 1);

        // A later event is folded in on the next advance.
        let third = advance_tool_projection(
            &run_id,
            &[tool_event(
                &run_id,
                "tool_start",
                r#"{"tool_id":"t2","tool_name":"edit","tool_args":"{\"path\":\"/b.ts\"}"}"#,
                2,
            )],
        );
        assert_eq!(third.len(), 2);
        assert_eq!(third[1].name, "edit");
        assert_eq!(third[1].status, "running");

        let mut cache = TOOL_PROJECTION_CACHE.lock().unwrap_or_else(unpoison);
        cache.remove(&run_id);
    }

    #[test]
    fn tool_projection_accepts_object_args_and_enriches_streaming_start() {
        let run_id = format!("test_object_args_{}", std::process::id());
        let tools = advance_tool_projection(
            &run_id,
            &[
                tool_event(
                    &run_id,
                    "tool_start",
                    r#"{"tool_id":"t1","tool_name":"shell","tool_args":""}"#,
                    0,
                ),
                tool_event(
                    &run_id,
                    "tool_start",
                    r#"{"tool_id":"t1","tool_name":"shell","tool_args":{"command":"pwd"}}"#,
                    1,
                ),
                tool_event(&run_id, "tool_end", r#"{"tool_id":"t1","text":"ok"}"#, 2),
            ],
        );

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].input.as_deref(), Some(r#"{"command":"pwd"}"#));
        assert_eq!(tools[0].status, "completed");
        assert_eq!(
            get_tool_call_input(&run_id, "t1").expect("read object input"),
            Some(r#"{"command":"pwd"}"#.to_string())
        );

        let mut cache = TOOL_PROJECTION_CACHE.lock().unwrap_or_else(unpoison);
        cache.remove(&run_id);
    }

    #[test]
    fn tool_projection_empty_advance_on_uncached_run_returns_nothing() {
        // A read-only advance for a run with no cached projection must not
        // create an entry — it returns the empty list.
        let uncached = format!("test_uncached_{}", std::process::id());
        assert!(advance_tool_projection(&uncached, &[]).is_empty());
    }

    #[test]
    fn bulk_sweep_cannot_evict_a_run_with_an_open_tool_call() {
        // Regression (review round 2): list_runs is created_at DESC, so the
        // context panel's bulk poll advances the NEWEST (active) run first
        // and then 64+ history runs. Plain recency then leaves the active run
        // as the OLDEST entry mid-sweep and evicts it — and for a long tool
        // call (minutes between tool_start and tool_end, no projection events
        // in between) the tool_end persistence loses the tool_start input,
        // degrading artifact path extraction. Eviction must skip projections
        // that still hold an open tool call.
        let live = format!("test_open_live_{}", std::process::id());
        // The live run starts a long tool call.
        advance_tool_projection(
            &live,
            &[tool_event(
                &live,
                "tool_start",
                r#"{"tool_id":"t1","tool_name":"write","tool_args":"{\"path\":\"/a.ts\"}"}"#,
                0,
            )],
        );

        // Bulk sweep in real poll order: the live run first (re-advanced with
        // no new events), then more than a cache capacity of settled history
        // runs.
        advance_tool_projection(&live, &[]);
        let mut others = Vec::new();
        for i in 0..=TOOL_PROJECTION_CACHE_MAX {
            let other = format!("test_open_other_{i}_{}", std::process::id());
            others.push(other.clone());
            advance_tool_projection(
                &other,
                &[
                    tool_event(
                        &other,
                        "tool_start",
                        r#"{"tool_id":"x","tool_name":"read"}"#,
                        0,
                    ),
                    tool_event(&other, "tool_end", r#"{"tool_id":"x","text":"ok"}"#, 1),
                ],
            );
        }

        // Capacity pressure did evict history runs...
        assert!(
            others.iter().any(|id| tool_projection_cursor(id) == -1),
            "expected at least one settled run to be evicted"
        );
        // ...but the live run's open tool call survived: its structured input
        // is still readable at tool_end persistence...
        assert_eq!(
            get_tool_call_input(&live, "t1").expect("read live run input"),
            Some(r#"{"path":"/a.ts"}"#.to_string())
        );
        // ...and the tool_end folds onto the folded start (status resolves)
        // rather than producing an end-only record with no input.
        let tools = advance_tool_projection(
            &live,
            &[tool_event(
                &live,
                "tool_end",
                r#"{"tool_id":"t1","text":"ok"}"#,
                1,
            )],
        );
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].status, "completed");

        let mut cache = TOOL_PROJECTION_CACHE.lock().unwrap_or_else(unpoison);
        cache.remove(&live);
        for other in &others {
            cache.remove(other);
        }
    }

    #[test]
    fn project_tool_outputs_pairs_by_stable_tool_id() {
        let events = vec![
            tool_event(
                "r",
                "tool_start",
                r#"{"tool_id":"t1","tool_name":"shell","tool_args":"{\"command\":\"ls\"}"}"#,
                0,
            ),
            tool_event("r", "tool_end", r#"{"tool_id":"t1","text":"file.ts"}"#, 1),
            tool_event(
                "r",
                "tool_end",
                r#"{"tool_id":"t2","text":"x","error":"boom"}"#,
                2,
            ),
        ];

        let outputs = project_tool_outputs(&events, "t1");
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].kind, "text");
        assert!(outputs[0]
            .content
            .as_deref()
            .unwrap_or("")
            .contains("file.ts"));

        let errored = project_tool_outputs(&events, "t2");
        assert_eq!(errored.len(), 1);
        assert_eq!(errored[0].kind, "error");

        assert!(project_tool_outputs(&events, "missing").is_empty());
    }

    #[test]
    fn get_tool_call_input_reads_the_tool_projection() {
        let run_id = format!("test_tool_input_{}", std::process::id());
        advance_tool_projection(
            &run_id,
            &[tool_event(
                &run_id,
                "tool_start",
                r#"{"tool_id":"t1","tool_name":"write","tool_args":"{\"path\":\"/a.ts\"}"}"#,
                0,
            )],
        );

        assert_eq!(
            get_tool_call_input(&run_id, "t1").expect("read projected input"),
            Some(r#"{"path":"/a.ts"}"#.to_string())
        );
        assert_eq!(
            get_tool_call_input(&run_id, "other").expect("read missing input"),
            None
        );

        // Settling keeps the projection (settled events are immutable), so the
        // input stays readable after the run's event buffer is dropped.
        clear_run_event_buffer(&run_id);
        assert_eq!(
            get_tool_call_input(&run_id, "t1").expect("read after settle"),
            Some(r#"{"path":"/a.ts"}"#.to_string())
        );

        let mut cache = TOOL_PROJECTION_CACHE.lock().unwrap_or_else(unpoison);
        cache.remove(&run_id);
    }

    fn insert_thread(conn: &Connection, id: &str, agent_session_id: Option<&str>) {
        conn.execute(
            "INSERT INTO threads
                 (id, workspace_id, mode, title, status, pinned, readonly,
                  agent_session_id, created_at, updated_at)
             VALUES (?1, 'ws', 'chat', 'T', 'active', 0, 0, ?2, 1, 1)",
            params![id, agent_session_id],
        )
        .expect("insert thread");
    }

    fn insert_thread_run(conn: &Connection, run_id: &str, thread_id: &str, status: &str) {
        conn.execute(
            "INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, 1)",
            params![run_id, thread_id, status],
        )
        .expect("insert run");
    }

    /// `active_run_sessions` returns exactly the sessions of non-terminal runs,
    /// deduplicated, resolving the session id to `agent_session_id` when set and
    /// the thread id otherwise (blank/whitespace ids fall back to the thread id).
    #[test]
    fn active_run_sessions_resolves_and_filters() {
        let conn = test_conn();

        // Live run, thread has an agent session id -> resolves to that id.
        insert_thread(&conn, "tA", Some("sessA"));
        insert_thread_run(&conn, "rA", "tA", "running");

        // Live run, no agent session id -> resolves to the thread id.
        insert_thread(&conn, "tB", None);
        insert_thread_run(&conn, "rB", "tB", "waiting_approval");

        // Blank agent session id -> falls back to the thread id.
        insert_thread(&conn, "tC", Some("   "));
        insert_thread_run(&conn, "rC", "tC", "running");

        // Two live runs on one thread -> a single deduplicated session id.
        insert_thread(&conn, "tD", Some("sessD"));
        insert_thread_run(&conn, "rD1", "tD", "running");
        insert_thread_run(&conn, "rD2", "tD", "running");

        // Terminal-only thread -> excluded entirely.
        insert_thread(&conn, "tE", Some("sessE"));
        insert_thread_run(&conn, "rE", "tE", "completed");

        let mut sessions = {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT DISTINCT COALESCE(NULLIF(TRIM(t.agent_session_id), ''), t.id)
                         FROM runs r
                         JOIN threads t ON t.id = r.thread_id
                         WHERE r.status NOT IN ({TERMINAL_RUN_STATUSES_SQL})"
                ))
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        sessions.sort();
        assert_eq!(sessions, vec!["sessA", "sessD", "tB", "tC"]);
    }

    fn run_status(conn: &Connection, id: &str) -> String {
        conn.query_row(
            "SELECT status FROM runs WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("read run status")
    }

    fn running_input(run_id: &str) -> UpdateRunStatusInput {
        UpdateRunStatusInput {
            run_id: run_id.to_string(),
            status: "running".to_string(),
            error_message: None,
            error_type: None,
        }
    }

    /// B-13: a terminal run is never resurrected by the if-active CAS.
    #[test]
    fn if_active_skips_terminal_run() {
        let mut conn = test_conn();
        insert_run(&conn, "run_cancelled", "cancelled");
        let tx = conn.transaction().unwrap();
        let changed =
            update_run_status_if_active_tx(&tx, &running_input("run_cancelled"), 99).unwrap();
        tx.commit().unwrap();
        assert!(!changed);
        assert_eq!(run_status(&conn, "run_cancelled"), "cancelled");
    }

    /// a completed run is not rewritten to cancelled by a late
    /// abort (nor to any other status by a late completion projection).
    #[test]
    fn if_active_skips_completed_run() {
        let mut conn = test_conn();
        insert_run(&conn, "run_done", "completed");
        let cancel = UpdateRunStatusInput {
            run_id: "run_done".to_string(),
            status: "cancelled".to_string(),
            error_message: Some("Terminated by user.".to_string()),
            error_type: Some("abort_requested".to_string()),
        };
        let tx = conn.transaction().unwrap();
        let changed = update_run_status_if_active_tx(&tx, &cancel, 99).unwrap();
        tx.commit().unwrap();
        assert!(!changed);
        assert_eq!(run_status(&conn, "run_done"), "completed");
    }

    /// A non-terminal run does transition, and the cancelled cascade fires.
    #[test]
    fn if_active_cancels_active_run_and_cascades() {
        let mut conn = test_conn();
        insert_run(&conn, "run_live", "running");
        conn.execute(
            "INSERT INTO approval_requests (id, thread_id, run_id, kind, status, title, created_at, updated_at)
             VALUES ('ap1', 'thread', 'run_live', 'shell', 'pending', 't', 1, 1)",
            [],
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        let input = UpdateRunStatusInput {
            run_id: "run_live".to_string(),
            status: "cancelled".to_string(),
            error_message: Some("stop".to_string()),
            error_type: None,
        };
        let changed = update_run_status_if_active_tx(&tx, &input, 99).unwrap();
        tx.commit().unwrap();
        assert!(changed);
        assert_eq!(run_status(&conn, "run_live"), "cancelled");
        let approval_status: String = conn
            .query_row(
                "SELECT status FROM approval_requests WHERE id = 'ap1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(approval_status, "cancelled");
    }

    /// A cancelled transition whose approval cascade fails must propagate the
    /// SQL error (the `?` on `cancel_children_of_runs`), not swallow it.
    #[test]
    fn if_active_propagates_cancel_cascade_error() {
        let mut conn = test_conn();
        insert_run(&conn, "run_live2", "running");
        let tx = conn.transaction().unwrap();
        // Break the cascade: dropping the table makes the cancelled cascade's
        // UPDATE fail deterministically.
        tx.execute_batch("DROP TABLE approval_requests;").unwrap();
        let input = UpdateRunStatusInput {
            run_id: "run_live2".to_string(),
            status: "cancelled".to_string(),
            error_message: None,
            error_type: None,
        };
        let err = update_run_status_if_active_tx(&tx, &input, 99).unwrap_err();
        assert!(
            err.to_string().contains("approval_requests"),
            "cascade error must surface: {err}"
        );
    }

    #[test]
    fn append_run_event_dedups_replayed_sequences() {
        let run_id = format!("test_dedup_{}", std::process::id());
        let input = |sequence: i64| AppendRunEventInput {
            run_id: run_id.clone(),
            event_type: "text_chunk".to_string(),
            payload: Some(format!(r#"{{"text":"s{sequence}"}}"#)),
            sequence,
        };

        let first = append_run_event(input(0)).expect("first append");
        let replay = append_run_event(input(0)).expect("replay tolerated");
        assert_eq!(
            first.id, replay.id,
            "a replayed sequence returns the already-stored record"
        );
        append_run_event(input(1)).expect("advancing append");
        append_run_event(input(1)).expect("second replay tolerated");

        let events = list_run_events(&run_id).expect("list events");
        assert_eq!(
            events.len(),
            2,
            "replayed sequences must not duplicate the log"
        );
        assert_eq!(events[0].sequence, 0);
        assert_eq!(events[1].sequence, 1);
        clear_run_event_buffer(&run_id);
    }

    // ── connect()-backed read wrappers ─────────────────────────────────────

    use crate::store::db::test_support::guarded_conn;

    #[test]
    fn run_read_wrappers_query_per_thread() {
        let (_home, conn) = guarded_conn("runs_wrappers");
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, agent_session_id, created_at, updated_at
             ) VALUES
                 ('t1', 'ws1', 'chat', 'T1', 'sess1', 1, 1),
                 ('t2', 'ws1', 'chat', 'T2', NULL, 1, 1);
             INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES
                 ('r1_old', 't1', 'completed', 1, 1),
                 ('r1_new', 't1', 'running', 2, 2),
                 ('r2', 't2', 'failed', 3, 3);",
        )
        .expect("seed");
        drop(conn);

        // active_run_sessions: only the non-terminal run's session resolves.
        assert_eq!(
            active_run_sessions().expect("active sessions"),
            vec!["sess1".to_string()]
        );

        let runs = list_runs("t1").expect("list runs");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, "r1_new", "newest first");

        let latest = latest_run("t1").expect("latest").expect("some");
        assert_eq!(latest.id, "r1_new");
        assert!(latest_run("t_ghost").expect("latest").is_none());

        assert!(latest_run_infos(&[]).expect("empty").is_empty());
        let infos = latest_run_infos(&["t1".to_string(), "t2".to_string(), "ghost".to_string()])
            .expect("infos");
        assert_eq!(infos.len(), 2, "threads without runs are omitted");
        assert_eq!(infos[0].run_id, "r1_new");
        assert_eq!(infos[1].run_id, "r2");
        assert_eq!(infos[1].status, "failed");
    }

    #[test]
    fn fail_run_if_active_cas_guards_terminal_rows() {
        let (_home, conn) = guarded_conn("runs_fail_cas");
        conn.execute_batch(
            "INSERT INTO workspaces (
                 id, name, kind, path, cleanup_status, created_at, updated_at
             ) VALUES ('ws1', 'WS', 'temporary', '/tmp/ws1', 'active', 1, 1);
             INSERT INTO threads (
                 id, workspace_id, mode, title, created_at, updated_at
             ) VALUES ('t1', 'ws1', 'chat', 'T', 1, 1);
             INSERT INTO runs (id, thread_id, status, created_at, updated_at)
             VALUES ('r1', 't1', 'running', 1, 1);",
        )
        .expect("seed");
        drop(conn);

        assert!(fail_run_if_active("r1", "boom", "model_failed").expect("fail"));
        let row = get_run("r1").expect("get").expect("some");
        assert_eq!(row.status, "failed");
        assert_eq!(row.error_message.as_deref(), Some("boom"));
        assert_eq!(row.error_type.as_deref(), Some("model_failed"));
        assert!(row.ended_at.is_some());

        // Terminal now: the CAS refuses a second transition.
        assert!(!fail_run_if_active("r1", "again", "unknown").expect("no-op"));
        assert!(!fail_run_if_active("ghost", "x", "unknown").expect("missing"));
    }

    // ── tool projection: one-shot matrix ────────────────────────────────────

    /// A tool event with an explicit (possibly absent) payload.
    fn raw_tool_event(
        run_id: &str,
        event_type: &str,
        payload: Option<&str>,
        sequence: i64,
    ) -> RunEventRecord {
        RunEventRecord {
            id: format!("e{sequence}"),
            run_id: run_id.to_string(),
            event_type: event_type.to_string(),
            payload: payload.map(str::to_string),
            sequence,
            created_at: sequence,
        }
    }

    #[test]
    fn project_tool_calls_handles_announcements_edges() {
        let events = vec![
            // Stable id, then a no-content duplicate announce, then the
            // enriched execution announce (input + name win).
            tool_event("r", "toolcall_start", r#"{"tool_id":"a"}"#, 0),
            tool_event(
                "r",
                "tool_start",
                r#"{"tool_id":"a","tool_name":"read","tool_args":"{}"}"#,
                1,
            ),
            // No tool_id: synthetic `{name}_{seq}`.
            tool_event("r", "tool_start", r#"{"tool_name":"shell"}"#, 2),
            // Unparseable payload: falls back to the event id.
            tool_event("r", "tool_start", "{not json", 3),
            // No payload at all: same fallback.
            raw_tool_event("r", "tool_start", None, 4),
            // Non-tool events are ignored.
            tool_event("r", "text_chunk", "{}", 5),
            // Ends: by stable id, and by synthetic id.
            tool_event("r", "tool_end", r#"{"tool_id":"a","text":"done"}"#, 6),
            tool_event("r", "tool_result", r#"{"text":"out"}"#, 7), // no id: no match
        ];

        let tools = project_tool_calls(&events);
        assert_eq!(tools.len(), 4);
        let by_id = |id: &str| tools.iter().find(|tool| tool.id == id).expect(id);
        assert_eq!(by_id("a").name, "read");
        assert_eq!(by_id("a").input.as_deref(), Some("{}"));
        assert_eq!(by_id("a").status, "completed");
        assert!(by_id("a").ended_at.is_some());
        assert_eq!(by_id("shell_2").name, "shell");
        assert_eq!(by_id("e3").name, "");
        assert_eq!(by_id("e4").status, "running");
    }

    #[test]
    fn tool_end_status_edge_inputs() {
        // No payload / unparseable payload → completed.
        assert_eq!(tool_end_status(None, None), "completed");
        assert_eq!(tool_end_status(Some("{bad"), None), "completed");
        // Footer exit 1 with no command context is a failure…
        assert_eq!(
            tool_end_status(Some(r#"{"text":"x\n[exit: 1]"}"#), None),
            "failed"
        );
        // …but a soft-fail program exits 1 as a normal signal.
        assert_eq!(
            tool_end_status(Some(r#"{"text":"x\n[exit: 1]"}"#), Some("rg foo")),
            "completed"
        );
    }

    #[test]
    fn soft_fail_command_classification() {
        assert!(!is_soft_fail_command(None));
        assert!(!is_soft_fail_command(Some("   ")));
        assert!(!is_soft_fail_command(Some("grep a | wc")));
        assert!(is_soft_fail_command(Some("/usr/bin/grep pattern")));
        assert!(is_soft_fail_command(Some("C:\\tools\\findstr.exe x")));
        assert!(is_soft_fail_command(Some("test -f x")));
    }

    #[test]
    fn shell_command_from_input_decodes_doubly_encoded_json() {
        assert_eq!(shell_command_from_input(None), None);
        assert_eq!(shell_command_from_input(Some("{bad")), None);
        assert_eq!(
            shell_command_from_input(Some(r#"{"command":"ls"}"#)),
            Some("ls".to_string())
        );
        // A JSON string *containing* the JSON object (double-encoded).
        assert_eq!(
            shell_command_from_input(Some(r#""{\"command\":\"pwd\"}""#)),
            Some("pwd".to_string())
        );
        // A JSON string that doesn't itself parse: no command.
        assert_eq!(shell_command_from_input(Some(r#""not json""#)), None);
    }

    #[test]
    fn project_tool_outputs_synthetic_ids_and_empty_content() {
        let events = vec![
            // No tool_id anywhere: resolves to the event id…
            tool_event("r", "tool_end", r#"{"text":"plain"}"#, 1),
            // …or to `{name}_{seq}` when the payload names a tool…
            tool_event(
                "r",
                "tool_end",
                r#"{"tool_name":"shell","text":"named"}"#,
                2,
            ),
            // …and an empty payload object yields content None.
            tool_event("r", "tool_end", r#"{"tool_id":"t3"}"#, 3),
        ];

        let by_event_id = project_tool_outputs(&events, "e1");
        assert_eq!(by_event_id.len(), 1);
        assert_eq!(by_event_id[0].kind, "text");

        let by_name = project_tool_outputs(&events, "shell_2");
        assert_eq!(by_name.len(), 1);

        let empty = project_tool_outputs(&events, "t3");
        assert_eq!(empty.len(), 1);
        assert_eq!(empty[0].content, None);
    }

    // ── legacy JSONL disk log ───────────────────────────────────────────────

    use crate::auth_store::test_support::HomeGuard;

    fn text_event(run_id: &str, sequence: i64) -> AppendRunEventInput {
        AppendRunEventInput {
            run_id: run_id.to_string(),
            event_type: "text_chunk".to_string(),
            payload: Some(format!(r#"{{"text":"s{sequence}"}}"#)),
            sequence,
        }
    }

    #[test]
    fn legacy_disk_log_roundtrip_and_clear() {
        let _home = HomeGuard::new("runs_disk");
        let run_id = format!("disk_{}", std::process::id());

        // Flood the writer so the single disk-writer thread drains a backlog
        // that ends in the Close message (flush + ack), leaving a complete log.
        for sequence in 0..300 {
            append_run_event(text_event(&run_id, sequence)).expect("append");
        }
        flush_run_event_log_for_test(&run_id);
        // Force the disk readers (drop the in-memory copy).
        clear_run_event_buffer(&run_id);

        let all = list_run_events(&run_id).expect("read from disk");
        assert_eq!(all.len(), 300);
        let tail = list_run_events_since(&run_id, 250).expect("tail from disk");
        assert_eq!(tail.len(), 49, "sequences 251..=299");

        clear_all_run_events_files();
        assert!(
            !app_dir().expect("app dir").join("run_events").exists(),
            "the whole run-events dir is reclaimed"
        );
    }

    #[test]
    fn readers_reject_unsafe_run_ids() {
        let _home = HomeGuard::new("runs_bad_id");
        assert!(list_run_events("../evil").expect("read").is_empty());
        assert!(list_run_events("").expect("read").is_empty());
        assert!(list_run_events_since("a/b", 0).expect("tail").is_empty());
        // Persisting an event for an unusable id is dropped, not an error…
        append_run_event(text_event("bad/slash", 0)).expect("append bad id");
        clear_run_event_buffer("bad/slash");
        // …and so is one whose log path is squatted by a directory.
        let squat = app_dir()
            .expect("app dir")
            .join("run_events")
            .join("squat.jsonl");
        std::fs::create_dir_all(&squat).expect("squat a directory");
        append_run_event(text_event("squat", 0)).expect("append squatted");
        clear_run_event_buffer("squat");
        assert!(list_run_events("squat").expect("read squatted").is_empty());
    }

    #[test]
    fn append_dedup_tolerates_gaps_and_empty_buffer_entries() {
        // A sparse buffer: a replay-window sequence that isn't present is
        // appended, not mistaken for a duplicate.
        let run_id = format!("test_gap_{}", std::process::id());
        append_run_event(text_event(&run_id, 0)).expect("seq 0");
        append_run_event(text_event(&run_id, 9)).expect("seq 9");
        append_run_event(text_event(&run_id, 5)).expect("gap append");
        let events = list_run_events(&run_id).expect("list");
        assert_eq!(events.len(), 3);
        clear_run_event_buffer(&run_id);

        // An existing but EMPTY buffer entry (no last event to compare).
        let empty_run = seed_event_buffer("empty", 0);
        append_run_event(text_event(&empty_run, 3)).expect("append onto empty");
        assert_eq!(list_run_events(&empty_run).expect("list").len(), 1);
        clear_run_event_buffer(&empty_run);
    }

    #[test]
    fn tool_call_input_legacy_fallback_paths() {
        let run_id = format!("test_legacy_input_{}", std::process::id());

        // No projection state at all → straight to the legacy log.
        assert_eq!(
            get_tool_call_input(&run_id, "anything").expect("no events"),
            None
        );

        append_run_event(AppendRunEventInput {
            run_id: run_id.clone(),
            event_type: "tool_start".to_string(),
            payload: Some(r#"{"tool_id":"good","tool_args":"{\"path\":\"/x\"}"}"#.to_string()),
            sequence: 0,
        })
        .expect("good start");
        append_run_event(AppendRunEventInput {
            run_id: run_id.clone(),
            event_type: "tool_start".to_string(),
            payload: Some("{not json".to_string()),
            sequence: 1,
        })
        .expect("bad start");
        append_run_event(AppendRunEventInput {
            run_id: run_id.clone(),
            event_type: "tool_start".to_string(),
            payload: Some(r#"{"tool_id":"object","tool_args":{"path":"/object"}}"#.to_string()),
            sequence: 2,
        })
        .expect("object start");

        // The projection cache has no entry for this run, so every read falls
        // through to the legacy scan (newest first).
        assert_eq!(
            get_tool_call_input(&run_id, "good").expect("legacy read"),
            Some(r#"{"path":"/x"}"#.to_string())
        );
        assert_eq!(
            get_tool_call_input(&run_id, "object").expect("legacy object read"),
            Some(r#"{"path":"/object"}"#.to_string())
        );
        // A matched tool_start whose payload lacks tool_args → None.
        append_run_event(AppendRunEventInput {
            run_id: run_id.clone(),
            event_type: "tool_start".to_string(),
            payload: Some(r#"{"tool_id":"bare"}"#.to_string()),
            sequence: 3,
        })
        .expect("bare start");
        assert_eq!(get_tool_call_input(&run_id, "bare").expect("bare"), None);

        // With a cached projection that lacks the id, the legacy log answers.
        advance_tool_projection(
            &run_id,
            &[tool_event(
                &run_id,
                "tool_start",
                r#"{"tool_id":"cached","tool_name":"read"}"#,
                10,
            )],
        );
        assert_eq!(
            get_tool_call_input(&run_id, "good").expect("legacy after cache"),
            Some(r#"{"path":"/x"}"#.to_string())
        );

        clear_run_event_buffer(&run_id);
        let mut cache = TOOL_PROJECTION_CACHE.lock().unwrap_or_else(unpoison);
        cache.remove(&run_id);
    }
}

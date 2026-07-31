use std::collections::HashMap;
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
// is now reconstructed from run events in `list_tool_calls`.
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
/// warn before exit, and which sessions to abort on force-quit. Within a live
/// process this is a faithful "is anything running" signal — startup convergence
/// (`cancel_stale_approval_requests`) has already cancelled every orphaned
/// non-terminal run left by a previous process.
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
/// order. Backs the frontend's pushed live-preview projection: instead of cloning and
/// re-serializing the whole log every tick (O(n) per tick → O(n²) over a run),
/// only the events the caller hasn't seen cross IPC. `since_sequence < 0`
/// returns the full log (same as [`list_run_events`]).
pub fn list_run_events_since(
    run_id: &str,
    since_sequence: i64,
) -> Result<Vec<RunEventRecord>, crate::AppError> {
    if since_sequence < 0 {
        return Ok(read_run_events(run_id));
    }
    // Filter rather than slice: every buffer writer today appends in
    // monotonically increasing sequence order (one collector per run), but
    // that invariant is implicit — a scan keeps the tail correct even if a
    // future writer appends out of order, and still clones only the new
    // events. The scan is integer compares; the clone was the real cost.
    if let Ok(buf) = RUN_EVENT_BUFFER.lock() {
        if let Some(events) = buf.get(run_id) {
            return Ok(events
                .iter()
                .filter(|event| event.sequence > since_sequence)
                .cloned()
                .collect());
        }
    }
    Ok(read_events_from_disk(run_id)
        .into_iter()
        .filter(|event| event.sequence > since_sequence)
        .collect())
}

/// Whether any events for `run_id` exist locally (buffer or persisted log).
/// Cheap — no event cloning — so the incremental-read command can decide
/// whether an empty tail means "no new events" or "cold buffer, ask the agent".
pub fn has_run_events(run_id: &str) -> bool {
    if let Ok(buf) = RUN_EVENT_BUFFER.lock() {
        if buf.get(run_id).is_some_and(|events| !events.is_empty()) {
            return true;
        }
    }
    run_events_path(run_id).is_some_and(|path| path.exists())
}

pub fn list_run_events_bulk(
    run_ids: &[String],
) -> Result<Vec<(String, Vec<RunEventRecord>)>, crate::AppError> {
    let mut result = Vec::new();
    for rid in run_ids {
        let events = read_run_events(rid);
        if !events.is_empty() {
            result.push((rid.clone(), events));
        }
    }
    Ok(result)
}

/// In-memory buffer for streaming run events. A run's events live here while it
/// is active (fast streaming reads) and are also appended to a per-run JSONL
/// file on disk so the Runs panel/inspector survive an app restart. The buffer
/// entry is dropped once the run settles (see `clear_run_event_buffer`); reads
/// then fall back to the file. Keyed by run_id.
static RUN_EVENT_BUFFER: std::sync::LazyLock<
    std::sync::Mutex<HashMap<String, Vec<RunEventRecord>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

// ── Async disk writer ─────────────────────────────────────────────────
// Run events are appended to a per-run JSONL log so the Runs panel survives
// an app restart. Writes go through a single background thread holding one
// open BufWriter per active run, coalescing bursts into one flush — previously
// every event did its own open/append/close syscall trio, several times per
// second while streaming.
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
    /// Flush and close every writer (clear_all_data), then ack.
    CloseAll {
        ack: std::sync::mpsc::Sender<()>,
    },
}

static DISK_WRITER: std::sync::LazyLock<std::sync::mpsc::Sender<WriterMsg>> =
    std::sync::LazyLock::new(spawn_disk_writer);

fn spawn_disk_writer() -> std::sync::mpsc::Sender<WriterMsg> {
    let (tx, rx) = std::sync::mpsc::channel::<WriterMsg>();
    std::thread::Builder::new()
        .name("run-event-writer".to_string())
        .spawn(move || {
            let mut writers: HashMap<String, std::io::BufWriter<std::fs::File>> = HashMap::new();
            while let Ok(msg) = rx.recv() {
                match msg {
                    WriterMsg::Event(first) => {
                        write_event(&mut writers, &first);
                        // Coalesce a burst into one flush. Close messages are
                        // handled inline — a try_recv that consumed one must
                        // not drop it.
                        loop {
                            match rx.try_recv() {
                                Ok(WriterMsg::Event(record)) => {
                                    write_event(&mut writers, &record);
                                }
                                Ok(WriterMsg::Close { run_id, ack }) => {
                                    if let Some(mut writer) = writers.remove(&run_id) {
                                        let _ = writer.flush();
                                    }
                                    let _ = ack.send(());
                                }
                                Ok(WriterMsg::CloseAll { ack }) => {
                                    for (_, mut writer) in writers.drain() {
                                        let _ = writer.flush();
                                    }
                                    let _ = ack.send(());
                                }
                                Err(_) => break,
                            }
                        }
                        for writer in writers.values_mut() {
                            let _ = writer.flush();
                        }
                    }
                    WriterMsg::Close { run_id, ack } => {
                        if let Some(mut writer) = writers.remove(&run_id) {
                            let _ = writer.flush();
                        }
                        let _ = ack.send(());
                    }
                    WriterMsg::CloseAll { ack } => {
                        for (_, mut writer) in writers.drain() {
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

/// Flush + close every writer and wait for the ack (bounded).
fn close_all_disk_writers() {
    let (ack_tx, ack_rx) = std::sync::mpsc::channel();
    if DISK_WRITER
        .send(WriterMsg::CloseAll { ack: ack_tx })
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
    if let Ok(buf) = RUN_EVENT_BUFFER.lock() {
        if let Some(events) = buf.get(run_id) {
            return events.clone();
        }
    }
    read_events_from_disk(run_id)
}

pub fn append_run_event(input: AppendRunEventInput) -> Result<RunEventRecord, crate::AppError> {
    // Re-delivery guard: a run's events arrive in strictly increasing sequence
    // from its single writer, so a sequence at/below the buffered high-water
    // mark is a replay overlap or a cross-writer race duplicate — return the
    // already-stored record instead of appending a second copy. (The
    // projection-snapshot replace path clears the buffer first, so it
    // re-appends from an empty slate and is unaffected.)
    if let Ok(buf) = RUN_EVENT_BUFFER.lock() {
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

/// Drop a settled run's in-memory events (called on `agent_end`). The persisted
/// log stays, so reads still work — this only bounds memory so a long-lived app
/// doesn't accumulate every run's events forever. The disk writer is flushed
/// FIRST (its ack guarantees the log is complete), so the post-clear disk
/// reads see every event the buffer held.
pub fn clear_run_event_buffer(run_id: &str) {
    close_disk_writer(run_id);
    if let Ok(mut buf) = RUN_EVENT_BUFFER.lock() {
        buf.remove(run_id);
    }
}

/// Delete a run's persisted event log (called when the run/thread is deleted).
/// The writer is closed first — Windows refuses to delete an open file.
pub fn delete_run_events_file(run_id: &str) {
    close_disk_writer(run_id);
    if let Some(path) = run_events_path(run_id) {
        let _ = std::fs::remove_file(path);
    }
    if let Ok(mut cache) = TOOL_PROJECTION_CACHE.lock() {
        cache.remove(run_id);
    }
}

/// Remove the whole run-events directory (called by `clear_all_data`).
pub fn clear_all_run_events_files() {
    close_all_disk_writers();
    if let Ok(dir) = app_dir() {
        let _ = std::fs::remove_dir_all(dir.join("run_events"));
    }
    if let Ok(mut buf) = RUN_EVENT_BUFFER.lock() {
        buf.clear();
    }
    if let Ok(mut cache) = TOOL_PROJECTION_CACHE.lock() {
        cache.clear();
    }
}

/// Per-run incremental tool-call projection. The context panel polls tool
/// calls for every run every 1.5s; rebuilding from the full event log each
/// time costs O(events) per run per poll (clone + JSON parse of every event).
/// Events are append-only, so the projection can advance over just the new
/// tail instead. State survives the run settling (the events are then
/// immutable on disk) and is dropped only when the run's log is deleted.
struct ToolProjectionState {
    tools: Vec<ToolCallRecord>,
    index_by_id: HashMap<String, usize>,
    /// Highest event sequence folded into `tools` so far.
    last_sequence: i64,
}

static TOOL_PROJECTION_CACHE: std::sync::LazyLock<
    std::sync::Mutex<HashMap<String, ToolProjectionState>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Bound the cache (entries are small: one record per tool call).
const TOOL_PROJECTION_CACHE_MAX: usize = 64;

pub fn list_tool_calls(run_id: &str) -> Result<Vec<ToolCallRecord>, crate::AppError> {
    Ok(tool_calls_for_run(run_id))
}

/// Tool calls for many runs in one call — backs the context panel's poll,
/// which used to fan out one IPC round-trip per run every 1.5s. Every run id
/// appears in the result (with an empty vec when it has no tool activity) so
/// the caller's `Object.fromEntries` shape is unchanged.
pub fn list_tool_calls_bulk(
    run_ids: &[String],
) -> Result<Vec<(String, Vec<ToolCallRecord>)>, crate::AppError> {
    Ok(run_ids
        .iter()
        .map(|run_id| (run_id.clone(), tool_calls_for_run(run_id)))
        .collect())
}

/// The run's tool calls, advancing the cached projection over only the events
/// appended since the last call. The cache mutex is held across the event
/// read — the lock order (tool cache → event buffer) is used nowhere in
/// reverse, so there's no deadlock cycle, and holding it prevents two
/// concurrent pollers from double-applying the same tail.
fn tool_calls_for_run(run_id: &str) -> Vec<ToolCallRecord> {
    let Ok(mut cache) = TOOL_PROJECTION_CACHE.lock() else {
        // Poisoned lock: fall back to a one-shot full rebuild.
        return tool_calls_from_events(&read_run_events(run_id));
    };
    if cache.len() >= TOOL_PROJECTION_CACHE_MAX && !cache.contains_key(run_id) {
        cache.clear();
    }
    let last_sequence = cache
        .get(run_id)
        .map(|state| state.last_sequence)
        .unwrap_or(-1);
    let new_events = list_run_events_since(run_id, last_sequence).unwrap_or_default();
    if new_events.is_empty() {
        return cache
            .get(run_id)
            .map(|state| state.tools.clone())
            .unwrap_or_default();
    }
    let state = cache
        .entry(run_id.to_string())
        .or_insert_with(|| ToolProjectionState {
            tools: Vec::new(),
            index_by_id: HashMap::new(),
            last_sequence: -1,
        });
    for event in &new_events {
        apply_tool_event(state, event);
        if event.sequence > state.last_sequence {
            state.last_sequence = event.sequence;
        }
    }
    state.tools.clone()
}

/// Reconstruct each tool call from its tool_start / tool_end events (the
/// tool_calls table was dropped). Both events carry the agent's stable tool
/// id, so pair by id — a single "current" slot would mispair overlapping
/// (parallel) tool calls.
fn tool_calls_from_events(events: &[RunEventRecord]) -> Vec<ToolCallRecord> {
    let mut state = ToolProjectionState {
        tools: Vec::new(),
        index_by_id: HashMap::new(),
        last_sequence: -1,
    };
    for event in events {
        apply_tool_event(&mut state, event);
    }
    state.tools
}

/// Fold one event into the tool-call projection (see [`tool_calls_from_events`]).
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
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    (name, kind, input)
}

/// Tool-call status from its `tool_end` payload. An explicit `error` is a
/// failure; so is a shell command that exits non-zero — the agent returns that
/// as a *successful* result with an `[exit: N]` footer on the last line of the output
/// (no error field), so the text must be inspected. A bare grep/diff/test
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

pub fn get_tool_call_input(
    run_id: &str,
    tool_call_id: &str,
) -> Result<Option<String>, crate::AppError> {
    // Look for the tool_start event whose stable tool id matches and return its
    // input/args (buffer while active, else the persisted log).
    let events = read_run_events(run_id);
    for event in events.iter().rev() {
        if event.event_type == "tool_start" && event_tool_id(event).as_deref() == Some(tool_call_id)
        {
            if let Some(ref payload) = event.payload {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
                    return Ok(v
                        .get("tool_args")
                        .or(v.get("input"))
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string()));
                }
            }
            return Ok(None);
        }
    }
    Ok(None)
}

/// Reconstruct a tool call's output from the run's `tool_end` event that
/// carries the same stable tool id. `tool_end` carries the result text and any
/// error (agent/mod.rs broadcasts `text`/`error`). Reads the buffer while the
/// run is active, else the persisted log, so the inspector's stdout/stderr
/// panes survive an app restart.
pub fn list_tool_outputs(
    run_id: &str,
    tool_call_id: &str,
) -> Result<Vec<ToolOutputRecord>, crate::AppError> {
    let events = read_run_events(run_id);
    for event in &events {
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

        return Ok(vec![ToolOutputRecord {
            id: event.id.clone(),
            tool_call_id: tool_call_id.to_string(),
            kind: if error.is_some() { "error" } else { "text" }.to_string(),
            content,
            created_at: event.created_at,
        }]);
    }
    Ok(vec![])
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

    #[test]
    fn has_run_events_tracks_buffer_contents() {
        let run_id = seed_event_buffer("has", 3);
        assert!(has_run_events(&run_id));
        clear_run_event_buffer(&run_id);
        // Buffer entry gone; no disk log was written by the seed, so absent.
        assert!(!has_run_events(&run_id));
    }

    /// Push one event into the process-global buffer (no disk writes).
    fn push_tool_event(run_id: &str, event_type: &str, payload: &str, sequence: i64) {
        let mut buf = RUN_EVENT_BUFFER.lock().expect("lock event buffer");
        buf.entry(run_id.to_string())
            .or_default()
            .push(RunEventRecord {
                id: format!("e{sequence}"),
                run_id: run_id.to_string(),
                event_type: event_type.to_string(),
                payload: Some(payload.to_string()),
                sequence,
                created_at: sequence,
            });
    }

    #[test]
    fn tool_calls_advance_incrementally_without_duplicates() {
        let run_id = format!("test_tools_{}", std::process::id());
        push_tool_event(
            &run_id,
            "tool_start",
            r#"{"tool_id":"t1","tool_name":"read","tool_args":"{\"path\":\"/a.ts\"}"}"#,
            0,
        );
        push_tool_event(&run_id, "tool_end", r#"{"tool_id":"t1","text":"ok"}"#, 1);

        let first = list_tool_calls(&run_id).expect("first read");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].status, "completed");

        // A second identical read must not duplicate the already-folded events.
        let second = list_tool_calls(&run_id).expect("second read");
        assert_eq!(second.len(), 1);

        // A later event is folded in on the next read.
        push_tool_event(
            &run_id,
            "tool_start",
            r#"{"tool_id":"t2","tool_name":"edit","tool_args":"{\"path\":\"/b.ts\"}"}"#,
            2,
        );
        let third = list_tool_calls(&run_id).expect("third read");
        assert_eq!(third.len(), 2);
        assert_eq!(third[1].name, "edit");
        assert_eq!(third[1].status, "running");

        clear_run_event_buffer(&run_id);
        if let Ok(mut cache) = TOOL_PROJECTION_CACHE.lock() {
            cache.remove(&run_id);
        }
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
}

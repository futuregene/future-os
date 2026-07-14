use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use rusqlite::params;
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
#[allow(dead_code)]
pub struct RunEventRecord {
    pub id: String,
    pub run_id: String,
    pub event_type: String,
    pub payload: Option<String>,
    pub sequence: i64,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
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
#[allow(dead_code)]
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

sql_record!(pub(super) TOOL_CALL_COLUMNS, tool_call_from_row -> ToolCallRecord {
    id, run_id, name, kind, input, status, started_at, ended_at, created_at,
});

// TOOL_OUTPUT_COLUMNS & tool_output_from_row removed — table dropped

pub fn create_run(input: CreateRunInput) -> Result<RunRecord, crate::AppError> {
    let id = create_id("run");
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
    loaded(get_run(&id)?, "Created run")
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

/// Which runs' still-open children (pending approvals, running tool calls) a
/// cancel-cascade settles. The two scopes differ only in how a child's owning
/// run is matched — everything else about the cascade is identical, which is why
/// [`cancel_children_of_runs`] is shared between the single-run and startup paths.
pub(super) enum CancelScope<'a> {
    /// One run: match children whose `run_id` equals this id.
    Run(&'a str),
    /// Startup convergence: match children whose run is already terminal — plus
    /// run-less orphan approvals (`run_id IS NULL`), which can never settle
    /// themselves once their collector is gone.
    TerminalRuns,
}

/// Cancel the still-open approvals and running tool calls belonging to `scope`,
/// stamping the cancelled approvals with `note`. Shared by the `cancelled` path
/// of [`update_run_status_if_active`] (single run) and cleanup's startup
/// convergence (every terminal run). The run-membership predicate is either a
/// bound parameter (single run) or a splice of the constant terminal-status list
/// — no caller value is ever string-interpolated.
pub(super) fn cancel_children_of_runs(
    tx: &rusqlite::Transaction<'_>,
    scope: CancelScope<'_>,
    note: &str,
    now: i64,
) -> rusqlite::Result<()> {
    let terminal_membership =
        format!("run_id IN (SELECT id FROM runs WHERE status IN ({TERMINAL_RUN_STATUSES_SQL}))");
    // `?1` = now, `?2` (approvals only) = note, `?3`/`?2` (single run only) = run id.
    let (approval_where, _tool_where) = match scope {
        CancelScope::Run(_) => ("run_id = ?3".to_string(), "run_id = ?2".to_string()),
        CancelScope::TerminalRuns => (
            format!("(run_id IS NULL OR {terminal_membership})"),
            terminal_membership.clone(),
        ),
    };
    let approval_sql = format!(
        "UPDATE approval_requests
             SET status = 'cancelled',
                 decision_note = COALESCE(decision_note, ?2),
                 decided_at = COALESCE(decided_at, ?1),
                 updated_at = ?1
             WHERE status = 'pending' AND {approval_where}"
    );
    // tool_calls table dropped — only cancel approvals.
    match scope {
        CancelScope::Run(run_id) => {
            tx.execute(&approval_sql, params![now, note, run_id])?;
        }
        CancelScope::TerminalRuns => {
            tx.execute(&approval_sql, params![now, note])?;
        }
    }
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
            CancelScope::Run(&input.run_id),
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
    Ok(affected > 0)
}

pub fn list_run_events(run_id: &str) -> Result<Vec<RunEventRecord>, crate::AppError> {
    Ok(read_run_events(run_id))
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

/// Append one event as a JSON line to the run's log (best-effort; a failed
/// write just means that event won't survive a restart).
fn persist_event_to_disk(record: &RunEventRecord) {
    let Some(path) = run_events_path(&record.run_id) else {
        return;
    };
    let Ok(line) = serde_json::to_string(record) else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
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
    persist_event_to_disk(&record);
    Ok(record)
}

/// Drop a settled run's in-memory events (called on `agent_end`). The persisted
/// log stays, so reads still work — this only bounds memory so a long-lived app
/// doesn't accumulate every run's events forever.
pub fn clear_run_event_buffer(run_id: &str) {
    if let Ok(mut buf) = RUN_EVENT_BUFFER.lock() {
        buf.remove(run_id);
    }
}

/// Delete a run's persisted event log (called when the run/thread is deleted).
pub fn delete_run_events_file(run_id: &str) {
    if let Some(path) = run_events_path(run_id) {
        let _ = std::fs::remove_file(path);
    }
}

/// Remove the whole run-events directory (called by `clear_all_data`).
pub fn clear_all_run_events_files() {
    if let Ok(dir) = app_dir() {
        let _ = std::fs::remove_dir_all(dir.join("run_events"));
    }
    if let Ok(mut buf) = RUN_EVENT_BUFFER.lock() {
        buf.clear();
    }
}

pub fn list_tool_calls(run_id: &str) -> Result<Vec<ToolCallRecord>, crate::AppError> {
    // Reconstruct each tool call from its tool_start / tool_end events (the
    // tool_calls table was dropped). Both events carry the agent's stable tool
    // id, so pair by id — a single "current" slot would mispair overlapping
    // (parallel) tool calls.
    let events = read_run_events(run_id);

    let mut tools: Vec<ToolCallRecord> = Vec::new();
    let mut index_by_id: HashMap<String, usize> = HashMap::new();

    for event in &events {
        match event.event_type.as_str() {
            "tool_start" | "toolcall_start" => {
                // Stable agent tool id (not the ephemeral event id) so the
                // inspector's `list_tool_outputs(run_id, tool.id)` can correlate
                // this call with its `tool_end` output.
                let id = event_tool_id(event).unwrap_or_else(|| event.id.clone());
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
                    let command = bash_command_from_input(tools[idx].input.as_deref());
                    tools[idx].status =
                        tool_end_status(event.payload.as_deref(), command.as_deref());
                    tools[idx].ended_at = Some(event.created_at);
                }
            }
            _ => {}
        }
    }
    Ok(tools)
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
    let name = v
        .get("tool_name")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let kind = name.clone(); // tool_name doubles as kind (bash, write, edit, read)
    let input = v
        .get("tool_args")
        .or(v.get("input"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    (name, kind, input)
}

/// Tool-call status from its `tool_end` payload. An explicit `error` is a
/// failure; so is a bash command that exits non-zero — the agent returns that
/// as a *successful* result with `[exit code: N]` baked into the output text
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

/// The non-zero code from a `[exit code: N]` bash prefix, or None (exit 0 / not
/// a bash result). Mirrors the agent-bridge persist logic.
fn nonzero_exit_code(output: Option<&str>) -> Option<i64> {
    let rest = output?.trim_start().strip_prefix("[exit code: ")?;
    let (code, _) = rest.split_once(']')?;
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

/// Extract the bash `command` from a tool call's persisted input JSON (used to
/// exempt soft-fail commands). Handles a doubly-encoded JSON string input.
fn bash_command_from_input(input: Option<&str>) -> Option<String> {
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
        if event_tool_id(event).as_deref() != Some(tool_call_id) {
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
             VALUES ('ap1', 'thread', 'run_live', 'bash', 'pending', 't', 1, 1)",
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
}

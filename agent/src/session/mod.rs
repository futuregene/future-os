//! Session management — 1:1 compatible with Go internal/session/

mod persistence;

pub use persistence::SessionPersistence;

use crate::types::{Message, ToolCall};
use crate::utils::{default_session_dir, generate_entry_id, generate_id};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub const CURRENT_SESSION_VERSION: i32 = 3;

// Entry type constants (matching Go)
pub const ENTRY_TYPE_USER: &str = "user";
pub const ENTRY_TYPE_ASSISTANT: &str = "assistant";
pub const ENTRY_TYPE_TOOL: &str = "tool";
pub const ENTRY_TYPE_SYSTEM: &str = "system";
pub const ENTRY_TYPE_COMPACTION: &str = "compaction";
pub const ENTRY_TYPE_MODEL_CHANGE: &str = "model_change";
pub const ENTRY_TYPE_LABEL: &str = "label";
pub const ENTRY_TYPE_SESSION_INFO: &str = "session_info";
pub const ENTRY_TYPE_THINKING_LEVEL_CHANGE: &str = "thinking_level_change";
pub const ENTRY_TYPE_CUSTOM: &str = "custom";
pub const ENTRY_TYPE_CUSTOM_MESSAGE: &str = "custom_message";
/// Run lifecycle markers. These bound a run in the append-only journal:
/// `run_started` is written durably with the accepted user message, and
/// `run_terminal` is written at the run's commit boundary. A `run_started`
/// with no matching `run_terminal` identifies a run interrupted by a crash or
/// agent restart (see the restart-recovery protocol). They carry no model
/// content and are filtered out of every conversation/context projection.
pub const ENTRY_TYPE_RUN_STARTED: &str = "run_started";
pub const ENTRY_TYPE_RUN_TERMINAL: &str = "run_terminal";

/// Terminal state recorded on a `run_terminal` marker.
pub const RUN_STATE_COMPLETED: &str = "completed";
pub const RUN_STATE_ERROR: &str = "error";
pub const RUN_STATE_CANCELLED: &str = "cancelled";
pub const RUN_STATE_INCOMPLETE: &str = "incomplete";
/// Recovered terminal state for a run that has a durable `run_started` marker
/// but no `run_terminal` — i.e. the agent crashed or restarted before the run
/// committed. Such a run must never be presented as completed.
pub const RUN_STATE_INTERRUPTED_BY_RESTART: &str = "interrupted_by_restart";

/// True for entry types that are run lifecycle markers rather than
/// conversation content. Forks skip these (they belong to the parent's runs)
/// and every context/display projection filters them out.
pub fn is_run_marker(entry_type: &str) -> bool {
    matches!(entry_type, ENTRY_TYPE_RUN_STARTED | ENTRY_TYPE_RUN_TERMINAL)
}

/// Scan a session's entries for a run that began (has a `run_started` marker)
/// but never committed (no matching `run_terminal`). Returns the run_id of the
/// most recent such unterminated run, if any.
///
/// Runs are sequential per session, so this tracks the currently-open run: set
/// on `run_started`, cleared on the matching `run_terminal`. Anything still open
/// at the end was interrupted — by a crash, an agent restart, or a kill — and
/// must be recovered as `InterruptedByRestart`, never faked as completed. A
/// session rebuilt by a full rewrite carries no markers and yields `None`.
pub fn find_unterminated_run(entries: &[SessionEntry]) -> Option<String> {
    let mut open: Option<String> = None;
    for entry in entries {
        match entry.entry_type.as_str() {
            ENTRY_TYPE_RUN_STARTED => {
                if let Some(run_id) = entry
                    .content
                    .as_ref()
                    .and_then(|c| c.get("run_id"))
                    .and_then(|v| v.as_str())
                {
                    open = Some(run_id.to_string());
                }
            }
            ENTRY_TYPE_RUN_TERMINAL => {
                if let Some(run_id) = entry
                    .content
                    .as_ref()
                    .and_then(|c| c.get("run_id"))
                    .and_then(|v| v.as_str())
                {
                    // A terminal marker closes its own run; only clear the open
                    // run if it matches, so a stray terminal can't mask an older
                    // unterminated run.
                    if open.as_deref() == Some(run_id) {
                        open = None;
                    }
                }
            }
            _ => {}
        }
    }
    open
}

/// Return the durable terminal payload for `run_id`, if the journal contains
/// one. Scans from the end so a later healing rewrite/commit wins over an older
/// marker. The returned value is the marker's `content` object.
pub fn find_run_terminal(entries: &[SessionEntry], run_id: &str) -> Option<serde_json::Value> {
    entries.iter().rev().find_map(|entry| {
        if entry.entry_type != ENTRY_TYPE_RUN_TERMINAL {
            return None;
        }
        let content = entry.content.as_ref()?;
        (content.get("run_id").and_then(|value| value.as_str()) == Some(run_id))
            .then(|| content.clone())
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(rename = "role", default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(rename = "tool_calls", default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(
        deserialize_with = "deserialize_timestamp_lenient",
        default = "default_timestamp"
    )]
    pub timestamp: DateTime<Local>,
    #[serde(
        rename = "tool_call_id",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_args: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thinking: String,
    /// Structured per-entry metadata (not model-visible). For user entries this
    /// carries `{ "attachments": [{ path, kind, name }] }` — the files the user
    /// attached, referenced by original absolute path (never copied). Populated
    /// from `AgentMessage.metadata`; absent on entries without metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

/// Lenient timestamp deserializer: tries standard ISO 8601 first, then
/// falls back to appending the local timezone offset when the string is
/// missing one (common in hand-edited or migrated JSONL files). If both
/// fail, returns the current local time so the session entry is at least
/// loadable rather than dropped silently.
fn deserialize_timestamp_lenient<'de, D>(deserializer: D) -> Result<DateTime<Local>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    // Standard ISO 8601 (with timezone).
    if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
        return Ok(dt.with_timezone(&chrono::Local));
    }
    // ISO 8601 with space separator (common variant).
    if let Ok(dt) = DateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S%.f%:z") {
        return Ok(dt.with_timezone(&chrono::Local));
    }
    if let Ok(dt) = DateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S%:z") {
        return Ok(dt.with_timezone(&chrono::Local));
    }
    // Try appending local timezone offset.
    let local_offset = chrono::Local::now().offset().to_string();
    let with_tz = format!("{s}{local_offset}");
    if let Ok(dt) = DateTime::parse_from_rfc3339(&with_tz) {
        tracing::warn!(
            "Session entry had timestamp without timezone (\"{s}\"); \
             repaired to \"{with_tz}\". Consider fixing the source file."
        );
        return Ok(dt.with_timezone(&chrono::Local));
    }
    // Last resort: current time so the entry isn't lost.
    tracing::warn!(
        "Session entry has unparseable timestamp (\"{s}\"); \
         falling back to current time."
    );
    Ok(chrono::Local::now())
}

fn default_timestamp() -> DateTime<Local> {
    chrono::Local::now()
}

impl SessionEntry {
    pub fn new_user(role: &str, content: serde_json::Value) -> Self {
        Self {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_USER.to_string(),
            role: role.to_string(),
            content: Some(content),
            tool_calls: vec![],
            timestamp: Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }

    pub fn new_assistant(content: serde_json::Value, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_ASSISTANT.to_string(),
            role: "assistant".to_string(),
            content: Some(content),
            tool_calls,
            timestamp: Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }

    pub fn new_tool(call_id: &str, content: &str) -> Self {
        Self {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_TOOL.to_string(),
            role: "tool".to_string(),
            content: Some(serde_json::json!(content)),
            tool_calls: vec![],
            timestamp: Local::now(),
            tool_call_id: call_id.to_string(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }

    /// Build the `session_info` metadata entry prepended to every saved session.
    /// `content` holds the token/cost/name JSON snapshot; `model`/`thinking_level`
    /// pin the session's active settings. All other fields take entry defaults.
    pub fn session_info(
        content: serde_json::Value,
        _model: String,
        _thinking_level: String,
    ) -> Self {
        Self {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_SESSION_INFO.to_string(),
            role: ENTRY_TYPE_SYSTEM.to_string(),
            content: Some(content),
            tool_calls: vec![],
            timestamp: Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }

    /// Marker written durably with the accepted user message to record that a
    /// run with this canonical id began. `content` carries `{ run_id, epoch }`.
    pub fn run_started(run_id: &str, epoch: u64) -> Self {
        Self {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_RUN_STARTED.to_string(),
            role: ENTRY_TYPE_SYSTEM.to_string(),
            content: Some(serde_json::json!({ "run_id": run_id, "epoch": epoch })),
            tool_calls: vec![],
            timestamp: Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }

    /// Marker written at a run's commit boundary. `content` carries
    /// `{ run_id, state, run_tokens, run_duration_ms }` plus `error` when
    /// `state` is `error`. A run is only recoverable as completed once this
    /// marker is durable.
    pub fn run_terminal(
        run_id: &str,
        state: &str,
        run_tokens: i64,
        run_duration_ms: i64,
        error: Option<&str>,
    ) -> Self {
        let mut content = serde_json::json!({
            "run_id": run_id,
            "state": state,
            "run_tokens": run_tokens,
            "run_duration_ms": run_duration_ms,
        });
        if let Some(error) = error {
            content["error"] = serde_json::Value::String(error.to_string());
        }
        Self {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_RUN_TERMINAL.to_string(),
            role: ENTRY_TYPE_SYSTEM.to_string(),
            content: Some(content),
            tool_calls: vec![],
            timestamp: Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub version: i32,
    pub cwd: String,
    pub model: String,
    #[serde(rename = "base_url")]
    pub base_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(
        rename = "parent_session_id",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub parent_session_id: String,
    #[serde(rename = "leaf_id", default, skip_serializing_if = "String::is_empty")]
    pub leaf_id: String,
    pub entries: Vec<SessionEntry>,
    #[serde(rename = "created_at")]
    pub created_at: DateTime<Local>,
    #[serde(rename = "updated_at")]
    pub updated_at: DateTime<Local>,
}

/// Summary of a session for listing (matches Go SessionSummary)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub cwd: String,
    #[serde(rename = "updated_at")]
    pub updated_at: DateTime<Local>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(
        rename = "parent_session_id",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub parent_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_message: Option<String>,
    #[serde(default)]
    pub query_count: usize,
}

impl Session {
    pub fn new(cwd: &str, model: &str, base_url: &str) -> Self {
        let now = Local::now();
        Self {
            id: generate_id(),
            version: CURRENT_SESSION_VERSION,
            cwd: cwd.to_string(),
            model: model.to_string(),
            base_url: base_url.to_string(),
            name: String::new(),
            parent_session_id: String::new(),
            leaf_id: String::new(),
            entries: vec![],
            created_at: now,
            updated_at: now,
        }
    }

    /// Assemble a full session snapshot for persistence: an existing `id` and its
    /// `entries` (already carrying the prepended `session_info`), stamped with the
    /// current time. Used by the prompt persist path where the id is known.
    pub fn snapshot(
        id: String,
        cwd: String,
        model: String,
        name: String,
        parent_session_id: String,
        entries: Vec<SessionEntry>,
    ) -> Self {
        let now = Local::now();
        Self {
            id,
            version: CURRENT_SESSION_VERSION,
            cwd,
            model,
            base_url: String::new(),
            name,
            parent_session_id,
            leaf_id: String::new(),
            entries,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn get_session_name(&self) -> &str {
        &self.name
    }

    pub fn set_session_name(&mut self, name: &str) {
        self.name = name.trim().to_string();
    }

    pub fn get_base_url(&self) -> &str {
        &self.base_url
    }

    pub fn set_base_url(&mut self, url: &str) {
        self.base_url = url.to_string();
    }

    pub fn get_session_info(&self) -> Option<&serde_json::Value> {
        // The last session_info entry is authoritative: the append-only commit
        // path appends a fresh complete snapshot at the end of each run rather
        // than rewriting the file, so the newest metadata is always last.
        self.entries
            .iter()
            .rev()
            .find(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
            .and_then(|e| e.content.as_ref())
    }
}

pub struct Manager {
    pub dir: PathBuf,
}

impl Manager {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn default_for(cwd: &str) -> Self {
        Self {
            dir: default_session_dir(cwd),
        }
    }

    fn session_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.jsonl", id))
    }

    /// Append one or more entries to the session JSONL without rewriting
    /// the file.  Each entry is written as a single `write_all` syscall
    /// (JSON + newline pre-assembled) so a crash mid-write at most loses
    /// the last entry rather than producing a partially-written line.
    pub fn append_entries(&self, session_id: &str, entries: &[SessionEntry]) -> Result<()> {
        self.with_session_write_lock(session_id, |path| {
            Self::append_entries_locked(path, entries, false)
        })
    }

    /// Append entries with an fsync durability boundary. Used by the run commit
    /// path so a successful return guarantees the terminal marker and refreshed
    /// session_info are on disk, not just in the page cache.
    pub fn append_entries_synced(&self, session_id: &str, entries: &[SessionEntry]) -> Result<()> {
        self.with_session_write_lock(session_id, |path| {
            Self::append_entries_locked(path, entries, true)
        })
    }

    /// Atomically recover any previously-open run and append the accepted user
    /// message plus the new run's start marker under the same session write
    /// lock. This prevents a failed recovery append followed by a successful
    /// `run_started` from hiding the older open run forever.
    ///
    /// This is only for an existing JSONL. Brand-new sessions are created by the
    /// full snapshot path, which has no previous lifecycle marker to recover.
    pub fn append_run_start(
        &self,
        session_id: &str,
        user_entry: SessionEntry,
        run_started: SessionEntry,
    ) -> Result<()> {
        self.with_session_write_lock(session_id, |path| {
            let file = File::open(path).context("open session file for run recovery")?;
            let mut open: Option<String> = None;
            for line in BufReader::new(file).lines() {
                let line = line.context("read session line for run recovery")?;
                match Self::cheap_entry_type(&line) {
                    Some(ENTRY_TYPE_RUN_STARTED) | Some(ENTRY_TYPE_RUN_TERMINAL) => {}
                    _ => continue,
                }
                let Ok(entry) = serde_json::from_str::<SessionEntry>(&line) else {
                    continue;
                };
                let Some(run_id) = entry
                    .content
                    .as_ref()
                    .and_then(|content| content.get("run_id"))
                    .and_then(|value| value.as_str())
                else {
                    continue;
                };
                match entry.entry_type.as_str() {
                    ENTRY_TYPE_RUN_STARTED => open = Some(run_id.to_string()),
                    ENTRY_TYPE_RUN_TERMINAL if open.as_deref() == Some(run_id) => open = None,
                    _ => {}
                }
            }

            let mut entries = Vec::with_capacity(if open.is_some() { 3 } else { 2 });
            if let Some(interrupted_run_id) = open {
                entries.push(SessionEntry::run_terminal(
                    &interrupted_run_id,
                    RUN_STATE_INTERRUPTED_BY_RESTART,
                    0,
                    0,
                    None,
                ));
            }
            entries.push(user_entry);
            entries.push(run_started);
            Self::append_entries_locked(path, &entries, true)
        })
    }

    /// Cheaply scan the session file for an unterminated run — a `run_started`
    /// marker with no matching `run_terminal` — parsing only the small marker
    /// lines so large tool/assistant lines are never deserialized. Used by the
    /// restart-recovery path to detect a run interrupted by crash/restart
    /// without loading (and repairing) the whole conversation. Returns
    /// `Ok(None)` when the file is absent or has no open run.
    pub fn unterminated_run_id(&self, session_id: &str) -> Result<Option<String>> {
        let path = self.session_path(session_id);
        if !path.exists() {
            return Ok(None);
        }
        // Shared lock so a concurrent full rewrite's temp->final rename can't
        // race the scan (same lock load/save use).
        let lock_path = path.with_extension("jsonl.lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&lock_path)
            .context("open session lock file")?;
        let file_lock = fd_lock::RwLock::new(lock_file);
        let _guard = file_lock.read().context("acquire session read lock")?;

        let file = File::open(&path).context("open session file")?;
        let mut open: Option<String> = None;
        for line in BufReader::new(file).lines() {
            let line = line.context("read session line")?;
            if line.trim().is_empty() {
                continue;
            }
            // Only run marker lines need parsing; skip everything else via the
            // cheap `"type"` prefix scan.
            match Self::cheap_entry_type(&line) {
                Some(ENTRY_TYPE_RUN_STARTED) | Some(ENTRY_TYPE_RUN_TERMINAL) => {}
                _ => continue,
            }
            let Ok(entry) = serde_json::from_str::<SessionEntry>(&line) else {
                continue;
            };
            let Some(run_id) = entry
                .content
                .as_ref()
                .and_then(|c| c.get("run_id"))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            match entry.entry_type.as_str() {
                ENTRY_TYPE_RUN_STARTED => open = Some(run_id.to_string()),
                ENTRY_TYPE_RUN_TERMINAL if open.as_deref() == Some(run_id) => open = None,
                _ => {}
            }
        }
        Ok(open)
    }

    /// Run `f` while holding the session's advisory write lock (the same lock
    /// save/append/load use), so a read-modify-append stays atomic with respect
    /// to concurrent writers.
    fn with_session_write_lock<T>(
        &self,
        session_id: &str,
        f: impl FnOnce(&Path) -> Result<T>,
    ) -> Result<T> {
        let path = self.session_path(session_id);
        if !path.exists() {
            return Err(anyhow::anyhow!("session file does not exist yet"));
        }
        let lock_path = path.with_extension("jsonl.lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&lock_path)
            .context("open session lock file")?;
        let mut file_lock = fd_lock::RwLock::new(lock_file);
        let _guard = file_lock.write().context("acquire session write lock")?;
        f(&path)
    }

    /// Append entries to a session file whose advisory write lock is already
    /// held. When `sync` is true the file is fsync'd before returning,
    /// providing an explicit durability boundary (the run commit point).
    fn append_entries_locked(path: &Path, entries: &[SessionEntry], sync: bool) -> Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .with_context(|| format!("open session file for append: {}", path.display()))?;
        for entry in entries {
            let json = serde_json::to_string(entry).context("serialize entry")?;
            let mut line = json.into_bytes();
            line.push(b'\n');
            file.write_all(&line).context("write entry")?;
        }
        file.flush().context("flush")?;
        if sync {
            file.sync_all().context("fsync session file")?;
        }
        Ok(())
    }

    /// Update one field of the authoritative (last) `session_info` snapshot by
    /// appending a fresh, complete `session_info` entry — no full-file rewrite.
    ///
    /// The append-only commit path relies on the last session_info being a full
    /// snapshot, so a metadata update merges the new key over the latest content
    /// and appends the result as the new authoritative entry. This is the safe
    /// metadata path while a run is active: `load()` repairs dangling tool calls
    /// in memory for LLM consumption, and persisting that repaired snapshot
    /// before the real tool result arrives would create a duplicate tool entry.
    pub fn update_session_info(
        &self,
        session_id: &str,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        self.with_session_write_lock(session_id, |path| {
            // Read only the authoritative (last) session_info content. Identify
            // candidate lines cheaply first so large tool/assistant lines are
            // never deserialized.
            let file = File::open(path).context("open session file")?;
            let mut latest_info: Option<serde_json::Value> = None;
            for line in BufReader::new(file).lines() {
                let line = line.context("read session line")?;
                if line.trim().is_empty() {
                    continue;
                }
                if Self::cheap_entry_type(&line) != Some(ENTRY_TYPE_SESSION_INFO) {
                    continue;
                }
                if let Ok(entry) = serde_json::from_str::<SessionEntry>(&line) {
                    if let Some(content) = entry.content {
                        latest_info = Some(content);
                    }
                }
            }
            let mut info = latest_info
                .and_then(|v| v.as_object().cloned())
                .ok_or_else(|| anyhow!("session {session_id} has no session_info object"))?;
            info.insert(key.to_string(), value);

            let entry = SessionEntry::session_info(
                serde_json::Value::Object(info),
                String::new(),
                String::new(),
            );
            Self::append_entries_locked(path, &[entry], false)
        })
    }

    pub fn save(&self, session: &Session) -> Result<()> {
        let path = self.session_path(&session.id);
        fs::create_dir_all(&self.dir).context("create session dir")?;

        // Acquire an advisory file lock so concurrent saves to the same
        // session are serialised.  Without this, two prompts finishing at the
        // same time race on the temp→final rename, causing "rename temp to
        // final" errors and potentially lost entries.
        let lock_path = path.with_extension("jsonl.lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&lock_path)
            .context("open session lock file")?;
        let mut file_lock = fd_lock::RwLock::new(lock_file);
        let _guard = file_lock.write().context("acquire session write lock")?;

        Self::write_entries_atomically(&path, &session.entries)
    }

    fn write_entries_atomically(path: &Path, entries: &[SessionEntry]) -> Result<()> {
        // Write to a temp file and rename atomically so a mid-write crash
        // never leaves a partially-written (corrupt) JSONL behind.
        let tmp_path = path.with_extension("jsonl.tmp");
        let file = File::create(&tmp_path).context("create temp session file")?;
        let mut w = std::io::BufWriter::new(file);
        for entry in entries {
            let json = serde_json::to_string(entry).context("serialize entry")?;
            writeln!(w, "{}", json).context("write entry")?;
        }
        w.flush().context("flush")?;
        // Force data to disk before rename so a crash cannot leave a
        // renamed-but-empty file behind (OS may defer writes in page cache).
        let file = w
            .into_inner()
            .map_err(|_| anyhow::anyhow!("flush failed"))?;
        file.sync_all().context("fsync temp session file")?;

        // On Windows an external locker (antivirus, Windows Search, OneDrive)
        // can briefly hold the target after fsync, causing rename to fail with
        // a sharing violation.  Exponential-backoff retry tolerates those
        // transient holds while keeping the advisory write lock — no reader can
        // enter until we release _guard, so the retry is bounded only by the
        // external locker's hold time.
        let mut rename_attempts = 0u32;
        loop {
            match fs::rename(&tmp_path, path) {
                Ok(()) => break,
                Err(e) if rename_attempts >= 5 => {
                    return Err(e).context("rename temp to final after 5 attempts");
                }
                Err(e) => {
                    rename_attempts += 1;
                    let wait_ms = 50u64 << rename_attempts; // 50, 100, 200, 400, 800
                    tracing::warn!(
                        "rename attempt {rename_attempts} failed for {}: {e}; retrying in {wait_ms}ms",
                        path.display(),
                    );
                    std::thread::sleep(std::time::Duration::from_millis(wait_ms));
                }
            }
        }

        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Session> {
        let path = self.session_path(id);
        self.load_path(&path, id)
    }

    /// Strip assistant entries that have neither content nor tool_calls —
    /// the LLM API rejects these with HTTP 400.  Returns true if any were removed.
    fn strip_empty_assistants(entries: &mut Vec<SessionEntry>) -> bool {
        let before = entries.len();
        entries.retain(|e| {
            e.entry_type != ENTRY_TYPE_ASSISTANT || e.content.is_some() || !e.tool_calls.is_empty()
        });
        entries.len() != before
    }

    /// Content prefix of the placeholder tool-result entries written by
    /// `repair_dangling_tool_calls`. Used to recognise placeholders so a
    /// later-arriving REAL tool result with the same tool_call_id can
    /// replace them (see `dedupe_tool_entries`).
    const TOOL_LOST_PLACEHOLDER_PREFIX: &'static str = "[Tool execution lost —";

    fn entry_text_starts_with(entry: &SessionEntry, prefix: &str) -> bool {
        match &entry.content {
            Some(serde_json::Value::String(s)) => s.starts_with(prefix),
            Some(serde_json::Value::Array(arr)) => arr
                .first()
                .and_then(|b| b.get("text"))
                .and_then(|t| t.as_str())
                .is_some_and(|s| s.starts_with(prefix)),
            _ => false,
        }
    }

    /// Remove duplicate tool-result entries that share the same tool_call_id.
    ///
    /// Duplicates arise when a load-time repair wrote a "tool execution lost"
    /// placeholder for a dangling tool_call while the original agent process
    /// was still mid-tool, and the real tool result was appended afterwards.
    /// Two tool messages with the same tool_call_id make the LLM API reject
    /// the request with HTTP 400 ("Messages with role 'tool' must be a
    /// response to a preceding message with 'tool_calls'").
    ///
    /// When one of the duplicates is a placeholder and the other is a real
    /// result, the placeholder is dropped; otherwise the first entry wins.
    fn dedupe_tool_entries(entries: &mut Vec<SessionEntry>) -> bool {
        use std::collections::{HashMap, HashSet};
        // tool_call_id -> index of the entry currently kept for that id
        let mut kept: HashMap<String, usize> = HashMap::new();
        let mut drop_idx: Vec<usize> = vec![];
        for (i, e) in entries.iter().enumerate() {
            if e.entry_type != ENTRY_TYPE_TOOL || e.tool_call_id.is_empty() {
                continue;
            }
            match kept.get(&e.tool_call_id) {
                None => {
                    kept.insert(e.tool_call_id.clone(), i);
                }
                Some(&prev) => {
                    let prev_is_placeholder = Self::entry_text_starts_with(
                        &entries[prev],
                        Self::TOOL_LOST_PLACEHOLDER_PREFIX,
                    );
                    let cur_is_placeholder =
                        Self::entry_text_starts_with(e, Self::TOOL_LOST_PLACEHOLDER_PREFIX);
                    if prev_is_placeholder && !cur_is_placeholder {
                        // The real result arrived after the placeholder was
                        // written — drop the placeholder, keep the real one.
                        drop_idx.push(prev);
                        kept.insert(e.tool_call_id.clone(), i);
                    } else {
                        drop_idx.push(i);
                    }
                }
            }
        }
        if drop_idx.is_empty() {
            return false;
        }
        tracing::warn!(
            "Removing {} duplicate tool-result entries from session (shared tool_call_id)",
            drop_idx.len()
        );
        let drop: HashSet<usize> = drop_idx.into_iter().collect();
        let mut i = 0;
        entries.retain(|_| {
            let keep = !drop.contains(&i);
            i += 1;
            keep
        });
        true
    }

    /// Find every assistant entry with tool_calls that lacks matching tool
    /// responses in the entries that follow it (up to the next non-tool entry
    /// or run-marker boundary).  An orphaned assistant can appear anywhere in
    /// the journal — not just at the end — when a crash happens mid-turn and a
    /// later restart appends run markers + new user messages ahead of it.
    /// Insert placeholder tool-result entries immediately after each orphaned
    /// assistant so the conversation stays API-valid.
    fn repair_dangling_tool_calls(entries: &mut Vec<SessionEntry>) -> bool {
        use std::collections::HashSet;
        if entries.is_empty() {
            return false;
        }

        // True for entry types that end a tool-response window: after one of
        // these, any pending tool_call_ids are orphaned and need placeholders.
        fn ends_tool_window(entry_type: &str) -> bool {
            matches!(
                entry_type,
                "user" | "system" | "assistant"
                    | ENTRY_TYPE_RUN_STARTED
                    | ENTRY_TYPE_RUN_TERMINAL
            )
        }

        // Collect (insertion_index, Vec<placeholder_entries>) pairs.
        // Process later so earlier insertions don't invalidate indices.
        let now = chrono::Local::now();
        let mut repairs: Vec<(usize, Vec<SessionEntry>)> = Vec::new();
        let mut i = 0;
        while i < entries.len() {
            let entry = &entries[i];
            if entry.entry_type != ENTRY_TYPE_ASSISTANT || entry.tool_calls.is_empty() {
                i += 1;
                continue;
            }
            let pending: HashSet<String> = entry
                .tool_calls
                .iter()
                .map(|tc| tc.id.clone())
                .collect();

            // Walk forward to find which tool_call_ids already have responses.
            let mut matched: HashSet<String> = HashSet::new();
            let mut j = i + 1;
            while j < entries.len() {
                let next = &entries[j];
                if next.entry_type == ENTRY_TYPE_TOOL
                    && pending.contains(&next.tool_call_id)
                {
                    matched.insert(next.tool_call_id.clone());
                }
                if ends_tool_window(&next.entry_type) {
                    break;
                }
                j += 1;
            }

            let missing: Vec<_> = pending.difference(&matched).cloned().collect();
            if !missing.is_empty() {
                let placeholders: Vec<SessionEntry> = entry
                    .tool_calls
                    .iter()
                    .filter(|tc| missing.contains(&tc.id))
                    .map(|tc| {
                        let placeholder = format!(
                            "{} {} was not executed before the session \
                             was interrupted]",
                            Self::TOOL_LOST_PLACEHOLDER_PREFIX,
                            tc.function.name,
                        );
                        SessionEntry {
                            id: crate::utils::generate_id(),
                            entry_type: ENTRY_TYPE_TOOL.to_string(),
                            role: "tool".to_string(),
                            content: Some(serde_json::Value::String(placeholder)),
                            tool_calls: vec![],
                            timestamp: now,
                            tool_call_id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            tool_args: String::new(),
                            thinking: String::new(),
                            meta: None,
                        }
                    })
                    .collect();
                // Insert right after the assistant (i + 1).
                repairs.push((i + 1, placeholders));
                // Skip past what we already scanned.
                i = j;
                continue;
            }
            i += 1;
        }

        if repairs.is_empty() {
            return false;
        }

        // Apply insertions in reverse index order to keep positions valid.
        repairs.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        for (idx, placeholders) in repairs {
            for placeholder in placeholders.into_iter().rev() {
                entries.insert(idx, placeholder);
            }
        }
        true
    }

    pub(crate) fn load_path(&self, path: &Path, id: &str) -> Result<Session> {
        // Acquire a shared (read) advisory lock so a concurrent save() —
        // which takes an exclusive (write) lock — cannot execute its
        // temp → final rename while we are reading.  Without this, a read
        // racing a rename on Windows can encounter a sharing violation or
        // a partially-replaced file when an external locker (antivirus,
        // Windows Search, OneDrive) briefly holds the target.
        let lock_path = path.with_extension("jsonl.lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&lock_path)
            .context("open session lock file")?;
        let file_lock = fd_lock::RwLock::new(lock_file);
        let _guard = file_lock.read().context("acquire session read lock")?;

        let file = File::open(path).context("open session file")?;
        let reader = BufReader::new(file);
        let mut entries = vec![];
        let mut raw_lines: Vec<String> = vec![];
        for line in reader.lines() {
            let line = line.context("read line")?;
            if line.trim().is_empty() {
                continue;
            }
            raw_lines.push(line);
        }
        if raw_lines.is_empty() {
            return Err(anyhow!("session {} has no entries", id));
        }
        // Try each line; if the last line fails to parse (partial write from
        // a crash during append), skip it instead of rejecting the whole session.
        let len = raw_lines.len();
        for (i, line) in raw_lines.into_iter().enumerate() {
            match serde_json::from_str::<SessionEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(e) if i == len - 1 => {
                    tracing::warn!(
                        "Dropping malformed last line of session {id} (possibly \
                         from a crash during append): {e}"
                    );
                }
                Err(e) => {
                    return Err(anyhow!("parse entry at line {}: {}", i + 1, e));
                }
            }
        }
        if entries.is_empty() {
            return Err(anyhow!("session {} has no entries", id));
        }
        // Heal common session corruptions IN MEMORY ONLY so the conversation
        // is API-valid on resume: strip empty assistants, drop duplicate tool
        // results, and patch dangling tool_calls with placeholders.
        //
        // The healed entries are deliberately NOT written back to the file
        // here.  load_path is called from many read-only paths (session list,
        // summaries, get_session_entries, fork/clone) that can run while the
        // owning agent process is still mid-turn.  Persisting a placeholder
        // for a dangling tool_call at that moment corrupts the file: when the
        // running tool finishes, its real result is appended with the same
        // tool_call_id, producing duplicate tool messages that the LLM API
        // rejects with HTTP 400.  The in-memory heal is idempotent and cheap,
        // and the owning session's next save() persists the healed state.
        let stripped = Self::strip_empty_assistants(&mut entries);
        let deduped = Self::dedupe_tool_entries(&mut entries);
        let repaired = Self::repair_dangling_tool_calls(&mut entries);
        if stripped || deduped || repaired {
            tracing::info!(
                "Healed session {id} in memory (stripped_empty={stripped}, \
                 deduped_tools={deduped}, repaired_dangling={repaired})"
            );
        }
        let created_at = entries[0].timestamp;
        let updated_at = entries.last().map(|e| e.timestamp).unwrap_or(created_at);
        let cwd = entries
            .iter()
            .rev()
            .find_map(|e| {
                if e.entry_type == ENTRY_TYPE_SESSION_INFO {
                    e.content
                        .as_ref()
                        .and_then(|v| v.get("cwd"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let model = entries
            .iter()
            .rev()
            .find_map(|e| {
                if e.entry_type == ENTRY_TYPE_MODEL_CHANGE {
                    e.content
                        .as_ref()
                        .and_then(|c| c.get("model"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .or_else(|| {
                // ASSISTANT entries never carry model (agent_message_to_entry
                // always sets it to ""), so fall back to the session_info entry.
                entries
                    .iter()
                    .rev()
                    .find(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
                    .and_then(|e| e.content.as_ref())
                    .and_then(|c| c.get("model"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();
        let name = entries
            .iter()
            .rev()
            .find(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
            .and_then(|e| e.content.as_ref())
            .and_then(|c| c.get("session_name"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let parent_session_id = entries
            .iter()
            .rev()
            .find_map(|e| {
                if e.entry_type == ENTRY_TYPE_SESSION_INFO {
                    e.content
                        .as_ref()
                        .and_then(|v| v.get("parent_session_id"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let session = Session {
            id: id.to_string(),
            version: CURRENT_SESSION_VERSION,
            cwd,
            model,
            base_url: String::new(),
            name,
            parent_session_id,
            leaf_id: String::new(),
            entries,
            created_at,
            updated_at,
        };
        Ok(session)
    }

    /// Extract the `"type":"..."` value from a serialized entry line without
    /// parsing the whole JSON.  `SessionEntry` serializes `id` first and
    /// `type` second (struct field order is deterministic), so the marker
    /// always appears within the first ~80 bytes regardless of how large the
    /// `content` payload is.
    fn cheap_entry_type(line: &str) -> Option<&str> {
        // Boundary-safe head slice: a multi-byte char may straddle byte 96.
        let head = line.get(..96).unwrap_or(line);
        let start = head.find("\"type\":\"")? + 8;
        let end = head[start..].find('"')? + start;
        Some(&head[start..end])
    }

    /// Extract the last `"timestamp":"..."` occurrence from a line without
    /// parsing the whole JSON.  Used for `updated_at` from the final entry,
    /// which may itself be a huge tool-result line.
    fn cheap_timestamp(line: &str) -> Option<DateTime<Local>> {
        let start = line.rfind("\"timestamp\":\"")? + 13;
        let end = line[start..].find('"')? + start;
        let ts = chrono::DateTime::parse_from_rfc3339(&line[start..end]).ok()?;
        Some(ts.with_timezone(&Local))
    }

    /// Extract the display text of a user entry's content (first text block),
    /// trimmed and truncated to ~40 visible columns for the session list.
    fn summary_first_message(entry: &SessionEntry) -> Option<String> {
        let content_val = entry.content.as_ref()?;
        let text: String = if let Some(arr) = content_val.as_array() {
            // First text block only — a later one is the agent-injected
            // attachment-path list, not the user's message.
            arr.iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .next()
                .unwrap_or("")
                .to_string()
        } else if let Some(s) = content_val.as_str() {
            s.to_string()
        } else {
            String::new()
        };
        let truncated: String = truncate_visible(text.trim(), 40);
        if truncated.is_empty() {
            None
        } else {
            Some(truncated)
        }
    }

    /// Build a summary from a fully-loaded session (fallback path for files
    /// whose structure the cheap scanner doesn't recognise).
    fn summary_from_session(sess: &Session) -> SessionSummary {
        let mut first_message: Option<String> = None;
        let mut query_count: usize = 0;
        let mut session_info_name: Option<String> = None;
        for entry in &sess.entries {
            if entry.role == "user" {
                query_count += 1;
                if first_message.is_none() {
                    first_message = Self::summary_first_message(entry);
                }
            } else if entry.entry_type == ENTRY_TYPE_SESSION_INFO {
                // Last non-empty session_name wins (append-only commits add a
                // fresh session_info per run; a rename shows up in a later one).
                if let Some(ref content_val) = entry.content {
                    if let Some(n) = content_val.get("session_name").and_then(|v| v.as_str()) {
                        let trimmed = n.trim();
                        if !trimmed.is_empty() {
                            session_info_name = Some(trimmed.to_string());
                        }
                    }
                }
            }
        }
        SessionSummary {
            id: sess.id.clone(),
            cwd: sess.cwd.clone(),
            updated_at: sess.updated_at,
            model: sess.model.clone(),
            name: if !sess.name.is_empty() {
                Some(sess.name.clone())
            } else {
                session_info_name
            },
            parent_session_id: sess.parent_session_id.clone(),
            first_message,
            query_count,
        }
    }

    /// Build a SessionSummary by scanning the JSONL cheaply: fully parse only
    /// the small metadata lines (session_info / model_change / label) and the
    /// first user entry; every other line is inspected via a `"type"` prefix
    /// scan, so multi-hundred-KB tool/assistant lines are never deserialized.
    /// Returns None when the file has no usable session_info or a metadata
    /// line fails to parse — callers should fall back to a full `load_path`.
    fn read_summary(&self, path: &Path, id: &str) -> Option<SessionSummary> {
        let file = File::open(path).ok()?;
        let reader = BufReader::new(file);
        let mut cwd = String::new();
        let mut model = String::new();
        let mut name = String::new();
        let mut parent_session_id = String::new();
        let mut first_message: Option<String> = None;
        let mut query_count: usize = 0;
        let mut saw_session_info = false;
        let mut last_line = String::new();

        for line in reader.lines() {
            let line = line.ok()?;
            if line.trim().is_empty() {
                continue;
            }
            last_line = line;
            match Self::cheap_entry_type(&last_line) {
                Some(ENTRY_TYPE_SESSION_INFO) => {
                    let e: SessionEntry = serde_json::from_str(&last_line).ok()?;
                    saw_session_info = true;
                    if let Some(ref content) = e.content {
                        if let Some(c) = content.get("cwd").and_then(|v| v.as_str()) {
                            cwd = c.to_string();
                        }
                        if let Some(n) = content
                            .get("session_name")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            // Last non-empty wins: the append-only commit path
                            // appends a fresh session_info per run, so the newest
                            // name (e.g. after a rename) is the last one.
                            name = n.to_string();
                        }
                        if let Some(p) = content.get("parent_session_id").and_then(|v| v.as_str()) {
                            parent_session_id = p.to_string();
                        }
                    }
                    if let Some(ref content) = e.content {
                        if let Some(m) = content
                            .get("model")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                        {
                            model = m.to_string(); // last non-empty wins
                        }
                    }
                }
                Some(ENTRY_TYPE_MODEL_CHANGE) => {
                    let e: SessionEntry = serde_json::from_str(&last_line).ok()?;
                    if let Some(ref content) = e.content {
                        if let Some(m) = content.get("model").and_then(|v| v.as_str()) {
                            model = m.to_string(); // last one wins
                        }
                    }
                }
                Some(ENTRY_TYPE_USER) => {
                    query_count += 1;
                    if first_message.is_none() {
                        if let Ok(e) = serde_json::from_str::<SessionEntry>(&last_line) {
                            first_message = Self::summary_first_message(&e);
                        }
                    }
                }
                _ => {}
            }
        }

        if !saw_session_info || last_line.is_empty() {
            return None;
        }
        // updated_at: timestamp of the final entry (cheap extraction — the
        // last line may be a huge tool result), falling back to file mtime.
        let updated_at = Self::cheap_timestamp(&last_line).or_else(|| {
            std::fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .map(DateTime::<Local>::from)
        })?;

        Some(SessionSummary {
            id: id.to_string(),
            cwd,
            updated_at,
            model,
            name: if name.is_empty() { None } else { Some(name) },
            parent_session_id,
            first_message,
            query_count,
        })
    }

    /// List sessions for a cwd as lightweight summaries (no full JSONL
    /// parse per file — see `read_summary`).
    pub fn list_summaries(&self, cwd: &str) -> Result<Vec<SessionSummary>> {
        let mut summaries = self.list_all()?;
        if !cwd.is_empty() {
            summaries.retain(|s| s.cwd == cwd);
        }
        Ok(summaries)
    }

    /// List all sessions in the flat sessions directory.
    ///
    /// Files are scanned in parallel (each JSONL is an independent cheap
    /// line-scan via `read_summary`); with thousands of sessions on disk a
    /// sequential scan is the dominant startup cost for the GUI/TUI list.
    pub fn list_all(&self) -> Result<Vec<SessionSummary>> {
        if !self.dir.exists() {
            return Ok(vec![]);
        }
        let mut paths = vec![];
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                paths.push(path);
            }
        }
        let mut summaries = if paths.len() <= 8 {
            // Few files: thread-spawn overhead outweighs the scan time.
            let mut summaries = vec![];
            for path in &paths {
                self.try_push_summary(path, &mut summaries);
            }
            summaries
        } else {
            // Work-stealing over a shared index; capped worker count to
            // avoid I/O thrash on spinning disks.
            let workers = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(8)
                .min(paths.len());
            let next = std::sync::atomic::AtomicUsize::new(0);
            let per_worker: Vec<Vec<SessionSummary>> = std::thread::scope(|s| {
                let handles: Vec<_> = (0..workers)
                    .map(|_| {
                        s.spawn(|| {
                            let mut local = vec![];
                            loop {
                                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                if i >= paths.len() {
                                    break;
                                }
                                self.try_push_summary(&paths[i], &mut local);
                            }
                            local
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or_default())
                    .collect()
            });
            per_worker.into_iter().flatten().collect()
        };
        summaries.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        Ok(summaries)
    }

    fn try_push_summary(&self, path: &Path, summaries: &mut Vec<SessionSummary>) {
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            return;
        }
        let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        // Fast path: cheap line scan that never deserializes large
        // tool/assistant payloads.  Falls back to a full load for files the
        // scanner can't handle (legacy layouts, missing session_info).
        if let Some(summary) = self.read_summary(path, id) {
            summaries.push(summary);
        } else if let Ok(sess) = self.load_path(path, id) {
            summaries.push(Self::summary_from_session(&sess));
        }
    }

    /// Find a session by ID in the flat sessions directory
    pub fn find(&self, id: &str) -> Option<PathBuf> {
        let path = self.session_path(id);
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    /// Delete a session file
    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.session_path(id);
        // Also remove the lock file if present — no session means no lock.
        let lock_path = path.with_extension("jsonl.lock");
        let _ = fs::remove_file(&lock_path);
        fs::remove_file(path).map_err(|e| anyhow!("failed to delete session: {}", e))
    }
}

pub fn fork_session(parent: &Session, from_entry_id: &str) -> Session {
    let chain = for_each_entry(&parent.entries, from_entry_id);
    // If from_entry_id wasn't found, for_each_entry returns every entry
    // (never hits the break).  Guard against a bad entry ID silently
    // producing an unforked clone.
    if chain.is_empty() || chain.last().map(|e| e.id.as_str()) != Some(from_entry_id) {
        // Fall back to cloning the whole session without a cut — better
        // than losing history.  Callers should validate the entry ID first.
        tracing::warn!(
            "fork point {from_entry_id} not found in session {}; cloning without a cut",
            parent.id
        );
    }
    let mut entries: Vec<SessionEntry> = chain.into_iter().cloned().collect();
    for e in &mut entries {
        e.id = generate_entry_id();
    }
    // Read parent metadata from the authoritative (last) session_info snapshot.
    // The append-only commit path appends a fresh session_info per run, so the
    // fork must inherit the parent's CURRENT model/name/thinking level (last),
    // not the values recorded at session creation (first). The values live on
    // the SessionEntry struct fields (model, thinking_level) and also inside
    // the content JSON (created_by, session_name).
    let parent_info = parent
        .entries
        .iter()
        .rev()
        .find(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO);

    // Prefer the parent's actual level: the session_info struct field, then the
    // content JSON (forked parents carry it there) — only fall back to a literal
    // when neither is set, so a `low`/`medium` parent doesn't silently fork to
    // `high`.
    let parent_thinking_level = parent_info
        .and_then(|e| e.content.as_ref())
        .and_then(|c| c.get("thinking_level"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("high");

    let parent_model = parent_info
        .and_then(|e| e.content.as_ref())
        .and_then(|c| c.get("model"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&parent.model)
        .to_string();

    let parent_created_by = parent_info
        .and_then(|e| e.content.as_ref())
        .and_then(|c| c.get("created_by"))
        .and_then(|v| v.as_str())
        .unwrap_or("tui");

    // Derive fork name: read from session_info content.
    let parent_name = parent_info
        .and_then(|e| e.content.as_ref())
        .and_then(|c| c.get("session_name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&parent.name);
    let fork_name = if parent_name.is_empty() {
        "(fork)".to_string()
    } else {
        format!("{} (fork)", parent_name)
    };

    // Prepend session_info with metadata so the forked session carries
    // model, thinking level, parent id, and the fork name.
    let info = serde_json::json!({
        "cwd": parent.cwd,
        "session_name": fork_name,
        "parent_session_id": parent.id,
        "created_by": parent_created_by,
        "model": parent_model,
        "thinking_level": parent_thinking_level,
    });
    entries.insert(
        0,
        SessionEntry {
            id: generate_entry_id(),
            entry_type: ENTRY_TYPE_SESSION_INFO.to_string(),
            role: "system".to_string(),
            content: Some(info),
            tool_calls: vec![],
            timestamp: Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        },
    );
    let now = Local::now();
    Session {
        id: generate_id(),
        version: CURRENT_SESSION_VERSION,
        cwd: parent.cwd.clone(),
        model: parent_model.clone(),
        base_url: parent.base_url.clone(),
        name: fork_name,
        parent_session_id: parent.id.clone(),
        leaf_id: String::new(),
        entries,
        created_at: now,
        updated_at: now,
    }
}

fn for_each_entry<'a>(entries: &'a [SessionEntry], from_id: &str) -> Vec<&'a SessionEntry> {
    // Include all entries from the beginning up to and including from_id,
    // skipping the original session_info (fork_session prepends its own) and
    // run lifecycle markers (they belong to the parent's runs, not the fork).
    let mut result = vec![];
    for e in entries.iter() {
        if e.entry_type != ENTRY_TYPE_SESSION_INFO && !is_run_marker(&e.entry_type) {
            result.push(e);
        }
        if e.id == from_id {
            break;
        }
    }
    result
}

/// Rebuild in-memory messages from persisted entries when a session is loaded
/// (new_session restore / fork). `model_supports_images` gates image
/// re-hydration: GUI image attachments have their base64 stripped from the JSONL
/// (to keep it small — see `agent_message_to_entry`) and are re-read from their
/// on-disk paths here so the model still sees them after a reload. Legacy
/// `images`-field base64 (TUI / channels) is kept on disk and preserved as-is.
pub fn entries_to_agent_messages(
    entries: &[SessionEntry],
    model_supports_images: bool,
) -> Vec<crate::types::AgentMessage> {
    use crate::types::{AgentToolCall, ContentBlock};
    let mut msgs = vec![];
    for entry in entries {
        let role = match entry.entry_type.as_str() {
            "user" | "system" | "assistant" | "tool" => entry.entry_type.clone(),
            _ => continue,
        };

        let mut content: Vec<ContentBlock> = match &entry.content {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| match v.get("type").and_then(|t| t.as_str()) {
                    Some("image_url") => {
                        // Preserve an on-disk base64 image_url (channels/TUI); a
                        // stripped/empty one (GUI) is skipped — rebuilt from meta.
                        let url = v
                            .get("image_url")
                            .and_then(|u| u.get("url"))
                            .and_then(|u| u.as_str())
                            .unwrap_or("");
                        (!url.is_empty()).then(|| ContentBlock::image(url))
                    }
                    _ => {
                        let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("");
                        Some(ContentBlock::Text {
                            text: text.to_string(),
                        })
                    }
                })
                .collect(),
            Some(serde_json::Value::String(s)) => {
                vec![ContentBlock::Text { text: s.clone() }]
            }
            _ => vec![],
        };

        // Re-hydrate GUI image attachments from their paths (base64 was stripped
        // from the JSONL). Skipped for text-only models — they never got the
        // image; the file-path text block (if any) is already in `content`.
        if model_supports_images {
            if let Some(atts) = entry
                .meta
                .as_ref()
                .and_then(|m| m.get("attachments"))
                .and_then(|a| a.as_array())
            {
                for att in atts {
                    if att.get("kind").and_then(|k| k.as_str()) != Some("image") {
                        continue;
                    }
                    if let Some(path) = att.get("path").and_then(|p| p.as_str()) {
                        if let Some(url) = crate::utils::image_data_url_for_model(path) {
                            content.push(ContentBlock::image(url));
                        }
                    }
                }
            }
        }

        let tool_calls: Vec<AgentToolCall> = entry
            .tool_calls
            .iter()
            .map(|tc| AgentToolCall {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                args: tc.function.arguments.clone(),
            })
            .collect();

        msgs.push(crate::types::AgentMessage {
            role,
            content,
            thinking: entry.thinking.clone(),
            tool_calls,
            tool_call_id: entry.tool_call_id.clone(),
            name: entry.name.clone(),
            tool_args: entry.tool_args.clone(),
            metadata: entry.meta.as_ref().and_then(|m| m.as_object().cloned()),
        });
    }
    msgs
}

/// Build context messages from session entries (matching Go BuildContext)
pub fn build_context(entries: &[SessionEntry]) -> Vec<Message> {
    let mut msgs = vec![];
    for entry in entries {
        let role = match entry.entry_type.as_str() {
            "user" | "system" => entry.entry_type.clone(),
            "assistant" => "assistant".to_string(),
            "tool" => "tool".to_string(),
            _ => continue,
        };

        let content = entry.content.clone().unwrap_or(serde_json::Value::Null);
        let tool_calls: Vec<ToolCall> = entry.tool_calls.clone();
        msgs.push(Message {
            role,
            content: Some(content),
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: entry.tool_call_id.clone(),
            name: String::new(),
            tool_args: String::new(),
            reasoning_content: entry.thinking.clone(),
        });
    }
    msgs
}

/// Convert AgentMessage back to SessionEntry for persistence
pub fn agent_message_to_entry(msg: &crate::types::AgentMessage) -> SessionEntry {
    let entry_type = match msg.role.as_str() {
        "user" => ENTRY_TYPE_USER,
        "assistant" => ENTRY_TYPE_ASSISTANT,
        "tool" => ENTRY_TYPE_TOOL,
        "system" => ENTRY_TYPE_SYSTEM,
        _ => ENTRY_TYPE_USER,
    };

    // A GUI message records its images in `meta`, so their (multi-MB) base64
    // image_url blocks are redundant on disk — drop them to keep the JSONL small;
    // entries_to_agent_messages re-reads them from the attachment paths on load.
    // Legacy `images`-field images (TUI / channels) have no meta and are kept.
    let strip_image_blocks = msg
        .metadata
        .as_ref()
        .and_then(|m| m.get("attachments"))
        .and_then(|a| a.as_array())
        .is_some_and(|arr| {
            arr.iter()
                .any(|a| a.get("kind").and_then(|k| k.as_str()) == Some("image"))
        });
    let content_blocks: Vec<serde_json::Value> = msg
        .content
        .iter()
        .map(|b| serde_json::to_value(b).unwrap_or(serde_json::Value::Null))
        .filter(|v| {
            !(strip_image_blocks && v.get("type").and_then(|t| t.as_str()) == Some("image_url"))
        })
        .collect();
    let content = if content_blocks.is_empty() {
        None
    } else {
        Some(serde_json::Value::Array(content_blocks))
    };

    let tool_calls: Vec<crate::types::ToolCall> = msg
        .tool_calls
        .iter()
        .map(|tc| crate::types::ToolCall {
            id: tc.id.clone(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: tc.name.clone(),
                arguments: tc.args.clone(),
            },
        })
        .collect();

    SessionEntry {
        id: generate_entry_id(),
        entry_type: entry_type.to_string(),
        role: msg.role.clone(),
        content,
        tool_calls,
        timestamp: Local::now(),
        tool_call_id: msg.tool_call_id.clone(),
        name: msg.name.clone(),
        tool_args: msg.tool_args.clone(),
        thinking: msg.thinking.clone(),
        // Populated at the save site (session_prompt.rs): only the final
        // assistant entry of a run gets a non-zero value, and prior entries'
        // values are preserved from the previously-saved session.
        // Carry structured metadata (e.g. user attachments) into the JSONL so it
        // survives reload; the reverse mapping restores it in
        // entries_to_agent_messages.
        meta: msg.metadata.clone().map(serde_json::Value::Object),
    }
}

/// Truncate a string to max_vis visible columns. CJK characters count as 2,
/// everything else as 1. Matches approximate terminal rendering width.
pub fn truncate_visible(s: &str, max_vis: usize) -> String {
    let mut vis: usize = 0;
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        let w = if ('\u{1100}'..='\u{115f}').contains(&ch)   // Hangul Jamo
            || ('\u{2e80}'..='\u{a4cf}').contains(&ch)       // CJK radicals + Yi
            || ('\u{ac00}'..='\u{d7a3}').contains(&ch)       // Hangul Syllables
            || ('\u{f900}'..='\u{faff}').contains(&ch)       // CJK Compatibility
            || ('\u{fe30}'..='\u{fe4f}').contains(&ch)       // CJK Compatibility Forms
            || ('\u{ff00}'..='\u{ffef}').contains(&ch)       // Fullwidth Forms
            || ('\u{1f300}'..='\u{1f5ff}').contains(&ch)     // Misc Symbols
            || ('\u{1f900}'..='\u{1f9ff}').contains(&ch)     // Supplemental Symbols
            || ('\u{1f600}'..='\u{1f64f}').contains(&ch)     // Emoticons
            || ('\u{20000}'..='\u{2fffd}').contains(&ch)     // SIP
            || ('\u{30000}'..='\u{3fffd}').contains(&ch)
        // TIP
        {
            2
        } else {
            1
        };
        if vis + w > max_vis {
            break;
        }
        vis += w;
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── truncate_visible ───────────────────────────────────────────────────

    #[test]
    fn truncate_visible_ascii() {
        assert_eq!(truncate_visible("hello world", 5), "hello");
    }

    #[test]
    fn truncate_visible_cjk() {
        assert_eq!(truncate_visible("你好世界", 4), "你好");
    }

    #[test]
    fn truncate_visible_mixed() {
        assert_eq!(truncate_visible("ab你好cd", 4), "ab你");
    }

    #[test]
    fn truncate_visible_emoji() {
        assert_eq!(truncate_visible("a🦀b", 3), "a🦀");
    }

    #[test]
    fn truncate_visible_exact_fit() {
        assert_eq!(truncate_visible("hello", 5), "hello");
    }

    #[test]
    fn truncate_visible_zero() {
        assert_eq!(truncate_visible("hello", 0), "");
    }

    #[test]
    fn truncate_visible_empty_string() {
        assert_eq!(truncate_visible("", 10), "");
    }

    #[test]
    fn truncate_visible_cjk_never_splits() {
        // "中" is 2 cols. 3 cols budget → only "中" fits
        assert_eq!(truncate_visible("中文中文", 3), "中");
    }

    // ─── SessionEntry constructors ──────────────────────────────────────────

    #[test]
    fn new_user_entry() {
        let e = SessionEntry::new_user("user", serde_json::json!("hello"));
        assert_eq!(e.entry_type, ENTRY_TYPE_USER);
        assert_eq!(e.role, "user");
        assert!(!e.id.is_empty());
    }

    #[test]
    fn new_assistant_entry() {
        let tool_calls = vec![crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "shell".to_string(),
                arguments: serde_json::json!({"cmd": "ls"}),
            },
        }];
        let e = SessionEntry::new_assistant(serde_json::json!("answer"), tool_calls);
        assert_eq!(e.entry_type, ENTRY_TYPE_ASSISTANT);
        assert_eq!(e.role, "assistant");
        assert_eq!(e.tool_calls.len(), 1);
        assert_eq!(e.tool_calls[0].function.name, "shell");
    }

    #[test]
    fn new_tool_entry() {
        let e = SessionEntry::new_tool("call_123", "file contents here");
        assert_eq!(e.entry_type, ENTRY_TYPE_TOOL);
        assert_eq!(e.role, "tool");
        assert_eq!(e.tool_call_id, "call_123");
    }

    #[test]
    fn session_info_entry() {
        let content = serde_json::json!({"session_name": "test", "model": "gpt-4o", "thinking_level": "high"});
        let e = SessionEntry::session_info(content, "gpt-4o".to_string(), "high".to_string());
        assert_eq!(e.entry_type, ENTRY_TYPE_SESSION_INFO);
        assert_eq!(e.role, ENTRY_TYPE_SYSTEM);
        let c = e.content.as_ref().unwrap();
        assert_eq!(c["model"], "gpt-4o");
        assert_eq!(c["thinking_level"], "high");
    }

    #[test]
    fn entry_ids_are_unique() {
        let e1 = SessionEntry::new_user("user", serde_json::json!("a"));
        let e2 = SessionEntry::new_user("user", serde_json::json!("b"));
        assert_ne!(e1.id, e2.id);
    }

    // ─── Session basics ─────────────────────────────────────────────────────

    #[test]
    fn session_new_fields() {
        let s = Session::new("/tmp/test", "gpt-4o", "https://api.openai.com");
        assert_eq!(s.cwd, "/tmp/test");
        assert_eq!(s.model, "gpt-4o");
        assert_eq!(s.base_url, "https://api.openai.com");
        assert!(s.entries.is_empty());
        assert!(s.parent_session_id.is_empty());
    }

    #[test]
    fn session_name_get_set() {
        let mut s = Session::new("/tmp", "model", "");
        assert_eq!(s.get_session_name(), "");
        s.set_session_name("My Chat");
        assert_eq!(s.get_session_name(), "My Chat");
    }

    #[test]
    fn session_base_url_get_set() {
        let mut s = Session::new("/tmp", "model", "https://old.com");
        assert_eq!(s.get_base_url(), "https://old.com");
        s.set_base_url("https://new.com");
        assert_eq!(s.get_base_url(), "https://new.com");
    }

    // ─── build_context ──────────────────────────────────────────────────────

    #[test]
    fn build_context_from_entries() {
        let entries = vec![
            SessionEntry::new_user("user", serde_json::json!("hello")),
            SessionEntry::new_assistant(serde_json::json!("hi"), vec![]),
            SessionEntry::new_tool("c1", "output"),
        ];
        let msgs = build_context(&entries);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[2].role, "tool");
        assert_eq!(msgs[2].tool_call_id, "c1");
    }

    #[test]
    fn build_context_skips_non_message_types() {
        let mut compaction = SessionEntry::new_user("user", serde_json::json!("x"));
        compaction.entry_type = ENTRY_TYPE_COMPACTION.to_string();
        let entries = vec![
            SessionEntry::new_user("user", serde_json::json!("hello")),
            compaction,
        ];
        let msgs = build_context(&entries);
        assert_eq!(msgs.len(), 1); // compaction skipped
    }

    #[test]
    fn build_context_preserves_thinking() {
        let mut e = SessionEntry::new_assistant(serde_json::json!("answer"), vec![]);
        e.thinking = "let me think...".to_string();
        let msgs = build_context(&[e]);
        assert_eq!(msgs[0].reasoning_content, "let me think...");
    }

    #[test]
    fn build_context_empty_entries() {
        let msgs = build_context(&[]);
        assert!(msgs.is_empty());
    }

    // ─── agent_message_to_entry ─────────────────────────────────────────────

    #[test]
    fn agent_message_to_entry_user() {
        let msg = crate::types::AgentMessage {
            role: "user".to_string(),
            content: vec![crate::types::ContentBlock::text("hello")],
            ..Default::default()
        };
        let entry = agent_message_to_entry(&msg);
        assert_eq!(entry.entry_type, ENTRY_TYPE_USER);
        assert_eq!(entry.role, "user");
    }

    #[test]
    fn agent_message_to_entry_assistant_with_tool_calls() {
        let msg = crate::types::AgentMessage {
            role: "assistant".to_string(),
            content: vec![crate::types::ContentBlock::text("answer")],
            tool_calls: vec![crate::types::AgentToolCall {
                id: "c1".to_string(),
                name: "shell".to_string(),
                args: serde_json::json!({"cmd": "ls"}),
            }],
            ..Default::default()
        };
        let entry = agent_message_to_entry(&msg);
        assert_eq!(entry.entry_type, ENTRY_TYPE_ASSISTANT);
        assert_eq!(entry.tool_calls.len(), 1);
        assert_eq!(entry.tool_calls[0].function.name, "shell");
    }

    #[test]
    fn agent_message_to_entry_tool() {
        let msg = crate::types::AgentMessage {
            role: "tool".to_string(),
            content: vec![crate::types::ContentBlock::text("result")],
            tool_call_id: "c1".to_string(),
            ..Default::default()
        };
        let entry = agent_message_to_entry(&msg);
        assert_eq!(entry.entry_type, ENTRY_TYPE_TOOL);
        assert_eq!(entry.tool_call_id, "c1");
    }

    #[test]
    fn agent_message_to_entry_preserves_thinking() {
        let msg = crate::types::AgentMessage {
            role: "assistant".to_string(),
            content: vec![crate::types::ContentBlock::text("answer")],
            thinking: "reasoning here".to_string(),
            ..Default::default()
        };
        let entry = agent_message_to_entry(&msg);
        assert_eq!(entry.thinking, "reasoning here");
    }

    #[test]
    fn agent_message_to_entry_preserves_meta() {
        let mut meta = serde_json::Map::new();
        meta.insert("key".to_string(), serde_json::json!("value"));
        let msg = crate::types::AgentMessage {
            role: "user".to_string(),
            content: vec![crate::types::ContentBlock::text("hi")],
            metadata: Some(meta),
            ..Default::default()
        };
        let entry = agent_message_to_entry(&msg);
        assert!(entry.meta.is_some());
        assert_eq!(entry.meta.unwrap()["key"], "value");
    }

    #[test]
    fn agent_message_to_entry_unknown_role_defaults_user() {
        let msg = crate::types::AgentMessage {
            role: "custom_role".to_string(),
            content: vec![crate::types::ContentBlock::text("x")],
            ..Default::default()
        };
        let entry = agent_message_to_entry(&msg);
        assert_eq!(entry.entry_type, ENTRY_TYPE_USER);
    }

    // ─── entries_to_agent_messages ──────────────────────────────────────────

    #[test]
    fn entries_to_messages_basic() {
        let entries = vec![
            SessionEntry::new_user("user", serde_json::json!("hello")),
            SessionEntry::new_assistant(serde_json::json!("hi"), vec![]),
        ];
        let msgs = entries_to_agent_messages(&entries, false);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn entries_to_messages_skips_non_standard_types() {
        let mut compaction = SessionEntry::new_user("user", serde_json::json!("x"));
        compaction.entry_type = ENTRY_TYPE_COMPACTION.to_string();
        let entries = vec![
            SessionEntry::new_user("user", serde_json::json!("hello")),
            compaction,
        ];
        let msgs = entries_to_agent_messages(&entries, false);
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn entries_to_messages_string_content() {
        let entries = vec![SessionEntry::new_user(
            "user",
            serde_json::json!("plain string"),
        )];
        let msgs = entries_to_agent_messages(&entries, false);
        assert_eq!(msgs[0].text(), "plain string");
    }

    #[test]
    fn entries_to_messages_array_content() {
        let entries = vec![SessionEntry::new_user(
            "user",
            serde_json::json!([
                {"type": "text", "text": "first"},
                {"type": "text", "text": " second"},
            ]),
        )];
        let msgs = entries_to_agent_messages(&entries, false);
        assert_eq!(msgs[0].text(), "first second");
    }

    #[test]
    fn entries_to_messages_tool_calls() {
        let tool_calls = vec![crate::types::ToolCall {
            id: "c1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "read".to_string(),
                arguments: serde_json::json!({"path": "/tmp"}),
            },
        }];
        let entries = vec![SessionEntry::new_assistant(
            serde_json::json!("reading..."),
            tool_calls,
        )];
        let msgs = entries_to_agent_messages(&entries, false);
        assert_eq!(msgs[0].tool_calls.len(), 1);
        assert_eq!(msgs[0].tool_calls[0].name, "read");
    }

    #[test]
    fn entries_to_messages_empty_entries() {
        let msgs = entries_to_agent_messages(&[], false);
        assert!(msgs.is_empty());
    }

    // ─── Manager save/load/delete ───────────────────────────────────────────

    /// The lightweight summary scanner must produce the same SessionSummary
    /// as the full load_path-based fallback — including on files with huge
    /// tool payloads, model changes and labels.
    #[test]
    fn list_summaries_matches_full_load() {
        let dir = std::env::temp_dir().join(format!(
            "future_test_summary_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = Manager::new(dir.clone());
        let mut session = Session::new("/tmp/test", "gpt-4o", "");
        session.entries.push(SessionEntry::session_info(
            serde_json::json!({"session_name": "named-session", "cwd": "/tmp/test", "model": "gpt-4o", "thinking_level": "high"}),
            "gpt-4o".to_string(),
            "high".to_string(),
        ));
        session.entries.push(SessionEntry::new_user(
            "user",
            serde_json::json!("first question"),
        ));
        // A huge tool payload — the cheap scanner must skip it.
        session.entries.push(SessionEntry::new_assistant(
            serde_json::json!("calling tool"),
            vec![crate::types::ToolCall {
                id: "tc1".to_string(),
                call_type: "function".to_string(),
                function: crate::types::ToolCallFn {
                    name: "read".to_string(),
                    arguments: serde_json::json!({"path": "/big"}),
                },
            }],
        ));
        session
            .entries
            .push(SessionEntry::new_tool("tc1", &"x".repeat(500_000)));
        session.entries.push(SessionEntry::new_user(
            "user",
            serde_json::json!("second question"),
        ));
        manager.save(&session).unwrap();

        let summaries = manager.list_all().unwrap();
        assert_eq!(summaries.len(), 1);
        let fast = &summaries[0];

        let full = Manager::summary_from_session(&manager.load(&session.id).unwrap());
        assert_eq!(fast.id, full.id);
        assert_eq!(fast.cwd, full.cwd);
        assert_eq!(fast.model, full.model);
        assert_eq!(fast.name, full.name);
        assert_eq!(fast.first_message, full.first_message);
        assert_eq!(fast.query_count, full.query_count);
        assert_eq!(fast.updated_at, full.updated_at);
        // Sanity: the expected values themselves.
        assert_eq!(fast.query_count, 2);
        assert_eq!(fast.first_message.as_deref(), Some("first question"));
        assert_eq!(fast.name.as_deref(), Some("named-session"));
        assert_eq!(fast.model, "gpt-4o");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file without session_info falls back to the full-load summary path
    /// instead of being dropped from the list.
    #[test]
    fn list_summaries_falls_back_without_session_info() {
        let dir = std::env::temp_dir().join(format!(
            "future_test_summary_fb_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = Manager::new(dir.clone());
        let mut session = Session::new("/tmp/test", "gpt-4o", "");
        // No session_info entry — legacy/corrupt layout.
        session
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        manager.save(&session).unwrap();

        let summaries = manager.list_all().unwrap();
        assert_eq!(
            summaries.len(),
            1,
            "session must still be listed via fallback"
        );
        assert_eq!(summaries[0].first_message.as_deref(), Some("hello"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manager_save_and_load() {
        let dir = std::env::temp_dir().join(format!(
            "future_test_session_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = Manager::new(dir.clone());
        let mut session = Session::new("/tmp/test", "gpt-4o", "");
        // Add session_info entry (model/thinking_level are in content JSON)
        session.entries.push(SessionEntry::session_info(
            serde_json::json!({"session_name": "test", "cwd": "/tmp/test", "model": "gpt-4o", "thinking_level": "high"}),
            "gpt-4o".to_string(),
            "high".to_string(),
        ));
        session
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        manager.save(&session).unwrap();

        let loaded = manager.load(&session.id).unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.model, "gpt-4o");
        assert_eq!(loaded.entries.len(), 2);

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression test for the HTTP 400 "Messages with role 'tool' must be a
    /// response to a preceding message with 'tool_calls'" failure seen when
    /// resuming a session: a load-time repair previously PERSISTED a "tool
    /// execution lost" placeholder while the owning agent was still mid-tool;
    /// the real tool result was appended afterwards, leaving two tool entries
    /// with the same tool_call_id.  Load must now heal this in memory (keep
    /// the real result, drop the placeholder) without touching the file.
    #[test]
    fn load_dedupes_tool_results_preferring_real_over_placeholder() {
        let dir = std::env::temp_dir().join(format!(
            "future_test_dedupe_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = Manager::new(dir.clone());
        let mut session = Session::new("/tmp/test", "gpt-4o", "");
        session
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        session.entries.push(SessionEntry::new_assistant(
            serde_json::json!("running tool"),
            vec![crate::types::ToolCall {
                id: "tc1".to_string(),
                call_type: "function".to_string(),
                function: crate::types::ToolCallFn {
                    name: "shell".to_string(),
                    arguments: serde_json::json!({"cmd": "ls"}),
                },
            }],
        ));
        // Placeholder written by a stale repair, then the real result.
        session.entries.push(SessionEntry::new_tool(
            "tc1",
            "[Tool execution lost — shell was not executed before the session was interrupted]",
        ));
        session
            .entries
            .push(SessionEntry::new_tool("tc1", "real output"));
        manager.save(&session).unwrap();

        let loaded = manager.load(&session.id).unwrap();
        let tool_entries: Vec<_> = loaded
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
            .collect();
        assert_eq!(
            tool_entries.len(),
            1,
            "duplicate tool entries must be deduped"
        );
        assert_eq!(
            tool_entries[0].content.as_ref().unwrap(),
            &serde_json::json!("real output"),
            "the real result must win over the placeholder"
        );

        // The file on disk must NOT be rewritten by load: read-only callers
        // (session list, get_session_entries) can run while the owning agent
        // is mid-turn, and persisting repairs is what created the duplicates.
        let on_disk = std::fs::read_to_string(manager.session_path(&session.id)).unwrap();
        assert_eq!(
            on_disk.lines().count(),
            4,
            "load must not persist healed entries back to the session file"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two REAL tool results with the same tool_call_id: keep the first.
    #[test]
    fn load_dedupes_tool_results_keeping_first_real() {
        let dir = std::env::temp_dir().join(format!(
            "future_test_dedupe2_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = Manager::new(dir.clone());
        let mut session = Session::new("/tmp/test", "gpt-4o", "");
        session
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        session.entries.push(SessionEntry::new_assistant(
            serde_json::json!("running tool"),
            vec![crate::types::ToolCall {
                id: "tc1".to_string(),
                call_type: "function".to_string(),
                function: crate::types::ToolCallFn {
                    name: "shell".to_string(),
                    arguments: serde_json::json!({"cmd": "ls"}),
                },
            }],
        ));
        session
            .entries
            .push(SessionEntry::new_tool("tc1", "first result"));
        session
            .entries
            .push(SessionEntry::new_tool("tc1", "second result"));
        manager.save(&session).unwrap();

        let loaded = manager.load(&session.id).unwrap();
        let tool_entries: Vec<_> = loaded
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
            .collect();
        assert_eq!(tool_entries.len(), 1);
        assert_eq!(
            tool_entries[0].content.as_ref().unwrap(),
            &serde_json::json!("first result")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A dangling tool_call (assistant saved, tool never executed) is still
    /// patched with a placeholder in memory — but the file stays untouched.
    #[test]
    fn load_repairs_dangling_tool_calls_in_memory_only() {
        let dir = std::env::temp_dir().join(format!(
            "future_test_dangling_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = Manager::new(dir.clone());
        let mut session = Session::new("/tmp/test", "gpt-4o", "");
        session
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        session.entries.push(SessionEntry::new_assistant(
            serde_json::json!("running tool"),
            vec![crate::types::ToolCall {
                id: "tc1".to_string(),
                call_type: "function".to_string(),
                function: crate::types::ToolCallFn {
                    name: "shell".to_string(),
                    arguments: serde_json::json!({"cmd": "ls"}),
                },
            }],
        ));
        manager.save(&session).unwrap();

        let loaded = manager.load(&session.id).unwrap();
        let tool_entries: Vec<_> = loaded
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
            .collect();
        assert_eq!(
            tool_entries.len(),
            1,
            "dangling tool_call must get a placeholder"
        );
        assert_eq!(tool_entries[0].tool_call_id, "tc1");

        let on_disk = std::fs::read_to_string(manager.session_path(&session.id)).unwrap();
        assert_eq!(
            on_disk.lines().count(),
            2,
            "dangling repair must not be persisted to the session file"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn metadata_update_does_not_persist_dangling_tool_repair() {
        let dir = std::env::temp_dir().join(format!(
            "future_test_metadata_dangling_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = Manager::new(dir.clone());
        let mut session = Session::new("/tmp/test", "gpt-4o", "");
        session.entries.push(SessionEntry::session_info(
            serde_json::json!({"model": "gpt-4o"}),
            "gpt-4o".to_string(),
            String::new(),
        ));
        session.entries.push(SessionEntry::new_assistant(
            serde_json::json!("running tool"),
            vec![crate::types::ToolCall {
                id: "tc1".to_string(),
                call_type: "function".to_string(),
                function: crate::types::ToolCallFn {
                    name: "shell".to_string(),
                    arguments: serde_json::json!({"cmd": "ls"}),
                },
            }],
        ));
        manager.save(&session).unwrap();

        manager
            .update_session_info(
                &session.id,
                "session_name",
                serde_json::json!("renamed while running"),
            )
            .unwrap();

        let on_disk = std::fs::read_to_string(manager.session_path(&session.id)).unwrap();
        let entries: Vec<SessionEntry> = on_disk
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        // The metadata update appends a fresh authoritative session_info rather
        // than rewriting through the repair pipeline, so no synthetic tool result
        // is persisted for the dangling tool_call.
        assert!(
            entries
                .iter()
                .all(|entry| entry.entry_type != ENTRY_TYPE_TOOL),
            "metadata update must not persist a synthetic tool result"
        );
        // The dangling assistant entry is left untouched (still exactly one).
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.entry_type == ENTRY_TYPE_ASSISTANT)
                .count(),
            1
        );
        // The authoritative (last) session_info carries the new name; the update
        // is append-only, so it lands after the original session_info.
        let last_info = entries
            .iter()
            .rev()
            .find(|entry| entry.entry_type == ENTRY_TYPE_SESSION_INFO)
            .expect("a session_info snapshot must exist");
        assert_eq!(
            last_info
                .content
                .as_ref()
                .and_then(|content| content.get("session_name")),
            Some(&serde_json::json!("renamed while running"))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_markers_have_correct_type_and_content() {
        let started = SessionEntry::run_started("run-1", 7);
        assert_eq!(started.entry_type, ENTRY_TYPE_RUN_STARTED);
        assert_eq!(started.role, ENTRY_TYPE_SYSTEM);
        let c = started.content.as_ref().unwrap();
        assert_eq!(c["run_id"], "run-1");
        assert_eq!(c["epoch"], 7);

        let terminal = SessionEntry::run_terminal("run-1", RUN_STATE_COMPLETED, 42, 1500, None);
        assert_eq!(terminal.entry_type, ENTRY_TYPE_RUN_TERMINAL);
        let c = terminal.content.as_ref().unwrap();
        assert_eq!(c["run_id"], "run-1");
        assert_eq!(c["state"], "completed");
        assert_eq!(c["run_tokens"], 42);
        assert_eq!(c["run_duration_ms"], 1500);
        assert!(c.get("error").is_none());

        let failed = SessionEntry::run_terminal("run-1", RUN_STATE_ERROR, 0, 10, Some("boom"));
        assert_eq!(failed.content.as_ref().unwrap()["error"], "boom");

        assert!(is_run_marker(ENTRY_TYPE_RUN_STARTED));
        assert!(is_run_marker(ENTRY_TYPE_RUN_TERMINAL));
        assert!(!is_run_marker(ENTRY_TYPE_ASSISTANT));
        assert!(!is_run_marker(ENTRY_TYPE_SESSION_INFO));
    }

    fn temp_manager(tag: &str) -> (std::path::PathBuf, Manager) {
        let dir = std::env::temp_dir().join(format!("future-{tag}-{}", generate_id()));
        let manager = Manager::new(dir.clone());
        (dir, manager)
    }

    #[test]
    fn load_reads_last_session_info_as_authoritative() {
        let (dir, manager) = temp_manager("last-info");
        let info_v0 = SessionEntry::session_info(
            serde_json::json!({"cwd": "/a", "model": "old", "session_name": "first", "parent_session_id": ""}),
            "old".to_string(),
            "low".to_string(),
        );
        let session = Session::snapshot(
            "s-last".to_string(),
            "/a".to_string(),
            "old".to_string(),
            "first".to_string(),
            String::new(),
            vec![
                info_v0,
                SessionEntry::new_user("user", serde_json::json!("hi")),
            ],
        );
        manager.save(&session).unwrap();
        // Append a newer authoritative session_info (as a run commit would).
        let info_v1 = SessionEntry::session_info(
            serde_json::json!({"cwd": "/a", "model": "new", "session_name": "renamed", "parent_session_id": ""}),
            "new".to_string(),
            "high".to_string(),
        );
        manager.append_entries("s-last", &[info_v1]).unwrap();

        let loaded = manager.load("s-last").unwrap();
        assert_eq!(loaded.model, "new");
        assert_eq!(loaded.name, "renamed");
        assert_eq!(loaded.get_session_info().unwrap()["model"], "new");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn update_session_info_appends_complete_snapshot() {
        let (dir, manager) = temp_manager("update-info");
        let info = SessionEntry::session_info(
            serde_json::json!({"cwd": "/a", "model": "m1", "session_name": "n1", "tokens_in": 5}),
            "m1".to_string(),
            "low".to_string(),
        );
        let session = Session::snapshot(
            "s-upd".to_string(),
            "/a".to_string(),
            "m1".to_string(),
            "n1".to_string(),
            String::new(),
            vec![
                info,
                SessionEntry::new_user("user", serde_json::json!("hi")),
            ],
        );
        manager.save(&session).unwrap();

        // Update one field; the appended snapshot must remain complete (other
        // fields are merged over the latest session_info, not lost).
        manager
            .update_session_info("s-upd", "model", serde_json::json!("m2"))
            .unwrap();

        let loaded = manager.load("s-upd").unwrap();
        let info = loaded.get_session_info().unwrap();
        assert_eq!(info["model"], "m2");
        assert_eq!(info["session_name"], "n1");
        assert_eq!(info["tokens_in"], 5);
        // A new session_info was appended (append-only), not rewritten in place.
        let info_count = loaded
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
            .count();
        assert_eq!(info_count, 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn entries_to_agent_messages_skips_run_markers_and_extra_info() {
        let entries = vec![
            SessionEntry::session_info(
                serde_json::json!({"model": "m"}),
                "m".to_string(),
                "low".to_string(),
            ),
            SessionEntry::new_user("user", serde_json::json!("q")),
            SessionEntry::run_started("run-1", 1),
            SessionEntry::new_assistant(serde_json::json!("a"), vec![]),
            SessionEntry::run_terminal("run-1", RUN_STATE_COMPLETED, 1, 1, None),
            SessionEntry::session_info(
                serde_json::json!({"model": "m"}),
                "m".to_string(),
                "low".to_string(),
            ),
        ];
        let msgs = entries_to_agent_messages(&entries, false);
        // Only the user + assistant content enters the model context; run
        // markers and session_info snapshots are filtered out.
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn fork_inherits_last_session_info_and_skips_markers() {
        let (dir, manager) = temp_manager("fork-last");
        let info_v0 = SessionEntry::session_info(
            serde_json::json!({"cwd": "/a", "model": "old", "session_name": "orig", "thinking_level": "low"}),
            "old".to_string(),
            "low".to_string(),
        );
        let user = SessionEntry::new_user("user", serde_json::json!("hi"));
        let user_id = user.id.clone();
        let session = Session::snapshot(
            "s-fork".to_string(),
            "/a".to_string(),
            "old".to_string(),
            "orig".to_string(),
            String::new(),
            vec![info_v0, user, SessionEntry::run_started("r", 1)],
        );
        manager.save(&session).unwrap();
        // A later run commit changes the model and adds a terminal marker plus a
        // fresh authoritative session_info.
        let info_v1 = SessionEntry::session_info(
            serde_json::json!({"cwd": "/a", "model": "new", "session_name": "renamed", "thinking_level": "high"}),
            "new".to_string(),
            "high".to_string(),
        );
        manager
            .append_entries(
                "s-fork",
                &[
                    SessionEntry::new_assistant(serde_json::json!("a"), vec![]),
                    SessionEntry::run_terminal("r", RUN_STATE_COMPLETED, 1, 1, None),
                    info_v1,
                ],
            )
            .unwrap();

        let parent = manager.load("s-fork").unwrap();
        let forked = fork_session(&parent, &user_id);
        // The fork inherits the CURRENT (last) model/name, not the original.
        assert_eq!(forked.model, "new");
        assert!(forked.name.contains("renamed"));
        // No run markers or duplicate session_info leak into the fork.
        assert!(!forked.entries.iter().any(|e| is_run_marker(&e.entry_type)));
        let info_count = forked
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
            .count();
        assert_eq!(info_count, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn append_only_run_journal_roundtrips() {
        // Simulate a full append-only run lifecycle on disk and verify the
        // journal loads cleanly: the model context excludes markers, the
        // authoritative session_info is the last one, and the run boundary is
        // recorded by a started + terminal marker pair.
        let (dir, manager) = temp_manager("journal");
        let info = SessionEntry::session_info(
            serde_json::json!({"cwd": "/a", "model": "m", "session_name": "n", "tokens_out": 0}),
            "m".to_string(),
            "low".to_string(),
        );
        let session = Session::snapshot(
            "s-journal".to_string(),
            "/a".to_string(),
            "m".to_string(),
            "n".to_string(),
            String::new(),
            vec![info],
        );
        manager.save(&session).unwrap();

        // Run 1: user + run_started appended at accept; assistant + run_terminal
        // + refreshed session_info appended at commit.
        manager
            .append_entries(
                "s-journal",
                &[
                    SessionEntry::new_user("user", serde_json::json!("q1")),
                    SessionEntry::run_started("run-1", 1),
                ],
            )
            .unwrap();
        let commit_info = SessionEntry::session_info(
            serde_json::json!({"cwd": "/a", "model": "m", "session_name": "n", "tokens_out": 100}),
            "m".to_string(),
            "low".to_string(),
        );
        manager
            .append_entries(
                "s-journal",
                &[
                    SessionEntry::new_assistant(serde_json::json!("a1"), vec![]),
                    SessionEntry::run_terminal("run-1", RUN_STATE_COMPLETED, 100, 500, None),
                    commit_info,
                ],
            )
            .unwrap();

        let loaded = manager.load("s-journal").unwrap();
        // Authoritative metadata is the last (commit) snapshot.
        assert_eq!(loaded.get_session_info().unwrap()["tokens_out"], 100);
        // Model context is exactly the conversation, no markers.
        let msgs = entries_to_agent_messages(&loaded.entries, false);
        assert_eq!(msgs.len(), 2);
        // The run boundary is recoverable from the markers.
        let started: Vec<_> = loaded
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_RUN_STARTED)
            .collect();
        let terminal: Vec<_> = loaded
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_RUN_TERMINAL)
            .collect();
        assert_eq!(started.len(), 1);
        assert_eq!(terminal.len(), 1);
        assert_eq!(started[0].content.as_ref().unwrap()["run_id"], "run-1");
        assert_eq!(terminal[0].content.as_ref().unwrap()["state"], "completed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn find_unterminated_run_detects_missing_terminal() {
        // No markers → nothing open.
        assert_eq!(find_unterminated_run(&[]), None);
        assert_eq!(
            find_unterminated_run(&[SessionEntry::new_user("user", serde_json::json!("hi"))]),
            None
        );

        // A started+terminal pair is closed.
        let closed = vec![
            SessionEntry::run_started("run-1", 1),
            SessionEntry::run_terminal("run-1", RUN_STATE_COMPLETED, 1, 1, None),
        ];
        assert_eq!(find_unterminated_run(&closed), None);

        // A started marker with no terminal is interrupted.
        let open = vec![
            SessionEntry::new_user("user", serde_json::json!("hi")),
            SessionEntry::run_started("run-2", 1),
        ];
        assert_eq!(find_unterminated_run(&open), Some("run-2".to_string()));

        // Multiple runs: only the last, unterminated one is reported.
        let mixed = vec![
            SessionEntry::run_started("run-1", 1),
            SessionEntry::run_terminal("run-1", RUN_STATE_COMPLETED, 1, 1, None),
            SessionEntry::run_started("run-2", 2),
            SessionEntry::run_terminal("run-2", RUN_STATE_ERROR, 0, 1, Some("boom")),
            SessionEntry::run_started("run-3", 3),
        ];
        assert_eq!(find_unterminated_run(&mixed), Some("run-3".to_string()));

        // A terminal for a different run does not mask an older open run.
        let stray = vec![
            SessionEntry::run_started("run-a", 1),
            SessionEntry::run_terminal("run-other", RUN_STATE_COMPLETED, 1, 1, None),
        ];
        assert_eq!(find_unterminated_run(&stray), Some("run-a".to_string()));
    }

    #[test]
    fn unterminated_run_id_scans_only_markers_from_disk() {
        let (dir, manager) = temp_manager("unterminated-scan");
        let info = SessionEntry::session_info(
            serde_json::json!({"cwd": "/a", "model": "m", "session_name": "n"}),
            "m".to_string(),
            "low".to_string(),
        );
        let session = Session::snapshot(
            "s-scan".to_string(),
            "/a".to_string(),
            "m".to_string(),
            "n".to_string(),
            String::new(),
            vec![info],
        );
        manager.save(&session).unwrap();

        // No markers yet.
        assert_eq!(manager.unterminated_run_id("s-scan").unwrap(), None);
        // Absent file → None, not an error.
        assert_eq!(manager.unterminated_run_id("does-not-exist").unwrap(), None);

        // A completed run leaves nothing open.
        manager
            .append_entries(
                "s-scan",
                &[
                    SessionEntry::new_user("user", serde_json::json!("q1")),
                    SessionEntry::run_started("run-1", 1),
                    SessionEntry::new_assistant(serde_json::json!("a1"), vec![]),
                    SessionEntry::run_terminal("run-1", RUN_STATE_COMPLETED, 5, 50, None),
                ],
            )
            .unwrap();
        assert_eq!(manager.unterminated_run_id("s-scan").unwrap(), None);

        // An interrupted run (started, no terminal) is detected — even with a
        // large assistant line in between, which the cheap scan must skip.
        let big = "x".repeat(10_000);
        manager
            .append_entries(
                "s-scan",
                &[
                    SessionEntry::new_user("user", serde_json::json!("q2")),
                    SessionEntry::run_started("run-2", 2),
                    SessionEntry::new_assistant(serde_json::json!(big), vec![]),
                ],
            )
            .unwrap();
        assert_eq!(
            manager.unterminated_run_id("s-scan").unwrap(),
            Some("run-2".to_string())
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn append_run_start_closes_previous_run_in_same_boundary() {
        let (dir, manager) = temp_manager("atomic-run-start");
        let session = Session::snapshot(
            "s-atomic".to_string(),
            "/a".to_string(),
            "m".to_string(),
            "n".to_string(),
            String::new(),
            vec![SessionEntry::session_info(
                serde_json::json!({"cwd": "/a", "model": "m"}),
                "m".to_string(),
                "low".to_string(),
            )],
        );
        manager.save(&session).unwrap();
        manager
            .append_entries(
                "s-atomic",
                &[
                    SessionEntry::new_user("user", serde_json::json!("old")),
                    SessionEntry::run_started("old-run", 1),
                ],
            )
            .unwrap();

        manager
            .append_run_start(
                "s-atomic",
                SessionEntry::new_user("user", serde_json::json!("new")),
                SessionEntry::run_started("new-run", 2),
            )
            .unwrap();

        let loaded = manager.load("s-atomic").unwrap();
        assert_eq!(
            find_run_terminal(&loaded.entries, "old-run")
                .and_then(|value| value.get("state").cloned()),
            Some(serde_json::json!(RUN_STATE_INTERRUPTED_BY_RESTART))
        );
        assert_eq!(
            find_unterminated_run(&loaded.entries),
            Some("new-run".to_string())
        );
        let old_terminal_index = loaded
            .entries
            .iter()
            .position(|entry| {
                entry.entry_type == ENTRY_TYPE_RUN_TERMINAL
                    && entry.content.as_ref().and_then(|value| value.get("run_id"))
                        == Some(&serde_json::json!("old-run"))
            })
            .unwrap();
        let new_user_index = loaded
            .entries
            .iter()
            .position(|entry| entry.content.as_ref() == Some(&serde_json::json!("new")))
            .unwrap();
        assert!(old_terminal_index < new_user_index);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn manager_delete() {
        let dir = std::env::temp_dir().join(format!(
            "future_test_delete_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = Manager::new(dir.clone());
        let session = Session::new("/tmp/test", "model", "");
        manager.save(&session).unwrap();
        assert!(manager.find(&session.id).is_some());

        manager.delete(&session.id).unwrap();
        assert!(manager.find(&session.id).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manager_find_nonexistent() {
        let dir = std::env::temp_dir().join("future_test_find_none");
        let manager = Manager::new(dir);
        assert!(manager.find("nonexistent_id").is_none());
    }

    #[test]
    fn append_entries_persists_and_loads() {
        let dir = std::env::temp_dir().join(format!(
            "future_test_append_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = Manager::new(dir.clone());
        let mut session = Session::new("/tmp/test", "gpt-4o", "");
        session
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        manager.save(&session).unwrap();

        // Append a second user entry
        let appended = vec![SessionEntry::new_assistant(
            serde_json::json!("hi there"),
            vec![],
        )];
        manager.append_entries(&session.id, &appended).unwrap();

        // Append to non-existent session should error
        let result = manager.append_entries("nonexistent", &appended);
        assert!(result.is_err());

        // Load and verify both entries are present
        let loaded = manager.load(&session.id).unwrap();
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].role, "user");
        assert_eq!(loaded.entries[1].role, "assistant");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cheap_entry_type_extracts_type_field() {
        // Entry format: id first, type second
        let line = r#"{"id":"abc","type":"user","role":"user","content":"hello"}"#;
        assert_eq!(Manager::cheap_entry_type(line), Some("user"));

        let line2 = r#"{"id":"def","type":"assistant","role":"assistant"}"#;
        assert_eq!(Manager::cheap_entry_type(line2), Some("assistant"));

        let line3 = r#"{"id":"ghi","type":"tool","tool_call_id":"t1"}"#;
        assert_eq!(Manager::cheap_entry_type(line3), Some("tool"));

        // Line without type field
        let no_type = r#"{"id":"xyz"}"#;
        assert_eq!(Manager::cheap_entry_type(no_type), None);

        // Very short line
        assert_eq!(Manager::cheap_entry_type("{"), None);
    }

    #[test]
    fn cheap_timestamp_extracts_last_timestamp() {
        let line = r#"{"id":"x","timestamp":"2026-07-23T10:30:00+08:00","other":"data","timestamp":"2026-07-23T11:00:00+08:00"}"#;
        let ts = Manager::cheap_timestamp(line).unwrap();
        // Just verify we get a valid timestamp, regardless of local timezone
        assert!(ts.timestamp() > 0);

        // Line without valid timestamp
        assert!(Manager::cheap_timestamp(r#"{"id":"x"}"#).is_none());
    }

    #[test]
    fn summary_first_message_from_string_content() {
        let e = SessionEntry::new_user("user", serde_json::json!("hello world from the user"));
        let summary = Manager::summary_first_message(&e).unwrap();
        assert!(summary.len() <= 40);
        assert_eq!(summary, "hello world from the user");
    }

    #[test]
    fn summary_first_message_from_array_content() {
        let e = SessionEntry::new_user(
            "user",
            serde_json::json!([
                {"type": "text", "text": "first message block "},
                {"type": "text", "text": "second block"}
            ]),
        );
        // summary_first_message only takes the FIRST text block
        let summary = Manager::summary_first_message(&e).unwrap();
        assert_eq!(summary, "first message block");
    }

    #[test]
    fn summary_first_message_truncates_to_40() {
        let long = "a".repeat(100);
        let e = SessionEntry::new_user("user", serde_json::json!(long));
        let summary = Manager::summary_first_message(&e).unwrap();
        assert_eq!(summary.len(), 40);
    }

    #[test]
    fn summary_first_message_empty_content_returns_none() {
        let mut e = SessionEntry::new_user("user", serde_json::json!(""));
        e.content = Some(serde_json::json!("   "));
        assert!(Manager::summary_first_message(&e).is_none());
    }

    #[test]
    fn session_snapshot_preserves_all_fields() {
        let entries = vec![SessionEntry::new_user("user", serde_json::json!("hello"))];
        let snap = Session::snapshot(
            "sid-123".to_string(),
            "/tmp/proj".to_string(),
            "claude-sonnet".to_string(),
            "My Session".to_string(),
            "parent-456".to_string(),
            entries,
        );
        assert_eq!(snap.id, "sid-123");
        assert_eq!(snap.cwd, "/tmp/proj");
        assert_eq!(snap.model, "claude-sonnet");
        assert_eq!(snap.get_session_name(), "My Session");
        assert_eq!(snap.parent_session_id, "parent-456");
        assert_eq!(snap.entries.len(), 1);
    }

    #[test]
    fn get_session_info_extracts_from_entries() {
        let mut session = Session::new("/tmp/test", "gpt-4o", "");
        session.entries.push(SessionEntry::session_info(
            serde_json::json!({"model": "gpt-4o", "thinking_level": "high"}),
            "gpt-4o".to_string(),
            "high".to_string(),
        ));
        session
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));

        let info = session.get_session_info().unwrap();
        assert_eq!(info["model"], "gpt-4o");
        assert_eq!(info["thinking_level"], "high");

        // Session without session_info entry
        let empty = Session::new("/tmp/test", "gpt-4o", "");
        assert!(empty.get_session_info().is_none());
    }

    #[test]
    fn deserialize_timestamp_space_separator() {
        let json = r#"{"id":"t","type":"u","timestamp":"2026-07-23 10:30:00+08:00"}"#;
        let entry: SessionEntry = serde_json::from_str(json).unwrap();
        // Timezone conversion depends on CI location, just verify it parses
        assert!(entry.timestamp.timestamp() > 0);
    }

    #[test]
    fn deserialize_timestamp_with_fractional_space() {
        let json = r#"{"id":"t","type":"u","timestamp":"2026-07-23 10:30:00.500+08:00"}"#;
        let entry: SessionEntry = serde_json::from_str(json).unwrap();
        assert!(entry.timestamp.timestamp() > 0);
    }

    #[test]
    fn deserialize_timestamp_unparseable_falls_back() {
        let json = r#"{"id":"t","type":"u","timestamp":"not-a-timestamp"}"#;
        let entry: SessionEntry = serde_json::from_str(json).unwrap();
        // Should fall back to current time (not an error)
        let now = chrono::Local::now();
        let diff = (now - entry.timestamp).num_seconds().abs();
        assert!(diff < 5, "fallback time should be close to now");
    }

    // ─── fork_session edge cases ────────────────────────────────────────────

    #[test]
    fn fork_session_bad_entry_id_clones_all() {
        let mut parent = Session::new("/tmp", "model", "");
        parent
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        let forked = fork_session(&parent, "nonexistent_id");
        // Should still produce a session with entries (fallback behavior)
        assert!(!forked.entries.is_empty());
    }

    #[test]
    fn fork_session_preserves_model() {
        let mut parent = Session::new("/tmp", "my-model", "");
        parent
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        let forked = fork_session(&parent, &parent.entries[0].id);
        assert_eq!(forked.model, "my-model");
    }

    #[test]
    fn fork_session_generates_new_ids() {
        let mut parent = Session::new("/tmp", "model", "");
        parent
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        let original_id = parent.entries[0].id.clone();
        let forked = fork_session(&parent, &original_id);
        // Forked entries should have different IDs
        let forked_user_entry = forked
            .entries
            .iter()
            .find(|e| e.entry_type == ENTRY_TYPE_USER)
            .unwrap();
        assert_ne!(forked_user_entry.id, original_id);
    }

    #[test]
    fn fork_session_name_suffix() {
        let mut parent = Session::new("/tmp", "model", "");
        parent.set_session_name("Original Chat");
        parent
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        let forked = fork_session(&parent, &parent.entries[0].id);
        assert!(forked.name.contains("fork"));
    }

    /// Regression: after a crash between assistant(tool_calls) persist and tool
    /// execution, a subsequent restart appends run markers + a new user message
    /// ahead of the orphaned tool_calls.  `repair_dangling_tool_calls` must find
    /// the orphaned assistant even when it is NOT the last entry, and insert
    /// placeholder tool responses so the conversation stays API-valid.
    #[test]
    fn repair_dangling_tool_calls_finds_orphan_after_restart() {
        let (dir, manager) = temp_manager("repair-orphan-restart");
        let session = Session::snapshot(
            "s-repair".to_string(),
            "/tmp".to_string(),
            "m".to_string(),
            "n".to_string(),
            String::new(),
            vec![SessionEntry::session_info(
                serde_json::json!({"cwd": "/tmp", "model": "m"}),
                "m".to_string(),
                "low".to_string(),
            )],
        );
        manager.save(&session).unwrap();

        // --- Simulate a crash mid-run ---
        // The assistant message with tool_calls was persisted, but the tool
        // results were NOT — the process died between save_callback and
        // on_tool_result.
        let tc = ToolCall {
            id: "tc1".to_string(),
            call_type: "function".to_string(),
            function: crate::types::ToolCallFn {
                name: "read".to_string(),
                arguments: serde_json::json!({"path": "/etc/hosts"}),
            },
        };
        manager
            .append_entries(
                "s-repair",
                &[
                    SessionEntry::new_user("user", serde_json::json!("old question")),
                    SessionEntry::run_started("old-run", 1),
                    SessionEntry::new_assistant(
                        serde_json::json!("let me read that"),
                        vec![tc],
                    ),
                ],
            )
            .unwrap();

        // --- Simulate restart: new prompt arrives ---
        // append_run_start closes old-run (appends run_terminal: interrupted)
        // and appends the new user + run_started entries.  The orphaned
        // assistant(tool_calls) is now buried in the middle, NOT at the end.
        manager
            .append_run_start(
                "s-repair",
                SessionEntry::new_user("user", serde_json::json!("new question")),
                SessionEntry::run_started("new-run", 2),
            )
            .unwrap();

        // --- Load: repair_dangling_tool_calls should fire ---
        let loaded = manager.load("s-repair").unwrap();

        // The repair must have inserted a placeholder tool result for tc1.
        let tool_entries: Vec<_> = loaded
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
            .collect();
        assert!(
            !tool_entries.is_empty(),
            "repair_dangling_tool_calls should insert placeholder tool entries"
        );
        let placeholder = &tool_entries[0];
        assert_eq!(placeholder.tool_call_id, "tc1");
        assert!(
            placeholder
                .content
                .as_ref()
                .and_then(|c| c.as_str())
                .is_some_and(|s| s.starts_with(Manager::TOOL_LOST_PLACEHOLDER_PREFIX)),
            "placeholder content should start with '{}', got: {:?}",
            Manager::TOOL_LOST_PLACEHOLDER_PREFIX,
            placeholder.content
        );

        // The placeholder must appear IMMEDIATELY after the orphaned
        // assistant, before the run_terminal marker — otherwise the API
        // would see a user/tool ordering violation.
        let assistant_idx = loaded
            .entries
            .iter()
            .position(|e| {
                e.entry_type == ENTRY_TYPE_ASSISTANT && !e.tool_calls.is_empty()
            })
            .unwrap();
        let tool_idx = loaded
            .entries
            .iter()
            .position(|e| e.entry_type == ENTRY_TYPE_TOOL)
            .unwrap();
        assert_eq!(
            tool_idx,
            assistant_idx + 1,
            "placeholder tool entry must immediately follow the orphaned assistant"
        );

        // --- Verify entries_to_agent_messages is API-valid ---
        // After repair, every assistant with tool_calls must have matching
        // tool entries.  We verify by converting to AgentMessage and checking
        // that each assistant's tool_call_ids all have tool responses.
        let msgs = entries_to_agent_messages(&loaded.entries, false);
        let mut pending: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for msg in &msgs {
            match msg.role.as_str() {
                "assistant" => {
                    // An assistant with tool_calls must not appear while
                    // there are still pending tool_call_ids.
                    assert!(
                        pending.is_empty(),
                        "pending tool_call_ids ({:?}) before new assistant",
                        pending
                    );
                    for tc in &msg.tool_calls {
                        pending.insert(tc.id.clone());
                    }
                }
                "tool" => {
                    let removed = pending.remove(&msg.tool_call_id);
                    assert!(
                        removed,
                        "tool entry with tool_call_id={} has no matching \
                         assistant tool_call",
                        msg.tool_call_id
                    );
                }
                _ => {
                    // A user/system message between tool_calls and their
                    // responses is an API violation.
                    assert!(
                        pending.is_empty(),
                        "pending tool_call_ids ({:?}) before non-tool message role={}",
                        pending,
                        msg.role
                    );
                }
            }
        }
        assert!(
            pending.is_empty(),
            "unresolved tool_call_ids after message walk: {:?}",
            pending
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod image_persistence_tests {
    use super::*;
    use crate::types::{AgentMessage, ContentBlock};

    fn write_png(tag: &str) -> std::path::PathBuf {
        let img = image::RgbImage::from_fn(8, 8, |_, _| image::Rgb([1u8, 2, 3]));
        let p = std::env::temp_dir().join(format!(
            "futureos-sess-img-{}-{}.png",
            std::process::id(),
            tag
        ));
        img.save(&p).unwrap();
        p
    }

    fn user_msg_with_image_meta() -> AgentMessage {
        let mut meta = serde_json::Map::new();
        meta.insert(
            "attachments".to_string(),
            serde_json::json!([{"path": "/x.png", "kind": "image", "name": "x.png"}]),
        );
        AgentMessage {
            role: "user".to_string(),
            content: vec![
                ContentBlock::text("hi"),
                ContentBlock::image("data:image/png;base64,AAAA"),
            ],
            thinking: String::new(),
            tool_calls: vec![],
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            metadata: Some(meta),
        }
    }

    #[test]
    fn base64_image_is_stripped_from_jsonl_when_backed_by_meta() {
        let entry = agent_message_to_entry(&user_msg_with_image_meta());
        let arr = entry.content.unwrap();
        let arr = arr.as_array().unwrap();
        // The base64 image_url block is gone; only the text block persists.
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "text");
    }

    #[test]
    fn image_is_rehydrated_from_meta_path_on_reload() {
        let png = write_png("rehydrate");
        // A reloaded user entry: text-only content (image stripped), meta points
        // at the on-disk image.
        let mut entry =
            SessionEntry::new_user("user", serde_json::json!([{"type": "text", "text": "hi"}]));
        entry.meta = Some(serde_json::json!({
            "attachments": [{"path": png.to_string_lossy(), "kind": "image", "name": "x.png"}]
        }));

        let has_image = |msgs: &[AgentMessage]| {
            msgs[0]
                .content
                .iter()
                .any(|b| matches!(b, ContentBlock::Image { .. }))
        };

        // Image-capable model → rebuilt from the path.
        assert!(has_image(&entries_to_agent_messages(
            std::slice::from_ref(&entry),
            true
        )));
        // Text-only model → not rebuilt.
        assert!(!has_image(&entries_to_agent_messages(&[entry], false)));

        std::fs::remove_file(&png).ok();
    }

    #[test]
    fn legacy_image_url_without_meta_is_preserved() {
        // A channels/TUI message (base64 image_url in content, no meta) keeps its
        // image on both save and reload.
        let msg = AgentMessage {
            role: "user".to_string(),
            content: vec![
                ContentBlock::text("hi"),
                ContentBlock::image("data:image/png;base64,ZZZZ"),
            ],
            thinking: String::new(),
            tool_calls: vec![],
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            metadata: None,
        };
        let entry = agent_message_to_entry(&msg);
        // Not stripped on save.
        let arr = entry.content.clone().unwrap();
        assert_eq!(arr.as_array().unwrap().len(), 2);
        // Preserved on reload (no re-read needed; base64 is on disk).
        let msgs = entries_to_agent_messages(&[entry], true);
        assert!(msgs[0]
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::Image { .. })));
    }
}

#[cfg(test)]
mod fork_tests {
    use super::*;
    use crate::types::AgentMessage;

    fn make_entry(id: &str, entry_type: &str, role: &str, content: &str) -> SessionEntry {
        SessionEntry {
            id: id.to_string(),
            entry_type: entry_type.to_string(),
            role: role.to_string(),
            content: Some(serde_json::json!(content)),
            tool_calls: vec![],
            timestamp: chrono::Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }

    #[test]
    fn fork_session_copies_entries_up_to_fork_point() {
        let mut parent = Session::new("/tmp/test", "test-model", "");
        let u1 = make_entry("u1", ENTRY_TYPE_USER, "user", "hello");
        let a1 = make_entry("a1", ENTRY_TYPE_ASSISTANT, "assistant", "hi there");
        let u2 = make_entry("u2", ENTRY_TYPE_USER, "user", "help me");
        let a2 = make_entry("a2", ENTRY_TYPE_ASSISTANT, "assistant", "sure!");
        parent.entries = vec![u1.clone(), a1.clone(), u2.clone(), a2.clone()];

        // Fork at a1: should include u1 + a1 (skipping original session_info)
        let forked = fork_session(&parent, &a1.id);

        // session_info is prepended, so total entries = 1 (info) + 2 (u1, a1)
        assert_eq!(forked.entries.len(), 3);
        assert_eq!(forked.entries[1].entry_type, ENTRY_TYPE_USER);
        assert_eq!(forked.entries[2].entry_type, ENTRY_TYPE_ASSISTANT);
    }

    #[test]
    fn entries_to_messages_roundtrip_preserves_history_count() {
        // Simulate: a forked session with history is created, but
        // messages is empty → first prompt save would truncate disk.
        let mut parent = Session::new("/tmp/test", "test-model", "");
        let u1 = make_entry("u1", ENTRY_TYPE_USER, "user", "hello");
        let a1 = make_entry("a1", ENTRY_TYPE_ASSISTANT, "assistant", "hi");
        let a1_id = a1.id.clone();
        parent.entries = vec![u1, a1];

        let forked = fork_session(&parent, &a1_id);

        // Bug scenario (old code): messages starts empty, so only the new
        // user message would be saved — history entries are dropped.
        let empty_msgs: Vec<AgentMessage> = vec![];
        let entries_from_empty: Vec<SessionEntry> =
            empty_msgs.iter().map(agent_message_to_entry).collect();
        assert!(
            entries_from_empty.is_empty(),
            "old code: empty messages → no entries → history lost on save"
        );

        // Fix scenario: entries are loaded into messages first.
        // (model_accepts_images=false → images not rehydrated, but text
        //  entries still convert correctly.)
        let msgs = entries_to_agent_messages(&forked.entries, false);
        // session_info is skipped by entries_to_agent_messages (role="system"
        // doesn't match user/assistant/tool), but the user+assistant entries
        // should both convert.
        assert_eq!(
            msgs.len(),
            2,
            "fixed code: forked entries (user + assistant) → 2 messages"
        );
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");

        // When the first prompt runs, self.messages now has history + new msg,
        // so save() preserves everything.
        let mut msgs_with_prompt = msgs;
        msgs_with_prompt.push(AgentMessage {
            role: "user".to_string(),
            content: vec![crate::types::ContentBlock::text("new question")],
            thinking: String::new(),
            tool_calls: vec![],
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            metadata: None,
        });
        let entries_with_history: Vec<SessionEntry> = msgs_with_prompt
            .iter()
            .map(agent_message_to_entry)
            .collect();
        assert!(
            entries_with_history.len() >= 3,
            "fixed code: history (2) + new user (1) = {} entries (expected >= 3)",
            entries_with_history.len()
        );
    }
}

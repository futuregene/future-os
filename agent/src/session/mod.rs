//! Session management — 1:1 compatible with Go internal/session/

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
pub const ENTRY_TYPE_BRANCH_SUMMARY: &str = "branch_summary";
pub const ENTRY_TYPE_CUSTOM: &str = "custom";
pub const ENTRY_TYPE_CUSTOM_MESSAGE: &str = "custom_message";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchSummaryMeta {
    #[serde(rename = "from_id", skip_serializing_if = "Option::is_none")]
    pub from_id: Option<String>,
    #[serde(rename = "from_hook", skip_serializing_if = "Option::is_none")]
    pub from_hook: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: String,
    #[serde(
        rename = "parent_id",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub parent_id: String,
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(
        rename = "thinking_level",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub thinking_level: String,
    #[serde(
        rename = "branch_summary",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub branch_summary: Option<BranchSummaryMeta>,
    #[serde(
        rename = "custom_type",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub custom_type: String,
    #[serde(
        rename = "custom_data",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub custom_data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
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
    /// Output (completion) tokens for the reply this entry belongs to. Only the
    /// final assistant entry of a run carries a non-zero value; used by the GUI
    /// to show per-reply token counts when reloading history from JSONL.
    #[serde(rename = "output_tokens", default, skip_serializing_if = "is_zero_i64")]
    pub output_tokens: i64,
    /// Wall-clock duration of the run this entry belongs to, in milliseconds.
    /// Set alongside `output_tokens` on the final assistant entry of a run.
    #[serde(rename = "duration_ms", default, skip_serializing_if = "is_zero_i64")]
    pub duration_ms: i64,
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
    use serde::de;
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

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}

impl SessionEntry {
    pub fn new_user(role: &str, content: serde_json::Value) -> Self {
        Self {
            id: generate_entry_id(),
            parent_id: String::new(),
            entry_type: ENTRY_TYPE_USER.to_string(),
            role: role.to_string(),
            content: Some(content),
            tool_calls: vec![],
            timestamp: Local::now(),
            summary: String::new(),
            model: String::new(),
            label: String::new(),
            thinking_level: String::new(),
            branch_summary: None,
            custom_type: String::new(),
            custom_data: None,
            display: String::new(),
            provider: String::new(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            output_tokens: 0,
            duration_ms: 0,
            meta: None,
        }
    }

    pub fn new_assistant(content: serde_json::Value, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            id: generate_entry_id(),
            parent_id: String::new(),
            entry_type: ENTRY_TYPE_ASSISTANT.to_string(),
            role: "assistant".to_string(),
            content: Some(content),
            tool_calls,
            timestamp: Local::now(),
            summary: String::new(),
            model: String::new(),
            label: String::new(),
            thinking_level: String::new(),
            branch_summary: None,
            custom_type: String::new(),
            custom_data: None,
            display: String::new(),
            provider: String::new(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            output_tokens: 0,
            duration_ms: 0,
            meta: None,
        }
    }

    pub fn new_tool(call_id: &str, content: &str) -> Self {
        Self {
            id: generate_entry_id(),
            parent_id: String::new(),
            entry_type: ENTRY_TYPE_TOOL.to_string(),
            role: "tool".to_string(),
            content: Some(serde_json::json!(content)),
            tool_calls: vec![],
            timestamp: Local::now(),
            summary: String::new(),
            model: String::new(),
            label: String::new(),
            thinking_level: String::new(),
            branch_summary: None,
            custom_type: String::new(),
            custom_data: None,
            display: String::new(),
            provider: String::new(),
            tool_call_id: call_id.to_string(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            output_tokens: 0,
            duration_ms: 0,
            meta: None,
        }
    }

    /// Build the `session_info` metadata entry prepended to every saved session.
    /// `content` holds the token/cost/name JSON snapshot; `model`/`thinking_level`
    /// pin the session's active settings. All other fields take entry defaults.
    pub fn session_info(content: serde_json::Value, model: String, thinking_level: String) -> Self {
        Self {
            id: generate_entry_id(),
            parent_id: String::new(),
            entry_type: ENTRY_TYPE_SESSION_INFO.to_string(),
            role: ENTRY_TYPE_SYSTEM.to_string(),
            content: Some(content),
            tool_calls: vec![],
            timestamp: Local::now(),
            summary: String::new(),
            model,
            label: String::new(),
            thinking_level,
            branch_summary: None,
            custom_type: String::new(),
            custom_data: None,
            display: String::new(),
            provider: String::new(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            output_tokens: 0,
            duration_ms: 0,
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
        self.entries
            .iter()
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
    pub fn append_entries(
        &self,
        session_id: &str,
        entries: &[SessionEntry],
    ) -> Result<()> {
        use std::io::Write;
        let path = self.session_path(session_id);
        if !path.exists() {
            return Err(anyhow::anyhow!("session file does not exist yet"));
        }
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .with_context(|| format!("open session file for append: {}", path.display()))?;
        for entry in entries {
            let json = serde_json::to_string(entry).context("serialize entry")?;
            let mut line = json.into_bytes();
            line.push(b'\n');
            file.write_all(&line).context("write entry")?;
        }
        file.flush().context("flush")?;
        Ok(())
    }

    pub fn save(&self, session: &Session) -> Result<()> {
        let path = self.session_path(&session.id);
        fs::create_dir_all(&self.dir).context("create session dir")?;
        // Write to a temp file and rename atomically so a mid-write crash
        // never leaves a partially-written (corrupt) JSONL behind.
        let tmp_path = path.with_extension("jsonl.tmp");
        let file = File::create(&tmp_path).context("create temp session file")?;
        let mut w = std::io::BufWriter::new(file);
        for entry in &session.entries {
            let json = serde_json::to_string(entry).context("serialize entry")?;
            writeln!(w, "{}", json).context("write entry")?;
        }
        w.flush().context("flush")?;
        drop(w);
        fs::rename(&tmp_path, &path).context("rename temp to final")?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Session> {
        let path = self.session_path(id);
        self.load_path(&path, id)
    }

    pub(crate) fn load_path(&self, path: &Path, id: &str) -> Result<Session> {
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
        let created_at = entries[0].timestamp;
        let updated_at = entries.last().map(|e| e.timestamp).unwrap_or(created_at);
        let cwd = entries
            .iter()
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
                if e.entry_type == ENTRY_TYPE_MODEL_CHANGE && !e.model.is_empty() {
                    Some(e.model.clone())
                } else {
                    None
                }
            })
            .or_else(|| {
                // ASSISTANT entries never carry model (agent_message_to_entry
                // always sets it to ""), so fall back to the session_info entry.
                entries
                    .iter()
                    .find(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
                    .and_then(|e| {
                        if !e.model.is_empty() {
                            Some(e.model.clone())
                        } else {
                            None
                        }
                    })
            })
            .unwrap_or_default();
        let name = entries
            .iter()
            .rev()
            .find(|e| e.entry_type == ENTRY_TYPE_LABEL && !e.label.is_empty())
            .map(|e| e.label.clone())
            .or_else(|| {
                // Fall back to session_info.session_name when no LABEL entry
                // exists (e.g. sessions that were auto-named but never
                // explicitly renamed).
                entries
                    .iter()
                    .find(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
                    .and_then(|e| e.content.as_ref())
                    .and_then(|c| c.get("session_name"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();
        let parent_session_id = entries
            .iter()
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

    pub fn list(&self, cwd: &str) -> Result<Vec<Session>> {
        fs::create_dir_all(&self.dir).ok();
        let mut sessions = vec![];
        if self.dir.exists() {
            for entry in fs::read_dir(&self.dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                    continue;
                }
                let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if let Ok(sess) = self.load_path(&path, id) {
                    if sess.cwd == cwd || cwd.is_empty() {
                        sessions.push(sess);
                    }
                }
            }
        }
        sessions.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        Ok(sessions)
    }

    /// List all sessions in the flat sessions directory
    pub fn list_all(&self) -> Result<Vec<SessionSummary>> {
        if !self.dir.exists() {
            return Ok(vec![]);
        }
        let mut summaries = vec![];
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            self.try_push_summary(&path, &mut summaries);
        }
        summaries.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        Ok(summaries)
    }

    fn try_push_summary(&self, path: &Path, summaries: &mut Vec<SessionSummary>) {
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            return;
        }
        let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if let Ok(sess) = self.load_path(path, id) {
            // Scan entries for user messages: first user message and total count.
            // Also read session_name from the session_info entry as a fallback
            // for the name when the Session-level name field is empty (older
            // sessions saved before the name was plumbed through).
            let mut first_message: Option<String> = None;
            let mut query_count: usize = 0;
            let mut session_info_name: Option<String> = None;
            for entry in &sess.entries {
                if entry.role == "user" {
                    query_count += 1;
                    if first_message.is_none() {
                        if let Some(ref content_val) = entry.content {
                            let text: String = if let Some(arr) = content_val.as_array() {
                                // First text block only — a later one is the agent-
                                // injected attachment-path list, not the user's message.
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
                            // Trim, then truncate to ~40 visible-width (≈20 CJK chars)
                            let trimmed = text.trim();
                            let truncated: String = truncate_visible(trimmed, 40);
                            if !truncated.is_empty() {
                                first_message = Some(truncated);
                            }
                        }
                    }
                } else if entry.entry_type == ENTRY_TYPE_SESSION_INFO && session_info_name.is_none()
                {
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
            summaries.push(SessionSummary {
                id: sess.id,
                cwd: sess.cwd,
                updated_at: sess.updated_at,
                model: sess.model,
                name: if !sess.name.is_empty() {
                    Some(sess.name)
                } else {
                    session_info_name.clone()
                },
                parent_session_id: sess.parent_session_id.clone(),
                first_message,
                query_count,
            });
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
        fs::remove_file(path).map_err(|e| anyhow!("failed to delete session: {}", e))
    }
}

pub fn fork_session(parent: &Session, from_entry_id: &str) -> Session {
    let chain = for_each_entry(&parent.entries, from_entry_id);
    // for_each_entry returns root→target order; clone as-is (IDs are regenerated below).
    let mut entries: Vec<SessionEntry> = chain.into_iter().cloned().collect();
    for e in &mut entries {
        e.id = generate_entry_id();
    }
    // Read parent metadata from the session_info entry.  The values live on
    // the SessionEntry struct fields (model, thinking_level) and also inside
    // the content JSON (created_by, session_name).
    let parent_info = parent
        .entries
        .first()
        .filter(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO);

    // Prefer the parent's actual level: the session_info struct field, then the
    // content JSON (forked parents carry it there) — only fall back to a literal
    // when neither is set, so a `low`/`medium` parent doesn't silently fork to
    // `high`.
    let parent_thinking_level = parent_info
        .map(|e| e.thinking_level.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            parent_info
                .and_then(|e| e.content.as_ref())
                .and_then(|c| c.get("thinking_level"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("high");

    let parent_model = parent_info
        .and_then(|e| {
            if !e.model.is_empty() {
                Some(e.model.as_str())
            } else {
                None
            }
        })
        .unwrap_or(&parent.model)
        .to_string();

    let parent_created_by = parent_info
        .and_then(|e| e.content.as_ref())
        .and_then(|c| c.get("created_by"))
        .and_then(|v| v.as_str())
        .unwrap_or("tui");

    // Derive fork name: read from session_info content first, then LABEL.
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
            parent_id: String::new(),
            entry_type: ENTRY_TYPE_SESSION_INFO.to_string(),
            role: "system".to_string(),
            content: Some(info),
            tool_calls: vec![],
            timestamp: Local::now(),
            summary: String::new(),
            model: parent_model.clone(),
            label: String::new(),
            thinking_level: parent_thinking_level.to_string(),
            branch_summary: None,
            custom_type: String::new(),
            custom_data: None,
            display: String::new(),
            provider: String::new(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            output_tokens: 0,
            duration_ms: 0,
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
    // skipping the original session_info (fork_session prepends its own).
    let mut result = vec![];
    for e in entries.iter() {
        if e.entry_type != ENTRY_TYPE_SESSION_INFO {
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
        parent_id: String::new(),
        entry_type: entry_type.to_string(),
        role: msg.role.clone(),
        content,
        tool_calls,
        timestamp: Local::now(),
        summary: String::new(),
        model: String::new(),
        label: String::new(),
        thinking_level: String::new(),
        branch_summary: None,
        custom_type: String::new(),
        custom_data: None,
        display: String::new(),
        provider: String::new(),
        tool_call_id: msg.tool_call_id.clone(),
        name: msg.name.clone(),
        tool_args: msg.tool_args.clone(),
        thinking: msg.thinking.clone(),
        // Populated at the save site (session_prompt.rs): only the final
        // assistant entry of a run gets a non-zero value, and prior entries'
        // values are preserved from the previously-saved session.
        output_tokens: 0,
        duration_ms: 0,
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
            parent_id: String::new(),
            entry_type: entry_type.to_string(),
            role: role.to_string(),
            content: Some(serde_json::json!(content)),
            tool_calls: vec![],
            timestamp: chrono::Local::now(),
            summary: String::new(),
            model: String::new(),
            label: String::new(),
            thinking_level: String::new(),
            branch_summary: None,
            custom_type: String::new(),
            custom_data: None,
            display: String::new(),
            provider: String::new(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            output_tokens: 0,
            duration_ms: 0,
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

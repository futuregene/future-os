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

    pub fn save(&self, session: &Session) -> Result<()> {
        let path = self.session_path(&session.id);
        fs::create_dir_all(&self.dir).context("create session dir")?;
        let file = File::create(&path).context("create session file")?;
        let mut w = std::io::BufWriter::new(file);
        for entry in &session.entries {
            let json = serde_json::to_string(entry).context("serialize entry")?;
            writeln!(w, "{}", json).context("write entry")?;
        }
        w.flush().context("flush")?;
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
        for line in reader.lines() {
            let line = line.context("read line")?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: SessionEntry = serde_json::from_str(&line).context("parse entry")?;
            entries.push(entry);
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
                if (e.entry_type == ENTRY_TYPE_MODEL_CHANGE || e.entry_type == ENTRY_TYPE_ASSISTANT)
                    && !e.model.is_empty()
                {
                    Some(e.model.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        let name = entries
            .iter()
            .rev()
            .find(|e| e.entry_type == ENTRY_TYPE_LABEL && !e.label.is_empty())
            .map(|e| e.label.clone())
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
            // Scan entries for user messages: first user message and total count
            let mut first_message: Option<String> = None;
            let mut query_count: usize = 0;
            for entry in &sess.entries {
                if entry.role == "user" {
                    query_count += 1;
                    if first_message.is_none() {
                        if let Some(ref content_val) = entry.content {
                            let text: String = if let Some(arr) = content_val.as_array() {
                                arr.iter()
                                    .filter_map(|b| {
                                        b.get("text").and_then(|t| t.as_str())
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" ")
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
                }
            }
            summaries.push(SessionSummary {
                id: sess.id,
                cwd: sess.cwd,
                updated_at: sess.updated_at,
                model: sess.model,
                name: if sess.name.is_empty() {
                    None
                } else {
                    Some(sess.name)
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
    let mut entries: Vec<SessionEntry> = chain.into_iter().cloned().rev().collect();
    for e in &mut entries {
        e.id = generate_entry_id();
    }
    // Prepend session_info with parent_session_id so tree relationships survive save/load
    let info = serde_json::json!({
        "cwd": parent.cwd,
        "model": parent.model,
        "parent_session_id": parent.id,
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
            model: parent.model.clone(),
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
        },
    );
    let now = Local::now();
    Session {
        id: generate_id(),
        version: CURRENT_SESSION_VERSION,
        cwd: parent.cwd.clone(),
        model: parent.model.clone(),
        base_url: parent.base_url.clone(),
        name: String::new(),
        parent_session_id: parent.id.clone(),
        leaf_id: String::new(),
        entries,
        created_at: now,
        updated_at: now,
    }
}

fn for_each_entry<'a>(entries: &'a [SessionEntry], from_id: &str) -> Vec<&'a SessionEntry> {
    let mut result = vec![];
    for e in entries.iter() {
        if e.id == from_id {
            result.push(e);
            break;
        }
    }
    // Walk parent chain
    if let Some(first) = result.first() {
        if !first.parent_id.is_empty() {
            for e in entries.iter().rev() {
                if e.id == first.parent_id {
                    result.insert(0, e);
                    break;
                }
            }
        }
    }
    result
}

/// Convert SessionEntry to AgentMessage for TUI display
pub fn entries_to_agent_messages(entries: &[SessionEntry]) -> Vec<crate::types::AgentMessage> {
    use crate::types::{AgentToolCall, ContentBlock};
    let mut msgs = vec![];
    for entry in entries {
        let role = match entry.entry_type.as_str() {
            "user" | "system" | "assistant" | "tool" => entry.entry_type.clone(),
            _ => continue,
        };

        let content: Vec<ContentBlock> = match &entry.content {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .map(|v| {
                    let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    ContentBlock::Text {
                        text: text.to_string(),
                    }
                })
                .collect(),
            Some(serde_json::Value::String(s)) => {
                vec![ContentBlock::Text { text: s.clone() }]
            }
            _ => vec![],
        };

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
            thinking: String::new(),
            tool_calls,
            tool_call_id: entry.tool_call_id.clone(),
            name: entry.name.clone(),
            tool_args: entry.tool_args.clone(),
            metadata: None,
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

    let content_blocks: Vec<serde_json::Value> = msg
        .content
        .iter()
        .map(|b| serde_json::to_value(b).unwrap_or(serde_json::Value::Null))
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
    }
}

/// Truncate a string to max_vis visible columns. CJK characters count as 2,
/// everything else as 1. Matches approximate terminal rendering width.
fn truncate_visible(s: &str, max_vis: usize) -> String {
    let mut vis: usize = 0;
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        let w = if ch >= '\u{1100}' && ch <= '\u{115f}'   // Hangul Jamo
            || ch >= '\u{2e80}' && ch <= '\u{a4cf}'       // CJK radicals + Yi
            || ch >= '\u{ac00}' && ch <= '\u{d7a3}'       // Hangul Syllables
            || ch >= '\u{f900}' && ch <= '\u{faff}'       // CJK Compatibility
            || ch >= '\u{fe30}' && ch <= '\u{fe4f}'       // CJK Compatibility Forms
            || ch >= '\u{ff00}' && ch <= '\u{ffef}'       // Fullwidth Forms
            || ch >= '\u{1f300}' && ch <= '\u{1f5ff}'     // Misc Symbols
            || ch >= '\u{1f900}' && ch <= '\u{1f9ff}'     // Supplemental Symbols
            || ch >= '\u{1f600}' && ch <= '\u{1f64f}'     // Emoticons
            || ch >= '\u{20000}' && ch <= '\u{2fffd}'     // SIP
            || ch >= '\u{30000}' && ch <= '\u{3fffd}'     // TIP
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

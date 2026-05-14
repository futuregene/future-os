//! Session management — 1:1 compatible with Go internal/session/

use crate::types::{Message, ToolCall};
use crate::utils::{default_session_dir, encode_cwd, generate_entry_id, generate_id};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use chrono::{DateTime, Local};

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
    #[serde(rename = "parent_id", default, skip_serializing_if = "String::is_empty")]
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
    #[serde(rename = "thinking_level", default, skip_serializing_if = "String::is_empty")]
    pub thinking_level: String,
    #[serde(rename = "branch_summary", default, skip_serializing_if = "Option::is_none")]
    pub branch_summary: Option<BranchSummaryMeta>,
    #[serde(rename = "custom_type", default, skip_serializing_if = "String::is_empty")]
    pub custom_type: String,
    #[serde(rename = "custom_data", default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
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
        }
    }

    pub fn new_tool(_call_id: &str, content: &str) -> Self {
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
    #[serde(rename = "parent_session_id", default, skip_serializing_if = "String::is_empty")]
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
        self.name = name.to_string();
    }

    pub fn get_base_url(&self) -> &str {
        &self.base_url
    }

    pub fn set_base_url(&mut self, url: &str) {
        self.base_url = url.to_string();
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
        Self { dir: default_session_dir(cwd) }
    }

    fn session_dir(&self, cwd: &str) -> PathBuf {
        self.dir.join(encode_cwd(cwd))
    }

    fn session_path(&self, cwd: &str, id: &str) -> PathBuf {
        self.session_dir(cwd).join(format!("{}.jsonl", id))
    }

    pub fn save(&self, session: &Session) -> Result<()> {
        let path = self.session_path(&session.cwd, &session.id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create session dir")?;
        }
        let file = File::create(&path).context("create session file")?;
        let mut w = std::io::BufWriter::new(file);
        for entry in &session.entries {
            let json = serde_json::to_string(entry).context("serialize entry")?;
            writeln!(w, "{}", json).context("write entry")?;
        }
        w.flush().context("flush")?;
        Ok(())
    }

    pub fn load(&self, id: &str, cwd: &str) -> Result<Session> {
        let path = self.session_path(cwd, id);
        let file = File::open(&path).context("open session file")?;
        let reader = BufReader::new(file);
        let mut entries = vec![];
        for line in reader.lines() {
            let line = line.context("read line")?;
            if line.trim().is_empty() { continue; }
            let entry: SessionEntry = serde_json::from_str(&line).context("parse entry")?;
            entries.push(entry);
        }
        if entries.is_empty() {
            return Err(anyhow!("session {} has no entries", id));
        }
        let created_at = entries[0].timestamp;
        let cwd = cwd.to_string();
        let session = Session {
            id: id.to_string(),
            version: CURRENT_SESSION_VERSION,
            cwd,
            model: String::new(),
            base_url: String::new(),
            name: String::new(),
            parent_session_id: String::new(),
            leaf_id: String::new(),
            entries,
            created_at,
            updated_at: Local::now(),
        };
        Ok(session)
    }

    pub fn list(&self, cwd: &str) -> Result<Vec<Session>> {
        let dir = self.session_dir(cwd);
        fs::create_dir_all(&dir).ok();
        let mut sessions = vec![];
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if let Ok(sess) = self.load(id, cwd) {
                sessions.push(sess);
            }
        }
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(sessions)
    }
    
    /// List all sessions across all CWD directories
    pub fn list_all(&self) -> Result<Vec<SessionSummary>> {
        if !self.dir.exists() {
            return Ok(vec![]);
        }
        let mut summaries = vec![];
        
        for cwd_entry in fs::read_dir(&self.dir)? {
            let cwd_entry = cwd_entry?;
            if !cwd_entry.file_type()?.is_dir() {
                continue;
            }
            
            for entry in fs::read_dir(cwd_entry.path())? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                    continue;
                }
                let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let cwd = cwd_entry.file_name().to_str().unwrap_or("");
                
                if let Ok(sess) = self.load(id, cwd) {
                    summaries.push(SessionSummary {
                        id: sess.id,
                        cwd: sess.cwd,
                        updated_at: sess.updated_at,
                        model: sess.model,
                        name: if sess.name.is_empty() { None } else { Some(sess.name) },
                    });
                }
            }
        }
        
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(summaries)
    }
    
    /// Delete a session file
    pub fn delete(&self, id: &str, cwd: &str) -> Result<()> {
        let path = self.session_path(cwd, id);
        fs::remove_file(path).map_err(|e| anyhow!("failed to delete session: {}", e))
    }
}

pub fn fork_session(parent: &Session, from_entry_id: &str) -> Session {
    let chain = for_each_entry(&parent.entries, from_entry_id);
    let mut entries: Vec<SessionEntry> = chain.into_iter().cloned().rev().collect();
    for e in &mut entries {
        e.id = generate_entry_id();
    }
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
        let reasoning = if entry.thinking_level.is_empty() { String::new() } else { String::new() };

        msgs.push(Message {
            role,
            content: Some(content),
            tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
            tool_call_id: String::new(),
            name: String::new(),
            reasoning_content: reasoning,
        });
    }
    msgs
}

//! In-memory session models: `Session` and the listing-only `SessionSummary`.

use super::entry::{SessionEntry, ENTRY_TYPE_SESSION_INFO};
use crate::utils::generate_id;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

pub const CURRENT_SESSION_VERSION: i32 = 3;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}

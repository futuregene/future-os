//! Session store: maps Feishu (chat_id, thread_id) → agent session_id.
//! Persisted as JSON file.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub chat_id: String,
    pub thread_id: Option<String>,
    pub session_id: String,
    pub created_at: String,
    pub last_active: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreData {
    sessions: Vec<SessionEntry>,
}

pub struct SessionStore {
    path: PathBuf,
    /// In-memory lookup: "chat_id:thread_id" → session_id
    data: RwLock<HashMap<String, SessionEntry>>,
}

impl SessionStore {
    pub fn new(path: PathBuf) -> Self {
        let mut store = Self {
            path,
            data: RwLock::new(HashMap::new()),
        };
        store.load_from_disk();
        store
    }

    fn session_key(chat_id: &str, thread_id: Option<&str>) -> String {
        match thread_id {
            Some(tid) if !tid.is_empty() => format!("{}:{}", chat_id, tid),
            _ => chat_id.to_string(),
        }
    }

    /// Look up or create a session for a chat.
    /// Returns (session_id, is_new).
    pub fn get_or_create(&self, chat_id: &str, thread_id: Option<&str>) -> (String, bool) {
        let key = Self::session_key(chat_id, thread_id);
        {
            let data = self.data.read().unwrap();
            if let Some(entry) = data.get(&key) {
                return (entry.session_id.clone(), false);
            }
        }
        // Create new entry (session will be created by bridge via gRPC)
        let session_id = String::new(); // Placeholder — filled by bridge
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let entry = SessionEntry {
            chat_id: chat_id.to_string(),
            thread_id: thread_id.map(|s| s.to_string()),
            session_id: session_id.clone(),
            created_at: now.clone(),
            last_active: now,
        };
        {
            let mut data = self.data.write().unwrap();
            data.insert(key, entry);
        }
        (session_id, true)
    }

    /// Update the session_id for a chat (called after gRPC new_session returns).
    pub fn set_session_id(&self, chat_id: &str, thread_id: Option<&str>, session_id: &str) {
        let key = Self::session_key(chat_id, thread_id);
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let entry = SessionEntry {
            chat_id: chat_id.to_string(),
            thread_id: thread_id.map(|s| s.to_string()),
            session_id: session_id.to_string(),
            created_at: now.clone(),
            last_active: now,
        };
        {
            let mut data = self.data.write().unwrap();
            data.insert(key, entry);
        }
        let _ = self.save_to_disk();
    }

    /// Get session_id for a chat.
    pub fn get(&self, chat_id: &str, thread_id: Option<&str>) -> Option<String> {
        let key = Self::session_key(chat_id, thread_id);
        let data = self.data.read().unwrap();
        data.get(&key).map(|e| e.session_id.clone())
    }

    /// Remove and reset the session for a chat (used by /reset).
    pub fn reset(&self, chat_id: &str, thread_id: Option<&str>) {
        let key = Self::session_key(chat_id, thread_id);
        let mut data = self.data.write().unwrap();
        data.remove(&key);
        let _ = self.save_to_disk();
    }

    /// Update last active timestamp.
    pub fn touch(&self, chat_id: &str, thread_id: Option<&str>) {
        let key = Self::session_key(chat_id, thread_id);
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let mut data = self.data.write().unwrap();
        if let Some(entry) = data.get_mut(&key) {
            entry.last_active = now;
        }
    }

    fn load_from_disk(&mut self) {
        if let Ok(content) = std::fs::read_to_string(&self.path) {
            if let Ok(store) = serde_json::from_str::<StoreData>(&content) {
                let mut data = self.data.write().unwrap();
                for entry in store.sessions {
                    let key = Self::session_key(&entry.chat_id, entry.thread_id.as_deref());
                    data.insert(key, entry);
                }
            }
        }
    }

    fn save_to_disk(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = self.data.read().unwrap();
        let store = StoreData {
            sessions: data.values().cloned().collect(),
        };
        std::fs::write(&self.path, serde_json::to_string_pretty(&store)?)?;
        Ok(())
    }
}

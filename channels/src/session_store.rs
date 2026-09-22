//! Session store: maps Feishu (chat_id, thread_id) → agent session_id.
//! Persisted as JSON file.

use anyhow::Result;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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
    save_lock: Mutex<()>,
    // Do not overwrite an unreadable/corrupt source with a fresh empty mapping.
    load_error: Option<String>,
}

impl SessionStore {
    pub fn new(path: PathBuf) -> Self {
        let mut store = Self {
            path,
            data: RwLock::new(HashMap::new()),
            save_lock: Mutex::new(()),
            load_error: None,
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
            let data = self.data.read();
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
            let mut data = self.data.write();
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
            let mut data = self.data.write();
            data.insert(key, entry);
        }
        if let Err(error) = self.save_to_disk() {
            tracing::warn!(path = %self.path.display(), %error, "Session mapping was not persisted");
        }
    }

    /// Get session_id for a chat.
    pub fn get(&self, chat_id: &str, thread_id: Option<&str>) -> Option<String> {
        let key = Self::session_key(chat_id, thread_id);
        let data = self.data.read();
        data.get(&key).map(|e| e.session_id.clone())
    }

    /// Remove and reset the session for a chat (used by /new).
    pub fn reset(&self, chat_id: &str, thread_id: Option<&str>) {
        let key = Self::session_key(chat_id, thread_id);
        {
            let mut data = self.data.write();
            data.remove(&key);
        } // write lock dropped before save_to_disk acquires read lock
        if let Err(error) = self.save_to_disk() {
            tracing::warn!(path = %self.path.display(), %error, "Session mapping was not persisted");
        }
    }

    /// Update last active timestamp.
    pub fn touch(&self, chat_id: &str, thread_id: Option<&str>) {
        let key = Self::session_key(chat_id, thread_id);
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let mut data = self.data.write();
        if let Some(entry) = data.get_mut(&key) {
            entry.last_active = now;
        }
    }

    fn load_from_disk(&mut self) {
        let result = std::fs::read_to_string(&self.path)
            .map_err(anyhow::Error::from)
            .and_then(|content| Ok(serde_json::from_str::<StoreData>(&content)?));
        match result {
            Ok(store) => {
                let mut data = self.data.write();
                for entry in store.sessions {
                    let key = Self::session_key(&entry.chat_id, entry.thread_id.as_deref());
                    data.insert(key, entry);
                }
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => {
                tracing::warn!(path = %self.path.display(), %error, "Cannot load session mappings; repair the file and restart before persisting changes");
                self.load_error = Some(error.to_string());
            }
        }
    }

    fn save_to_disk(&self) -> Result<()> {
        let _save = self.save_lock.lock();
        if let Some(error) = &self.load_error {
            anyhow::bail!("Refusing to overwrite unreadable session store: {error}");
        }
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        std::fs::create_dir_all(parent)?;
        let data = self.data.read();
        let store = StoreData {
            sessions: data.values().cloned().collect(),
        };
        // Unique, owner-only temporary file on the destination volume. Readers
        // observe either complete JSON version; failed writes leave the old one.
        let mut pending = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(pending.as_file_mut(), &store)?;
        pending.as_file().sync_all()?;
        pending.persist(&self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique temp path per test so parallel `cargo test` runs don't collide.
    fn temp_store_path(test_name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("future-channel-tests")
            .join(format!("{}-{}", test_name, uuid::Uuid::new_v4()));
        dir.join("sessions.json")
    }

    #[test]
    fn get_or_create_marks_new_and_reuses_existing() {
        let store = SessionStore::new(temp_store_path("create"));
        let (_, is_new) = store.get_or_create("oc_1", None);
        assert!(is_new);
        let (_, is_new) = store.get_or_create("oc_1", None);
        assert!(!is_new, "second call must reuse the existing entry");
    }

    #[test]
    fn threads_get_independent_sessions() {
        let store = SessionStore::new(temp_store_path("threads"));
        let (_, a_new) = store.get_or_create("oc_1", None);
        let (_, b_new) = store.get_or_create("oc_1", Some("omt_thread"));
        assert!(a_new && b_new);

        store.set_session_id("oc_1", None, "sid-root");
        store.set_session_id("oc_1", Some("omt_thread"), "sid-thread");
        assert_eq!(store.get("oc_1", None).as_deref(), Some("sid-root"));
        assert_eq!(
            store.get("oc_1", Some("omt_thread")).as_deref(),
            Some("sid-thread")
        );
    }

    #[test]
    fn empty_thread_id_is_treated_as_no_thread() {
        let store = SessionStore::new(temp_store_path("empty-thread"));
        store.set_session_id("oc_1", None, "sid");
        // "" must key identically to None — otherwise every empty thread_id
        // from the WS event would silently fork the session mapping.
        assert_eq!(store.get("oc_1", Some("")).as_deref(), Some("sid"));
    }

    #[test]
    fn reset_removes_mapping() {
        let store = SessionStore::new(temp_store_path("reset"));
        store.set_session_id("oc_1", None, "sid");
        store.reset("oc_1", None);
        assert_eq!(store.get("oc_1", None), None);
    }

    #[test]
    fn touch_updates_last_active() {
        let store = SessionStore::new(temp_store_path("touch"));
        store.set_session_id("oc_1", None, "sid");
        let before = store.data.read().get("oc_1").map(|e| e.last_active.clone());
        store.touch("oc_1", None);
        let after = store.data.read().get("oc_1").map(|e| e.last_active.clone());
        assert!(before.is_some() && after.is_some());
        // Touching a missing chat must not create an entry.
        store.touch("oc_missing", None);
        assert_eq!(store.get("oc_missing", None), None);
    }

    #[test]
    fn persists_and_reloads_from_disk() {
        let path = temp_store_path("persist");
        {
            let store = SessionStore::new(path.clone());
            store.set_session_id("oc_1", None, "sid-1");
            store.set_session_id("oc_2", Some("t"), "sid-2");
        } // store dropped; data only on disk now

        let reloaded = SessionStore::new(path);
        assert_eq!(reloaded.get("oc_1", None).as_deref(), Some("sid-1"));
        assert_eq!(reloaded.get("oc_2", Some("t")).as_deref(), Some("sid-2"));
    }

    #[test]
    fn corrupt_disk_file_starts_empty() {
        let path = temp_store_path("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json {{{").unwrap();
        let store = SessionStore::new(path);
        assert_eq!(store.get("oc_1", None), None);
    }

    #[test]
    fn corrupt_source_is_preserved_when_memory_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        std::fs::write(&path, "{truncated").unwrap();
        let store = SessionStore::new(path.clone());
        store.set_session_id("chat", None, "new-session");
        assert!(store.save_to_disk().is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "{truncated");
        assert_eq!(store.get("chat", None).as_deref(), Some("new-session"));
    }

    #[test]
    fn concurrent_saves_always_publish_complete_json_and_retain_all_mappings() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let store = Arc::new(SessionStore::new(path.clone()));
        store.set_session_id("initial", None, "initial");
        let done = Arc::new(AtomicBool::new(false));
        let reader_done = done.clone();
        let reader_path = path.clone();
        let reader = std::thread::spawn(move || {
            while !reader_done.load(Ordering::Acquire) {
                let text = std::fs::read_to_string(&reader_path).unwrap();
                serde_json::from_str::<StoreData>(&text).unwrap();
                std::thread::yield_now();
            }
        });
        let writers: Vec<_> = (0..4)
            .map(|worker| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for n in 0..20 {
                        store.set_session_id(&format!("chat-{worker}-{n}"), None, "session");
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        done.store(true, Ordering::Release);
        reader.join().unwrap();
        let reloaded = SessionStore::new(path);
        assert_eq!(reloaded.data.read().len(), 81);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn save_failure_leaves_in_memory_state_intact() {
        // Parent dir is impossible to create (a regular file sits in the
        // path) — save_to_disk errors, but callers swallow it and the
        // in-memory mapping must still work.
        let blocker = temp_store_path("save-blocked");
        std::fs::create_dir_all(blocker.parent().unwrap()).unwrap();
        std::fs::write(&blocker, b"I am a file, not a directory").unwrap();
        let path = blocker.join("child").join("sessions.json");
        let store = SessionStore::new(path);
        store.set_session_id("oc_1", None, "sid-1");
        assert_eq!(store.get("oc_1", None).as_deref(), Some("sid-1"));
        store.reset("oc_1", None);
        assert_eq!(store.get("oc_1", None), None);
    }
}

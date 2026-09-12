//! SQLite session persistence, ordered appends and transactional run recovery.
//!
//! Summary scanning and listing live in [`super::summary`].

use super::entry::{SessionEntry, ENTRY_TYPE_MODEL_CHANGE, ENTRY_TYPE_SESSION_INFO};
#[cfg(test)]
use super::entry::{ENTRY_TYPE_ASSISTANT, ENTRY_TYPE_SYSTEM, ENTRY_TYPE_TOOL};
use super::model::Session;
use super::projection::hydrate_entry_projections;
use super::repair::{dedupe_tool_entries, repair_dangling_tool_calls, strip_empty_assistants};
use super::run_journal::RUN_STATE_INTERRUPTED_BY_RESTART;
use super::sqlite_store::SqliteStore;
use crate::utils::default_session_dir;
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

const DISPLAY_ENTRIES_CACHE_MAX: usize = 12;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SessionRevision {
    revision: i64,
}

struct DisplayEntriesCacheEntry {
    session_id: String,
    version: SessionRevision,
    entries: Arc<Vec<serde_json::Value>>,
}

pub struct Manager {
    pub dir: PathBuf,
    store: OnceLock<SqliteStore>,
    initialization: parking_lot::Mutex<()>,
    /// Bounded LRU of display projections. Pagination requests for one stable
    /// SQLite revision slice this shared projection instead of loading and
    /// projecting the complete journal again for every page.
    display_entries_cache: parking_lot::Mutex<Vec<DisplayEntriesCacheEntry>>,
    /// Test-only save-failure injection (number of saves left to fail).
    #[cfg(test)]
    pub(crate) fail_saves_remaining: std::sync::atomic::AtomicU64,
}

impl Manager {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            store: OnceLock::new(),
            initialization: parking_lot::Mutex::new(()),
            display_entries_cache: parking_lot::Mutex::new(Vec::new()),
            #[cfg(test)]
            fail_saves_remaining: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn default_for(cwd: &str) -> Self {
        Self::new(default_session_dir(cwd))
    }

    #[cfg(test)]
    pub(crate) fn test_execute(&self, sql: &str) {
        let sql = sql.to_owned();
        self.storage()
            .unwrap()
            .db
            .call(move |db| {
                db.execute_batch(&sql)?;
                Ok(())
            })
            .unwrap();
    }

    pub fn database_path(&self) -> PathBuf {
        if self.dir.file_name().and_then(|s| s.to_str()) == Some("sessions") {
            self.dir.parent().unwrap_or(&self.dir).join("agent.db")
        } else {
            self.dir.join("agent.db")
        }
    }

    /// Called before serving RPC, while the Agent instance lock is held.
    pub fn initialize(&self) -> Result<()> {
        self.storage().map(|_| ())
    }

    pub(crate) fn storage(&self) -> Result<&SqliteStore> {
        if let Some(store) = self.store.get() {
            return Ok(store);
        }
        let _guard = self.initialization.lock();
        if self.store.get().is_none() {
            let store = SqliteStore::open(&self.database_path())?;
            store.import_legacy(&self.dir, None)?;
            let _ = self.store.set(store);
        }
        Ok(self.store.get().expect("initialized SQLite store"))
    }

    pub fn retry_legacy_import(&self, id: &str) -> Result<()> {
        self.storage()?.import_legacy(&self.dir, Some(id))
    }

    pub fn import_records(&self) -> Result<Vec<super::ImportRecord>> {
        self.storage()?.import_records()
    }

    pub fn contains(&self, id: &str) -> Result<bool> {
        self.storage()?.contains(id)
    }

    pub(crate) fn history_page(
        &self,
        id: &str,
        before: Option<i64>,
        offset: Option<i64>,
        limit: Option<i64>,
    ) -> Result<serde_json::Value> {
        let id = id.to_owned();
        let page = self.storage()?.db.call(move |db| {
            let tx = db.transaction()?;
            let skipped: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM legacy_imports WHERE session_id=?1 AND status='skipped')", [&id], |r| r.get(0))?;
            if skipped { anyhow::bail!("session migration was skipped"); }
            let page = super::history_index::read_page(&tx, &id, before, offset, limit)?;
            tx.commit()?;
            Ok(page)
        })?;
        super::history_index::materialize(page, before, offset)
    }

    pub(crate) fn session_revision(&self, id: &str) -> Result<SessionRevision> {
        Ok(SessionRevision {
            revision: self
                .storage()?
                .revision(id)?
                .ok_or_else(|| anyhow!("session not found"))?,
        })
    }

    pub(crate) fn cached_display_entries(
        &self,
        id: &str,
        version: &SessionRevision,
    ) -> Option<Arc<Vec<serde_json::Value>>> {
        let mut cache = self.display_entries_cache.lock();
        let index = cache
            .iter()
            .position(|entry| entry.session_id == id && entry.version == *version)?;
        let entry = cache.remove(index);
        let entries = entry.entries.clone();
        cache.push(entry);
        Some(entries)
    }

    pub(crate) fn cache_display_entries(
        &self,
        id: &str,
        version: SessionRevision,
        entries: Arc<Vec<serde_json::Value>>,
    ) {
        let mut cache = self.display_entries_cache.lock();
        cache.retain(|entry| entry.session_id != id);
        cache.push(DisplayEntriesCacheEntry {
            session_id: id.to_string(),
            version,
            entries,
        });
        if cache.len() > DISPLAY_ENTRIES_CACHE_MAX {
            cache.remove(0);
        }
    }

    fn invalidate_display_entries(&self, id: &str) {
        self.display_entries_cache
            .lock()
            .retain(|entry| entry.session_id != id);
    }

    fn encoded(entries: &[SessionEntry]) -> Result<Vec<serde_json::Value>> {
        entries
            .iter()
            .map(|entry| Ok(serde_json::from_str(&Self::serialize_entry(entry)?)?))
            .collect()
    }

    pub fn append_entries(&self, id: &str, entries: &[SessionEntry]) -> Result<()> {
        self.storage()?.append(id, Self::encoded(entries)?)?;
        self.invalidate_display_entries(id);
        Ok(())
    }

    pub fn append_entries_synced(&self, id: &str, entries: &[SessionEntry]) -> Result<()> {
        self.append_entries(id, entries)
    }

    pub fn append_run_start(
        &self,
        id: &str,
        user: SessionEntry,
        started: SessionEntry,
    ) -> Result<()> {
        let id = id.to_owned();
        let values = Self::encoded(&[user, started])?;
        self.storage()?.db.call(move |db| {
            let tx = db.transaction()?;
            let existing: Vec<SessionEntry> = super::sqlite_store::read_run_markers(&tx, &id)?
                .into_iter()
                .map(serde_json::from_value)
                .collect::<std::result::Result<_, _>>()?;
            if let Some(run) = super::find_unterminated_run(&existing) {
                let terminal =
                    SessionEntry::run_terminal(&run, RUN_STATE_INTERRUPTED_BY_RESTART, 0, 0, None);
                super::sqlite_store::insert_entries(&tx, &id, Self::encoded(&[terminal])?)?;
            }
            super::sqlite_store::insert_entries(&tx, &id, values)?;
            tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1", [&id])?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn unterminated_run_id(&self, id: &str) -> Result<Option<String>> {
        if !self.contains(id)? {
            return Ok(None);
        }
        let id = id.to_owned();
        let entries: Vec<SessionEntry> = self
            .storage()?
            .db
            .call(move |db| super::sqlite_store::read_run_markers(db, &id))?
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?;
        Ok(super::find_unterminated_run(&entries))
    }

    pub fn update_session_info(&self, id: &str, key: &str, value: serde_json::Value) -> Result<()> {
        let (id, key) = (id.to_owned(), key.to_owned());
        self.storage()?.db.call(move |db| {
            let tx = db.transaction()?;
            let payload: String = tx.query_row("SELECT payload FROM entry_records WHERE session_id=?1 AND entry_type='session_info' ORDER BY position DESC LIMIT 1", [&id], |row| row.get(0))?;
            let entry: serde_json::Value = serde_json::from_str(&payload)?;
            let mut info = entry["content"].as_object().cloned()
                .ok_or_else(|| anyhow!("session has no session_info object"))?;
            info.insert(key, value);
            let entry = SessionEntry::session_info(
                serde_json::Value::Object(info),
                String::new(),
                String::new(),
            );
            super::sqlite_store::insert_entries(&tx, &id, Self::encoded(&[entry])?)?;
            tx.execute("UPDATE sessions SET revision=revision+1 WHERE id=?1", [&id])?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn save(&self, session: &Session) -> Result<()> {
        #[cfg(test)]
        if self
            .fail_saves_remaining
            .load(std::sync::atomic::Ordering::Acquire)
            > 0
        {
            self.fail_saves_remaining
                .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            return Err(anyhow!("injected session save failure"));
        }
        self.storage()?
            .replace(&session.id, Self::encoded(&session.entries)?)?;
        self.invalidate_display_entries(&session.id);
        Ok(())
    }

    pub(crate) fn serialize_entry(entry: &SessionEntry) -> Result<String> {
        let value = serde_json::to_value(entry).context("serialize entry")?;
        serde_json::to_string(&super::records::canonical_entry(value)).context("serialize entry")
    }

    pub fn load(&self, id: &str) -> Result<Session> {
        let mut entries: Vec<SessionEntry> = self.storage()?.typed_entries(id)?;
        for entry in &mut entries {
            hydrate_entry_projections(entry);
        }
        // Runtime repair is a projection only; importing never calls this path.
        let stripped = strip_empty_assistants(&mut entries);
        let deduped = dedupe_tool_entries(&mut entries);
        let repaired = repair_dangling_tool_calls(&mut entries);
        if stripped || deduped || repaired {
            tracing::info!(
                "Healed session {id} in memory (stripped_empty={stripped}, \
                 deduped_tools={deduped}, repaired_dangling={repaired})"
            );
        }
        self.restore_session_times(Self::session_from_entries(id, entries)?)
    }

    /// Runtime metadata and identities only; never opens historical bodies.
    pub(crate) fn load_metadata(&self, id: &str) -> Result<Session> {
        let session_id = id.to_owned();
        let rows = self.storage()?.db.call(move |db| {
            let mut stmt = db.prepare(
                "SELECT payload FROM history_shapes WHERE session_id=?1 ORDER BY position",
            )?;
            let rows = stmt
                .query_map([session_id], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })?;
        let entries = rows
            .into_iter()
            .map(|row| serde_json::from_str(&row))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.restore_session_times(Self::session_from_entries(id, entries)?)
    }

    pub(super) fn restore_session_times(&self, mut session: Session) -> Result<Session> {
        let id = session.id.clone();
        let (created, updated): (Option<i64>, Option<i64>) =
            self.storage()?.db.call(move |db| {
                Ok(db.query_row(
                    "SELECT created_at_ms,updated_at_ms FROM sessions WHERE id=?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?)
            })?;
        if let Some(created) = created.and_then(chrono::DateTime::from_timestamp_millis) {
            session.created_at = created.with_timezone(&chrono::Local);
        }
        if let Some(updated) = updated.and_then(chrono::DateTime::from_timestamp_millis) {
            session.updated_at = updated.with_timezone(&chrono::Local);
        }
        Ok(session)
    }

    pub(crate) fn session_from_entries(id: &str, entries: Vec<SessionEntry>) -> Result<Session> {
        let created_at = entries
            .first()
            .ok_or_else(|| anyhow!("session has no display entries"))?
            .timestamp;
        let updated_at = entries
            .iter()
            .map(|e| e.timestamp)
            .max()
            .unwrap_or(created_at);
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
            .find(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
            .and_then(|e| e.content.as_ref())
            .and_then(|c| c.get("model"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                entries.iter().rev().find_map(|e| {
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
            version: crate::session::CURRENT_SESSION_VERSION,
            cwd,
            model,
            name,
            parent_session_id,
            leaf_id: String::new(),
            entries,
            created_at,
            updated_at,
        };
        Ok(session)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        self.storage()?.delete(id)?;
        self.invalidate_display_entries(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::projection::{agent_message_to_entry, entries_to_agent_messages};
    use crate::session::repair::{entry_text_starts_with, TOOL_LOST_PLACEHOLDER_PREFIX};
    use crate::session::run_journal::RUN_STATE_COMPLETED;
    use crate::session::{ENTRY_TYPE_RUN_STARTED, ENTRY_TYPE_RUN_TERMINAL};
    use crate::types::ToolCall;
    use crate::utils::generate_id;
    use chrono::Local;

    fn raw_lines(manager: &Manager, id: &str) -> String {
        manager
            .storage()
            .unwrap()
            .entries(id)
            .unwrap()
            .into_iter()
            .map(|v| format!("{v}\n"))
            .collect()
    }

    fn temp_manager(tag: &str) -> (std::path::PathBuf, Manager) {
        let dir = std::env::temp_dir().join(format!("future-{tag}-{}", generate_id()));
        let manager = Manager::new(dir.clone());
        (dir, manager)
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
        let mut session = Session::new("/tmp/test", "gpt-4o");
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

    #[test]
    fn manager_writes_only_canonical_message_blocks_and_rehydrates_projections() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(dir.path().to_path_buf());
        let mut provider_metadata = crate::types::ProviderMetadata::new();
        provider_metadata.insert("anthropic".into(), serde_json::json!({"signature": "sig"}));
        let message = crate::types::AgentMessage {
            role: "assistant".into(),
            content: vec![
                crate::types::ContentBlock::reasoning("thought", provider_metadata),
                crate::types::ContentBlock::text("answer"),
                crate::types::ContentBlock::tool_call(
                    "call-1",
                    "lookup",
                    serde_json::json!({"q": "rust"}),
                    Default::default(),
                ),
            ],
            ..Default::default()
        };
        let mut session = Session::new("/tmp/test", "claude");
        session.entries.push(agent_message_to_entry(&message));
        manager.save(&session).unwrap();

        let disk = raw_lines(&manager, &session.id);
        let assistant: serde_json::Value = disk
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .find(|value: &serde_json::Value| value["type"] == "assistant")
            .unwrap();
        assert!(assistant.get("thinking").is_none());
        assert!(assistant.get("tool_calls").is_none());
        assert_eq!(assistant["content"][0]["type"], "reasoning");
        assert_eq!(
            assistant["content"][0]["provider_metadata"]["anthropic"]["signature"],
            "sig"
        );

        let loaded = manager.load(&session.id).unwrap();
        let assistant = loaded
            .entries
            .iter()
            .find(|entry| entry.entry_type == ENTRY_TYPE_ASSISTANT)
            .unwrap();
        assert_eq!(assistant.thinking, "thought");
        assert_eq!(assistant.tool_calls.len(), 1);
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
        let mut session = Session::new("/tmp/test", "gpt-4o");
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
            &serde_json::json!([{
                "type": "tool_result",
                "tool_call_id": "tc1",
                "content": "real output"
            }]),
            "the real result must win over the placeholder"
        );

        // The file on disk must NOT be rewritten by load: read-only callers
        // (session list, get_session_entries) can run while the owning agent
        // is mid-run, and persisting repairs is what created the duplicates.
        let on_disk = raw_lines(&manager, &session.id);
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
        let mut session = Session::new("/tmp/test", "gpt-4o");
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
            &serde_json::json!([{
                "type": "tool_result",
                "tool_call_id": "tc1",
                "content": "first result"
            }])
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
        let mut session = Session::new("/tmp/test", "gpt-4o");
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

        let on_disk = raw_lines(&manager, &session.id);
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
        let mut session = Session::new("/tmp/test", "gpt-4o");
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

        let on_disk = raw_lines(&manager, &session.id);
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
            crate::session::find_run_terminal(&loaded.entries, "old-run")
                .and_then(|value| value.get("state").cloned()),
            Some(serde_json::json!(RUN_STATE_INTERRUPTED_BY_RESTART))
        );
        assert_eq!(
            crate::session::find_unterminated_run(&loaded.entries),
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
            .position(|entry| entry_text_starts_with(entry, "new"))
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
        let session = Session::new("/tmp/test", "model");
        manager.save(&session).unwrap();
        assert!(manager.contains(&session.id).unwrap());
        manager.delete(&session.id).unwrap();
        assert!(!manager.contains(&session.id).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manager_find_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(dir.path().to_path_buf());
        assert!(!manager.contains("nonexistent_id").unwrap());
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
        let mut session = Session::new("/tmp/test", "gpt-4o");
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
    fn load_legacy_jsonl_rehydrates_string_content_thinking_and_tool_calls() {
        let dir = std::env::temp_dir().join(format!(
            "future_test_legacy_jsonl_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let manager = Manager::new(dir.clone());
        std::fs::write(
            manager.dir.join("legacy.jsonl"),
            concat!(
                r#"{"id":"u","type":"user","role":"user","content":"plain user"}"#, "\n",
                r#"{"id":"a","type":"assistant","role":"assistant","content":"plain answer","thinking":"legacy reasoning","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":{"path":"/tmp/a"}}}]}"#, "\n",
                r#"{"id":"t","type":"tool","role":"tool","content":"legacy result","tool_call_id":"call_1"}"#, "\n",
            ),
        )
        .unwrap();

        let loaded = manager.load("legacy").unwrap();
        assert!(entry_text_starts_with(&loaded.entries[0], "plain"));
        let messages = entries_to_agent_messages(&loaded.entries, false);
        assert_eq!(messages[0].text(), "plain user");
        assert!(matches!(
            messages[1].content.first(),
            Some(crate::types::ContentBlock::Reasoning { text, .. }) if text == "legacy reasoning"
        ));
        assert!(matches!(
            messages[1].content.iter().find(|block| matches!(block, crate::types::ContentBlock::ToolCall { .. })),
            Some(crate::types::ContentBlock::ToolCall { id, name, args, .. })
                if id == "call_1" && name == "read" && args == &serde_json::json!({"path": "/tmp/a"})
        ));
        assert!(matches!(
            messages[2].content.as_slice(),
            [crate::types::ContentBlock::ToolResult { tool_call_id, content, .. }]
                if tool_call_id == "call_1" && content == "legacy result"
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Regression: after a crash between assistant(tool_calls) persist and tool
    /// execution, a subsequent restart appends run markers + a new user message
    /// ahead of the orphaned tool_calls.  `repair_dangling_tool_calls` must find
    /// the orphaned assistant even when it is NOT the last entry, and insert
    /// placeholder tool responses so the conversation stays API-valid.
    #[test]
    fn load_reattaches_late_real_tool_result_without_duplicate_placeholder() {
        let (dir, manager) = temp_manager("late-tool-result");
        let assistant = SessionEntry::new_assistant(
            serde_json::json!("reading"),
            vec![ToolCall {
                id: "late-call".into(),
                call_type: "function".into(),
                function: crate::types::ToolCallFn {
                    name: "read".into(),
                    arguments: serde_json::json!({}),
                },
            }],
        );
        let real = SessionEntry::new_tool("late-call", "real result");
        let real_id = real.id.clone();
        let session = Session::snapshot(
            "late-session".into(),
            "/tmp".into(),
            "mock".into(),
            "late".into(),
            String::new(),
            vec![
                SessionEntry::new_user("user", serde_json::json!("question")),
                SessionEntry::run_started("old-run", 1),
                assistant,
                SessionEntry::run_terminal("old-run", "cancelled", 0, 0, None),
                real,
            ],
        );
        manager.save(&session).unwrap();
        let loaded = manager.load("late-session").unwrap();
        let tools: Vec<_> = loaded
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
            .collect();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].id, real_id);
        let a = loaded
            .entries
            .iter()
            .position(|e| e.entry_type == ENTRY_TYPE_ASSISTANT)
            .unwrap();
        assert_eq!(loaded.entries[a + 1].id, real_id);
        std::fs::remove_dir_all(dir).unwrap();
    }

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
                    SessionEntry::new_assistant(serde_json::json!("let me read that"), vec![tc]),
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
                .is_some_and(|s| s.starts_with(TOOL_LOST_PLACEHOLDER_PREFIX)),
            "placeholder content should start with '{}', got: {:?}",
            TOOL_LOST_PLACEHOLDER_PREFIX,
            placeholder.content
        );

        // The placeholder must appear IMMEDIATELY after the orphaned
        // assistant, before the run_terminal marker — otherwise the API
        // would see a user/tool ordering violation.
        let assistant_idx = loaded
            .entries
            .iter()
            .position(|e| e.entry_type == ENTRY_TYPE_ASSISTANT && !e.tool_calls.is_empty())
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
        let mut pending: std::collections::HashSet<String> = std::collections::HashSet::new();
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
                    for tc in msg.tool_calls() {
                        pending.insert(tc.id.clone());
                    }
                }
                "tool" => {
                    let tool_call_id = msg.tool_call_id();
                    let removed = pending.remove(&tool_call_id);
                    assert!(
                        removed,
                        "tool entry with tool_call_id={} has no matching \
                         assistant tool_call",
                        tool_call_id
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

    #[test]
    fn append_run_start_closes_an_unterminated_run() {
        let (_dir, manager) = temp_manager("runstart-open");
        let snapshot = Session::snapshot(
            "s1".to_string(),
            "/tmp".to_string(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![
                SessionEntry::new_user("user", serde_json::json!("first")),
                SessionEntry::run_started("run-open", 1),
            ],
        );
        manager.save(&snapshot).unwrap();

        manager
            .append_run_start(
                "s1",
                SessionEntry::new_user("user", serde_json::json!("second")),
                SessionEntry::run_started("run-new", 2),
            )
            .unwrap();
        let loaded = manager.load("s1").unwrap();
        // The open run was closed with an interrupted terminal marker.
        let healed = loaded
            .entries
            .iter()
            .find(|e| {
                e.entry_type == ENTRY_TYPE_RUN_TERMINAL
                    && e.content
                        .as_ref()
                        .and_then(|c| c.get("run_id"))
                        .and_then(|v| v.as_str())
                        == Some("run-open")
            })
            .expect("interrupted terminal appended");
        assert_eq!(
            healed.content.as_ref().unwrap()["state"],
            RUN_STATE_INTERRUPTED_BY_RESTART
        );
        // …followed by the new user message and the new run's start marker.
        let last = loaded.entries.last().unwrap();
        assert_eq!(last.entry_type, ENTRY_TYPE_RUN_STARTED);
    }

    #[test]
    fn append_run_start_on_clean_history_appends_two_entries() {
        let (_dir, manager) = temp_manager("runstart-clean");
        let snapshot = Session::snapshot(
            "s1".to_string(),
            "/tmp".to_string(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![
                SessionEntry::new_user("user", serde_json::json!("first")),
                SessionEntry::run_started("run-a", 1),
                SessionEntry::run_terminal("run-a", RUN_STATE_COMPLETED, 1, 1, None),
            ],
        );
        manager.save(&snapshot).unwrap();
        let before = manager.load("s1").unwrap().entries.len();
        manager
            .append_run_start(
                "s1",
                SessionEntry::new_user("user", serde_json::json!("second")),
                SessionEntry::run_started("run-b", 2),
            )
            .unwrap();
        let after = manager.load("s1").unwrap().entries.len();
        assert_eq!(after - before, 2, "no healing entry needed");
    }

    #[test]
    fn dedupe_tool_entries_drops_placeholder_when_real_result_arrives() {
        let (_dir, manager) = temp_manager("dedupe");
        let placeholder = SessionEntry::new_tool(
            "call-1",
            "[Tool execution lost — worker crashed before the result was written]",
        );
        let real = SessionEntry::new_tool("call-1", "real output");
        let snapshot = Session::snapshot(
            "s1".to_string(),
            "/tmp".to_string(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![
                SessionEntry::new_user("user", serde_json::json!("hi")),
                placeholder,
                real,
            ],
        );
        manager.save(&snapshot).unwrap();
        let loaded = manager.load("s1").unwrap();
        let tool_entries: Vec<_> = loaded
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
            .collect();
        assert_eq!(tool_entries.len(), 1);
        assert_eq!(
            tool_entries[0].content.as_ref().unwrap(),
            &serde_json::json!([{
                "type": "tool_result",
                "tool_call_id": "call-1",
                "content": "real output"
            }])
        );
    }

    #[test]
    fn load_reads_model_from_model_change_entries() {
        let (_dir, manager) = temp_manager("load-model-change");
        let snapshot = Session::snapshot(
            "s1".to_string(),
            "/tmp".to_string(),
            String::new(),
            String::new(),
            String::new(),
            vec![
                SessionEntry::new_user("user", serde_json::json!("hi")),
                SessionEntry {
                    id: generate_id(),
                    entry_type: ENTRY_TYPE_MODEL_CHANGE.to_string(),
                    role: ENTRY_TYPE_SYSTEM.to_string(),
                    content: Some(serde_json::json!({"model": "deepseek/deepseek-chat"})),
                    tool_calls: vec![],
                    timestamp: Local::now(),
                    tool_call_id: String::new(),
                    name: String::new(),
                    tool_args: String::new(),
                    thinking: String::new(),
                    meta: None,
                },
            ],
        );
        manager.save(&snapshot).unwrap();
        let loaded = manager.load("s1").unwrap();
        assert_eq!(loaded.model, "deepseek/deepseek-chat");
    }

    #[test]
    fn dedupe_placeholder_detects_array_content_form() {
        let (_dir, manager) = temp_manager("dedupe-array");
        let mut placeholder = SessionEntry::new_tool("call-1", "ignored");
        placeholder.content = Some(serde_json::json!([
            {"type": "text", "text": "[Tool execution lost — worker crashed]"}
        ]));
        let real = SessionEntry::new_tool("call-1", "real output");
        let snapshot = Session::snapshot(
            "s1".to_string(),
            "/tmp".to_string(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![
                SessionEntry::new_user("user", serde_json::json!("hi")),
                placeholder,
                real,
            ],
        );
        manager.save(&snapshot).unwrap();
        let loaded = manager.load("s1").unwrap();
        let tools: Vec<_> = loaded
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_TOOL)
            .collect();
        assert_eq!(tools.len(), 1);
        assert_eq!(
            tools[0].content.as_ref().unwrap(),
            &serde_json::json!([{
                "type": "tool_result",
                "tool_call_id": "call-1",
                "content": "real output"
            }])
        );
    }

    #[cfg(unix)]
    #[test]
    fn load_rejects_session_with_only_blank_lines() {
        let (_dir, manager) = temp_manager("blank-only");
        std::fs::create_dir_all(&manager.dir).unwrap();
        std::fs::write(manager.dir.join("blank.jsonl"), "\n\n  \n").unwrap();
        let error = manager.load("blank").unwrap_err();
        assert!(error.to_string().contains("has no entries"), "{error}");
    }

    #[test]
    fn delete_without_run_data_is_ok() {
        let (_dir, manager) = temp_manager("delete-no-run-data");
        let session = Session::snapshot(
            "del".to_string(),
            "/x".to_string(),
            "m".to_string(),
            "n".to_string(),
            String::new(),
            vec![SessionEntry::new_user("user", serde_json::json!("hi"))],
        );
        manager.save(&session).unwrap();
        manager.delete("del").unwrap();
        assert!(!manager.contains("del").unwrap());
    }

    #[test]
    fn serialize_entry_empty_content_skips_content_reinsert() {
        // Non-array/non-string content → `_ => Vec::new()` (empty blocks) and
        // the `!blocks.is_empty()` guard skips re-inserting `content`.
        let mut entry = SessionEntry::new_user("user", serde_json::json!("x"));
        entry.content = None;
        let json = Manager::serialize_entry(&entry).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.get("content").is_none());
        assert!(value.get("thinking").is_none());
    }

    #[test]
    fn serialize_entry_tool_with_non_text_block_extracts_empty_text() {
        // A tool entry whose only block is a reasoning block exercises the
        // `_ => None` arm of the text-extraction filter and still emits a
        // synthetic (empty) tool_result.
        let mut tool = SessionEntry::new_tool("tc1", "result");
        tool.content = Some(serde_json::json!([serde_json::to_value(
            crate::types::ContentBlock::reasoning("reason", Default::default())
        )
        .unwrap()]));
        let json = Manager::serialize_entry(&tool).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let content = value.get("content").unwrap().as_array().unwrap();
        assert!(content
            .iter()
            .any(|b| { b.get("type").and_then(|t| t.as_str()) == Some("tool_result") }));
    }
}

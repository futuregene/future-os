//! JSONL persistence for sessions: append/save/load, atomic writes, advisory
//! file locking, run recovery, and orphaned run-data reclamation.
//!
//! Summary scanning and listing live in [`super::summary`].

use super::entry::{
    SessionEntry, ENTRY_TYPE_ASSISTANT, ENTRY_TYPE_MODEL_CHANGE, ENTRY_TYPE_RUN_STARTED,
    ENTRY_TYPE_RUN_TERMINAL, ENTRY_TYPE_SESSION_INFO, ENTRY_TYPE_SYSTEM, ENTRY_TYPE_TOOL,
    ENTRY_TYPE_USER,
};
use super::model::Session;
use super::projection::hydrate_entry_projections;
use super::repair::{dedupe_tool_entries, repair_dangling_tool_calls, strip_empty_assistants};
use super::run_journal::RUN_STATE_INTERRUPTED_BY_RESTART;
use crate::utils::default_session_dir;
use anyhow::{anyhow, Context, Result};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub struct Manager {
    pub dir: PathBuf,
    /// Test-only save-failure injection (number of saves left to fail).
    #[cfg(test)]
    pub(crate) fail_saves_remaining: std::sync::atomic::AtomicU64,
}

impl Manager {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            #[cfg(test)]
            fail_saves_remaining: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn default_for(cwd: &str) -> Self {
        Self::new(default_session_dir(cwd))
    }

    pub(crate) fn session_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.jsonl", id))
    }

    /// Agent-owned event data for a session. Queued prompts are intentionally
    /// in-memory only and never written below this path.
    fn run_data_root(&self) -> PathBuf {
        let run_events_dir =
            if self.dir.file_name().and_then(|name| name.to_str()) == Some("sessions") {
                self.dir.parent().unwrap_or(&self.dir).join("run-events")
            } else {
                self.dir.join(".run-events")
            };
        run_events_dir
    }

    pub fn run_data_path(&self, id: &str) -> PathBuf {
        self.run_data_root().join(id)
    }

    /// Reclaim Agent-owned run data whose transcript no longer exists. This is
    /// safe at startup before sessions are hydrated; live deletion uses the
    /// same transcript-as-commit-point rule.
    pub fn gc_orphan_run_data(&self) -> Result<usize> {
        let root = self.run_data_root();
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        let mut removed = 0;
        // The file_name/to_str `?`s inside the closure skip non-directories
        // and non-UTF-8 names on the same lines, so those (Linux-only)
        // defensive edges share regions with the common path.
        for (path, session_id) in entries.flatten().filter_map(|entry| {
            let path = entry.path();
            let session_id = path.file_name()?.to_str()?.to_string();
            path.is_dir().then_some((path, session_id))
        }) {
            if !self.session_path(&session_id).exists() {
                fs::remove_dir_all(&path)?;
                removed += 1;
            }
        }
        Ok(removed)
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
            let json = Self::serialize_entry(entry)?;
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
            let json = Self::serialize_entry(entry)?;
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

    fn serialize_entry(entry: &SessionEntry) -> Result<String> {
        let mut value = serde_json::to_value(entry).context("serialize entry")?;
        if matches!(
            entry.entry_type.as_str(),
            ENTRY_TYPE_USER | ENTRY_TYPE_ASSISTANT | ENTRY_TYPE_TOOL | ENTRY_TYPE_SYSTEM
        ) {
            if let Some(object) = value.as_object_mut() {
                let mut blocks: Vec<crate::types::ContentBlock> = match &entry.content {
                    Some(serde_json::Value::Array(values)) => values
                        .iter()
                        .filter_map(|value| serde_json::from_value(value.clone()).ok())
                        .collect(),
                    Some(serde_json::Value::String(text)) => {
                        vec![crate::types::ContentBlock::text(text)]
                    }
                    _ => Vec::new(),
                };
                if !entry.thinking.is_empty()
                    && !blocks
                        .iter()
                        .any(|block| matches!(block, crate::types::ContentBlock::Reasoning { .. }))
                {
                    blocks.insert(
                        0,
                        crate::types::ContentBlock::reasoning(&entry.thinking, Default::default()),
                    );
                }
                if !blocks
                    .iter()
                    .any(|block| matches!(block, crate::types::ContentBlock::ToolCall { .. }))
                {
                    blocks.extend(entry.tool_calls.iter().map(|call| {
                        crate::types::ContentBlock::tool_call(
                            &call.id,
                            &call.function.name,
                            call.function.arguments.clone(),
                            Default::default(),
                        )
                    }));
                }
                if entry.entry_type == ENTRY_TYPE_TOOL
                    && !blocks
                        .iter()
                        .any(|block| matches!(block, crate::types::ContentBlock::ToolResult { .. }))
                {
                    let text = blocks
                        .iter()
                        .filter_map(|block| match block {
                            crate::types::ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    blocks
                        .retain(|block| !matches!(block, crate::types::ContentBlock::Text { .. }));
                    blocks.push(crate::types::ContentBlock::tool_result(
                        &entry.tool_call_id,
                        text,
                        false,
                    ));
                }
                if !blocks.is_empty() {
                    object.insert("content".into(), serde_json::to_value(blocks)?);
                }
                object.remove("thinking");
                object.remove("tool_calls");
                object.remove("tool_call_id");
                object.remove("name");
                object.remove("tool_args");
            }
        }
        serde_json::to_string(&value).context("serialize entry")
    }

    pub fn load(&self, id: &str) -> Result<Session> {
        let path = self.session_path(id);
        self.load_path(&path, id)
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
                Ok(mut entry) => {
                    hydrate_entry_projections(&mut entry);
                    entries.push(entry);
                }
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
        // owning agent process is still mid-run.  Persisting a placeholder
        // for a dangling tool_call at that moment corrupts the file: when the
        // running tool finishes, its real result is appended with the same
        // tool_call_id, producing duplicate tool messages that the LLM API
        // rejects with HTTP 400.  The in-memory heal is idempotent and cheap,
        // and the owning session's next save() persists the healed state.
        let stripped = strip_empty_assistants(&mut entries);
        let deduped = dedupe_tool_entries(&mut entries);
        let repaired = repair_dangling_tool_calls(&mut entries);
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
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(anyhow!("failed to delete session: {}", error)),
        }

        // The session transcript is the deletion commit point. Once it is
        // gone, reclaim every Agent-owned event derivative below this
        // directory. The in-memory scheduler is fenced separately by the RPC
        // deletion path. A missing directory is the normal legacy case.
        let run_data_path = self.run_data_path(id);
        match fs::remove_dir_all(&run_data_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(anyhow!(
                    "session deleted but failed to reclaim run data at {}: {}",
                    run_data_path.display(),
                    error
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::projection::{agent_message_to_entry, entries_to_agent_messages};
    use crate::session::repair::{entry_text_starts_with, TOOL_LOST_PLACEHOLDER_PREFIX};
    use crate::session::run_journal::RUN_STATE_COMPLETED;
    use crate::types::ToolCall;
    use crate::utils::generate_id;
    use chrono::Local;

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

        let disk = std::fs::read_to_string(manager.session_path(&session.id)).unwrap();
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
    fn orphan_run_data_gc_preserves_sessions_with_transcripts() {
        let (dir, manager) = temp_manager("orphan-run-data");
        std::fs::create_dir_all(manager.run_data_path("orphan")).unwrap();
        std::fs::create_dir_all(manager.run_data_path("live")).unwrap();
        std::fs::write(
            manager.run_data_path("orphan").join("run.jsonl"),
            b"event\n",
        )
        .unwrap();

        let session = Session::snapshot(
            "live".to_string(),
            "/tmp".to_string(),
            "test-model".to_string(),
            "live".to_string(),
            String::new(),
            vec![SessionEntry::new_user("user", serde_json::json!("hi"))],
        );
        manager.save(&session).unwrap();

        assert_eq!(manager.gc_orphan_run_data().unwrap(), 1);
        assert!(!manager.run_data_path("orphan").exists());
        assert!(manager.run_data_path("live").exists());
        std::fs::remove_dir_all(dir).ok();
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
        let run_data_path = manager.run_data_path(&session.id);
        std::fs::create_dir_all(&run_data_path).unwrap();
        std::fs::write(run_data_path.join("run-event.jsonl"), b"event\n").unwrap();
        assert!(manager.find(&session.id).is_some());
        assert!(run_data_path.exists());

        manager.delete(&session.id).unwrap();
        assert!(manager.find(&session.id).is_none());
        assert!(!run_data_path.exists());

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
            manager.session_path("legacy"),
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
    fn gc_orphan_run_data_handles_missing_root_and_stray_files() {
        let (dir, manager) = temp_manager("gc-missing");
        // No run-events root at all → Ok(0).
        assert_eq!(manager.gc_orphan_run_data().unwrap(), 0);
        // A stray FILE under the root is skipped (only dirs are reclaimed).
        let root = manager.run_data_path("x").parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("stray-file"), "x").unwrap();
        assert_eq!(manager.gc_orphan_run_data().unwrap(), 0);
        assert!(root.join("stray-file").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn load_skips_corrupt_last_line_but_rejects_corrupt_middle_line() {
        let (_dir, manager) = temp_manager("corrupt-tail");
        let snapshot = Session::snapshot(
            "s1".to_string(),
            "/tmp".to_string(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![SessionEntry::new_user("user", serde_json::json!("ok"))],
        );
        manager.save(&snapshot).unwrap();
        let path = manager.find("s1").unwrap();
        // Append a half-written (corrupt) line — a crash during append.
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push_str("{\"id\":\"partial\",\"timestamp\":\"2026");
        std::fs::write(&path, content).unwrap();
        let loaded = manager.load("s1").unwrap();
        assert_eq!(loaded.entries.len(), 1, "corrupt tail skipped");

        // A corrupt MIDDLE line is a hard error.
        let valid =
            serde_json::to_string(&SessionEntry::new_user("user", serde_json::json!("x"))).unwrap();
        std::fs::write(&path, format!("{valid}\n{{corrupt\n{valid}\n")).unwrap();
        let err = manager.load("s1").unwrap_err();
        assert!(err.to_string().contains("parse entry at line 2"));
    }

    #[test]
    fn delete_reclaims_run_data_directory() {
        let (dir, manager) = temp_manager("delete-gc");
        let snapshot = Session::snapshot(
            "s1".to_string(),
            "/tmp".to_string(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![SessionEntry::new_user("user", serde_json::json!("hi"))],
        );
        manager.save(&snapshot).unwrap();
        std::fs::create_dir_all(manager.run_data_path("s1")).unwrap();
        manager.delete("s1").unwrap();
        assert!(!manager.run_data_path("s1").exists());
        let _ = std::fs::remove_dir_all(dir);
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
    fn load_skips_blank_lines_and_rejects_all_corrupt_files() {
        let (_dir, manager) = temp_manager("load-blanks");
        let snapshot = Session::snapshot(
            "s1".to_string(),
            "/tmp".to_string(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![SessionEntry::new_user("user", serde_json::json!("x"))],
        );
        manager.save(&snapshot).unwrap();
        let path = manager.find("s1").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("\n\n{content}\n")).unwrap();
        let loaded = manager.load("s1").unwrap();
        assert_eq!(loaded.entries.len(), 1);

        // A file whose only line is corrupt-but-last degrades to "no entries".
        std::fs::write(&path, "{corrupt\n").unwrap();
        assert!(manager.load("s1").is_err());
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
    fn gc_orphan_run_data_reclaims_and_skips_mixed_entries() {
        // A stray FILE among the orphan dirs exercises the is_dir filter.
        let (_dir, manager) = temp_manager("gc-mixed");
        let root = manager.run_data_root();
        std::fs::create_dir_all(root.join("orphan")).unwrap();
        std::fs::write(root.join("stray-file"), "x").unwrap();
        manager.gc_orphan_run_data().unwrap();
        assert!(!root.join("orphan").exists());
        assert!(root.join("stray-file").exists());
    }

    /// Session file exercising every scan-skip arm: blank line, cheap-matched
    /// but unparseable marker, marker without run_id, terminal for a
    /// different run.
    fn write_scan_edge_file(manager: &Manager, id: &str) {
        std::fs::create_dir_all(&manager.dir).unwrap();
        let path = manager.session_path(id);
        let info = SessionEntry::session_info(
            serde_json::json!({"cwd": "/x", "model": "m"}),
            "m".to_string(),
            "low".to_string(),
        );
        let mut lines = vec![serde_json::to_string(&info).unwrap()];
        lines.push(String::new()); // blank line
                                   // Parses, but content carries no run_id.
        lines.push(
            r#"{"id":"r0","type":"run_started","role":"system","content":{},"timestamp":"2024-01-02T03:04:05+08:00"}"#.to_string(),
        );
        // Terminal for a run that is not open.
        lines.push(
            r#"{"id":"r1","type":"run_terminal","role":"system","content":{"run_id":"other"},"timestamp":"2024-01-02T03:04:06+08:00"}"#.to_string(),
        );
        // Unparseable cheap-matched marker LAST: append validation only
        // tolerates a trailing fragment.
        lines.push(r#"{"type":"run_started",BROKEN"#.to_string());
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn unterminated_run_id_skips_malformed_marker_lines() {
        let (_dir, manager) = temp_manager("scan-edges");
        write_scan_edge_file(&manager, "scan");
        assert_eq!(manager.unterminated_run_id("scan").unwrap(), None);
    }

    #[test]
    fn append_run_start_skips_malformed_marker_lines() {
        let (_dir, manager) = temp_manager("append-scan-edges");
        write_scan_edge_file(&manager, "scan");
        let entry = SessionEntry::new_user("user", serde_json::json!("hi"));
        let started = SessionEntry::run_started("run-new", 1);
        manager.append_run_start("scan", entry, started).unwrap();
        // Full load rejects the trailing fragment, so verify via raw bytes.
        let raw = std::fs::read_to_string(manager.session_path("scan")).unwrap();
        assert!(raw.contains("run-new"), "{raw}");
    }

    #[test]
    fn update_session_info_skips_blank_and_broken_lines() {
        let (_dir, manager) = temp_manager("update-info-edges");
        std::fs::create_dir_all(&manager.dir).unwrap();
        let info = SessionEntry::session_info(
            serde_json::json!({"cwd": "/x", "model": "old"}),
            "old".to_string(),
            "low".to_string(),
        );
        let lines = [
            serde_json::to_string(&info).unwrap(),
            String::new(),
            // Broken session_info LAST: append validation tolerates only a
            // trailing fragment, and the scan's parse-fail arm sees it too.
            r#"{"type":"session_info",BROKEN"#.to_string(),
        ];
        std::fs::write(manager.session_path("upd"), lines.join("\n") + "\n").unwrap();
        manager
            .update_session_info("upd", "model", serde_json::json!("new"))
            .unwrap();
        // The appended info entry sits after the trailing fragment, which a
        // strict full load would reject — verify via raw bytes instead.
        let raw = std::fs::read_to_string(manager.session_path("upd")).unwrap();
        assert!(raw.contains("\"new\""), "{raw}");
    }

    #[test]
    fn save_retries_rename_onto_directory_then_gives_up() {
        let (_dir, manager) = temp_manager("rename-retry");
        // <id>.jsonl as a DIRECTORY: every rename attempt fails with EISDIR
        // (root-immune), exhausting the retry loop.
        std::fs::create_dir_all(&manager.dir).unwrap();
        std::fs::create_dir(manager.session_path("retry")).unwrap();
        let session = Session::snapshot(
            "retry".to_string(),
            "/x".to_string(),
            "m".to_string(),
            "n".to_string(),
            String::new(),
            vec![SessionEntry::new_user("user", serde_json::json!("hi"))],
        );
        let error = manager.save(&session).unwrap_err();
        assert!(
            format!("{error:#}").contains("rename temp to final"),
            "{error:#}"
        );
    }

    #[test]
    fn load_rejects_session_with_only_blank_lines() {
        let (_dir, manager) = temp_manager("blank-only");
        std::fs::create_dir_all(&manager.dir).unwrap();
        std::fs::write(manager.session_path("blank"), "\n\n  \n").unwrap();
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
        assert!(!manager.session_path("del").exists());
    }

    #[test]
    fn delete_reports_run_data_reclaim_failure() {
        let (_dir, manager) = temp_manager("delete-reclaim-fail");
        let session = Session::snapshot(
            "del2".to_string(),
            "/x".to_string(),
            "m".to_string(),
            "n".to_string(),
            String::new(),
            vec![SessionEntry::new_user("user", serde_json::json!("hi"))],
        );
        manager.save(&session).unwrap();
        // Run-data path occupied by a regular FILE: remove_dir_all fails with
        // a non-NotFound error (root-immune).
        let run_data = manager.run_data_path("del2");
        std::fs::create_dir_all(run_data.parent().unwrap()).unwrap();
        std::fs::write(&run_data, "not a directory").unwrap();
        let error = manager.delete("del2").unwrap_err();
        assert!(
            error.to_string().contains("failed to reclaim run data"),
            "{error}"
        );
    }

    #[test]
    fn gc_orphan_run_data_reports_non_notfound_read_dir_error() {
        let (dir, manager) = temp_manager("gc-notdir");
        // A regular FILE where the run-events root directory should be makes
        // read_dir return a non-NotFound error (ENOTDIR), not Ok(0).
        std::fs::create_dir_all(&dir).unwrap();
        let root = manager.run_data_root();
        std::fs::write(&root, "not a directory").unwrap();
        let error = manager.gc_orphan_run_data().unwrap_err();
        assert!(!error.to_string().is_empty());
        let _ = std::fs::remove_dir_all(dir);
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

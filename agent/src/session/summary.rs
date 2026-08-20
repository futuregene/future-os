//! Cheap JSONL summary scanning and session listing.
//!
//! These methods live in a separate `impl Manager` block because they form a
//! distinct concern: turning session files on disk into lightweight
//! `SessionSummary` listings without ever deserializing the large tool /
//! assistant payload lines. `cheap_entry_type` / `cheap_timestamp` are the
//! shared low-level scanners (also used by the run-recovery path in
//! [`super::manager`]).

use super::entry::{
    SessionEntry, ENTRY_TYPE_MODEL_CHANGE, ENTRY_TYPE_SESSION_INFO, ENTRY_TYPE_USER,
};
use super::model::{Session, SessionSummary};
use super::projection::truncate_visible;
use super::Manager;
use anyhow::Result;
use chrono::{DateTime, Local};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;

impl Manager {
    /// Extract the `"type":"..."` value from a serialized entry line without
    /// parsing the whole JSON.  `SessionEntry` serializes `id` first and
    /// `type` second (struct field order is deterministic), so the marker
    /// always appears within the first ~80 bytes regardless of how large the
    /// `content` payload is.
    pub(crate) fn cheap_entry_type(line: &str) -> Option<&str> {
        // Boundary-safe head slice: a multi-byte char may straddle byte 96.
        let head = line.get(..96).unwrap_or(line);
        let start = head.find("\"type\":\"")? + 8;
        let end = head[start..].find('"')? + start;
        Some(&head[start..end])
    }

    /// Extract the last `"timestamp":"..."` occurrence from a line without
    /// parsing the whole JSON.  Used for `updated_at` from the final entry,
    /// which may itself be a huge tool-result line.
    pub(crate) fn cheap_timestamp(line: &str) -> Option<DateTime<Local>> {
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

    /// List the ids of every session in the flat sessions directory, WITHOUT
    /// reading any file contents.
    ///
    /// This is the safe primitive for client-side reconciliation (e.g. the
    /// GUI's orphan-thread cleanup): a session whose JSONL is momentarily
    /// unreadable, truncated, or corrupt still has a file on disk, so its id
    /// must still be reported as live. Enumerating by filename alone means a
    /// transient read failure can never be mistaken for "the session was
    /// deleted" — the exact misclassification that would hard-delete a
    /// client's mirror thread. Only a genuine directory-listing error surfaces
    /// as `Err` (callers treat that as "unknown state, delete nothing").
    pub fn list_ids(&self) -> Result<Vec<String>> {
        if !self.dir.exists() {
            return Ok(vec![]);
        }
        // Non-jsonl entries, non-UTF-8 stems, and empty stems all map to None
        // inside the closure, sharing regions with the common path.
        let mut ids = vec![];
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            let id = (path.extension().and_then(|s| s.to_str()) == Some("jsonl"))
                .then(|| path.file_stem().and_then(|s| s.to_str()))
                .flatten()
                .filter(|id| !id.is_empty());
            if let Some(id) = id {
                ids.push(id.to_string());
            }
        }
        Ok(ids)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::entry::ENTRY_TYPE_SYSTEM;
    use crate::utils::generate_id;

    fn temp_manager(tag: &str) -> (std::path::PathBuf, Manager) {
        let dir = std::env::temp_dir().join(format!("future-{tag}-{}", generate_id()));
        let manager = Manager::new(dir.clone());
        (dir, manager)
    }

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
    fn summary_first_message_non_textual_content_is_none() {
        let entry = SessionEntry::new_user("user", serde_json::json!(42));
        assert!(Manager::summary_first_message(&entry).is_none());
    }

    #[test]
    fn list_all_enumerates_and_filters_by_cwd() {
        let (dir, manager) = temp_manager("list-all");
        for (id, cwd) in [("s-a", "/ws/one"), ("s-b", "/ws/two")] {
            let snapshot = Session::snapshot(
                id.to_string(),
                cwd.to_string(),
                "mock".to_string(),
                String::new(),
                String::new(),
                vec![SessionEntry::new_user("user", serde_json::json!("hi"))],
            );
            manager.save(&snapshot).unwrap();
        }
        let all = manager.list_all().unwrap();
        assert_eq!(all.len(), 2);
        // Directory missing → empty list.
        let (dir2, missing) = temp_manager("list-missing");
        drop(dir2);
        assert!(missing.list_all().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn summary_reads_model_change_and_fallback_mtime() {
        let (_dir, manager) = temp_manager("summary");
        let snapshot = Session::snapshot(
            "s1".to_string(),
            "/tmp".to_string(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![
                SessionEntry::session_info(
                    serde_json::json!({"cwd": "/tmp", "model": "first/model"}),
                    "first/model".to_string(),
                    "low".to_string(),
                ),
                SessionEntry::new_user("user", serde_json::json!("hi")),
                SessionEntry {
                    id: generate_id(),
                    entry_type: ENTRY_TYPE_MODEL_CHANGE.to_string(),
                    role: ENTRY_TYPE_SYSTEM.to_string(),
                    content: Some(serde_json::json!({"model": "second/model"})),
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
        let summaries = manager.list_all().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].model, "second/model");
    }

    #[test]
    fn list_all_uses_parallel_workers_for_many_sessions() {
        let (dir, manager) = temp_manager("list-parallel");
        for i in 0..12 {
            let snapshot = Session::snapshot(
                format!("s-{i}"),
                "/tmp".to_string(),
                "mock".to_string(),
                String::new(),
                String::new(),
                vec![SessionEntry::new_user("user", serde_json::json!("hi"))],
            );
            manager.save(&snapshot).unwrap();
        }
        let all = manager.list_all().unwrap();
        assert_eq!(all.len(), 12);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn summary_reads_name_parent_and_first_message() {
        let (dir, manager) = temp_manager("summary-rich");
        let snapshot = Session::snapshot(
            "s1".to_string(),
            "/tmp".to_string(),
            "mock".to_string(),
            String::new(),
            String::new(),
            vec![
                SessionEntry::session_info(
                    serde_json::json!({
                        "cwd": "/tmp",
                        "model": "mock",
                        "session_name": "Rich Name",
                        "parent_session_id": "parent-1"
                    }),
                    "mock".to_string(),
                    "low".to_string(),
                ),
                SessionEntry::new_user("user", serde_json::json!("the first question")),
            ],
        );
        manager.save(&snapshot).unwrap();
        let summaries = manager.list_summaries("").unwrap();
        let s = summaries.iter().find(|s| s.id == "s1").unwrap();
        assert_eq!(s.name.as_deref(), Some("Rich Name"));
        assert_eq!(s.parent_session_id, "parent-1");
        assert_eq!(s.first_message.as_deref(), Some("the first question"));
        // cwd filter narrows the list.
        assert!(manager
            .list_summaries("/tmp")
            .unwrap()
            .iter()
            .any(|s| s.id == "s1"));
        assert!(manager.list_summaries("/elsewhere").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn summary_from_session_reads_session_info_name() {
        // The full-load fallback path: a session_info entry whose content
        // carries a non-empty session_name supplies the summary name when the
        // Session's own name field is empty ("last non-empty wins").
        let info = SessionEntry::session_info(
            serde_json::json!({"session_name": "  Renamed Chat  "}),
            "m".to_string(),
            String::new(),
        );
        let older = SessionEntry::session_info(
            serde_json::json!({"session_name": "First Name"}),
            "m".to_string(),
            String::new(),
        );
        // A session_info entry WITHOUT a session_name key is skipped over.
        let nameless = SessionEntry::session_info(
            serde_json::json!({"tokens_in": 5}),
            "m".to_string(),
            String::new(),
        );
        let sess = Session::snapshot(
            "s".to_string(),
            "/x".to_string(),
            "m".to_string(),
            String::new(),
            String::new(),
            vec![
                older,
                SessionEntry::new_user("user", serde_json::json!("hi")),
                nameless,
                info,
            ],
        );
        let summary = Manager::summary_from_session(&sess);
        assert_eq!(summary.name.as_deref(), Some("Renamed Chat"));
        assert_eq!(summary.query_count, 1);
    }

    #[test]
    fn try_push_summary_ignores_non_jsonl_directly() {
        // list_all pre-filters by extension, so this guard never fires through
        // the public path; call the helper directly to pin the defense.
        let (_dir, manager) = temp_manager("push-non-jsonl");
        let mut summaries = vec![];
        manager.try_push_summary(Path::new("notes.txt"), &mut summaries);
        assert!(summaries.is_empty());
    }

    #[test]
    fn read_summary_handles_missing_fields_and_mtime_fallback() {
        let (_dir, manager) = temp_manager("summary-edges");
        std::fs::create_dir_all(&manager.dir).unwrap();
        let lines: Vec<String> = vec![
            // session_info with content present but no session_name/model keys.
            r#"{"id":"i0","type":"session_info","role":"system","content":{},"timestamp":"2024-01-02T03:04:05+08:00"}"#.to_string(),
            // session_info with no content at all.
            r#"{"id":"i1","type":"session_info","role":"system","timestamp":"2024-01-02T03:04:06+08:00"}"#.to_string(),
            // model_change with no content.
            r#"{"id":"i2","type":"model_change","role":"system","timestamp":"2024-01-02T03:04:07+08:00"}"#.to_string(),
            String::new(),
            // Last line: no timestamp substring -> cheap_timestamp falls back
            // to the file mtime for updated_at.
            r#"{"id":"u0","type":"user","role":"user","content":"hello"}"#.to_string(),
        ];
        std::fs::write(manager.session_path("sum"), lines.join("\n") + "\n").unwrap();
        let summaries = manager.list_summaries("").unwrap();
        let summary = summaries.iter().find(|s| s.id == "sum").unwrap();
        assert_eq!(summary.query_count, 1);
    }

    #[test]
    fn summary_fallback_uses_full_load_for_unscannable_files() {
        let (_dir, manager) = temp_manager("summary-fallback");
        std::fs::create_dir_all(&manager.dir).unwrap();
        // A content-less session_info is followed by a broken session_info as
        // the LAST line: the cheap scanner aborts (None) and the summary
        // falls back to a full load, which tolerates the trailing fragment.
        let lines = [
            r#"{"id":"i0","type":"session_info","role":"system","timestamp":"2024-01-02T03:04:05+08:00"}"#.to_string(),
            r#"{"id":"u0","type":"user","role":"user","content":"hi","timestamp":"2024-01-02T03:04:06+08:00"}"#.to_string(),
            r#"{"type":"session_info",BROKEN"#.to_string(),
        ];
        std::fs::write(manager.session_path("legacy"), lines.join("\n") + "\n").unwrap();
        let summaries = manager.list_summaries("").unwrap();
        assert!(
            summaries.iter().any(|s| s.id == "legacy"),
            "fallback produced a summary: {summaries:?}"
        );
    }

    #[test]
    fn list_ids_skips_non_jsonl_files() {
        let (_dir, manager) = temp_manager("ids-stray");
        std::fs::create_dir_all(&manager.dir).unwrap();
        std::fs::write(manager.dir.join("notes.txt"), "hello").unwrap();
        std::fs::write(manager.dir.join("real.jsonl"), "{}").unwrap();
        assert_eq!(manager.list_ids().unwrap(), vec!["real".to_string()]);
    }

    #[test]
    fn list_summaries_skips_non_jsonl_files() {
        let (_dir, manager) = temp_manager("summaries-stray");
        std::fs::create_dir_all(&manager.dir).unwrap();
        std::fs::write(manager.dir.join("notes.txt"), "hello").unwrap();
        assert!(manager.list_summaries("").unwrap().is_empty());
    }

    #[test]
    fn list_ids_reports_files_even_when_unreadable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = Manager::new(dir.path().to_path_buf());
        std::fs::write(dir.path().join("good.jsonl"), "{}").unwrap();
        // A corrupt/truncated file is still a live session: its id MUST be
        // listed (the orphan cleanup relies on this so a transient read
        // failure is never mistaken for a deleted session).
        std::fs::write(dir.path().join("corrupt.jsonl"), "{ not json").unwrap();
        // A non-jsonl file (e.g. a lock or stray) must be ignored.
        std::fs::write(dir.path().join("notes.txt"), "hi").unwrap();

        let mut ids = manager.list_ids().unwrap();
        ids.sort();
        assert_eq!(ids, vec!["corrupt".to_string(), "good".to_string()]);
    }

    #[test]
    fn list_ids_empty_when_dir_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nope");
        let manager = Manager::new(missing);
        assert!(manager.list_ids().unwrap().is_empty());
    }
}

//! SQLite-backed session listings.
use super::entry::{SessionEntry, ENTRY_TYPE_SESSION_INFO};
use super::model::{Session, SessionSummary};
use super::projection::first_message_title;
use super::Manager;
use anyhow::Result;

impl Manager {
    /// Extract the display text of a user entry's content (first text block),
    /// rendered as plain text and truncated to ~40 visible columns for the
    /// session list.
    pub(crate) fn summary_first_message(entry: &SessionEntry) -> Option<String> {
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
        let truncated: String = first_message_title(&text);
        if truncated.is_empty() {
            None
        } else {
            Some(truncated)
        }
    }

    /// Build a summary from metadata/user entries (or a full session in tests).
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
            created_at: sess.created_at,
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

    pub fn list_summaries(&self, cwd: &str) -> Result<Vec<SessionSummary>> {
        let mut summaries = self.list_all()?;
        if !cwd.is_empty() {
            summaries.retain(|s| s.cwd == cwd);
        }
        Ok(summaries)
    }

    /// Include skipped migration identities so clients do not delete their mirrors.
    pub fn list_ids(&self) -> Result<Vec<String>> {
        self.storage()?.ids(true)
    }

    pub fn list_all(&self) -> Result<Vec<SessionSummary>> {
        let store = self.storage()?;
        let (batch, legacy) =
            super::content::reader::Snapshot::batch_summaries(store, store.ids(false)?)?;
        let mut summaries = Vec::with_capacity(batch.len() + legacy.len());
        for row in batch {
            if row.needs_preview_fallback {
                summaries.push(super::content::reader::Snapshot::summary(
                    store,
                    &row.summary.id,
                )?);
            } else {
                summaries.push(row.summary);
            }
        }
        for id in legacy {
            let (rows, timestamp, query_count) = store.summary_rows(&id)?;
            let mut entries = rows
                .into_iter()
                .map(|row| serde_json::from_str::<SessionEntry>(&row))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            // A lightweight tail preserves ordering, including histories with
            // no metadata/user rows. No display repair is needed for a list.
            entries.push(serde_json::from_value(serde_json::json!({
                "id": "summary-tail", "type": "summary-tail", "timestamp": timestamp
            }))?);
            let mut summary = Self::summary_from_session(
                &self.restore_session_times(Self::session_from_entries(&id, entries)?)?,
            );
            summary.query_count = query_count;
            summaries.push(summary);
        }
        // ids() is id-ordered. Preserve that deterministic tie-break across
        // the separate canonical/legacy assembly paths as well as timestamps.
        summaries.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::generate_id;

    #[test]
    fn canonical_list_uses_thin_metadata_and_only_the_first_user_text_field() {
        let root = tempfile::tempdir().unwrap();
        let manager = Manager::new(root.path().join("sessions"));
        let store = manager.storage().unwrap();
        store.reserve_segmented("s").unwrap();
        store.replace("s",vec![
            serde_json::json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"cwd":"/synthetic","model":"mock","session_name":"named","parent_session_id":"parent","system_prompt":"rules".repeat(10000),"source_meta":{"provenance":"source".repeat(5000)}}}),
            serde_json::json!({"id":"u","type":"user","role":"user","timestamp":"2026-10-26T00:00:01Z","content":[{"type":"text","text":"[other](futureos://session/parent) 这个会话的主题"},{"type":"image_url","image_url":{"url":"data:image/png;base64,".repeat(3000)}}]}),
            serde_json::json!({"id":"a","type":"assistant","role":"assistant","timestamp":"2026-10-26T00:00:02Z","content":"irrelevant answer".repeat(10000)}),
        ]).unwrap();
        let expected = Manager::summary_from_session(&manager.load("s").unwrap());
        store.db.call(|db| {
            db.execute_batch("DROP VIEW entry_records; DROP VIEW resolved_message_blocks; DROP VIEW block_records")?;
            db.execute("DELETE FROM content_segments WHERE content_id IN (SELECT content_id FROM content_references WHERE session_id='s' AND (owner_id IN ('info','a') OR field='/content/1/image_url'))",[])?;
            Ok(())
        }).unwrap();
        let actual = manager.list_all().unwrap().remove(0);
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(actual.first_message.as_deref(), Some("这个会话的主题"));
        assert!(
            manager.load("s").is_err(),
            "unused fields in the fixture must actually be unreadable"
        );
    }

    #[test]
    fn canonical_listing_uses_bounded_read_jobs_instead_of_one_per_session() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(dir.path().join("sessions"));
        let store = manager.storage().unwrap();
        for index in 0..40 {
            let id = format!("bulk-{index:02}");
            store.reserve_segmented(&id).unwrap();
            store
                .replace(
                    &id,
                    vec![
                        serde_json::json!({"id":format!("{id}-info"),"type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"cwd":"/bulk","model":"mock"}}),
                        serde_json::json!({"id":format!("{id}-user"),"type":"user","role":"user","timestamp":"2026-10-26T00:00:01Z","content":format!("question {index}")}),
                    ],
                )
                .unwrap();
        }
        let (batch, legacy) = super::super::content::reader::Snapshot::batch_summaries(
            store,
            store.ids(false).unwrap(),
        )
        .unwrap();
        assert!(legacy.is_empty());
        assert_eq!(batch.len(), 40);
        assert_eq!(
            batch
                .iter()
                .filter(|row| row.needs_preview_fallback)
                .count(),
            0,
            "plain inline titles must not fall back"
        );
        assert!(batch
            .iter()
            .any(|row| row.summary.first_message.as_deref() == Some("question 39")));
        let before = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
        let summaries = manager.list_all().unwrap();
        let after = store.db.metrics()["readerJob"]["count"].as_u64().unwrap();
        assert_eq!(summaries.len(), 40);
        assert!(summaries
            .iter()
            .any(|summary| summary.first_message.as_deref() == Some("question 39")));
        assert!(
            after - before <= 3,
            "40 canonical summaries used {} read jobs; expected listing and id enumeration only",
            after - before
        );
    }

    #[test]
    fn bulk_listing_reads_a_referenced_first_message_without_a_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(dir.path().join("sessions"));
        let store = manager.storage().unwrap();
        store.reserve_segmented("referenced-title").unwrap();
        let title = format!("needle {}", "长文本".repeat(8_000));
        store
            .replace(
                "referenced-title",
                vec![
                    serde_json::json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"cwd":"/bulk","model":"mock"}}),
                    serde_json::json!({"id":"u","type":"user","role":"user","timestamp":"2026-10-26T00:00:01Z","content":[{"type":"text","text":title}]}),
                ],
            )
            .unwrap();
        let exact =
            super::super::content::reader::Snapshot::summary(store, "referenced-title").unwrap();
        let (batch, legacy) = super::super::content::reader::Snapshot::batch_summaries(
            store,
            store.ids(false).unwrap(),
        )
        .unwrap();
        assert!(legacy.is_empty());
        assert!(!batch[0].needs_preview_fallback);
        let listed = manager
            .list_all()
            .unwrap()
            .into_iter()
            .find(|summary| summary.id == "referenced-title")
            .unwrap();
        assert_eq!(listed.first_message, exact.first_message);
        assert!(listed
            .first_message
            .as_deref()
            .is_some_and(|title| title.starts_with("needle 长文本")));
    }

    #[test]
    fn bulk_listing_matches_exact_titles_for_image_only_and_multiple_text_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(dir.path().join("sessions"));
        let store = manager.storage().unwrap();
        let info = |id: &str| serde_json::json!({"id":format!("{id}-info"),"type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"cwd":"/bulk","model":"mock"}});

        store.reserve_segmented("image-first").unwrap();
        store
            .replace(
                "image-first",
                vec![
                    info("image-first"),
                    serde_json::json!({"id":"image","type":"user","role":"user","timestamp":"2026-10-26T00:00:01Z","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,AA=="}}]}),
                    serde_json::json!({"id":"later","type":"user","role":"user","timestamp":"2026-10-26T00:00:02Z","content":"not the first title"}),
                ],
            )
            .unwrap();

        let mut blocks = (0..11)
            .map(|index| serde_json::json!({"type":"future_block","index":index}))
            .collect::<Vec<_>>();
        blocks[2] =
            serde_json::json!({"type":"text","text":format!("first text {}", "一".repeat(8_000))});
        blocks[10] =
            serde_json::json!({"type":"text","text":format!("later text {}", "二".repeat(8_000))});
        store.reserve_segmented("numeric-fields").unwrap();
        store
            .replace(
                "numeric-fields",
                vec![
                    info("numeric-fields"),
                    serde_json::json!({"id":"many","type":"user","role":"user","timestamp":"2026-10-26T00:00:01Z","content":blocks}),
                ],
            )
            .unwrap();

        store.reserve_segmented("blank-reference").unwrap();
        store
            .replace(
                "blank-reference",
                vec![
                    info("blank-reference"),
                    serde_json::json!({"id":"blank","type":"user","role":"user","timestamp":"2026-10-26T00:00:01Z","content":"\n ".repeat(12_000)}),
                ],
            )
            .unwrap();

        let listed = manager.list_all().unwrap();
        for id in ["image-first", "numeric-fields", "blank-reference"] {
            let exact = super::super::content::reader::Snapshot::summary(store, id).unwrap();
            let bulk = listed.iter().find(|summary| summary.id == id).unwrap();
            assert_eq!(
                bulk.first_message, exact.first_message,
                "title mismatch for {id}"
            );
        }
        assert_eq!(
            listed
                .iter()
                .find(|summary| summary.id == "numeric-fields")
                .unwrap()
                .first_message
                .as_deref()
                .map(|title| &title[.."first text".len()]),
            Some("first text")
        );
    }

    /// A first message whose text never left the entries row.
    ///
    /// Canonical writers externalize a long body into `content_references` and
    /// blank the block text, so the listing reads the title page by page. But a
    /// row written before that externalization (or restored from one) carries
    /// the text inline in `entries.content_json`, and the listing has to read
    /// that shape too — otherwise an old session loses its preview entirely.
    #[test]
    fn bulk_listing_reads_a_first_message_stored_inline_in_its_entries_row() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(dir.path().join("sessions"));
        let store = manager.storage().unwrap();
        store.reserve_segmented("inline-title").unwrap();
        store
            .replace(
                "inline-title",
                vec![
                    serde_json::json!({"id":"info","type":"session_info","timestamp":"2026-10-26T00:00:00Z","content":{"cwd":"/bulk","model":"mock"}}),
                    serde_json::json!({"id":"u","type":"user","role":"user","timestamp":"2026-10-26T00:00:01Z","content":"where is the log?"}),
                ],
            )
            .unwrap();
        // The pre-externalization shape: the text is in the entries row itself,
        // with no reference and no block text beside it.
        store
            .db
            .call(|db| {
                let sql = "UPDATE entries SET content_json=?1 WHERE entry_type='user'";
                db.execute(sql, [serde_json::json!("where is the log?").to_string()])?;
                let sql = "DELETE FROM content_references \
                           WHERE owner_kind='entry' AND field LIKE '/content/%/text'";
                db.execute(sql, [])?;
                let sql = "UPDATE message_blocks SET text=NULL";
                db.execute(sql, [])?;
                Ok(())
            })
            .unwrap();

        let exact =
            super::super::content::reader::Snapshot::summary(store, "inline-title").unwrap();
        assert_eq!(exact.first_message.as_deref(), Some("where is the log?"));
        let (batch, legacy) = super::super::content::reader::Snapshot::batch_summaries(
            store,
            store.ids(false).unwrap(),
        )
        .unwrap();
        assert!(legacy.is_empty());
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].summary.first_message, exact.first_message);
        assert!(
            !batch[0].needs_preview_fallback,
            "inline text is the preview; nothing is left for the caller to resolve"
        );
        let listed = manager
            .list_all()
            .unwrap()
            .into_iter()
            .find(|summary| summary.id == "inline-title")
            .unwrap();
        assert_eq!(listed.first_message.as_deref(), Some("where is the log?"));
    }

    fn temp_manager(tag: &str) -> (std::path::PathBuf, Manager) {
        let dir = std::env::temp_dir().join(format!("future-{tag}-{}", generate_id()));
        let manager = Manager::new(dir.clone());
        (dir, manager)
    }

    #[test]
    fn summary_uses_indexed_rows_and_preserves_tail_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(dir.path().join("sessions"));
        let store = manager.storage().unwrap();
        store.replace("synthetic", vec![
            serde_json::json!({"id":"u", "type":"user", "role":"user", "content":"question", "timestamp":"2026-01-01T00:00:00Z"}),
            // Valid storage JSON, deliberately not a decodable SessionEntry:
            // listing must not deserialize assistant bodies or fields.
            serde_json::json!({"id":"a", "type":"assistant", "role":123, "content":"x".repeat(100_000), "timestamp":"2026-01-02T00:00:00Z"}),
        ]).unwrap();
        let (rows, _, _) = store.summary_rows("synthetic").unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].len() < 1000);
        let summaries = manager.list_all().unwrap();
        assert_eq!(summaries[0].query_count, 1);
        assert_eq!(summaries[0].first_message.as_deref(), Some("question"));
        assert_eq!(summaries[0].updated_at.timestamp(), 1767312000);
        assert!(manager.load("synthetic").is_err());
    }

    #[test]
    fn summary_without_user_or_metadata_is_still_listed() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(dir.path().join("sessions"));
        let mut session = Session::new("", "");
        session.entries.push(SessionEntry::new_assistant(
            serde_json::json!("synthetic"),
            Vec::new(),
        ));
        manager.save(&session).unwrap();
        let summary = manager.list_all().unwrap().remove(0);
        assert_eq!(summary.id, session.id);
        assert_eq!(summary.query_count, 0);
        assert_eq!(
            summary.updated_at.timestamp_millis(),
            session.entries[0].timestamp.timestamp_millis()
        );
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
        let mut session = Session::new("/tmp/test", "gpt-4o");
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
        let mut session = Session::new("/tmp/test", "gpt-4o");
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
    fn summary_first_message_drops_a_conversation_reference() {
        let e = SessionEntry::new_user(
            "user",
            serde_json::json!("[登录修复](futureos://session/s-1) 还是坏的"),
        );
        // The value the sidebar preview and an imported title are built from.
        assert_eq!(Manager::summary_first_message(&e).unwrap(), "还是坏的");
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
    fn list_all_lists_many_sessions() {
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
    fn list_ids_empty_when_dir_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nope");
        let manager = Manager::new(missing);
        assert!(manager.list_ids().unwrap().is_empty());
    }
}

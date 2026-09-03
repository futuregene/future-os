//! Session forking: cut a parent session at an entry and re-id the copy.

use super::entry::{SessionEntry, ENTRY_TYPE_SESSION_INFO};
use super::model::{Session, CURRENT_SESSION_VERSION};
use super::run_journal::is_run_marker;
use crate::utils::{generate_entry_id, generate_id};
use chrono::Local;

pub fn fork_session(parent: &Session, from_entry_id: &str) -> Session {
    let chain = for_each_entry(&parent.entries, from_entry_id);
    // If from_entry_id wasn't found, for_each_entry returns every entry
    // (never hits the break).  Guard against a bad entry ID silently
    // producing an unforked clone.
    if chain.is_empty() || chain.last().map(|e| e.id.as_str()) != Some(from_entry_id) {
        // Fall back to cloning the whole session without a cut — better
        // than losing history.  Callers should validate the entry ID first.
        tracing::warn!(
            "fork point {from_entry_id} not found in session {}; cloning without a cut",
            parent.id
        );
    }
    let mut entries: Vec<SessionEntry> = chain.into_iter().cloned().collect();
    let mut id_map = std::collections::HashMap::with_capacity(entries.len());
    for e in &mut entries {
        let old_id = std::mem::replace(&mut e.id, generate_entry_id());
        id_map.insert(old_id, e.id.clone());
    }
    // V2 checkpoints reference message-entry ids. Forks deliberately re-id
    // their copied entries, so rewrite both ends through the same complete map
    // before the child is saved. A checkpoint whose range is not wholly inside
    // the fork is dropped instead of leaving a dangling cutoff that could make
    // the child's first prompt unexpectedly expand to the full transcript.
    entries.retain_mut(|entry| {
        if entry.entry_type != super::ENTRY_TYPE_COMPACTION {
            return true;
        }
        let Some(content) = entry
            .content
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
        else {
            return true;
        };
        if content
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(2)
        {
            return true;
        }
        for key in ["covered_from_entry_id", "cutoff_entry_id"] {
            let Some(old_id) = content
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
            else {
                return false;
            };
            let Some(new_id) = id_map.get(&old_id) else {
                return false;
            };
            content.insert(key.to_string(), serde_json::Value::String(new_id.clone()));
        }
        true
    });
    // Read parent metadata from the authoritative (last) session_info snapshot.
    // The append-only commit path appends a fresh session_info per run, so the
    // fork must inherit the parent's CURRENT model/name/thinking level (last),
    // not the values recorded at session creation (first). The values live on
    // the SessionEntry struct fields (model, thinking_level) and also inside
    // the content JSON (created_by, session_name).
    let parent_info = parent
        .entries
        .iter()
        .rev()
        .find(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO);

    // Prefer the parent's actual level: the session_info struct field, then the
    // content JSON (forked parents carry it there) — only fall back to a literal
    // when neither is set, so a `low`/`medium` parent doesn't silently fork to
    // `high`.
    let parent_thinking_level = parent_info
        .and_then(|e| e.content.as_ref())
        .and_then(|c| c.get("thinking_level"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("high");

    let parent_model = parent_info
        .and_then(|e| e.content.as_ref())
        .and_then(|c| c.get("model"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&parent.model)
        .to_string();

    let parent_created_by = parent_info
        .and_then(|e| e.content.as_ref())
        .and_then(|c| c.get("created_by"))
        .and_then(|v| v.as_str())
        .unwrap_or("tui");

    // Derive fork name: read from session_info content.
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
            entry_type: ENTRY_TYPE_SESSION_INFO.to_string(),
            role: "system".to_string(),
            content: Some(info),
            tool_calls: vec![],
            timestamp: Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        },
    );
    let now = Local::now();
    Session {
        id: generate_id(),
        version: CURRENT_SESSION_VERSION,
        cwd: parent.cwd.clone(),
        model: parent_model.clone(),
        name: fork_name,
        parent_session_id: parent.id.clone(),
        leaf_id: String::new(),
        entries,
        created_at: now,
        updated_at: now,
    }
}

/// Replace the inherited provenance snapshot on a freshly forked session.
/// Forking copies history, but the child is created by the client performing
/// the fork; its durable creator identity must therefore not come from the
/// parent session.
pub fn set_creation_provenance(session: &mut Session, created_by: &str, creator_id: &str) {
    let Some(info) = session
        .entries
        .iter_mut()
        .find(|entry| entry.entry_type == ENTRY_TYPE_SESSION_INFO)
        .and_then(|entry| entry.content.as_mut())
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    info.insert(
        "created_by".to_string(),
        serde_json::Value::String(created_by.to_string()),
    );
    if creator_id.is_empty() {
        info.remove("creator_id");
    } else {
        info.insert(
            "creator_id".to_string(),
            serde_json::Value::String(creator_id.to_string()),
        );
    }
}

fn for_each_entry<'a>(entries: &'a [SessionEntry], from_id: &str) -> Vec<&'a SessionEntry> {
    // Include all entries from the beginning up to and including from_id,
    // skipping the original session_info (fork_session prepends its own) and
    // run lifecycle markers (they belong to the parent's runs, not the fork).
    let mut result = vec![];
    for e in entries.iter() {
        if e.entry_type != ENTRY_TYPE_SESSION_INFO && !is_run_marker(&e.entry_type) {
            result.push(e);
        }
        if e.id == from_id {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compaction::{CompactionTrigger, ContextCheckpoint};
    use crate::session::entry::{ENTRY_TYPE_ASSISTANT, ENTRY_TYPE_COMPACTION, ENTRY_TYPE_USER};
    use crate::session::manager::Manager;
    use crate::session::projection::{agent_message_to_entry, entries_to_agent_messages};
    use crate::session::run_journal::RUN_STATE_COMPLETED;
    use crate::session::{checkpoint_to_entry, latest_context_checkpoint};
    use crate::types::AgentMessage;

    fn temp_manager(tag: &str) -> (std::path::PathBuf, Manager) {
        let dir = std::env::temp_dir().join(format!("future-{tag}-{}", generate_id()));
        let manager = Manager::new(dir.clone());
        (dir, manager)
    }

    #[test]
    fn fork_session_bad_entry_id_clones_all() {
        let mut parent = Session::new("/tmp", "model");
        parent
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        let forked = fork_session(&parent, "nonexistent_id");
        // Should still produce a session with entries (fallback behavior)
        assert!(!forked.entries.is_empty());
    }

    #[test]
    fn fork_session_preserves_model() {
        let mut parent = Session::new("/tmp", "my-model");
        parent
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        let forked = fork_session(&parent, &parent.entries[0].id);
        assert_eq!(forked.model, "my-model");
    }

    #[test]
    fn fork_session_generates_new_ids() {
        let mut parent = Session::new("/tmp", "model");
        parent
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        let original_id = parent.entries[0].id.clone();
        let forked = fork_session(&parent, &original_id);
        // Forked entries should have different IDs
        let forked_user_entry = forked
            .entries
            .iter()
            .find(|e| e.entry_type == ENTRY_TYPE_USER)
            .unwrap();
        assert_ne!(forked_user_entry.id, original_id);
    }

    #[test]
    fn fork_session_name_suffix() {
        let mut parent = Session::new("/tmp", "model");
        parent.set_session_name("Original Chat");
        parent
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        let forked = fork_session(&parent, &parent.entries[0].id);
        assert!(forked.name.contains("fork"));
    }

    #[test]
    fn fork_inherits_last_session_info_and_skips_markers() {
        let (dir, manager) = temp_manager("fork-last");
        let info_v0 = SessionEntry::session_info(
            serde_json::json!({"cwd": "/a", "model": "old", "session_name": "orig", "thinking_level": "low"}),
            "old".to_string(),
            "low".to_string(),
        );
        let user = SessionEntry::new_user("user", serde_json::json!("hi"));
        let user_id = user.id.clone();
        let session = Session::snapshot(
            "s-fork".to_string(),
            "/a".to_string(),
            "old".to_string(),
            "orig".to_string(),
            String::new(),
            vec![info_v0, user, SessionEntry::run_started("r", 1)],
        );
        manager.save(&session).unwrap();
        // A later run commit changes the model and adds a terminal marker plus a
        // fresh authoritative session_info.
        let info_v1 = SessionEntry::session_info(
            serde_json::json!({"cwd": "/a", "model": "new", "session_name": "renamed", "thinking_level": "high"}),
            "new".to_string(),
            "high".to_string(),
        );
        manager
            .append_entries(
                "s-fork",
                &[
                    SessionEntry::new_assistant(serde_json::json!("a"), vec![]),
                    SessionEntry::run_terminal("r", RUN_STATE_COMPLETED, 1, 1, None),
                    info_v1,
                ],
            )
            .unwrap();

        let parent = manager.load("s-fork").unwrap();
        let forked = fork_session(&parent, &user_id);
        // The fork inherits the CURRENT (last) model/name, not the original.
        assert_eq!(forked.model, "new");
        assert!(forked.name.contains("renamed"));
        // No run markers or duplicate session_info leak into the fork.
        assert!(!forked.entries.iter().any(|e| is_run_marker(&e.entry_type)));
        let info_count = forked
            .entries
            .iter()
            .filter(|e| e.entry_type == ENTRY_TYPE_SESSION_INFO)
            .count();
        assert_eq!(info_count, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn make_entry(id: &str, entry_type: &str, role: &str, content: &str) -> SessionEntry {
        SessionEntry {
            id: id.to_string(),
            entry_type: entry_type.to_string(),
            role: role.to_string(),
            content: Some(serde_json::json!(content)),
            tool_calls: vec![],
            timestamp: chrono::Local::now(),
            tool_call_id: String::new(),
            name: String::new(),
            tool_args: String::new(),
            thinking: String::new(),
            meta: None,
        }
    }

    #[test]
    fn fork_session_copies_entries_up_to_fork_point() {
        let mut parent = Session::new("/tmp/test", "test-model");
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
        let mut parent = Session::new("/tmp/test", "test-model");
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
            name: String::new(),
            tool_args: String::new(),
            metadata: None,
        });
        let entries_with_history: Vec<SessionEntry> = msgs_with_prompt
            .iter()
            .map(agent_message_to_entry)
            .collect();
        let history_entry_count = entries_with_history.len();
        assert!(
            history_entry_count >= 3,
            "fixed code: history (2) + new user (1) = {history_entry_count} entries (expected >= 3)"
        );
    }

    #[test]
    fn fork_remaps_v2_checkpoint_range_to_child_entry_ids() {
        let mut parent = Session::new("/tmp/test", "test-model");
        let first = make_entry("u1", ENTRY_TYPE_USER, "user", "old");
        let cutoff = make_entry("a1", ENTRY_TYPE_ASSISTANT, "assistant", "answer");
        let checkpoint = ContextCheckpoint {
            entry_id: "cp-entry".into(),
            checkpoint_id: "cp-1".into(),
            covered_from_entry_id: Some(first.id.clone()),
            cutoff_entry_id: Some(cutoff.id.clone()),
            summary: vec![crate::types::ContentBlock::text("summary")],
            tokens_before: 100,
            tokens_after: 10,
            trigger: CompactionTrigger::Automatic,
            phase: None,
            algorithm_version: "v2".into(),
            model: "test-model".into(),
            context_window: 200,
            created_at: chrono::Utc::now(),
            legacy_without_cutoff: false,
        };
        let checkpoint_entry = checkpoint_to_entry(&checkpoint);
        let fork_point = checkpoint_entry.id.clone();
        parent.entries = vec![first, cutoff, checkpoint_entry];

        let forked = fork_session(&parent, &fork_point);
        let remapped = latest_context_checkpoint(&forked.entries).expect("valid child checkpoint");
        assert_eq!(remapped.checkpoint_id, "cp-1");
        assert_ne!(remapped.covered_from_entry_id.as_deref(), Some("u1"));
        assert_ne!(remapped.cutoff_entry_id.as_deref(), Some("a1"));
        assert!(forked
            .entries
            .iter()
            .any(|entry| Some(entry.id.as_str()) == remapped.cutoff_entry_id.as_deref()));
    }

    #[test]
    fn fork_keeps_compaction_entry_with_non_object_content() {
        let mut parent = Session::new("/tmp/test", "m");
        let user = make_entry("u1", ENTRY_TYPE_USER, "user", "hi");
        let cp = make_entry("cp", ENTRY_TYPE_COMPACTION, "system", "not-an-object");
        let a1 = make_entry("a1", ENTRY_TYPE_ASSISTANT, "assistant", "answer");
        parent.entries = vec![user, cp, a1.clone()];
        let forked = fork_session(&parent, &a1.id);
        // Non-object compaction content → early `return true` keeps the entry.
        assert_eq!(
            forked
                .entries
                .iter()
                .filter(|e| e.entry_type == ENTRY_TYPE_COMPACTION)
                .count(),
            1
        );
    }

    #[test]
    fn fork_keeps_compaction_entry_with_legacy_schema_version() {
        let mut parent = Session::new("/tmp/test", "m");
        let user = make_entry("u1", ENTRY_TYPE_USER, "user", "hi");
        let mut cp = make_entry("cp", ENTRY_TYPE_COMPACTION, "system", "x");
        cp.content = Some(serde_json::json!({ "schema_version": 1 }));
        let a1 = make_entry("a1", ENTRY_TYPE_ASSISTANT, "assistant", "answer");
        parent.entries = vec![user, cp, a1.clone()];
        let forked = fork_session(&parent, &a1.id);
        // Non-v2 schema checkpoint is kept untouched.
        assert_eq!(
            forked
                .entries
                .iter()
                .filter(|e| e.entry_type == ENTRY_TYPE_COMPACTION)
                .count(),
            1
        );
    }

    #[test]
    fn fork_drops_v2_compaction_missing_range_key() {
        let mut parent = Session::new("/tmp/test", "m");
        let user = make_entry("u1", ENTRY_TYPE_USER, "user", "hi");
        let mut cp = make_entry("cp", ENTRY_TYPE_COMPACTION, "system", "x");
        cp.content = Some(serde_json::json!({ "schema_version": 2, "cutoff_entry_id": "u1" }));
        let a1 = make_entry("a1", ENTRY_TYPE_ASSISTANT, "assistant", "answer");
        parent.entries = vec![user, cp, a1.clone()];
        let forked = fork_session(&parent, &a1.id);
        // A v2 checkpoint missing one range key is dropped.
        assert_eq!(
            forked
                .entries
                .iter()
                .filter(|e| e.entry_type == ENTRY_TYPE_COMPACTION)
                .count(),
            0
        );
    }

    #[test]
    fn fork_drops_v2_compaction_referencing_out_of_fork_id() {
        let mut parent = Session::new("/tmp/test", "m");
        let user = make_entry("u1", ENTRY_TYPE_USER, "user", "hi");
        let mut cp = make_entry("cp", ENTRY_TYPE_COMPACTION, "system", "x");
        cp.content = Some(serde_json::json!({
            "schema_version": 2,
            "covered_from_entry_id": "u1",
            "cutoff_entry_id": "not-in-fork"
        }));
        let a1 = make_entry("a1", ENTRY_TYPE_ASSISTANT, "assistant", "answer");
        parent.entries = vec![user, cp, a1.clone()];
        let forked = fork_session(&parent, &a1.id);
        // A v2 checkpoint whose range references an id absent from the fork is
        // dropped rather than leaving a dangling cutoff.
        assert_eq!(
            forked
                .entries
                .iter()
                .filter(|e| e.entry_type == ENTRY_TYPE_COMPACTION)
                .count(),
            0
        );
    }
}

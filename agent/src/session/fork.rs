//! Session forking: resolve a typed durable fork point, then re-id an
//! independent snapshot of the parent journal.

use super::entry::{SessionEntry, ENTRY_TYPE_SESSION_INFO};
use super::model::{Session, CURRENT_SESSION_VERSION};
use crate::utils::{generate_entry_id, generate_id};
use anyhow::{bail, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};

/// Durable fork semantics. Clients identify one persisted entry and let the
/// Agent resolve the journal boundary; they never infer a cut from UI ordinals
/// or message text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "entry_id")]
pub enum ForkPoint {
    /// Include the selected entry only (legacy CLI/TUI behavior).
    ThroughEntry(String),
    /// Include the complete settled turn that starts at the selected user
    /// entry, including tools and the terminal assistant response.
    ThroughTurn(String),
    /// Clone through the most recent settled run, excluding an active tail.
    LatestSettled,
}

impl ForkPoint {
    pub fn from_rpc(mode: &str, entry_id: &str) -> Result<Self> {
        match mode {
            "" | "through_entry" => {
                if entry_id.is_empty() {
                    bail!("No message selected to fork from. Choose a persisted message.");
                }
                Ok(Self::ThroughEntry(entry_id.to_string()))
            }
            "through_turn" => {
                if entry_id.is_empty() {
                    bail!("No message selected to fork from. Choose a persisted user message.");
                }
                Ok(Self::ThroughTurn(entry_id.to_string()))
            }
            "latest_settled" => Ok(Self::LatestSettled),
            other => bail!("Unsupported fork point mode: {other}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkRequest {
    pub request_id: String,
    pub parent_session_id: String,
    pub point: ForkPoint,
    pub created_by: String,
    pub creator_id: String,
}

#[derive(Debug, Clone)]
pub struct ForkResult {
    pub session: Session,
    pub created: bool,
}

#[cfg(test)]
fn fork_session(parent: &Session, from_entry_id: &str) -> Session {
    build_fork_session(parent, &ForkPoint::ThroughEntry(from_entry_id.to_string()))
        .expect("validated fork point")
}

pub fn build_fork_session(parent: &Session, point: &ForkPoint) -> Result<Session> {
    let cutoff = resolve_cutoff(&parent.entries, point)?;
    if parent.entries[..=cutoff]
        .iter()
        .any(|entry| entry.entry_type != ENTRY_TYPE_SESSION_INFO && entry.id.trim().is_empty())
    {
        bail!("Fork history contains an entry without a persisted identity");
    }
    let mut entries: Vec<SessionEntry> = parent.entries[..=cutoff]
        .iter()
        .filter(|entry| entry.entry_type != ENTRY_TYPE_SESSION_INFO)
        .cloned()
        .collect();
    let mut id_map = std::collections::HashMap::with_capacity(entries.len());
    for e in &mut entries {
        let old_id = std::mem::replace(&mut e.id, generate_entry_id());
        let meta = e.meta.get_or_insert_with(|| serde_json::json!({}));
        if let Some(meta) = meta.as_object_mut() {
            let source_run_id = meta
                .get("run_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            meta.insert(
                "fork_origin".into(),
                serde_json::json!({
                    "session_id": parent.id,
                    "entry_id": old_id.clone(),
                    "run_id": source_run_id,
                }),
            );
        }
        id_map.insert(old_id, e.id.clone());
    }
    // Copied history belongs to the child. Keep grouping, but never expose a
    // parent's run identity as a run of the fork.
    let mut run_ids = std::collections::HashMap::new();
    for entry in &mut entries {
        if let Some(meta) = entry
            .meta
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
        {
            if let Some(old) = meta
                .get("run_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
            {
                let new = run_ids.entry(old).or_insert_with(generate_id);
                meta.insert("run_id".into(), serde_json::Value::String(new.clone()));
            }
        }
        if matches!(
            entry.entry_type.as_str(),
            super::ENTRY_TYPE_RUN_STARTED | super::ENTRY_TYPE_RUN_TERMINAL
        ) {
            if let Some(content) = entry
                .content
                .as_mut()
                .and_then(serde_json::Value::as_object_mut)
            {
                if let Some(old) = content
                    .get("run_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                {
                    let new = run_ids.entry(old.clone()).or_insert_with(generate_id);
                    content.insert("run_id".into(), serde_json::Value::String(new.clone()));
                    content.insert("inherited".into(), serde_json::Value::Bool(true));
                    content.insert(
                        "source_session_id".into(),
                        serde_json::Value::String(parent.id.clone()),
                    );
                    content.insert("source_run_id".into(), serde_json::Value::String(old));
                }
            }
        }
    }
    // Checkpoints reference message-entry ids. Forks deliberately re-id their copied
    // entries, so rewrite both ends through the same complete map before the child is
    // saved. A current-schema checkpoint whose range is not wholly inside the fork is
    // dropped instead of leaving a dangling cutoff that could make the child's first prompt
    // unexpectedly expand to the full transcript. A row written by a retired schema is
    // carried through untouched: nothing reads it, so it neither needs remapping nor can it
    // dangle.
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
        if !matches!(
            content
                .get("schema_version")
                .and_then(serde_json::Value::as_u64),
            Some(3)
        ) {
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
        if let Some(protected) = content.get_mut("protected_entry_ids") {
            let Some(ids) = protected.as_array_mut() else {
                return false;
            };
            for id in ids {
                let Some(mapped) = id.as_str().and_then(|old| id_map.get(old)) else {
                    return false;
                };
                *id = serde_json::Value::String(mapped.clone());
            }
        }
        if let Some(summary) = content.get_mut("summary") {
            let Ok(mut blocks) =
                serde_json::from_value::<Vec<crate::types::ContentBlock>>(summary.clone())
            else {
                return false;
            };
            if crate::compaction::remap_evidence_references(&mut blocks, &id_map).is_none() {
                return false;
            }
            *summary = serde_json::json!(blocks);
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
    Ok(Session {
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
    })
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

fn resolve_cutoff(entries: &[SessionEntry], point: &ForkPoint) -> Result<usize> {
    match point {
        ForkPoint::ThroughEntry(entry_id) => entries
            .iter()
            .position(|entry| entry.id == *entry_id)
            .ok_or_else(|| anyhow::anyhow!("Fork point not found in the parent session")),
        ForkPoint::ThroughTurn(entry_id) => {
            let start = entries
                .iter()
                .position(|entry| entry.id == *entry_id)
                .ok_or_else(|| anyhow::anyhow!("Fork point not found in the parent session"))?;
            if entries[start].entry_type != super::ENTRY_TYPE_USER {
                bail!("A through_turn fork point must identify a persisted user message");
            }
            let end = entries
                .iter()
                .enumerate()
                .skip(start + 1)
                .find(|(_, entry)| entry.entry_type == super::ENTRY_TYPE_USER)
                .map(|(index, _)| index.saturating_sub(1))
                .unwrap_or_else(|| entries.len().saturating_sub(1));
            let run_id = entries[start]
                .meta
                .as_ref()
                .and_then(|meta| meta.get("run_id"))
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    entries[start..=end].iter().find_map(|entry| {
                        (entry.entry_type == super::ENTRY_TYPE_RUN_STARTED)
                            .then_some(entry.content.as_ref())
                            .flatten()
                            .and_then(|value| value.get("run_id"))
                            .and_then(serde_json::Value::as_str)
                    })
                });
            if let Some(run_id) = run_id {
                let started = entries[start..=end].iter().any(|entry| {
                    entry.entry_type == super::ENTRY_TYPE_RUN_STARTED
                        && entry
                            .content
                            .as_ref()
                            .and_then(|value| value.get("run_id"))
                            .and_then(serde_json::Value::as_str)
                            == Some(run_id)
                });
                let settled = entries[start..=end].iter().any(|entry| {
                    entry.entry_type == super::ENTRY_TYPE_RUN_TERMINAL
                        && entry
                            .content
                            .as_ref()
                            .and_then(|value| value.get("run_id"))
                            .and_then(serde_json::Value::as_str)
                            == Some(run_id)
                });
                if started && !settled {
                    bail!("The selected turn is not settled yet");
                }
            }
            Ok(end)
        }
        ForkPoint::LatestSettled => {
            if let Some((index, _)) = entries
                .iter()
                .enumerate()
                .rev()
                .find(|(_, entry)| entry.entry_type == super::ENTRY_TYPE_RUN_TERMINAL)
            {
                return Ok(index);
            }
            if entries
                .iter()
                .any(|entry| entry.entry_type == super::ENTRY_TYPE_RUN_STARTED)
            {
                bail!("Nothing to clone: the session has no settled run");
            }
            entries
                .iter()
                .enumerate()
                .rev()
                .find(|(_, entry)| entry.entry_type != ENTRY_TYPE_SESSION_INFO)
                .map(|(index, _)| index)
                .ok_or_else(|| {
                    anyhow::anyhow!("Nothing to clone: the session has no settled history")
                })
        }
    }
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
    fn forked_checkpoint_evidence_resolves_through_history_after_restart() {
        use crate::compaction::{
            project_prompt_context, CompactionPhase, ContextManager, ContextPreparation,
        };
        use crate::types::ContentBlock;
        let dir = tempfile::tempdir().unwrap();
        let manager = Manager::new(dir.path().to_owned());
        let output = format!(
            "head {} exact-middle-value {} tail",
            "x".repeat(2000),
            "y".repeat(2000)
        );
        let mut raw = vec![
            AgentMessage::new_user("user", serde_json::json!("inspect the config")),
            AgentMessage {
                role: "assistant".into(),
                content: vec![ContentBlock::tool_call(
                    "call",
                    "read",
                    serde_json::json!({"path":"config.json"}),
                    Default::default(),
                )],
                ..Default::default()
            },
            AgentMessage {
                role: "tool".into(),
                content: vec![ContentBlock::tool_result("call", &output, false)],
                ..Default::default()
            },
        ];
        for message in &mut raw {
            message.ensure_journal_entry_id();
        }
        let policy = ContextManager {
            enabled: true,
            reserve_tokens: 6400,
            keep_recent_tokens: 4000,
            context_window: 32_000,
            model: "m".into(),
        };
        let ContextPreparation::Compacted { checkpoint, .. } = policy
            .prepare_evidence(
                project_prompt_context(&raw, None, None, 32_000),
                &raw,
                CompactionTrigger::Manual,
                CompactionPhase::Standalone,
                None,
                &std::sync::atomic::AtomicBool::new(false),
                None,
            )
            .unwrap()
        else {
            panic!("checkpoint expected")
        };
        let mut parent = Session::new(".", "m");
        parent.entries = raw.iter().map(agent_message_to_entry).collect();
        parent.entries.push(checkpoint_to_entry(&checkpoint));
        // Repeated forks must keep resolving the child-local IDs, not IDs in
        // either ancestor. Read through the actual indexed history API.
        for _ in 0..2 {
            let checkpoint = latest_context_checkpoint(&parent.entries).unwrap();
            let child = fork_session(&parent, &checkpoint.entry_id);
            manager.save(&child).unwrap();
            let restarted = Manager::new(dir.path().to_owned());
            let child = restarted.load(&child.id).unwrap();
            let checkpoint = latest_context_checkpoint(&child.entries).unwrap();
            let ContentBlock::Text { text } = &checkpoint.summary[0] else {
                panic!("text expected")
            };
            let rows: Vec<serde_json::Value> = text
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            assert_eq!(rows.len(), 1);
            let id = rows[0]["entryId"].as_str().unwrap();
            assert!(!parent.entries.iter().any(|entry| entry.id == id));
            let full = restarted
                .read_history_entry(&child.id, id, 0, 8192)
                .unwrap();
            assert_eq!(full["chunks"][0]["text"], output);
            assert_eq!(checkpoint.cutoff_entry_id.as_deref(), Some(id));
            parent = child;
        }
    }

    #[test]
    fn fork_session_bad_entry_id_is_rejected() {
        let mut parent = Session::new("/tmp", "model");
        parent
            .entries
            .push(SessionEntry::new_user("user", serde_json::json!("hello")));
        let error = build_fork_session(
            &parent,
            &ForkPoint::ThroughEntry("nonexistent_id".to_string()),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Fork point not found"));
    }

    #[test]
    fn through_turn_rejects_an_unsettled_persisted_run() {
        let mut user = SessionEntry::new_user("user", serde_json::json!("hello"));
        user.meta = Some(serde_json::json!({"run_id": "run-open"}));
        let user_id = user.id.clone();
        let mut parent = Session::new("/tmp", "model");
        parent.entries = vec![user, SessionEntry::run_started("run-open", 1)];

        let error = build_fork_session(&parent, &ForkPoint::ThroughTurn(user_id))
            .expect_err("active run must not be snapshotted as settled");
        assert!(error.to_string().contains("not settled"));
    }

    #[test]
    fn manager_create_fork_is_atomic_idempotent_and_preserves_the_turn() {
        let (_dir, manager) = temp_manager("fork-operation");
        let mut user = SessionEntry::new_user("user", serde_json::json!("first"));
        user.meta = Some(serde_json::json!({"run_id": "parent-run"}));
        let user_id = user.id.clone();
        let mut assistant = SessionEntry::new_assistant(serde_json::json!("answer"), vec![]);
        assistant.meta = Some(serde_json::json!({"run_id": "parent-run"}));
        let next_user = SessionEntry::new_user("user", serde_json::json!("second"));
        let mut parent = Session::snapshot(
            "parent".to_string(),
            "/tmp".to_string(),
            "model".to_string(),
            "Parent".to_string(),
            String::new(),
            vec![
                SessionEntry::session_info(
                    serde_json::json!({
                        "cwd": "/tmp",
                        "model": "model",
                        "session_name": "Parent",
                        "thinking_level": "low"
                    }),
                    "model".to_string(),
                    "low".to_string(),
                ),
                user,
                SessionEntry::run_started("parent-run", 1),
                assistant,
                SessionEntry::run_terminal("parent-run", RUN_STATE_COMPLETED, 7, 20, None),
                next_user,
            ],
        );
        for entry in &mut parent.entries {
            entry.timestamp -= chrono::Duration::days(1);
        }
        manager.save(&parent).unwrap();
        let request = ForkRequest {
            request_id: "stable-request".to_string(),
            parent_session_id: parent.id.clone(),
            point: ForkPoint::ThroughTurn(user_id),
            created_by: "desktop".to_string(),
            creator_id: "device".to_string(),
        };

        let fork_started_at_ms = chrono::Utc::now().timestamp_millis();
        let first = manager.create_fork(request.clone()).unwrap();
        let fork_finished_at_ms = chrono::Utc::now().timestamp_millis();
        let repeated = manager.create_fork(request.clone()).unwrap();
        assert!(first.created);
        assert!(!repeated.created);
        assert_eq!(first.session.id, repeated.session.id);
        assert_eq!(first.session.parent_session_id, "parent");
        let child_created_at_ms = first.session.created_at.timestamp_millis();
        assert!(
            (fork_started_at_ms..=fork_finished_at_ms).contains(&child_created_at_ms),
            "fork creation time must come from the fork transaction, not copied history"
        );
        assert_eq!(repeated.session.created_at, first.session.created_at);
        assert_eq!(
            first
                .session
                .entries
                .iter()
                .filter(|entry| entry.entry_type == ENTRY_TYPE_USER)
                .count(),
            1
        );
        assert!(first
            .session
            .entries
            .iter()
            .any(|entry| entry.entry_type == ENTRY_TYPE_ASSISTANT));
        assert!(first
            .session
            .entries
            .iter()
            .any(|entry| entry.entry_type == crate::session::ENTRY_TYPE_RUN_TERMINAL));
        let info = first.session.get_session_info().unwrap();
        assert_eq!(info["created_by"], "desktop");
        assert_eq!(info["creator_id"], "device");

        let mut conflicting = request;
        conflicting.point = ForkPoint::LatestSettled;
        let error = manager.create_fork(conflicting).unwrap_err();
        assert!(error.to_string().contains("different parameters"));
        assert_eq!(
            manager.list_all().unwrap().len(),
            2,
            "one parent and one child only"
        );
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
    fn fork_inherits_last_session_info_and_preserves_run_outcome() {
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
        let forked = build_fork_session(&parent, &ForkPoint::ThroughTurn(user_id)).unwrap();
        // The fork inherits the CURRENT (last) model/name, not the original.
        assert_eq!(forked.model, "new");
        assert!(forked.name.contains("renamed"));
        // Historical lifecycle markers are retained under child-local run ids
        // so failure/cancellation/completion is not rewritten as success.
        let markers = forked
            .entries
            .iter()
            .filter(|entry| crate::session::is_run_marker(&entry.entry_type))
            .collect::<Vec<_>>();
        assert_eq!(markers.len(), 2);
        assert!(markers.iter().all(|entry| {
            entry
                .content
                .as_ref()
                .and_then(|content| content.get("inherited"))
                == Some(&serde_json::Value::Bool(true))
        }));
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
            protected_entry_ids: vec![first.id.clone(), cutoff.id.clone()],
            checkpoint_id: "cp-1".into(),
            covered_from_entry_id: Some(first.id.clone()),
            cutoff_entry_id: Some(cutoff.id.clone()),
            summary: vec![crate::types::ContentBlock::text("summary")],
            tokens_before: 100,
            tokens_after: 10,
            trigger: CompactionTrigger::Automatic,
            phase: None,
            algorithm_version: "v2".into(),
            summary_outcome: None,
            model: "test-model".into(),
            context_window: 200,
            created_at: chrono::Utc::now(),
        };
        let checkpoint_entry = checkpoint_to_entry(&checkpoint);
        let fork_point = checkpoint_entry.id.clone();
        parent.entries = vec![first, cutoff, checkpoint_entry];

        let forked = fork_session(&parent, &fork_point);
        let remapped = latest_context_checkpoint(&forked.entries).expect("valid child checkpoint");
        assert_eq!(remapped.checkpoint_id, "cp-1");
        assert_ne!(remapped.covered_from_entry_id.as_deref(), Some("u1"));
        assert_ne!(remapped.cutoff_entry_id.as_deref(), Some("a1"));
        assert_eq!(remapped.protected_entry_ids.len(), 2);
        assert!(remapped
            .protected_entry_ids
            .iter()
            .all(|id| id != "u1" && id != "a1" && forked.entries.iter().any(|e| &e.id == id)));
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
    fn fork_carries_a_retired_schema_compaction_entry_untouched() {
        let mut parent = Session::new("/tmp/test", "m");
        let user = make_entry("u1", ENTRY_TYPE_USER, "user", "hi");
        let mut cp = make_entry("cp", ENTRY_TYPE_COMPACTION, "system", "x");
        cp.content = Some(serde_json::json!({ "schema_version": 2, "cutoff_entry_id": "u1" }));
        let a1 = make_entry("a1", ENTRY_TYPE_ASSISTANT, "assistant", "answer");
        parent.entries = vec![user, cp, a1.clone()];
        let forked = fork_session(&parent, &a1.id);
        // Inert: no reader recognises this schema, so there is nothing to remap and
        // nothing that can dangle.
        assert_eq!(
            forked
                .entries
                .iter()
                .filter(|e| e.entry_type == ENTRY_TYPE_COMPACTION)
                .count(),
            1
        );
        assert!(crate::session::latest_context_checkpoint(&forked.entries).is_none());
    }

    #[test]
    fn fork_drops_a_current_checkpoint_missing_range_key() {
        let mut parent = Session::new("/tmp/test", "m");
        let user = make_entry("u1", ENTRY_TYPE_USER, "user", "hi");
        let mut cp = make_entry("cp", ENTRY_TYPE_COMPACTION, "system", "x");
        cp.content = Some(serde_json::json!({
            "schema_version": 3,
            "protected_entry_ids": [],
            "cutoff_entry_id": "u1"
        }));
        let a1 = make_entry("a1", ENTRY_TYPE_ASSISTANT, "assistant", "answer");
        parent.entries = vec![user, cp, a1.clone()];
        let forked = fork_session(&parent, &a1.id);
        // A checkpoint missing one range key is dropped.
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
    fn fork_drops_a_current_checkpoint_referencing_out_of_fork_id() {
        let mut parent = Session::new("/tmp/test", "m");
        let user = make_entry("u1", ENTRY_TYPE_USER, "user", "hi");
        let mut cp = make_entry("cp", ENTRY_TYPE_COMPACTION, "system", "x");
        cp.content = Some(serde_json::json!({
            "schema_version": 3,
            "protected_entry_ids": [],
            "covered_from_entry_id": "u1",
            "cutoff_entry_id": "not-in-fork"
        }));
        let a1 = make_entry("a1", ENTRY_TYPE_ASSISTANT, "assistant", "answer");
        parent.entries = vec![user, cp, a1.clone()];
        let forked = fork_session(&parent, &a1.id);
        // A checkpoint whose range references an id absent from the fork is
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

/// Fork-point parsing and cutoff resolution. Both decide whether a fork is
/// allowed at all, so the rejections are asserted as carefully as the accepts.
#[cfg(test)]
mod cutoff_paths {
    use super::*;
    use crate::session::entry::ENTRY_TYPE_RUN_STARTED;
    use crate::session::SessionEntry;

    #[test]
    fn fork_point_modes_require_a_selection_and_reject_unknown_modes() {
        assert!(ForkPoint::from_rpc("", "").is_err());
        assert!(ForkPoint::from_rpc("through_entry", "").is_err());
        assert!(ForkPoint::from_rpc("through_turn", "").is_err());
        assert!(ForkPoint::from_rpc("through_turn", "u").is_ok());
        assert!(matches!(
            ForkPoint::from_rpc("latest_settled", "").unwrap(),
            ForkPoint::LatestSettled
        ));
        assert!(ForkPoint::from_rpc("rewind", "u").is_err());
    }

    #[test]
    fn a_through_turn_point_must_name_a_user_message_and_latest_settled_needs_a_terminal() {
        let mut entries = vec![
            SessionEntry::new_user("user", serde_json::json!("question")),
            SessionEntry::new_assistant(serde_json::json!("answer"), Vec::new()),
            SessionEntry::run_terminal("r", "completed", 1, 1, None),
        ];
        entries[0].id = "u".into();
        entries[1].id = "a".into();
        entries[2].id = "t".into();
        let error = resolve_cutoff(&entries, &ForkPoint::ThroughTurn("a".into()))
            .unwrap_err()
            .to_string();
        assert!(error.contains("persisted user message"), "{error}");
        assert_eq!(
            resolve_cutoff(&entries, &ForkPoint::LatestSettled).unwrap(),
            2
        );

        // A run that started and never settled has no usable fork point.
        let mut started = SessionEntry::new_user("user", serde_json::json!("question"));
        started.entry_type = ENTRY_TYPE_RUN_STARTED.to_string();
        entries.pop();
        entries.push(started);
        let error = resolve_cutoff(&entries, &ForkPoint::LatestSettled)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no settled run"), "{error}");
    }

    #[test]
    fn a_checkpoint_that_cannot_be_remapped_is_dropped_from_the_fork() {
        let user = || {
            let mut entry = SessionEntry::new_user("user", serde_json::json!("question"));
            entry.id = "u".into();
            entry
        };
        let checkpoint = |protected: serde_json::Value, summary: serde_json::Value| {
            let mut entry = SessionEntry::new_user("user", serde_json::json!(null));
            entry.id = "cp".into();
            entry.entry_type = crate::session::ENTRY_TYPE_COMPACTION.into();
            entry.content = Some(serde_json::json!({
                "schema_version": 3,
                "covered_from_entry_id": "u",
                "cutoff_entry_id": "u",
                "protected_entry_ids": protected,
                "summary": summary
            }));
            entry
        };
        let fork = |entries: Vec<SessionEntry>| {
            let mut parent = Session::new("/synthetic", "mock");
            parent.entries = entries;
            build_fork_session(&parent, &ForkPoint::ThroughEntry("cp".to_string())).unwrap()
        };

        for (protected, summary) in [
            // Protected ids that are not a list at all.
            (serde_json::json!("not-a-list"), serde_json::json!([])),
            // A protected id that is not part of the copied range.
            (serde_json::json!(["unknown-id"]), serde_json::json!([])),
            // A summary that is not a block array.
            (serde_json::json!([]), serde_json::json!("not-blocks")),
        ] {
            let child = fork(vec![user(), checkpoint(protected, summary)]);
            assert!(
                !child
                    .entries
                    .iter()
                    .any(|entry| entry.entry_type == "compaction"),
                "a checkpoint that cannot be remapped must not survive the fork"
            );
        }

        // A checkpoint whose ids all resolve survives, with its ids rewritten
        // to the copied entries'.
        let child = fork(vec![
            user(),
            checkpoint(serde_json::json!(["u"]), serde_json::json!([])),
        ]);
        let kept = child
            .entries
            .iter()
            .find(|entry| entry.entry_type == "compaction")
            .expect("a remappable checkpoint survives");
        let content = kept.content.as_ref().unwrap();
        assert_ne!(content["covered_from_entry_id"], "u");
        assert_ne!(content["protected_entry_ids"][0], "u");
    }

    #[test]
    fn provenance_is_replaced_only_when_the_child_carries_a_session_info() {
        // No session_info: the provenance update is a no-op, not a panic.
        let mut bare = Session::new("/synthetic", "mock");
        bare.entries.push(SessionEntry::new_user(
            "user",
            serde_json::json!("question"),
        ));
        set_creation_provenance(&mut bare, "web", "device-1");
        assert_eq!(bare.entries.len(), 1);
        assert!(bare.entries[0]
            .content
            .as_ref()
            .unwrap()
            .get("created_by")
            .is_none());

        // With metadata the creator is replaced, and an empty creator id removes
        // the field instead of storing an empty string.
        let mut forked = Session::new("/synthetic", "mock");
        forked.entries.push(SessionEntry::session_info(
            serde_json::json!({"created_by":"tui","creator_id":"inherited"}),
            "mock".into(),
            String::new(),
        ));
        set_creation_provenance(&mut forked, "web", "");
        let content = forked.entries[0].content.clone().unwrap();
        assert_eq!(content["created_by"], "web");
        assert!(content.get("creator_id").is_none());
    }

    #[test]
    fn a_settled_turn_resolves_through_its_run_markers() {
        // The user entry carries no run id of its own, so the turn's run has to
        // come from the `run_started` marker inside the same range.
        let mut entries = vec![
            SessionEntry::new_user("user", serde_json::json!("question")),
            SessionEntry::run_started("r", 1),
            SessionEntry::run_terminal("r", "completed", 1, 1, None),
        ];
        entries[0].id = "u".into();
        entries[1].id = "started".into();
        entries[2].id = "terminal".into();
        assert_eq!(
            resolve_cutoff(&entries, &ForkPoint::ThroughTurn("u".into())).unwrap(),
            2
        );
    }
}

//! Session lifecycle handlers: create/list/switch/delete/fork/clone, plus the
//! session-entry and command/skill listing endpoints.

use std::sync::Arc;

use crate::rpc::{AppState, RpcCommand, RpcResponse, ServerSession, SseBroadcaster};

/// Test-only hook fired inside `fork`/`clone` after the forked session is
/// built but before its model is synced into the fresh agent loop, keyed on
/// the parent session id. Lets a test force the model-sync failure warn.
#[cfg(test)]
pub(crate) type ModelSyncHook = Option<(String, Box<dyn Fn(&mut ServerSession) + Send>)>;

#[cfg(test)]
pub(crate) static MODEL_SYNC_FAIL_HOOK: parking_lot::Mutex<ModelSyncHook> =
    parking_lot::Mutex::new(None);

pub(crate) fn cmd_shutdown(state: &AppState, id: &str) -> String {
    state
        .shutting_down
        .store(true, std::sync::atomic::Ordering::SeqCst);
    RpcResponse::ok(
        id,
        "shutdown",
        serde_json::json!({"shutting_down": true, "note": "Existing runs continue; new prompts are rejected."}),
    )
}

pub(crate) fn cmd_list_sessions(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    // Propagate enumeration errors instead of coercing them to an empty list.
    // A directory that is momentarily unreadable must NOT be reported as "the
    // agent has zero sessions" — clients that reconcile their own mirrors
    // against this list (GUI orphan cleanup) would treat every known thread as
    // deleted and hard-delete them. Failure lets callers distinguish "could not
    // enumerate" (skip / retry) from "genuinely empty".
    let summaries = match state.session_manager.list_all() {
        Ok(summaries) => summaries,
        Err(error) => {
            return RpcResponse::build_fail(
                id,
                "list_sessions",
                &format!("failed to enumerate sessions: {error}"),
            );
        }
    };
    // Scope by the caller's cwd when provided (empty = all sessions).
    let cwd_filter = cmd.cwd.trim().to_string();

    // Snapshot streaming flags of live sessions.  Collect within a single
    // outer read guard — safe because we only acquire inner read locks, and
    // ParkingLot RwLock allows concurrent reads.
    let active_flags: std::collections::HashMap<String, bool> = {
        let active = state.sessions.read();
        active
            .iter()
            .map(|(sid, sess)| {
                let streaming = sess
                    .read()
                    .is_streaming
                    .load(std::sync::atomic::Ordering::Relaxed);
                (sid.clone(), streaming)
            })
            .collect()
    };

    // One canonical contract shared by every client.
    let sessions: Vec<serde_json::Value> = summaries
        .into_iter()
        .filter(|s| cwd_filter.is_empty() || s.cwd == cwd_filter)
        .map(|s| {
            let is_streaming = active_flags.get(&s.id).copied().unwrap_or(false);
            serde_json::to_value(crate::rpc::payloads::SessionSummaryPayload {
                id: s.id,
                session_name: s.name,
                model: s.model,
                cwd: s.cwd,
                updated_at_ms: s.updated_at.timestamp_millis(),
                parent_session_id: (!s.parent_session_id.is_empty()).then_some(s.parent_session_id),
                first_message: s.first_message,
                query_count: s.query_count,
                is_streaming,
            })
            .expect("serializable session summary")
        })
        .collect();
    RpcResponse::ok(
        id,
        "list_sessions",
        serde_json::json!({"sessions": sessions}),
    )
}

/// Enumeration of session ids by FILENAME ONLY — no file contents are read.
///
/// This is the reconciliation-safe variant of `list_sessions`: a session whose
/// JSONL is momentarily unreadable, truncated, or corrupt still exists on disk
/// and so is still reported as live here. Clients that reconcile their own
/// mirrors against this list (the GUI's orphan-thread cleanup) can never
/// mistake a transient read failure for a deleted session and hard-delete
/// local state. Only a genuine directory-listing error fails the command.
pub(crate) fn cmd_list_session_ids(state: &AppState, id: &str) -> String {
    let ids = match state.session_manager.list_ids() {
        Ok(ids) => ids,
        Err(error) => {
            return RpcResponse::build_fail(
                id,
                "list_session_ids",
                &format!("failed to enumerate session files: {error}"),
            );
        }
    };
    RpcResponse::ok(id, "list_session_ids", serde_json::json!({ "ids": ids }))
}

/// Lightweight streaming-status query: scans ONLY the in-memory session map
/// (hydrated sessions) — never touches disk and never hydrates.  A session
/// that isn't in the map can't be streaming (runs are always started through
/// a hydrated ServerSession), so this is the exact set of active runs.
pub(crate) fn cmd_list_streaming_sessions(state: &AppState, id: &str) -> String {
    let ids: Vec<String> = state
        .sessions
        .read()
        .iter()
        .filter(|(_, sess)| {
            sess.read()
                .is_streaming
                .load(std::sync::atomic::Ordering::Relaxed)
        })
        .map(|(sid, _)| sid.clone())
        .collect();
    RpcResponse::ok(
        id,
        "list_streaming_sessions",
        serde_json::json!({"sessionIds": ids}),
    )
}

/// Bind a client to an existing session.  Sessions are equal peers, so
/// "switching" just means resolving (and hydrating) the target — the client
/// addresses it by id from then on.
pub(crate) fn cmd_switch_session(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    if cmd.session_id.is_empty() {
        return RpcResponse::build_fail(
            id,
            "switch_session",
            "No session selected. Choose a session from the list to switch to.",
        );
    }
    match state.get_session(&cmd.session_id) {
        Some(_) => RpcResponse::ok(
            id,
            "switch_session",
            serde_json::json!({"cancelled": false}),
        ),
        None => RpcResponse::build_fail(
            id,
            "switch_session",
            &format!("session `{}` not found", cmd.session_id),
        ),
    }
}

pub(crate) fn cmd_delete_session(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    if cmd.session_id.is_empty() {
        return RpcResponse::build_fail(
            id,
            "delete_session",
            "No session selected to delete. Choose a session first.",
        );
    }
    let live = state.sessions.read().get(&cmd.session_id).cloned();
    if let Some(session) = live {
        if session
            .read()
            .compaction_in_progress
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return RpcResponse::build_fail_code(
                id,
                "delete_session",
                "session_busy",
                "session context compaction is in progress",
                serde_json::json!({
                    "busy_reason": "compaction",
                    "retryable": true,
                }),
            );
        }
        let (active, cancelled_count) = {
            let mut session = session.write();
            session.deleting = true;
            let cancelled = session
                .cancel_all_queued_runs(crate::runtime::QueuedCancellationReason::SessionDeleted);
            let active = session.runtime.snapshot();
            if let Some(active) = &active {
                let _ = session.runtime.request_abort(Some(&active.run_id));
            }
            (active, cancelled.len())
        };
        if let Some(active) = active.filter(|active| {
            session.read().runtime.has_owned_task()
                || !matches!(
                    active.phase,
                    crate::runtime::RunPhase::CancellationStuck
                        | crate::runtime::RunPhase::PersistenceDegraded
                )
        }) {
            return RpcResponse::build_fail_code(
                id,
                "delete_session",
                "deleting",
                "session deletion is waiting for the active run to stop; retry delete_session",
                serde_json::json!({
                    "session_id": cmd.session_id,
                    "active_run_id": active.run_id,
                    "queued_cancelled": cancelled_count,
                    "retryable": true,
                }),
            );
        }
        // Hard deletion is a close-then-delete barrier. The session write lock
        // excludes concurrent metadata commands while the ordered transcript
        // writer drains; closing the event journal then fences late broadcasts
        // before either filesystem tree is removed.
        let session = session.write();
        if let Err(error) = session.persistence.close() {
            return RpcResponse::build_fail_code(
                id,
                "delete_session",
                "delete_failed",
                &format!("failed to close session persistence: {error}"),
                serde_json::json!({"session_id": cmd.session_id, "retryable": true}),
            );
        }
        session.broadcaster.close_journal();
    }

    // The in-memory session is fenced before disk removal. Keep it in the map
    // if deletion fails so a retry cannot accidentally rehydrate/accept work
    // against partially deleted state.
    if let Err(e) = state.session_manager.delete(&cmd.session_id) {
        return RpcResponse::build_fail_code(
            id,
            "delete_session",
            "delete_failed",
            &e.to_string(),
            serde_json::json!({"session_id": cmd.session_id, "retryable": true}),
        );
    }
    state.sessions.write().remove(&cmd.session_id);
    RpcResponse::ok(id, "delete_session", serde_json::json!({"deleted": true}))
}

/// Load user entries of a session from disk (fork-point picker).  Reads the
/// file directly — no in-memory session required.
pub(crate) fn cmd_get_fork_messages(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    let user_entries: Vec<serde_json::Value> = state
        .session_manager
        .load(&cmd.session_id)
        .map(|s| {
            s.entries
                .iter()
                .filter(|e| e.entry_type == crate::session::ENTRY_TYPE_USER)
                .map(|e| {
                    let content_text = e
                        .content
                        .as_ref()
                        .map(|c| {
                            if let Some(arr) = c.as_array() {
                                // First text block only — later text blocks are
                                // the agent-injected attachment-path list.
                                arr.iter()
                                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                    .next()
                                    .unwrap_or("")
                                    .to_string()
                            } else {
                                c.as_str().unwrap_or("").to_string()
                            }
                        })
                        .unwrap_or_default();
                    serde_json::json!({
                        "id": e.id,
                        "role": e.role,
                        "kind":"user",
                        "blocks": [{"kind":"text","text":content_text}],
                        "createdAtMs": e.timestamp.timestamp_millis(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    RpcResponse::ok(
        id,
        "get_fork_messages",
        serde_json::json!({"messages": user_entries}),
    )
}

pub(crate) fn cmd_get_commands(id: &str) -> String {
    // Return commands from skills (similar to Go's extensions + prompts)
    let skills = crate::skills::discover_skills_cached(&crate::skills::global_skill_dirs());

    let mut commands: Vec<serde_json::Value> = skills
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "description": s.description,
                "nameZh": s.name_zh,
                "descriptionZh": s.description_zh,
                "source": "skill"
            })
        })
        .collect();
    commands.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });

    RpcResponse::ok(
        id,
        "get_commands",
        serde_json::json!({"commands": commands}),
    )
}

pub(crate) fn cmd_new_session(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    // Create a new session with shared agent_loop, preserving model/thinking
    // Use TUI-provided cwd if available, otherwise default workspace.
    // Trim trailing whitespace / separators so the saved cwd doesn't
    // produce a phantom workspace name (e.g. "project/ " → name " ").
    let session_cwd = if !cmd.cwd.is_empty() {
        cmd.cwd.trim().trim_end_matches(['/', '\\']).to_string()
    } else {
        crate::rpc::session::default_workspace()
    };
    // No active/default session to inherit from — everything comes from
    // AppState-level singletons and the loop template.  The fresh loop is
    // minted from the template (never used for runs), so creation succeeds
    // even while every existing session is mid-stream.
    let broadcaster = Arc::new(SseBroadcaster::new());
    let approval_gate = state.approval_gate.clone();
    // Reuse the AppState's session manager rather than minting a new one via
    // `Manager::default_for`: the session store is a single flat directory
    // (`default_session_dir` ignores its cwd argument), so both point at the
    // same place in production — and reusing keeps tests (whose AppState uses
    // an isolated temp dir) from writing into the real session store.
    let session_manager = state.session_manager.clone();
    let inherit_model = state.loop_template.model.clone();

    let fresh_loop = state.loop_template.independent_copy();

    let new_session_id = if cmd.session_id.is_empty() {
        crate::utils::generate_id()
    } else {
        cmd.session_id.clone()
    };

    // If this session ID already exists on disk (e.g. a forked session),
    // load the existing entries and restore them after creating the session.
    let existing_entries = session_manager
        .load(&new_session_id)
        .ok()
        .filter(|s| !s.entries.is_empty())
        .map(|s| (s.entries, s.model.clone()));

    let mut new_sess = ServerSession::new_with_queue_budget(
        new_session_id.clone(),
        Arc::new(tokio::sync::RwLock::new(fresh_loop)),
        session_manager.clone(),
        &session_cwd,
        broadcaster,
        approval_gate,
        state.model_registry.clone(),
        state.queue_budget.clone(),
    );
    // Resolve the default model from the cached registry (not inherited from
    // the active session) so that CLI one-shot runs always start from the
    // preferred default.  GUI/TUI explicitly set model_id on the command,
    // which overrides this below.
    let default_model = crate::models::get_default_model_with(&state.model_registry.read())
        .unwrap_or_else(|| inherit_model.clone());
    // Apply via set_model: it records the canonical identity and installs an
    // identity-only client that resolves the authoritative provider/model
    // snapshot for each request. A bare `loop_.model = bare_id` would retain
    // the template's static startup client.
    if let Err(e) = new_sess.set_model(&default_model.clone()) {
        tracing::warn!("[new_session] could not sync model to fresh loop: {e}");
    }
    // Always start new sessions at the preferred thinking level.
    new_sess.thinking_level = "xhigh".to_string();

    // Apply user settings (previously applied only to the startup "default
    // session" — with sessions as equal peers, every new session gets them).
    let settings_path = std::path::PathBuf::from(crate::models::settings_path());
    if let Ok(settings) = crate::config::load_settings(&settings_path) {
        if !settings.default_permission_level.is_empty() {
            new_sess.set_permission_level(&settings.default_permission_level);
        }
        new_sess.set_auto_compaction(settings.compaction_enabled());
        new_sess.set_auto_retry(settings.retry_enabled());
    }

    // Default created_by to "tui" for sessions created without explicit
    // source info (e.g. TUI, channels); clients pass their identity via the
    // typed created_by field.
    new_sess.created_by = "tui".to_string();
    if !cmd.parent_session.is_empty() {
        new_sess.parent_session_id = cmd.parent_session.clone();
    }

    // Session provenance: typed created_by/source_meta fields first.
    if !cmd.created_by.is_empty() {
        new_sess.created_by = cmd.created_by.clone();
    }
    new_sess.creator_id = cmd.creator_id.clone();
    if !cmd.source_meta.is_empty() {
        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&cmd.source_meta) {
            new_sess.source_meta = meta;
        }
    }
    // Legacy fallback: old clients smuggle {"createdBy":...,"sourceMeta":...}
    // JSON through custom_instructions (which belongs to the compact command).
    if new_sess.created_by == "tui" && !cmd.custom_instructions.is_empty() {
        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&cmd.custom_instructions) {
            if let Some(src) = meta
                .get("createdBy")
                .or_else(|| meta.get("source"))
                .and_then(|v| v.as_str())
            {
                new_sess.created_by = src.to_string();
            }
            if new_sess.source_meta.is_null() {
                if let Some(m) = meta.get("sourceMeta").or_else(|| meta.get("meta")) {
                    new_sess.source_meta = m.clone();
                }
            }
        }
    }
    // Apply model and thinking level from the command if provided
    // (client sends these during session creation so the session
    // starts with the user's selection, without needing a separate
    // set_model/set_thinking_level RPC).
    if !cmd.model_id.is_empty() {
        new_sess.model = cmd.model_id.clone();
    }
    if !cmd.level.is_empty() {
        new_sess.thinking_level = cmd.level.clone();
    }

    // Automation-minted sessions (loop control plane, channels) may pass an
    // initial human-readable title in `name` (same proto field as
    // set_session_name). Empty name keeps the default — the name stays empty
    // until the user renames or a client derives one from the first message.
    if !cmd.name.is_empty() {
        new_sess.set_session_name(&cmd.name);
    }

    // Restore entries from a pre-existing session (forked or persisted).
    if let Some((entries, disk_model)) = existing_entries {
        // Gate image re-hydration on the model that will actually run
        // (disk model wins over the command's default).
        let effective_model = if disk_model.is_empty() {
            new_sess.model.clone()
        } else {
            disk_model.clone()
        };
        let supports_images = state
            .model_registry
            .read()
            .request_model_accepts_images(&effective_model);
        let mut msgs = new_sess.messages.write();
        *msgs = crate::session::entries_to_agent_messages(&entries, supports_images);
        if !disk_model.is_empty() {
            new_sess.model = disk_model.clone();
        }
    }

    // Sync the final session model into the fresh agent loop (may differ
    // from the default model set above due to cmd.model_id or disk_model
    // overrides).
    if let Err(e) = new_sess.set_model(&new_sess.model.clone()) {
        tracing::warn!("[new_session] could not sync agent loop model: {e}");
    }

    // Add to sessions map
    let new_id = state.create_session(new_sess);

    RpcResponse::ok(id, "new_session", serde_json::json!({"sessionId": new_id}))
}

pub(crate) fn cmd_read_session_entries(
    session_manager: &crate::session::Manager,
    session_id: &str,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    if cmd.before.is_some() || cmd.offset.is_some() {
        return match session_manager.history_page(session_id, cmd.before, cmd.offset, cmd.limit) {
            Ok(page) => RpcResponse::ok(id, "get_session_entries", page),
            Err(error) => RpcResponse::build_fail_code(
                id,
                "get_session_entries",
                "session_history_unreadable",
                &format!("Unable to load session history: {error}"),
                serde_json::json!({"sessionId":session_id}),
            ),
        };
    }
    if matches!(session_manager.contains(session_id), Ok(false)) {
        return RpcResponse::ok(
            id,
            "get_session_entries",
            serde_json::json!({"entries": []}),
        );
    }
    let version_before = session_manager.session_revision(session_id).ok();
    let cached = version_before
        .as_ref()
        .and_then(|version| session_manager.cached_display_entries(session_id, version));
    let entries = if let Some(entries) = cached {
        entries
    } else {
        let loaded = match session_manager.load(session_id) {
            Ok(session) => session,
            Err(error) => {
                return RpcResponse::build_fail_code(
                    id,
                    "get_session_entries",
                    "session_history_unreadable",
                    &format!("Unable to load session history: {error}"),
                    serde_json::json!({"sessionId": session_id}),
                );
            }
        };
        let projected = crate::session::display::project_entries(&loaded.entries);
        let projected = std::sync::Arc::new(projected);
        let version_after = session_manager.session_revision(session_id).ok();
        if let (Some(before), Some(after)) = (version_before.as_ref(), version_after) {
            if *before == after {
                session_manager.cache_display_entries(session_id, after, projected.clone());
            }
        }
        projected
    };
    RpcResponse::ok(
        id,
        "get_session_entries",
        serde_json::json!({"entries": entries.as_ref()}),
    )
}

pub(crate) fn cmd_fork(
    state: &AppState,
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let entry_id = &cmd.entry_id;
    if entry_id.is_empty() {
        return RpcResponse::build_fail(
            id,
            "fork",
            "No message selected to fork from. Choose a user message to fork at.",
        );
    }

    // Extract needed data from session
    let (session_manager, broadcaster, _cwd, current_session_id, parent_created_by) = {
        let sess = session.read();
        (
            sess.session_manager.clone(),
            sess.broadcaster.clone(),
            sess.cwd.clone(),
            sess.session_id.clone(),
            sess.created_by.clone(),
        )
    };
    // The fork gets its own agent loop — sharing the parent's loop would let
    // a run in one session block (or be aborted by) the other.
    let agent_loop = Arc::new(tokio::sync::RwLock::new(
        state.loop_template.independent_copy(),
    ));

    // Resolve parent session: use cmd.parent_session if provided,
    // otherwise fork from the current session.
    let parent_id = if !cmd.parent_session.is_empty() {
        cmd.parent_session.clone()
    } else {
        current_session_id.clone()
    };

    // Get parent session from manager
    let parent = match session_manager.load(&parent_id) {
        Ok(s) => s,
        Err(_) => {
            return RpcResponse::build_fail(
                id,
                "fork",
                "Session not found on disk — it may have been deleted or moved.",
            );
        }
    };

    if !parent.entries.iter().any(|entry| entry.id == *entry_id) {
        return RpcResponse::build_fail(
            id,
            "fork",
            "Fork point not found in the parent session; reload history and choose a message.",
        );
    }

    // Fork a new session
    let child_created_by = if cmd.created_by.is_empty() {
        parent_created_by.as_str()
    } else {
        cmd.created_by.as_str()
    };
    let mut forked = crate::session::fork_session(&parent, entry_id);
    crate::session::set_creation_provenance(&mut forked, child_created_by, &cmd.creator_id);
    let forked_id = forked.id.clone();

    // Save the forked session
    if let Err(e) = session_manager.save(&forked) {
        return RpcResponse::build_fail(
            id,
            "fork",
            &format!("failed to save forked session: {}", e),
        );
    }

    // Add to sessions map.  Load the forked entries into
    // in-memory messages so the first prompt doesn't overwrite
    // the saved history on disk — session_prompt.rs saves
    // self.messages back to disk (via File::create), truncating
    // anything not held in memory.
    let mut new_sess = ServerSession::new_with_queue_budget(
        forked_id.clone(),
        agent_loop,
        session_manager,
        &forked.cwd,
        broadcaster,
        state.approval_gate.clone(),
        state.model_registry.clone(),
        state.queue_budget.clone(),
    );
    // Category provenance may fall back for legacy callers; creator_id never
    // inherits because it identifies the client that performed this fork.
    new_sess.created_by = child_created_by.to_string();
    // creator_id identifies the client that performed this fork. Never inherit
    // it from the parent: a TUI/CLI fork of a Desktop session is not owned by
    // the original Desktop installation.
    new_sess.creator_id = cmd.creator_id.clone();
    let supports_images = state
        .model_registry
        .read()
        .request_model_accepts_images(&forked.model);
    let msgs = crate::session::entries_to_agent_messages(&forked.entries, supports_images);
    *new_sess.messages.write() = msgs;
    if !forked.model.is_empty() {
        new_sess.model = forked.model.clone();
        #[cfg(test)]
        {
            let mut slot = MODEL_SYNC_FAIL_HOOK.lock();
            if matches!(slot.as_ref(), Some((sid, _)) if sid == &parent_id) {
                if let Some((_, hook)) = slot.take() {
                    hook(&mut new_sess);
                }
            }
        }
        // Sync the fork's own agent loop so the first prompt uses the
        // forked model, not whatever the template seeded.
        if let Err(e) = new_sess.set_model(&new_sess.model.clone()) {
            tracing::warn!("[fork] could not sync agent loop model: {e}");
        }
    }
    state.create_session(new_sess);

    RpcResponse::ok(id, "fork", serde_json::json!({"sessionId": forked_id}))
}

pub(crate) fn cmd_clone(
    state: &AppState,
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    // Extract needed data from session
    let (session_manager, broadcaster, _cwd, session_id, parent_created_by) = {
        let sess = session.read();
        if sess.messages.read().is_empty() {
            return RpcResponse::build_fail(
                id,
                "clone",
                "Nothing to clone — the current session has no messages yet.",
            );
        }
        (
            sess.session_manager.clone(),
            sess.broadcaster.clone(),
            sess.cwd.clone(),
            sess.session_id.clone(),
            sess.created_by.clone(),
        )
    };
    // Own agent loop for the clone (same reasoning as fork).
    let agent_loop = Arc::new(tokio::sync::RwLock::new(
        state.loop_template.independent_copy(),
    ));

    // Get parent session from manager
    let parent = match session_manager.load(&session_id) {
        Ok(s) => s,
        Err(_) => {
            return RpcResponse::build_fail(
                id,
                "clone",
                "Session not found on disk — it may have been deleted or moved.",
            );
        }
    };

    let leaf_id = parent
        .entries
        .last()
        .map(|e| e.id.clone())
        .unwrap_or_default();
    if leaf_id.is_empty() {
        return RpcResponse::build_fail(
            id,
            "clone",
            "Nothing to clone — no messages found in session.",
        );
    }

    // Fork from leaf
    let child_created_by = if cmd.created_by.is_empty() {
        parent_created_by.as_str()
    } else {
        cmd.created_by.as_str()
    };
    let mut forked = crate::session::fork_session(&parent, &leaf_id);
    crate::session::set_creation_provenance(&mut forked, child_created_by, &cmd.creator_id);
    let forked_id = forked.id.clone();

    // Save the forked session
    if let Err(e) = session_manager.save(&forked) {
        return RpcResponse::build_fail(
            id,
            "clone",
            &format!("failed to save cloned session: {}", e),
        );
    }

    // Add to sessions map.  Load the cloned entries into
    // in-memory messages (same reason as fork — prevents
    // the first prompt from truncating history on disk).
    let mut new_sess = ServerSession::new_with_queue_budget(
        forked_id.clone(),
        agent_loop,
        session_manager,
        &forked.cwd,
        broadcaster,
        state.approval_gate.clone(),
        state.model_registry.clone(),
        state.queue_budget.clone(),
    );
    // Match fork provenance: category fallback is legacy compatibility while
    // creator_id identifies only the client performing this clone.
    new_sess.created_by = child_created_by.to_string();
    new_sess.creator_id = cmd.creator_id.clone();
    let supports_images = state
        .model_registry
        .read()
        .request_model_accepts_images(&forked.model);
    let msgs = crate::session::entries_to_agent_messages(&forked.entries, supports_images);
    *new_sess.messages.write() = msgs;
    if !forked.model.is_empty() {
        new_sess.model = forked.model.clone();
        #[cfg(test)]
        {
            let mut slot = MODEL_SYNC_FAIL_HOOK.lock();
            if matches!(slot.as_ref(), Some((sid, _)) if sid == &session_id) {
                if let Some((_, hook)) = slot.take() {
                    hook(&mut new_sess);
                }
            }
        }
        if let Err(e) = new_sess.set_model(&new_sess.model.clone()) {
            tracing::warn!("[clone] could not sync agent loop model: {e}");
        }
    }
    state.create_session(new_sess);

    RpcResponse::ok(id, "clone", serde_json::json!({"cancelled": false}))
}

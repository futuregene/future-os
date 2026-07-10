use std::sync::Arc;

use super::{
    generate_session_html, get_state_internal, AppState, ApprovalDecision, ApprovalDecisionStatus,
    RpcCommand, RpcResponse, ServerSession, SseBroadcaster,
};

pub fn handle_command_internal(state: &AppState, cmd: RpcCommand) -> String {
    let id = &cmd.id;
    let cmd_type = &cmd.cmd_type;

    if cmd_type == "list_models" {
        return list_models_response(id);
    }

    // Credential refresh operates on every session, not one — handle it before
    // resolving a target session (which would needlessly create/load one).
    if cmd_type == "reload_auth" {
        state.reload_all_credentials();
        return RpcResponse::ok(id, "reload_auth", serde_json::json!({}));
    }

    // Get the target session based on session_id, or use default
    let session = state.get_session(&cmd.session_id);

    match cmd_type.as_str() {
        "prompt" => {
            let Some(session) = state.find_session(&cmd.session_id) else {
                return RpcResponse::build_fail(
                    id,
                    "prompt",
                    &format!(
                        "session `{}` does not exist; create it before sending a prompt",
                        cmd.session_id
                    ),
                );
            };
            let mut sess = session.write().unwrap();
            if sess.is_streaming.load(std::sync::atomic::Ordering::Relaxed) {
                RpcResponse::build_fail(
                    id,
                    "prompt",
                    "agent is still streaming; wait or abort first",
                )
            } else {
                let _ = sess.prompt(&cmd.message, &cmd.images, &cmd.streaming_behavior);
                RpcResponse::ok(id, "prompt", serde_json::json!({}))
            }
        }
        "steer" => {
            let _ = session.write().unwrap().steer(&cmd.message);
            RpcResponse::ok(id, "steer", serde_json::json!({}))
        }
        "follow_up" => {
            let _ = session.write().unwrap().follow_up(&cmd.message);
            RpcResponse::ok(id, "follow_up", serde_json::json!({}))
        }
        "abort" => {
            // abort() only needs &self — take a read lock so a concurrent
            // reader (get_state polling) can never make the abort a no-op,
            // which a failed try_write() silently did.
            let session_id = {
                let sess = session.read().unwrap();
                sess.abort();
                sess.session_id.clone()
            };
            state
                .approval_gate
                .cancel_session(&session_id, "Cancelled because the run was terminated.");
            RpcResponse::ok(id, "abort", serde_json::json!({}))
        }
        "approval_decision" => {
            let (approved, status) = match cmd.mode.as_str() {
                "approved" => (true, ApprovalDecisionStatus::Approved),
                "rejected" => (false, ApprovalDecisionStatus::Rejected),
                "cancelled" => (false, ApprovalDecisionStatus::Cancelled),
                _ => {
                    return RpcResponse::build_fail(
                        id,
                        "approval_decision",
                        "mode must be approved, rejected, or cancelled",
                    );
                }
            };
            match state.approval_gate.decide(
                &cmd.entry_id,
                ApprovalDecision {
                    approved,
                    note: cmd.message.clone(),
                    status,
                },
            ) {
                Ok(()) => RpcResponse::ok(
                    id,
                    "approval_decision",
                    serde_json::json!({"approvalRequestId": cmd.entry_id, "status": cmd.mode}),
                ),
                Err(error) => RpcResponse::build_fail(id, "approval_decision", &error),
            }
        }
        "new_session" => {
            // Create a new session with shared agent_loop, preserving model/thinking
            // Use TUI-provided cwd if available, otherwise default workspace
            let session_cwd = if !cmd.cwd.is_empty() {
                cmd.cwd.clone()
            } else {
                super::session::default_workspace()
            };
            let active_id = state.get_active_session_id();
            let session = state.get_session(&active_id);
            // Snapshot everything we need from the active session up front, then
            // drop its lock — nothing below should keep the active ServerSession
            // borrowed while we fall back to other loops for the config template.
            let (
                active_loop,
                inherit_model,
                inherit_thinking,
                event_bus,
                broadcaster,
                approval_gate,
                session_manager,
            ) = {
                let sess = session.read().unwrap();
                (
                    sess.agent_loop.clone(),
                    sess.model.clone(),
                    sess.thinking_level.clone(),
                    sess.event_bus.clone(),
                    sess.broadcaster.clone(),
                    sess.approval_gate.clone(),
                    sess.session_manager.clone(),
                )
            };

            // Build the fresh loop's config template from an *idle* loop. The
            // active session's loop is held under a write lock for the whole turn
            // while it streams, so a `try_read` on it fails mid-run — which used to
            // make "start a second conversation while the first is running" fail
            // outright. Fall back to the default session's loop (never used for
            // prompts by the GUI, so effectively always idle) so concurrent
            // sessions can be created. Clients call `set_model` on the new session
            // right after, so this template is only a seed.
            let snapshot = |loop_arc: &Arc<tokio::sync::RwLock<crate::agent::Loop>>| {
                loop_arc.try_read().ok().map(|loop_guard| {
                    let config = crate::types::AgentConfig {
                        system_prompt: loop_guard.config.system_prompt.clone(),
                        max_turns: loop_guard.config.max_turns,
                        thinking_budget: loop_guard.config.thinking_budget,
                        max_retries: loop_guard.config.max_retries,
                        tools_execution_mode: loop_guard.config.tools_execution_mode.clone(),
                        ..Default::default()
                    };
                    (
                        loop_guard.provider.clone(),
                        loop_guard.model.clone(),
                        loop_guard.tools.clone(),
                        config,
                        loop_guard.verbose,
                    )
                })
            };

            let template = snapshot(&active_loop).or_else(|| {
                let default_loop = state.session.read().unwrap().agent_loop.clone();
                snapshot(&default_loop)
            });
            let Some((provider, model, tools, config, verbose)) = template else {
                return RpcResponse::build_fail(
                    id,
                    "new_session",
                    "agent is busy; wait for the current run to finish before starting a new session",
                );
            };
            let mut fresh_loop = crate::agent::Loop::new(provider, &model)
                .with_tools(tools)
                .with_config(config);
            fresh_loop.verbose = verbose;

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

            let mut new_sess = ServerSession::new_with_shared_loop(
                new_session_id.clone(),
                Arc::new(tokio::sync::RwLock::new(fresh_loop)),
                Arc::new(crate::session::Manager::default_for(&session_cwd)),
                &session_cwd,
                event_bus,
                broadcaster,
                approval_gate,
            );
            // Preserve model and thinking level from the current session
            new_sess.model = inherit_model.clone();
            *new_sess.compaction_model.write().unwrap() = inherit_model;
            new_sess.thinking_level = inherit_thinking;

            // Parse source metadata from custom_instructions (JSON).
            // Client passes {"createdBy":"gui","sourceMeta":{...}} or
            // {"source":"tui","meta":{...}}.
            if !cmd.custom_instructions.is_empty() {
                if let Ok(meta) =
                    serde_json::from_str::<serde_json::Value>(&cmd.custom_instructions)
                {
                    if let Some(src) = meta
                        .get("createdBy")
                        .or_else(|| meta.get("source"))
                        .and_then(|v| v.as_str())
                    {
                        new_sess.created_by = src.to_string();
                    }
                    if let Some(m) = meta.get("sourceMeta").or_else(|| meta.get("meta")) {
                        new_sess.source_meta = m.clone();
                    }
                }
            }

            // Restore entries from a pre-existing session (forked or persisted).
            if let Some((entries, disk_model)) = existing_entries {
                let mut msgs = new_sess.messages.write().unwrap();
                *msgs = crate::session::entries_to_agent_messages(&entries);
                if !disk_model.is_empty() {
                    new_sess.model = disk_model.clone();
                    *new_sess.compaction_model.write().unwrap() = disk_model;
                }
            }

            // Add to sessions map
            let new_id = state.create_session(new_sess);

            RpcResponse::ok(id, "new_session", serde_json::json!({"sessionId": new_id}))
        }
        "get_state" => {
            let state_val = get_state_internal(state, &cmd.session_id);
            RpcResponse::ok(id, "get_state", state_val)
        }
        "get_messages" => {
            let msgs = session.read().unwrap().get_messages();
            RpcResponse::ok(id, "get_messages", serde_json::json!({"messages": msgs}))
        }
        "get_events_since" => {
            // P1: backfill current-run events with idx > since_idx (Bridge reconnect).
            let (run_id, events, min_idx) = {
                let sess = session.read().unwrap();
                sess.broadcaster.events_since(&cmd.run_id, cmd.since_idx)
            };
            // A full backfill (`since_idx < 0`) whose earliest buffered event is
            // past idx 0 means the run's opening was dropped on buffer overflow —
            // tell the client so it can flag the gap rather than show a truncated
            // reconstruction as if complete.
            let truncated = cmd.since_idx < 0 && min_idx > 0;
            let events: Vec<serde_json::Value> = events
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "type": e.event_type,
                        "data": e.data,
                        "runId": e.run_id,
                        "idx": e.idx,
                    })
                })
                .collect();
            RpcResponse::ok(
                id,
                "get_events_since",
                serde_json::json!({"runId": run_id, "events": events, "truncated": truncated}),
            )
        }
        "set_model" => {
            let _ = session.write().unwrap().set_model(&cmd.model_id);
            RpcResponse::ok(id, "set_model", serde_json::json!({"model": cmd.model_id}))
        }
        "set_thinking_level" => {
            session.write().unwrap().set_thinking_level(&cmd.level);
            RpcResponse::ok(id, "set_thinking_level", serde_json::json!({}))
        }
        "set_steering_mode" => {
            session.write().unwrap().set_steering_mode(&cmd.mode);
            RpcResponse::ok(id, "set_steering_mode", serde_json::json!({}))
        }
        "set_follow_up_mode" => {
            session.write().unwrap().set_follow_up_mode(&cmd.mode);
            RpcResponse::ok(id, "set_follow_up_mode", serde_json::json!({}))
        }
        "compact" => {
            let result = session.write().unwrap().compact(&cmd.custom_instructions);
            match result {
                Ok(r) => RpcResponse::ok(id, "compact", r),
                Err(e) => RpcResponse::build_fail(id, "compact", &e.to_string()),
            }
        }
        "set_auto_compaction" => {
            session.write().unwrap().set_auto_compaction(cmd.enabled);
            RpcResponse::ok(id, "set_auto_compaction", serde_json::json!({}))
        }
        "set_auto_retry" => {
            session.write().unwrap().set_auto_retry(cmd.enabled);
            RpcResponse::ok(id, "set_auto_retry", serde_json::json!({}))
        }
        "set_system_prompt" => {
            session
                .write()
                .unwrap()
                .set_system_prompt(&cmd.system_prompt);
            RpcResponse::ok(id, "set_system_prompt", serde_json::json!({}))
        }
        "set_tools" => {
            session.write().unwrap().set_tools(&cmd.tools);
            RpcResponse::ok(id, "set_tools", serde_json::json!({"tools": cmd.tools}))
        }
        "disable_tools" => {
            session.write().unwrap().disable_tools();
            RpcResponse::ok(id, "disable_tools", serde_json::json!({}))
        }
        "disable_builtin_tools" => {
            session.write().unwrap().disable_builtin_tools();
            RpcResponse::ok(id, "disable_builtin_tools", serde_json::json!({}))
        }
        "append_system_prompt" => {
            session
                .write()
                .unwrap()
                .append_system_prompt(&cmd.system_prompt);
            RpcResponse::ok(id, "append_system_prompt", serde_json::json!({}))
        }
        "set_ephemeral" => {
            session.write().unwrap().set_ephemeral(cmd.ephemeral);
            RpcResponse::ok(
                id,
                "set_ephemeral",
                serde_json::json!({"ephemeral": cmd.ephemeral}),
            )
        }
        "bash" => {
            let result = session.write().unwrap().execute_bash(&cmd.command);
            match result {
                Ok(r) => RpcResponse::ok(id, "bash", r),
                Err(e) => RpcResponse::build_fail(id, "bash", &e.to_string()),
            }
        }
        "get_session_stats" => {
            let stats = session.read().unwrap().get_session_stats();
            RpcResponse::ok(id, "get_session_stats", stats)
        }
        "list_sessions" => {
            // Use session_manager.list_all() to get all sessions from disk
            let summaries = session
                .read()
                .unwrap()
                .session_manager
                .list_all()
                .unwrap_or_default();
            // Convert to the format expected by TUI
            let sessions: Vec<serde_json::Value> = summaries
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "name": s.name,
                        "model": s.model,
                        "cwd": s.cwd,
                        "updated_at": s.updated_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                        "parent_session_id": s.parent_session_id,
                        "first_message": s.first_message,
                        "query_count": s.query_count,
                    })
                })
                .collect();
            RpcResponse::ok(
                id,
                "list_sessions",
                serde_json::json!({"sessions": sessions}),
            )
        }
        "switch_session" => {
            if cmd.session_id.is_empty() {
                return RpcResponse::build_fail(
                    id,
                    "switch_session",
                    "No session selected. Choose a session from the list to switch to.",
                );
            }
            let mut sess = session.write().unwrap();
            let result = match sess.switch_session(&cmd.session_id) {
                Ok(()) => {
                    if let Ok(mut active_id) = state.active_session_id.try_write() {
                        *active_id = cmd.session_id.clone();
                    }
                    // Give this session its own private broadcaster so events
                    // are only delivered to subscribers of this session.
                    sess.broadcaster = Arc::new(SseBroadcaster::new());
                    // Insert into sessions map so subsequent lookups by this
                    // session_id succeed (avoids fallback-to-default warning).
                    if let Ok(mut sessions) = state.sessions.try_write() {
                        sessions.insert(cmd.session_id.clone(), session.clone());
                    }
                    RpcResponse::ok(
                        id,
                        "switch_session",
                        serde_json::json!({"cancelled": false}),
                    )
                }
                Err(e) => RpcResponse::build_fail(id, "switch_session", &e.to_string()),
            };
            result
        }
        "delete_session" => {
            if cmd.session_id.is_empty() {
                return RpcResponse::build_fail(
                    id,
                    "delete_session",
                    "No session selected to delete. Choose a session first.",
                );
            }
            // Delete from disk
            if let Err(e) = session
                .read()
                .unwrap()
                .session_manager
                .delete(&cmd.session_id)
            {
                return RpcResponse::build_fail(id, "delete_session", &e.to_string());
            }
            // Remove from memory if present
            if let Ok(mut sessions) = state.sessions.try_write() {
                sessions.remove(&cmd.session_id);
            }
            RpcResponse::ok(id, "delete_session", serde_json::json!({"deleted": true}))
        }
        "fork" => {
            let entry_id = &cmd.entry_id;
            if entry_id.is_empty() {
                return RpcResponse::build_fail(
                    id,
                    "fork",
                    "No message selected to fork from. Choose a user message to fork at.",
                );
            }

            // Extract needed data from session
            let (agent_loop, session_manager, event_bus, broadcaster, _cwd, current_session_id) = {
                let sess = session.read().unwrap();
                (
                    sess.agent_loop.clone(),
                    sess.session_manager.clone(),
                    sess.event_bus.clone(),
                    sess.broadcaster.clone(),
                    sess.cwd.clone(),
                    sess.session_id.clone(),
                )
            };

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

            // Fork a new session
            let forked = crate::session::fork_session(&parent, entry_id);
            let forked_id = forked.id.clone();

            // Save the forked session
            if let Err(e) = session_manager.save(&forked) {
                return RpcResponse::build_fail(
                    id,
                    "fork",
                    &format!("failed to save forked session: {}", e),
                );
            }

            // Add to sessions map
            let new_sess = ServerSession::new_with_shared_loop(
                forked_id.clone(),
                agent_loop,
                session_manager,
                &forked.cwd,
                event_bus,
                broadcaster,
                state.approval_gate.clone(),
            );
            state.create_session(new_sess);

            RpcResponse::ok(id, "fork", serde_json::json!({"sessionId": forked_id}))
        }
        "get_fork_messages" => {
            // Load session from disk to get entry IDs (needed for fork).
            let (session_manager, session_id) = {
                let sess = session.read().unwrap();
                (sess.session_manager.clone(), sess.session_id.clone())
            };
            let user_entries: Vec<serde_json::Value> =
                session_manager
                    .load(&session_id)
                    .map(|s| {
                        s.entries
                        .iter()
                        .filter(|e| e.entry_type == crate::session::ENTRY_TYPE_USER)
                        .map(|e| {
                            let content_text = e.content.as_ref()
                                .map(|c| {
                                    if let Some(arr) = c.as_array() {
                                        arr.iter()
                                            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                            .collect::<Vec<_>>()
                                            .join(" ")
                                    } else {
                                        c.as_str().unwrap_or("").to_string()
                                    }
                                })
                                .unwrap_or_default();
                            serde_json::json!({
                                "id": e.id,
                                "role": e.role,
                                "content": content_text,
                                "timestamp": e.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
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
        "get_session_entries" => {
            // Return all displayable entries from a session (user, assistant,
            // tool). Used to import history into a forked GUI thread.
            let (session_manager, session_id) = {
                let sess = session.read().unwrap();
                (sess.session_manager.clone(), sess.session_id.clone())
            };
            let entries: Vec<serde_json::Value> =
                session_manager
                    .load(&session_id)
                    .map(|s| {
                        s.entries
                        .iter()
                        .filter(|e| {
                            matches!(
                                e.entry_type.as_str(),
                                "user" | "assistant" | "tool"
                            )
                        })
                        .map(|e| {
                            let content_text = e.content.as_ref()
                                .map(|c| {
                                    if let Some(arr) = c.as_array() {
                                        arr.iter()
                                            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                            .collect::<Vec<_>>()
                                            .join(" ")
                                    } else {
                                        c.as_str().unwrap_or("").to_string()
                                    }
                                })
                                .unwrap_or_default();
                            // Build the display content for this entry. Only include the
                            // actual visible text — no thinking or tool formatting.
                            // The forked session's messages should look identical to
                            // original GUI messages (which store thinking/tools in
                            // run events, not in the message content).
                            let full_content = if e.entry_type == "tool" {
                                // Tool entries: show the result text, or a placeholder.
                                if content_text.is_empty() {
                                    String::new()
                                } else {
                                    content_text
                                }
                            } else {
                                // User and assistant entries: just the text content.
                                content_text
                            };

                            serde_json::json!({
                                "id": e.id,
                                "role": e.role,
                                "content": full_content,
                                "name": e.name,
                                "tool_args": e.tool_args,
                                "timestamp": e.timestamp.to_rfc3339(),
                            })
                        })
                        .collect()
                    })
                    .unwrap_or_default();
            RpcResponse::ok(
                id,
                "get_session_entries",
                serde_json::json!({"entries": entries}),
            )
        }
        "get_last_assistant_text" => {
            let text = session.read().unwrap().get_last_assistant_text();
            RpcResponse::ok(
                id,
                "get_last_assistant_text",
                serde_json::json!({"text": if text.is_empty() { None } else { Some(text) }}),
            )
        }
        "set_session_name" => {
            let (session_manager, session_id) = {
                let mut sess = session.write().unwrap();
                sess.set_session_name(&cmd.name);
                (sess.session_manager.clone(), sess.session_id.clone())
            };
            // Persist label entry to session JSONL so name survives restarts
            if let Ok(mut s) = session_manager.load(&session_id) {
                s.entries.push(crate::session::SessionEntry {
                    id: crate::utils::generate_entry_id(),
                    parent_id: String::new(),
                    entry_type: crate::session::ENTRY_TYPE_LABEL.to_string(),
                    role: String::new(),
                    content: None,
                    tool_calls: vec![],
                    timestamp: chrono::Local::now(),
                    summary: String::new(),
                    model: String::new(),
                    label: cmd.name.clone(),
                    thinking_level: String::new(),
                    branch_summary: None,
                    custom_type: String::new(),
                    custom_data: None,
                    display: String::new(),
                    provider: String::new(),
                    tool_call_id: String::new(),
                    name: String::new(),
                    tool_args: String::new(),
                    thinking: String::new(),
                });
                s.name = cmd.name.clone();
                let _ = session_manager.save(&s);
            }
            RpcResponse::ok(id, "set_session_name", serde_json::json!({}))
        }
        "get_commands" => {
            // Return commands from skills (similar to Go's extensions + prompts)
            let skill_dirs = vec![
                crate::skills::APP_SKILLS_DIR.to_string(),
                crate::skills::PROJECT_SKILLS_DIR.to_string(),
                crate::skills::AGENTS_SKILLS_DIR.to_string(),
            ];
            let skills = crate::skills::discover_skills(&skill_dirs).unwrap_or_default();

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
        "abort_retry" => {
            session.read().unwrap().abort();
            RpcResponse::ok(id, "abort_retry", serde_json::json!({}))
        }
        "abort_bash" => {
            // Bash abort is handled by the agent loop
            RpcResponse::ok(id, "abort_bash", serde_json::json!({}))
        }
        "cycle_model" => {
            // Cycle to next available model.  Scoping is client-side (TUI/GUI).
            let registry = crate::models::Registry::new();
            let auth = crate::AuthStore::load();

            let models: Vec<String> = registry
                .all_models()
                .into_iter()
                .filter(|m| !m.api_key.is_empty() || auth.get(&m.provider).is_some())
                .map(|m| format!("{}/{}", m.provider, m.id))
                .collect();

            if models.is_empty() {
                return RpcResponse::ok(
                    id,
                    "cycle_model",
                    serde_json::json!({"model": "", "thinkingLevel": ""}),
                );
            }

            let current = session.read().unwrap().model.clone();
            let idx = models.iter().position(|m| m == &current).unwrap_or(0);
            let next_idx = (idx + 1) % models.len();
            let next_model = &models[next_idx];

            // Use set_model to update session, agent_loop, compat, and endpoint
            let _ = session.write().unwrap().set_model(next_model);

            RpcResponse::ok(
                id,
                "cycle_model",
                serde_json::json!({
                    "model": next_model,
                    "thinkingLevel": session.read().unwrap().thinking_level.clone(),
                    "isScoped": false
                }),
            )
        }
        "cycle_thinking_level" => {
            // Cycle thinking level: off -> minimal -> low -> medium -> high -> xhigh -> off
            let levels = ["off", "minimal", "low", "medium", "high", "xhigh"];
            let current = session.read().unwrap().thinking_level.clone();
            let idx = levels.iter().position(|l| *l == current).unwrap_or(0);
            let next_idx = (idx + 1) % levels.len();
            let next_level = levels[next_idx];

            // Update session thinking level and propagate to provider
            session.write().unwrap().set_thinking_level(next_level);

            RpcResponse::ok(
                id,
                "cycle_thinking_level",
                serde_json::json!({"level": next_level}),
            )
        }
        "set_enabled_models" => {
            // Scoped models are now managed entirely by the TUI/client.
            // The agent no longer reads enabled_models — list_models always
            // returns all available models.
            RpcResponse::ok(id, "set_enabled_models", serde_json::json!({}))
        }
        "clone" => {
            // Extract needed data from session
            let (agent_loop, session_manager, event_bus, broadcaster, _cwd, session_id) = {
                let sess = session.read().unwrap();
                if sess.messages.read().unwrap().is_empty() {
                    return RpcResponse::build_fail(
                        id,
                        "clone",
                        "Nothing to clone — the current session has no messages yet.",
                    );
                }
                (
                    sess.agent_loop.clone(),
                    sess.session_manager.clone(),
                    sess.event_bus.clone(),
                    sess.broadcaster.clone(),
                    sess.cwd.clone(),
                    sess.session_id.clone(),
                )
            };

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
            let forked = crate::session::fork_session(&parent, &leaf_id);
            let forked_id = forked.id.clone();

            // Save the forked session
            if let Err(e) = session_manager.save(&forked) {
                return RpcResponse::build_fail(
                    id,
                    "clone",
                    &format!("failed to save cloned session: {}", e),
                );
            }

            // Add to sessions map
            let new_sess = ServerSession::new_with_shared_loop(
                forked_id.clone(),
                agent_loop,
                session_manager,
                &forked.cwd,
                event_bus,
                broadcaster,
                state.approval_gate.clone(),
            );
            state.create_session(new_sess);

            RpcResponse::ok(id, "clone", serde_json::json!({"cancelled": false}))
        }
        "export_html" => {
            // Export session to HTML file
            let sess = session.read().unwrap();
            let session_id = sess.session_id();
            let model = sess.model.clone();
            let cwd = sess.cwd.clone();
            let messages = sess.get_messages();
            drop(sess);

            // Generate HTML
            let html = generate_session_html(&session_id, &model, &cwd, &messages);

            // Write to file
            let output_path = format!("/tmp/future_agent_export_{}.html", session_id);
            if let Err(e) = std::fs::write(&output_path, html) {
                return RpcResponse::build_fail(
                    id,
                    "export_html",
                    &format!("failed to write file: {}", e),
                );
            }

            RpcResponse::ok(id, "export_html", serde_json::json!({"path": output_path}))
        }
        "reload_config" => {
            // Re-discover skills and re-read context files, then rebuild system prompt.
            let (cwd, tools) = {
                let sess = session.read().unwrap();
                let loop_ = match sess.agent_loop.try_read() {
                    Ok(l) => l,
                    Err(_) => {
                        return RpcResponse::build_fail(
                            id,
                            "reload_config",
                            "agent is busy, retry in a moment",
                        );
                    }
                };
                (sess.cwd.clone(), loop_.tools.clone())
            };

            // Re-discover skills (blocking I/O, no locks held)
            let skill_dirs = vec![
                crate::skills::APP_SKILLS_DIR.to_string(),
                format!("{}/{}", cwd, crate::skills::PROJECT_SKILLS_DIR),
                crate::skills::AGENTS_SKILLS_DIR.to_string(),
            ];
            let skills = crate::skills::discover_skills(&skill_dirs).unwrap_or_default();
            let skill_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();

            // Re-read context files
            let mut agent_content = String::new();
            for fname in &["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
                let p = std::path::Path::new(&cwd).join(fname);
                if p.exists() {
                    if let Ok(content) = std::fs::read_to_string(&p) {
                        agent_content = content;
                        break;
                    }
                }
            }
            let context_lines: Vec<String> = if agent_content.is_empty() {
                vec![]
            } else {
                vec![agent_content.clone()]
            };

            // Rebuild system prompt
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let new_prompt = crate::prompt::build_prompt(&crate::prompt::PromptOptions {
                working_directory: cwd.clone(),
                date: today,
                tools: tools.clone(),
                skills: skills.clone(),
                agent_content: agent_content.clone(),
                ..Default::default()
            });

            // Update welcome_* state for get_state
            *state.welcome_skills.write().unwrap() = skill_names.clone();
            *state.welcome_context.write().unwrap() = context_lines;

            // Update running session's system prompt
            let sess = session.read().unwrap();
            if let Ok(mut r#loop) = sess.agent_loop.try_write() {
                r#loop.system_prompt = new_prompt.clone();
                r#loop.config.system_prompt = new_prompt;
            }

            RpcResponse::ok(
                id,
                "reload_config",
                serde_json::json!({
                    "skills": skill_names,
                    "contextFiles": if agent_content.is_empty() { vec![] } else { vec!["CLAUDE.md".to_string()] },
                }),
            )
        }
        "set_cwd" => {
            let mut sess = session.write().unwrap();
            sess.set_cwd(&cmd.cwd);
            RpcResponse::ok(id, "set_cwd", serde_json::json!({"cwd": cmd.cwd}))
        }
        "add_session_rule" => {
            // Same-run "allow in this workspace/chat": message = path glob,
            // mode = access ("read"|"write"). The GUI calls this alongside
            // writing the rule file so the rule takes effect this run too.
            session
                .read()
                .unwrap()
                .add_session_rule(&cmd.message, &cmd.mode);
            RpcResponse::ok(id, "add_session_rule", serde_json::json!({}))
        }
        "set_sandbox_policy" => {
            let Some(policy) = cmd.sandbox_policy else {
                return RpcResponse::build_fail(
                    id,
                    "set_sandbox_policy",
                    "missing sandbox_policy payload",
                );
            };
            let summary = serde_json::json!({
                "tier": policy.tier.as_str(),
                "sandboxAvailable": crate::sandbox::platform_sandbox_available(),
            });
            session.write().unwrap().set_sandbox_policy(policy);
            RpcResponse::ok(id, "set_sandbox_policy", summary)
        }
        "set_permission_level" => {
            let valid = ["all", "workspace", "none"];
            if !valid.contains(&cmd.level.as_str()) {
                return RpcResponse::build_fail(
                    id,
                    "set_permission_level",
                    &format!("invalid level: {}. valid: all, workspace, none", cmd.level),
                );
            }
            session.write().unwrap().set_permission_level(&cmd.level);
            RpcResponse::ok(
                id,
                "set_permission_level",
                serde_json::json!({"permissionLevel": cmd.level}),
            )
        }
        _ => RpcResponse::build_fail(id, cmd_type, &format!("unknown command: {}", cmd_type)),
    }
}

fn list_models_response(id: &str) -> String {
    let registry = crate::models::Registry::new();
    let auth = crate::AuthStore::load();

    // Always return all available models.  Scoping / defaults are client-side.
    let mut models: Vec<crate::models::Model> = registry
        .all_models()
        .into_iter()
        .filter(|model| !model.api_key.is_empty() || auth.get(&model.provider).is_some())
        .filter(|model| model.output.iter().any(|o| o == "text"))
        .collect();

    models.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    models.dedup_by(|left, right| left.id == right.id && left.provider == right.provider);

    let effective_default = models.first().map(|m| m.id.clone()).unwrap_or_default();

    let payload_models: Vec<serde_json::Value> = models
        .into_iter()
        .map(|model| {
            let id = model.id;
            let label = if model.name.is_empty() {
                id.clone()
            } else {
                model.name.clone()
            };
            let thinking_level = if model.reasoning { "high" } else { "off" };
            serde_json::json!({
                "id": id.clone(),
                "label": label,
                "provider": model.provider.clone(),
                "supportsImages": model.input.iter().any(|input| input == "image"),
                "thinkingLevel": thinking_level.to_string(),
                "contextWindow": model.context_window,
                "isDefault": id == effective_default,
            })
        })
        .collect();

    RpcResponse::ok(
        id,
        "list_models",
        serde_json::json!({
            "models": payload_models,
            "defaultModel": effective_default,
            "isScoped": false,
        }),
    )
}

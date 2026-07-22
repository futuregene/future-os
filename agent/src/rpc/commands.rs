use std::sync::Arc;

use super::{
    generate_session_html, get_state_internal, AppState, ApprovalDecision, ApprovalDecisionStatus,
    RpcCommand, RpcResponse, ServerSession, SseBroadcaster, SseEvent,
};

/// Session write lock. parking_lot locks have no poisoning, so this is a
/// plain `.write()` — the macro remains so ~100 call sites stay uniform (and
/// the `$id` is kept for symmetry with the pre-parking_lot error path).
macro_rules! wlock {
    ($session:expr, $id:expr) => {
        $session.write()
    };
}
/// Session read lock — see `wlock!`.
macro_rules! rlock {
    ($session:expr, $id:expr) => {
        $session.read()
    };
}

pub fn handle_command_internal(state: &AppState, cmd: RpcCommand) -> String {
    let id = &cmd.id;
    let cmd_type = &cmd.cmd_type;

    if cmd_type == "get_agent_info" {
        return get_agent_info_response(id);
    }
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
        "shutdown" => {
            state
                .shutting_down
                .store(true, std::sync::atomic::Ordering::SeqCst);
            RpcResponse::ok(
                id,
                "shutdown",
                serde_json::json!({"shutting_down": true, "note": "Existing runs continue; new prompts are rejected."}),
            )
        }
        "prompt" => {
            if state
                .shutting_down
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return RpcResponse::build_fail(
                    id,
                    "prompt",
                    "agent is shutting down; no new prompts accepted",
                );
            }
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
            let mut sess = wlock!(session, id);
            if sess.is_streaming.load(std::sync::atomic::Ordering::Relaxed) {
                RpcResponse::build_fail(
                    id,
                    "prompt",
                    "agent is still streaming; wait or abort first",
                )
            } else {
                match sess.prompt(&cmd.message, &cmd.images, &cmd.attachments) {
                    Ok(()) => RpcResponse::ok(id, "prompt", serde_json::json!({})),
                    Err(e) => RpcResponse::build_fail(id, "prompt", &e.to_string()),
                }
            }
        }
        "steer" => match wlock!(session, id).steer(&cmd.message) {
            Ok(()) => RpcResponse::ok(id, "steer", serde_json::json!({})),
            Err(e) => RpcResponse::build_fail(id, "steer", &e.to_string()),
        },
        "follow_up" => {
            if state
                .shutting_down
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return RpcResponse::build_fail(
                    id,
                    "follow_up",
                    "agent is shutting down; no new prompts accepted",
                );
            }
            match wlock!(session, id).follow_up(&cmd.message) {
                Ok(()) => RpcResponse::ok(id, "follow_up", serde_json::json!({})),
                Err(e) => RpcResponse::build_fail(id, "follow_up", &e.to_string()),
            }
        }
        "abort" => {
            // abort() only needs &self — take a read lock so a concurrent
            // reader (get_state polling) can never make the abort a no-op,
            // which a failed try_write() silently did.
            let session_id = {
                let sess = rlock!(session, id);
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
        "new_session" => cmd_new_session(state, &cmd, id),
        "get_state" => {
            let state_val = get_state_internal(state, &cmd.session_id);
            RpcResponse::ok(id, "get_state", state_val)
        }
        "get_messages" => {
            let msgs = rlock!(session, id).get_messages();
            RpcResponse::ok(id, "get_messages", serde_json::json!({"messages": msgs}))
        }
        "get_events_since" => {
            // P1: backfill current-run events with idx > since_idx (Bridge reconnect).
            let (run_id, events, min_idx) = {
                let sess = rlock!(session, id);
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
            let (result, model_id) = {
                let mut sess = wlock!(session, id);
                let model_id = cmd.model_id.clone();
                (sess.set_model(&model_id), model_id)
            };
            match result {
                Ok(()) => {
                    {
                        let sess = rlock!(session, id);
                        sess.broadcaster.broadcast(SseEvent::new(
                            "model_changed",
                            serde_json::json!({"model": model_id}),
                        ));
                    }
                    RpcResponse::ok(id, "set_model", serde_json::json!({"model": model_id}))
                }
                Err(e) => RpcResponse::build_fail(id, "set_model", &e.to_string()),
            }
        }
        "set_thinking_level" => {
            let level = cmd.level.clone();
            wlock!(session, id).set_thinking_level(&level);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "thinking_level_changed",
                serde_json::json!({"level": level}),
            ));
            RpcResponse::ok(id, "set_thinking_level", serde_json::json!({}))
        }
        "set_steering_mode" => {
            let mode = cmd.mode.clone();
            wlock!(session, id).set_steering_mode(&mode);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "steering_mode_changed",
                serde_json::json!({"mode": mode}),
            ));
            RpcResponse::ok(id, "set_steering_mode", serde_json::json!({}))
        }
        "set_follow_up_mode" => {
            let mode = cmd.mode.clone();
            wlock!(session, id).set_follow_up_mode(&mode);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "follow_up_mode_changed",
                serde_json::json!({"mode": mode}),
            ));
            RpcResponse::ok(id, "set_follow_up_mode", serde_json::json!({}))
        }
        "compact" => {
            let result = wlock!(session, id).compact(&cmd.custom_instructions);
            match result {
                Ok(r) => RpcResponse::ok(id, "compact", r),
                Err(e) => RpcResponse::build_fail(id, "compact", &e.to_string()),
            }
        }
        "set_auto_compaction" => {
            let enabled = cmd.enabled;
            wlock!(session, id).set_auto_compaction(enabled);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "auto_compaction_changed",
                serde_json::json!({"enabled": enabled}),
            ));
            RpcResponse::ok(id, "set_auto_compaction", serde_json::json!({}))
        }
        "set_auto_retry" => {
            wlock!(session, id).set_auto_retry(cmd.enabled);
            RpcResponse::ok(id, "set_auto_retry", serde_json::json!({}))
        }
        "set_system_prompt" => {
            session.write().set_system_prompt(&cmd.system_prompt);
            RpcResponse::ok(id, "set_system_prompt", serde_json::json!({}))
        }
        "set_tools" => {
            let tools = cmd.tools.clone();
            wlock!(session, id).set_tools(&tools);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "tools_changed",
                serde_json::json!({"tools": tools}),
            ));
            RpcResponse::ok(id, "set_tools", serde_json::json!({"tools": tools}))
        }
        "disable_tools" => {
            wlock!(session, id).disable_tools();
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "tools_changed",
                serde_json::json!({"tools": serde_json::Value::Array(vec![])}),
            ));
            RpcResponse::ok(id, "disable_tools", serde_json::json!({}))
        }
        "disable_builtin_tools" => {
            wlock!(session, id).disable_builtin_tools();
            RpcResponse::ok(id, "disable_builtin_tools", serde_json::json!({}))
        }
        "append_system_prompt" => {
            session.write().append_system_prompt(&cmd.system_prompt);
            RpcResponse::ok(id, "append_system_prompt", serde_json::json!({}))
        }
        "set_ephemeral" => {
            wlock!(session, id).set_ephemeral(cmd.ephemeral);
            RpcResponse::ok(
                id,
                "set_ephemeral",
                serde_json::json!({"ephemeral": cmd.ephemeral}),
            )
        }
        "shell" => {
            let result = wlock!(session, id).execute_shell(&cmd.command);
            match result {
                Ok(r) => RpcResponse::ok(id, "shell", r),
                Err(e) => RpcResponse::build_fail(id, "shell", &e.to_string()),
            }
        }
        "get_session_stats" => {
            let stats = rlock!(session, id).get_session_stats();
            RpcResponse::ok(id, "get_session_stats", stats)
        }
        "list_sessions" => {
            // Clone the session manager Arc outside the lock so list_all()'s
            // disk I/O doesn't block concurrent readers/writers.
            let session_manager = {
                let sess = rlock!(session, id);
                sess.session_manager.clone()
            };
            let summaries = session_manager.list_all().unwrap_or_default();

            // Snapshot active session streaming flags.  Collect within a
            // single outer read guard — this is safe because we only acquire
            // inner read locks (never writes), and ParkingLot RwLock allows
            // concurrent reads.
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

            let sessions: Vec<serde_json::Value> = summaries
                .into_iter()
                .map(|s| {
                    let is_streaming = active_flags.get(&s.id).copied().unwrap_or(false);
                    serde_json::json!({
                        "id": s.id,
                        "session_name": s.name,
                        "model": s.model,
                        "cwd": s.cwd,
                        "updated_at": s.updated_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                        "parent_session_id": s.parent_session_id,
                        "first_message": s.first_message,
                        "query_count": s.query_count,
                        "is_streaming": is_streaming,
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
            let mut sess = wlock!(session, id);
            let result = match sess.switch_session(&cmd.session_id) {
                Ok(()) => {
                    if let Some(mut active_id) = state.active_session_id.try_write() {
                        *active_id = cmd.session_id.clone();
                    }
                    // Give this session its own private broadcaster so events
                    // are only delivered to subscribers of this session.
                    sess.broadcaster = Arc::new(SseBroadcaster::new());
                    // Insert into sessions map so subsequent lookups by this
                    // session_id succeed (avoids fallback-to-default warning).
                    if let Some(mut sessions) = state.sessions.try_write() {
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
            if let Err(e) = session.read().session_manager.delete(&cmd.session_id) {
                return RpcResponse::build_fail(id, "delete_session", &e.to_string());
            }
            // Remove from memory if present
            if let Some(mut sessions) = state.sessions.try_write() {
                sessions.remove(&cmd.session_id);
            }
            RpcResponse::ok(id, "delete_session", serde_json::json!({"deleted": true}))
        }
        "fork" => cmd_fork(state, &session, &cmd, id),
        "get_fork_messages" => {
            // Load session from disk to get entry IDs (needed for fork).
            let (session_manager, session_id) = {
                let sess = rlock!(session, id);
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
            // Must not fall back to the default session when the requested id is
            // unrecognised — that leaks another conversation's entries into the
            // wrong caller (e.g. a GUI thread with no agent session yet would
            // see whichever session happens to be the default).
            if let Some(sess) = state.find_session(&cmd.session_id) {
                cmd_get_session_entries(&sess, id)
            } else {
                RpcResponse::ok(
                    id,
                    "get_session_entries",
                    serde_json::json!({"entries": []}),
                )
            }
        }
        "get_last_assistant_text" => {
            let text = rlock!(session, id).get_last_assistant_text();
            RpcResponse::ok(
                id,
                "get_last_assistant_text",
                serde_json::json!({"text": if text.is_empty() { None } else { Some(text) }}),
            )
        }
        "set_session_name" => {
            let (session_manager, session_id) = {
                let mut sess = wlock!(session, id);
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
                    output_tokens: 0,
                    duration_ms: 0,
                    meta: None,
                });
                s.name = cmd.name.clone();
                let _ = session_manager.save(&s);
            }
            let broadcaster = {
                let sess = rlock!(session, id);
                sess.broadcaster.clone()
            };
            broadcaster.broadcast(SseEvent::new(
                "session_name_changed",
                serde_json::json!({"name": cmd.name}),
            ));
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
            rlock!(session, id).abort();
            RpcResponse::ok(id, "abort_retry", serde_json::json!({}))
        }
        "abort_shell" => {
            // Shell abort is handled by the agent loop
            RpcResponse::ok(id, "abort_shell", serde_json::json!({}))
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

            let current = rlock!(session, id).model.clone();
            let idx = models.iter().position(|m| m == &current).unwrap_or(0);
            let next_idx = (idx + 1) % models.len();
            let next_model = &models[next_idx];

            // Use set_model to update session, agent_loop, compat, and endpoint
            if let Err(e) = wlock!(session, id).set_model(next_model) {
                return RpcResponse::build_fail(id, "cycle_model", &e.to_string());
            }
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "model_changed",
                serde_json::json!({"model": next_model}),
            ));

            RpcResponse::ok(
                id,
                "cycle_model",
                serde_json::json!({
                    "model": next_model,
                    "thinkingLevel": rlock!(session, id).thinking_level.clone(),
                    "isScoped": false
                }),
            )
        }
        "cycle_thinking_level" => {
            // Cycle thinking level: off -> minimal -> low -> medium -> high -> xhigh -> off
            let levels = ["off", "minimal", "low", "medium", "high", "xhigh"];
            let current = rlock!(session, id).thinking_level.clone();
            let idx = levels.iter().position(|l| *l == current).unwrap_or(0);
            let next_idx = (idx + 1) % levels.len();
            let next_level = levels[next_idx];

            // Update session thinking level and propagate to provider
            wlock!(session, id).set_thinking_level(next_level);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "thinking_level_changed",
                serde_json::json!({"level": next_level}),
            ));

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
        "clone" => cmd_clone(state, &session, id),
        "export_html" => {
            // Export session to HTML file
            let sess = rlock!(session, id);
            let session_id = sess.session_id();
            let model = sess.model.clone();
            let cwd = sess.cwd.clone();
            let messages = sess.get_messages();
            drop(sess);

            // Generate HTML
            let html = generate_session_html(&session_id, &model, &cwd, &messages);

            // Write to a unique temp file to avoid clobbering concurrent exports.
            let output_path = format!(
                "/tmp/future_agent_export_{}_{}.html",
                session_id,
                chrono::Local::now().format("%Y%m%d%H%M%S")
            );
            if let Err(e) = std::fs::write(&output_path, html) {
                return RpcResponse::build_fail(
                    id,
                    "export_html",
                    &format!("failed to write file: {}", e),
                );
            }

            RpcResponse::ok(id, "export_html", serde_json::json!({"path": output_path}))
        }
        "reload_config" => cmd_reload_config(state, &session, id),
        "set_cwd" => {
            // Trim trailing whitespace / separators so the saved cwd is
            // always a clean directory path — "project/ " produces a
            // phantom workspace name (" ") on import.
            let cwd: String = cmd.cwd.trim().trim_end_matches(['/', '\\']).to_string();
            let (session_manager, session_id) = {
                let mut sess = wlock!(session, id);
                sess.set_cwd(&cwd);
                (sess.session_manager.clone(), sess.session_id.clone())
            };
            // Persist to session JSONL so the cwd survives restarts.
            if let Ok(mut s) = session_manager.load(&session_id) {
                // Update the session_info entry's cwd in the content JSON.
                if let Some(info) = s
                    .entries
                    .iter_mut()
                    .find(|e| e.entry_type == crate::session::ENTRY_TYPE_SESSION_INFO)
                    .and_then(|e| e.content.as_mut())
                {
                    info["cwd"] = serde_json::Value::String(cwd.clone());
                }
                s.cwd = cwd.clone();
                let _ = session_manager.save(&s);
            }
            let broadcaster = {
                let sess = rlock!(session, id);
                sess.broadcaster.clone()
            };
            broadcaster.broadcast(SseEvent::new(
                "cwd_changed",
                serde_json::json!({"cwd": cwd}),
            ));
            RpcResponse::ok(id, "set_cwd", serde_json::json!({"cwd": cwd}))
        }
        "add_session_rule" => {
            // Same-run "allow in this workspace/chat": message = path glob,
            // mode = access ("read"|"write"). The GUI calls this alongside
            // writing the rule file so the rule takes effect this run too.
            session.read().add_session_rule(&cmd.message, &cmd.mode);
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
            let tier = policy.tier.as_str().to_string();
            wlock!(session, id).set_sandbox_policy(policy);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "sandbox_policy_changed",
                serde_json::json!({"tier": tier}),
            ));
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
            wlock!(session, id).set_permission_level(&cmd.level);
            let sess = rlock!(session, id);
            sess.broadcaster.broadcast(SseEvent::new(
                "permission_level_changed",
                serde_json::json!({"level": cmd.level}),
            ));
            RpcResponse::ok(
                id,
                "set_permission_level",
                serde_json::json!({"permissionLevel": cmd.level}),
            )
        }
        _ => RpcResponse::build_fail(id, cmd_type, &format!("unknown command: {}", cmd_type)),
    }
}

fn get_agent_info_response(id: &str) -> String {
    let dirs = vec![
        crate::skills::APP_SKILLS_DIR.to_string(),
        crate::skills::AGENTS_SKILLS_DIR.to_string(),
    ];
    let skills_count = crate::skills::discover_skills(&dirs)
        .map(|s| s.len())
        .unwrap_or(0);
    RpcResponse::ok(
        id,
        "get_agent_info",
        serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "skillsCount": skills_count,
        }),
    )
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

    // Use the same default-model resolution as cmd_new_session so the list
    // and actual session creation agree on which model is the default.
    let effective_default = crate::models::get_default_model()
        .and_then(|full| full.rsplit_once('/').map(|(_, id)| id.to_string()))
        .or_else(|| models.first().map(|m| m.id.clone()))
        .unwrap_or_default();

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

fn cmd_new_session(state: &AppState, cmd: &RpcCommand, id: &str) -> String {
    // Create a new session with shared agent_loop, preserving model/thinking
    // Use TUI-provided cwd if available, otherwise default workspace.
    // Trim trailing whitespace / separators so the saved cwd doesn't
    // produce a phantom workspace name (e.g. "project/ " → name " ").
    let session_cwd = if !cmd.cwd.is_empty() {
        cmd.cwd.trim().trim_end_matches(['/', '\\']).to_string()
    } else {
        super::session::default_workspace()
    };
    let active_id = state.get_active_session_id();
    let session = state.get_session(&active_id);
    // Snapshot what we need from the active session up front, then drop its
    // lock.  The new session's loop comes from the template (never used for
    // runs), so creation succeeds even while every existing session is
    // mid-stream — previously this failed with "agent is busy" whenever the
    // shared loop was read-locked by a run.
    let (inherit_model, event_bus, broadcaster, approval_gate, session_manager) = {
        let sess = rlock!(session, id);
        (
            sess.model.clone(),
            sess.event_bus.clone(),
            sess.broadcaster.clone(),
            sess.approval_gate.clone(),
            sess.session_manager.clone(),
        )
    };

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

    let mut new_sess = ServerSession::new(
        new_session_id.clone(),
        Arc::new(tokio::sync::RwLock::new(fresh_loop)),
        Arc::new(crate::session::Manager::default_for(&session_cwd)),
        &session_cwd,
        event_bus,
        broadcaster,
        approval_gate,
    );
    // Resolve the default model fresh from the registry (not inherited from
    // the active session) so that CLI one-shot runs always start from the
    // preferred default.  GUI/TUI explicitly set model_id on the command,
    // which overrides this below.
    let default_model = crate::models::get_default_model().unwrap_or_else(|| inherit_model.clone());
    new_sess.model = default_model.clone();
    // Also update the fresh loop's model so the LLM call uses the correct
    // bare model ID (the loop takes just the ID, not provider/id).
    // The template's model was inherited from the active session, which may
    // have been changed by a previous --model override.
    if let Ok(mut loop_) = new_sess.agent_loop.try_write() {
        // Strip provider/ prefix to get the bare model ID for LLM API calls.
        let bare_id = default_model
            .rsplit_once('/')
            .map(|(_, id)| id.to_string())
            .unwrap_or_else(|| default_model.clone());
        loop_.model = bare_id;
    }
    // Always start new sessions at the preferred thinking level.
    new_sess.thinking_level = "xhigh".to_string();

    // Default created_by to "tui" for sessions created without
    // explicit source info (e.g. TUI, channels). GUI passes
    // custom_instructions with createdBy: "gui".
    new_sess.created_by = "tui".to_string();
    if !cmd.parent_session.is_empty() {
        new_sess.parent_session_id = cmd.parent_session.clone();
    }

    // Parse source metadata from custom_instructions (JSON).
    // Client passes {"createdBy":"gui","sourceMeta":{...}}.
    if !cmd.custom_instructions.is_empty() {
        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&cmd.custom_instructions) {
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

    // Restore entries from a pre-existing session (forked or persisted).
    if let Some((entries, disk_model)) = existing_entries {
        // Gate image re-hydration on the model that will actually run
        // (disk model wins over the command's default).
        let effective_model = if disk_model.is_empty() {
            new_sess.model.clone()
        } else {
            disk_model.clone()
        };
        let supports_images = crate::models::model_accepts_images(&effective_model);
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

fn cmd_get_session_entries(session: &Arc<parking_lot::RwLock<ServerSession>>, id: &str) -> String {
    // Return displayable entries from a session plus the session_info
    // metadata entry (model, thinking_level, session_name, cwd).
    let (session_manager, session_id) = {
        let sess = rlock!(session, id);
        (sess.session_manager.clone(), sess.session_id.clone())
    };
    let entries: Vec<serde_json::Value> = session_manager
        .load(&session_id)
        .map(|s| {
            s.entries
                .iter()
                .filter(|e| {
                    matches!(
                        e.entry_type.as_str(),
                        "user" | "assistant" | "tool" | "session_info"
                    )
                })
                .map(|e| {
                    let content_text = e
                        .content
                        .as_ref()
                        .map(|c| {
                            if let Some(arr) = c.as_array() {
                                let texts = arr
                                    .iter()
                                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()));
                                if e.role == "user" {
                                    // A user entry's visible text is only their typed
                                    // message (the first text block). Any later text
                                    // block is agent-injected attachment context
                                    // (file paths), which must not leak into the bubble.
                                    texts.take(1).collect::<Vec<_>>().join(" ")
                                } else {
                                    texts.collect::<Vec<_>>().join(" ")
                                }
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

                    let mut entry = serde_json::json!({
                        "id": e.id,
                        "role": e.role,
                        "content": full_content,
                        "name": e.name,
                        "tool_args": e.tool_args,
                        "timestamp": e.timestamp.to_rfc3339(),
                    });
                    // Include thinking and tool_calls for the new agent-based
                    // message display (entryProjection.ts).
                    if !e.thinking.is_empty() {
                        entry["thinking"] = serde_json::Value::String(e.thinking.clone());
                    }
                    // Structured per-entry metadata (e.g. user attachments with
                    // their cached thumbnails) so the GUI can rebuild attachment
                    // chips after reload — the JSONL is the only message source.
                    if let Some(ref meta) = e.meta {
                        entry["meta"] = meta.clone();
                    }
                    if !e.tool_calls.is_empty() {
                        entry["tool_calls"] =
                            serde_json::to_value(&e.tool_calls).unwrap_or(serde_json::Value::Null);
                    }
                    // Per-reply metadata for the GUI's message footer
                    // ("time · N tokens"); set on the final assistant
                    // entry of each run.
                    if e.output_tokens > 0 {
                        entry["output_tokens"] = serde_json::json!(e.output_tokens);
                    }
                    if e.duration_ms > 0 {
                        entry["duration_ms"] = serde_json::json!(e.duration_ms);
                    }
                    // For session_info entries, include the original content
                    // JSON (session_name, cwd, parent_session_id, …) and the
                    // model / thinking_level struct fields so callers can
                    // read fork metadata without a second RPC.
                    if e.entry_type == crate::session::ENTRY_TYPE_SESSION_INFO {
                        if let Some(ref content) = e.content {
                            entry["content"] = content.clone();
                        }
                        if !e.model.is_empty() {
                            entry["model"] = serde_json::Value::String(e.model.clone());
                        }
                        if !e.thinking_level.is_empty() {
                            entry["thinking_level"] =
                                serde_json::Value::String(e.thinking_level.clone());
                        }
                    }
                    entry
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

fn cmd_fork(
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
    let (session_manager, event_bus, broadcaster, _cwd, current_session_id) = {
        let sess = rlock!(session, id);
        (
            sess.session_manager.clone(),
            sess.event_bus.clone(),
            sess.broadcaster.clone(),
            sess.cwd.clone(),
            sess.session_id.clone(),
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

    // Add to sessions map.  Load the forked entries into
    // in-memory messages so the first prompt doesn't overwrite
    // the saved history on disk — session_prompt.rs saves
    // self.messages back to disk (via File::create), truncating
    // anything not held in memory.
    let mut new_sess = ServerSession::new(
        forked_id.clone(),
        agent_loop,
        session_manager,
        &forked.cwd,
        event_bus,
        broadcaster,
        state.approval_gate.clone(),
    );
    let supports_images = crate::models::model_accepts_images(&forked.model);
    let msgs = crate::session::entries_to_agent_messages(&forked.entries, supports_images);
    *new_sess.messages.write() = msgs;
    if !forked.model.is_empty() {
        new_sess.model = forked.model.clone();
        // Sync the fork's own agent loop so the first prompt uses the
        // forked model, not whatever the template seeded.
        if let Err(e) = new_sess.set_model(&new_sess.model.clone()) {
            tracing::warn!("[fork] could not sync agent loop model: {e}");
        }
    }
    state.create_session(new_sess);

    RpcResponse::ok(id, "fork", serde_json::json!({"sessionId": forked_id}))
}

fn cmd_clone(
    state: &AppState,
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    // Extract needed data from session
    let (session_manager, event_bus, broadcaster, _cwd, session_id) = {
        let sess = rlock!(session, id);
        if sess.messages.read().is_empty() {
            return RpcResponse::build_fail(
                id,
                "clone",
                "Nothing to clone — the current session has no messages yet.",
            );
        }
        (
            sess.session_manager.clone(),
            sess.event_bus.clone(),
            sess.broadcaster.clone(),
            sess.cwd.clone(),
            sess.session_id.clone(),
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

    // Add to sessions map.  Load the cloned entries into
    // in-memory messages (same reason as fork — prevents
    // the first prompt from truncating history on disk).
    let mut new_sess = ServerSession::new(
        forked_id.clone(),
        agent_loop,
        session_manager,
        &forked.cwd,
        event_bus,
        broadcaster,
        state.approval_gate.clone(),
    );
    let supports_images = crate::models::model_accepts_images(&forked.model);
    let msgs = crate::session::entries_to_agent_messages(&forked.entries, supports_images);
    *new_sess.messages.write() = msgs;
    if !forked.model.is_empty() {
        new_sess.model = forked.model.clone();
        if let Err(e) = new_sess.set_model(&new_sess.model.clone()) {
            tracing::warn!("[clone] could not sync agent loop model: {e}");
        }
    }
    state.create_session(new_sess);

    RpcResponse::ok(id, "clone", serde_json::json!({"cancelled": false}))
}

fn cmd_reload_config(
    state: &AppState,
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    // Re-discover skills and re-read context files, then rebuild system prompt.
    let (cwd, tools) = {
        let sess = rlock!(session, id);
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
    *state.welcome_skills.write() = skill_names.clone();
    *state.welcome_context.write() = context_lines;

    // Update running session's system prompt
    let sess = rlock!(session, id);
    if let Ok(mut r#loop) = sess.agent_loop.try_write() {
        r#loop.system_prompt = new_prompt.clone();
        r#loop.config.system_prompt = new_prompt;
    }

    // Broadcast to all subscribers so other clients (TUI/GUI) update their
    // skill lists and context-file displays in near real-time.
    let sess = rlock!(session, id);
    sess.broadcaster.broadcast(SseEvent::new(
        "config_reloaded",
        serde_json::json!({
            "skills": skill_names,
            "contextFiles": if agent_content.is_empty() { vec![] } else { vec!["CLAUDE.md".to_string()] },
        }),
    ));

    RpcResponse::ok(
        id,
        "reload_config",
        serde_json::json!({
            "skills": skill_names,
            "contextFiles": if agent_content.is_empty() { vec![] } else { vec!["CLAUDE.md".to_string()] },
        }),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::Loop,
        rpc::ApprovalGate,
        types::{LLMProvider, Message, StreamEvent, ToolDef},
    };
    use std::collections::HashMap;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    struct EmptyProvider;

    #[async_trait::async_trait]
    impl LLMProvider for EmptyProvider {
        async fn stream_chat(
            &self,
            _model: String,
            _messages: Vec<Message>,
            _tools: Vec<ToolDef>,
            _system_prompt: String,
        ) -> anyhow::Result<ReceiverStream<StreamEvent>> {
            let (_tx, rx) = mpsc::channel(1);
            Ok(ReceiverStream::new(rx))
        }
    }

    fn test_workspace() -> String {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("futureos-cmd-test-{stamp}"))
            .to_string_lossy()
            .to_string()
    }

    fn make_app_state() -> AppState {
        let cwd = test_workspace();
        let session = ServerSession::new(
            "default".to_string(),
            Arc::new(tokio::sync::RwLock::new(Loop::new(
                Arc::new(EmptyProvider),
                "mock",
            ))),
            Arc::new(crate::session::Manager::default_for(&cwd)),
            &cwd,
            Arc::new(crate::events::EventBus::new()),
            Arc::new(SseBroadcaster::new()),
            ApprovalGate::default(),
        );
        AppState {
            session: Arc::new(parking_lot::RwLock::new(session)),
            sessions: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            active_session_id: Arc::new(parking_lot::RwLock::new("default".to_string())),
            welcome_version: "0.0.0".to_string(),
            welcome_cwd: cwd.clone(),
            welcome_skills: Arc::new(parking_lot::RwLock::new(vec![])),
            welcome_context: Arc::new(parking_lot::RwLock::new(vec![])),
            welcome_exts: vec![],
            explicit_session: false,
            broadcaster: Arc::new(SseBroadcaster::new()),
            event_bus: Arc::new(crate::events::EventBus::new()),
            approval_gate: ApprovalGate::default(),
            verbose: false,
            shutting_down: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            model_registry: Arc::new(parking_lot::RwLock::new(crate::models::Registry::new())),
            loop_template: Arc::new(Loop::new(Arc::new(EmptyProvider), "mock")),
        }
    }

    fn make_cmd(cmd_type: &str) -> RpcCommand {
        serde_json::from_str(&format!(
            r#"{{"id":"test_cmd","type":"{}","sessionId":""}}"#,
            cmd_type
        ))
        .unwrap()
    }

    fn parse_response(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn unknown_command_returns_error() {
        let state = make_app_state();
        let cmd = make_cmd("nonexistent_command");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("unknown command"));
    }

    #[test]
    fn get_agent_info_returns_version() {
        let state = make_app_state();
        let cmd = make_cmd("get_agent_info");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["version"].is_string());
    }

    #[test]
    fn get_state_returns_session_info() {
        let state = make_app_state();
        let cmd = make_cmd("get_state");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["sessionId"].is_string());
    }

    #[test]
    fn shutdown_sets_flag() {
        let state = make_app_state();
        let cmd = make_cmd("shutdown");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(state
            .shutting_down
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn prompt_rejected_after_shutdown() {
        let state = make_app_state();
        let cmd = make_cmd("shutdown");
        handle_command_internal(&state, cmd);
        let mut cmd = make_cmd("prompt");
        cmd.message = "hello".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("shutting down"));
    }

    #[test]
    fn set_permission_level_valid() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_permission_level");
        cmd.level = "workspace".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["permissionLevel"], "workspace");
    }

    #[test]
    fn set_permission_level_invalid() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_permission_level");
        cmd.level = "invalid_level".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("invalid level"));
    }

    #[test]
    fn set_thinking_level_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_thinking_level");
        cmd.level = "high".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn set_auto_compaction_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_auto_compaction");
        cmd.enabled = false;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn set_auto_retry_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_auto_retry");
        cmd.enabled = true;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn set_ephemeral_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_ephemeral");
        cmd.ephemeral = true;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["ephemeral"], true);
    }

    #[test]
    fn abort_works() {
        let state = make_app_state();
        let cmd = make_cmd("abort");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn get_messages_returns_empty() {
        let state = make_app_state();
        let cmd = make_cmd("get_messages");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["messages"].is_array());
    }

    #[test]
    fn get_session_stats_works() {
        let state = make_app_state();
        let cmd = make_cmd("get_session_stats");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["sessionId"].is_string());
    }

    #[test]
    fn cycle_thinking_level_advances() {
        let state = make_app_state();
        let cmd = make_cmd("cycle_thinking_level");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["level"].is_string());
    }

    #[test]
    fn set_enabled_models_accepted() {
        let state = make_app_state();
        let cmd = make_cmd("set_enabled_models");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn disable_tools_works() {
        let state = make_app_state();
        let cmd = make_cmd("disable_tools");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn disable_builtin_tools_works() {
        let state = make_app_state();
        let cmd = make_cmd("disable_builtin_tools");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn set_system_prompt_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_system_prompt");
        cmd.system_prompt = "You are helpful".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn append_system_prompt_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("append_system_prompt");
        cmd.system_prompt = "Extra instructions".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn set_cwd_trims_trailing_slash() {
        let state = make_app_state();
        let mut cmd = make_cmd("set_cwd");
        cmd.cwd = "/tmp/project/ ".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["cwd"], "/tmp/project");
    }

    #[test]
    fn set_sandbox_policy_missing_payload() {
        let state = make_app_state();
        let cmd = make_cmd("set_sandbox_policy");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("missing sandbox_policy"));
    }

    #[test]
    fn compact_empty_session() {
        let state = make_app_state();
        let cmd = make_cmd("compact");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["messagesRemoved"], 0);
    }

    #[test]
    fn approval_decision_invalid_mode() {
        let state = make_app_state();
        let mut cmd = make_cmd("approval_decision");
        cmd.mode = "invalid".to_string();
        cmd.entry_id = "req_1".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], false);
        assert!(resp["error"]
            .as_str()
            .unwrap()
            .contains("approved, rejected, or cancelled"));
    }

    #[test]
    fn shell_echo() {
        let state = make_app_state();
        std::fs::create_dir_all(&state.session.read().cwd).unwrap();
        let mut cmd = make_cmd("shell");
        cmd.command = "echo test_output".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["output"]
            .as_str()
            .unwrap()
            .contains("test_output"));
        assert_eq!(resp["data"]["exitCode"], 0);
    }

    #[test]
    fn abort_retry_works() {
        let state = make_app_state();
        let cmd = make_cmd("abort_retry");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn abort_shell_works() {
        let state = make_app_state();
        let cmd = make_cmd("abort_shell");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn list_sessions_returns_array() {
        let state = make_app_state();
        let cmd = make_cmd("list_sessions");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["sessions"].is_array());
    }

    #[test]
    fn reload_auth_works() {
        let state = make_app_state();
        let cmd = make_cmd("reload_auth");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }

    #[test]
    fn get_events_since_empty() {
        let state = make_app_state();
        let mut cmd = make_cmd("get_events_since");
        cmd.run_id = "run_1".to_string();
        cmd.since_idx = -1;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert!(resp["data"]["events"].is_array());
    }

    #[test]
    fn get_commands_returns_list() {
        let state = make_app_state();
        let cmd = make_cmd("get_commands");
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        let commands = resp["data"]["commands"].as_array().unwrap();
        // Commands list may be empty in minimal environments (no skills installed)
        assert!(commands.iter().all(|c| c.is_object()));
    }

    #[test]
    fn add_session_rule_works() {
        let state = make_app_state();
        let mut cmd = make_cmd("add_session_rule");
        cmd.message = "/tmp/**".to_string();
        cmd.mode = "read".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
    }
}

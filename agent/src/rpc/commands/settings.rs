//! Session-scoped settings and configuration handlers: model/thinking level,
//! tools, prompt steering, sandbox/permission policy, cwd, and config reload.

use std::sync::Arc;

use crate::rpc::{AppState, RpcCommand, RpcResponse, ServerSession, SseEvent};

pub(crate) fn handle_probe_sandbox(id: &str) -> String {
    match crate::sandbox::platform_sandbox_probe_product() {
        Ok(result) => RpcResponse::ok(
            id,
            "probe_sandbox",
            serde_json::to_value(result).unwrap_or_else(|_| {
                serde_json::json!({
                    "available": false,
                    "code": "serialization_failed",
                    "backend": "none"
                })
            }),
        ),
        Err(error) => RpcResponse::build_fail(
            id,
            "probe_sandbox",
            &format!("Sandbox availability could not be determined: {error}"),
        ),
    }
}

pub(crate) fn handle_probe_windows_sandbox(id: &str) -> String {
    match crate::sandbox::probe_windows_sandbox_product() {
        Ok(result) => {
            if result.diagnostic().is_some() {
                tracing::warn!(code = result.code, "Windows sandbox host probe unavailable");
            }
            RpcResponse::ok(
                id,
                "probe_windows_sandbox",
                serde_json::to_value(result).unwrap_or_else(
                    |_| serde_json::json!({"available": false, "code": "serialization_failed"}),
                ),
            )
        }
        Err(error) => RpcResponse::build_fail(
            id,
            "probe_windows_sandbox",
            &format!("Windows sandbox probe could not complete: {error}"),
        ),
    }
}

pub(crate) fn handle_reset_windows_sandbox(id: &str) -> String {
    match crate::sandbox::reset_windows_sandbox_capabilities() {
        Ok(removed) => RpcResponse::ok(
            id,
            "reset_windows_sandbox",
            serde_json::json!({"removedCapabilities": removed}),
        ),
        Err(error) => RpcResponse::build_fail(id, "reset_windows_sandbox", &error.to_string()),
    }
}

pub(crate) fn handle_set_model(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let (result, model_id) = {
        let mut sess = session.write();
        let model_id = cmd.model_id.clone();
        (sess.set_model(&model_id), model_id)
    };
    match result {
        Ok(()) => {
            {
                let sess = session.read();
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

pub(crate) fn handle_set_thinking_level(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let level = cmd.level.clone();
    session.write().set_thinking_level(&level);
    let sess = session.read();
    sess.broadcaster.broadcast(SseEvent::new(
        "thinking_level_changed",
        serde_json::json!({"level": level}),
    ));
    RpcResponse::ok(id, "set_thinking_level", serde_json::json!({}))
}

pub(crate) fn handle_cycle_model(
    state: &AppState,
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    // Cycle to next available model.  Scoping is client-side (TUI/GUI).
    // Use the cached registry — Registry::new() re-parses the 1.9 MB
    // catalog AND may do blocking network I/O (future provider
    // refresh) on every call.
    let registry = state.model_registry.read();
    let models: Vec<String> = registry
        .all_models()
        .into_iter()
        .filter(|m| registry.is_model_available(&format!("{}/{}", m.provider, m.id)))
        .map(|m| format!("{}/{}", m.provider, m.id))
        .collect();
    drop(registry);

    if models.is_empty() {
        return RpcResponse::ok(
            id,
            "cycle_model",
            serde_json::json!({"model": "", "thinkingLevel": ""}),
        );
    }

    let current = session.read().model.clone();
    let idx = models.iter().position(|m| m == &current).unwrap_or(0);
    let next_idx = (idx + 1) % models.len();
    let next_model = &models[next_idx];

    // Use set_model to update session, agent_loop, compat, and endpoint
    if let Err(e) = session.write().set_model(next_model) {
        return RpcResponse::build_fail(id, "cycle_model", &e.to_string());
    }
    let sess = session.read();
    sess.broadcaster.broadcast(SseEvent::new(
        "model_changed",
        serde_json::json!({"model": next_model}),
    ));

    RpcResponse::ok(
        id,
        "cycle_model",
        serde_json::json!({
            "model": next_model,
            "thinkingLevel": session.read().thinking_level.clone(),
            "isScoped": false
        }),
    )
}

pub(crate) fn handle_cycle_thinking_level(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    // Cycle thinking level: off -> minimal -> low -> medium -> high -> xhigh -> off
    let levels = ["off", "minimal", "low", "medium", "high", "xhigh"];
    let current = session.read().thinking_level.clone();
    let idx = levels.iter().position(|l| *l == current).unwrap_or(0);
    let next_idx = (idx + 1) % levels.len();
    let next_level = levels[next_idx];

    // Update session thinking level and propagate to provider
    session.write().set_thinking_level(next_level);
    let sess = session.read();
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

pub(crate) fn handle_compact(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    match session.read().compact(&cmd.custom_instructions) {
        Ok(result) => RpcResponse::ok(id, "compact", result),
        Err(error) => RpcResponse::build_fail(id, "compact", &error.to_string()),
    }
}

pub(crate) fn handle_set_auto_compaction(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let enabled = cmd.enabled;
    session.write().set_auto_compaction(enabled);
    let sess = session.read();
    sess.broadcaster.broadcast(SseEvent::new(
        "auto_compaction_changed",
        serde_json::json!({"enabled": enabled}),
    ));
    RpcResponse::ok(id, "set_auto_compaction", serde_json::json!({}))
}

pub(crate) fn handle_set_auto_retry(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    session.write().set_auto_retry(cmd.enabled);
    RpcResponse::ok(id, "set_auto_retry", serde_json::json!({}))
}

pub(crate) fn handle_set_system_prompt(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    session.write().set_system_prompt(&cmd.system_prompt);
    RpcResponse::ok(id, "set_system_prompt", serde_json::json!({}))
}

pub(crate) fn handle_set_tools(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let tools = cmd.tools.clone();
    session.write().set_tools(&tools);
    let sess = session.read();
    sess.broadcaster.broadcast(SseEvent::new(
        "tools_changed",
        serde_json::json!({"tools": tools}),
    ));
    RpcResponse::ok(id, "set_tools", serde_json::json!({"tools": tools}))
}

pub(crate) fn handle_disable_tools(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    session.write().disable_tools();
    let sess = session.read();
    sess.broadcaster.broadcast(SseEvent::new(
        "tools_changed",
        serde_json::json!({"tools": serde_json::Value::Array(vec![])}),
    ));
    RpcResponse::ok(id, "disable_tools", serde_json::json!({}))
}

pub(crate) fn handle_disable_builtin_tools(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    session.write().disable_builtin_tools();
    RpcResponse::ok(id, "disable_builtin_tools", serde_json::json!({}))
}

pub(crate) fn handle_append_system_prompt(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    session.write().append_system_prompt(&cmd.system_prompt);
    RpcResponse::ok(id, "append_system_prompt", serde_json::json!({}))
}

pub(crate) fn handle_set_ephemeral(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    session.write().set_ephemeral(cmd.ephemeral);
    RpcResponse::ok(
        id,
        "set_ephemeral",
        serde_json::json!({"ephemeral": cmd.ephemeral}),
    )
}

pub(crate) fn handle_shell(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let result = session.write().execute_shell(&cmd.command);
    match result {
        Ok(r) => RpcResponse::ok(id, "shell", r),
        Err(e) => RpcResponse::build_fail(id, "shell", &e.to_string()),
    }
}

pub(crate) fn handle_set_cwd(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    // Trim trailing whitespace / separators so the saved cwd is
    // always a clean directory path — "project/ " produces a
    // phantom workspace name (" ") on import.
    let cwd: String = cmd.cwd.trim().trim_end_matches(['/', '\\']).to_string();
    let (session_manager, session_id, persistence) = {
        let mut sess = session.write();
        sess.set_cwd(&cwd);
        (
            sess.session_manager.clone(),
            sess.session_id.clone(),
            sess.persistence.clone(),
        )
    };
    // Persist to session JSONL so the cwd survives restarts.
    if session_manager.find(&session_id).is_some() {
        if let Err(error) = persistence.update_info("cwd", serde_json::Value::String(cwd.clone())) {
            tracing::error!("Failed to persist cwd: {error:#}");
        }
    }
    let broadcaster = {
        let sess = session.read();
        sess.broadcaster.clone()
    };
    broadcaster.broadcast(SseEvent::new(
        "cwd_changed",
        serde_json::json!({"cwd": cwd}),
    ));
    RpcResponse::ok(id, "set_cwd", serde_json::json!({"cwd": cwd}))
}

pub(crate) fn handle_add_session_rule(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    // Same-run "allow in this workspace/chat": message = path glob,
    // mode = access ("read"|"write"). The GUI calls this alongside
    // writing the rule file so the rule takes effect this run too.
    session.read().add_session_rule(&cmd.message, &cmd.mode);
    RpcResponse::ok(id, "add_session_rule", serde_json::json!({}))
}

pub(crate) fn handle_set_sandbox_policy(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let Some(mut policy) = cmd.sandbox_policy.clone() else {
        return RpcResponse::build_fail(id, "set_sandbox_policy", "missing sandbox_policy payload");
    };
    let probe = match crate::sandbox::platform_sandbox_probe_product() {
        Ok(probe) => probe,
        Err(error) if policy.tier.as_str() == "sandbox" => {
            return RpcResponse::build_fail(
                id,
                "set_sandbox_policy",
                &format!("Sandbox availability could not be determined: {error}"),
            );
        }
        Err(_) => crate::sandbox::SandboxProbeResult {
            available: false,
            code: "probe_failed".to_string(),
            backend: "none".to_string(),
            path: None,
            version: None,
            capabilities: None,
        },
    };
    let requested_tier = policy.tier.as_str().to_string();
    let fallback = requested_tier == "sandbox" && !probe.available;
    if fallback {
        policy.tier = crate::sandbox::SandboxTier::Manual;
    }
    let tier = policy.tier.as_str().to_string();
    let summary = serde_json::json!({
        "tier": tier,
        "requestedTier": requested_tier,
        "sandboxAvailable": probe.available,
        "sandboxCode": probe.code,
        "sandboxBackend": probe.backend,
        "fallback": if fallback { Some("manual") } else { None },
    });
    session.write().set_sandbox_policy(policy);
    let sess = session.read();
    sess.broadcaster.broadcast(SseEvent::new(
        "sandbox_policy_changed",
        serde_json::json!({"tier": tier}),
    ));
    RpcResponse::ok(id, "set_sandbox_policy", summary)
}

pub(crate) fn handle_set_permission_level(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let valid = ["all", "workspace", "none"];
    if !valid.contains(&cmd.level.as_str()) {
        return RpcResponse::build_fail(
            id,
            "set_permission_level",
            &format!("invalid level: {}. valid: all, workspace, none", cmd.level),
        );
    }
    session.write().set_permission_level(&cmd.level);
    let sess = session.read();
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

pub(crate) fn handle_set_session_name(
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    cmd: &RpcCommand,
    id: &str,
) -> String {
    let (session_manager, session_id, persistence) = {
        let mut sess = session.write();
        sess.set_session_name(&cmd.name);
        (
            sess.session_manager.clone(),
            sess.session_id.clone(),
            sess.persistence.clone(),
        )
    };
    // Update session_info in the same order as run persistence.
    if session_manager.find(&session_id).is_some() {
        if let Err(error) =
            persistence.update_info("session_name", serde_json::Value::String(cmd.name.clone()))
        {
            tracing::error!("Failed to persist session name: {error:#}");
        }
    }
    let broadcaster = {
        let sess = session.read();
        sess.broadcaster.clone()
    };
    broadcaster.broadcast(SseEvent::new(
        "session_name_changed",
        serde_json::json!({"name": cmd.name}),
    ));
    RpcResponse::ok(id, "set_session_name", serde_json::json!({}))
}

pub(crate) fn cmd_reload_config(
    state: &AppState,
    session: &Arc<parking_lot::RwLock<ServerSession>>,
    id: &str,
) -> String {
    // Re-discover skills and re-read context files, then rebuild system prompt.
    let (cwd, tools, session_id) = {
        let sess = session.read();
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
        (
            sess.cwd.clone(),
            loop_.tools.clone(),
            sess.session_id.clone(),
        )
    };

    // Re-discover skills (blocking I/O, no locks held).  Invalidate the
    // 60s cache first — an explicit reload must see on-disk changes now.
    crate::skills::invalidate_skills_cache();
    let skills = crate::skills::discover_skills_cached(&crate::skills::global_skill_dirs());
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
        session_id: session_id.clone(),
        ..Default::default()
    });

    // Update welcome_* state for get_state
    *state.welcome_skills.write() = skill_names.clone();
    *state.welcome_context.write() = context_lines;

    // Update running session's system prompt
    let sess = session.read();
    if let Ok(mut r#loop) = sess.agent_loop.try_write() {
        r#loop.system_prompt = new_prompt.clone();
        r#loop.config.system_prompt = new_prompt;
    }

    // Broadcast to all subscribers so other clients (TUI/GUI) update their
    // skill lists and context-file displays in near real-time.
    let sess = session.read();
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

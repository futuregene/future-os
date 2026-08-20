//! RPC command dispatch.
//!
//! [`handle_command_internal`] is the single entry point for every RPC command.
//! Handlers are grouped by domain in sibling modules:
//! - `providers`: sessionless auth / model / provider commands
//! - `session_lifecycle`: create/list/switch/delete/fork/clone/entries
//! - `run_control`: prompt enqueue, cancel, abort, approval
//! - `settings`: model/thinking/tools/sandbox/permission/cwd and config reload
//! - `observability`: state/messages/events/stats/export

mod observability;
mod providers;
mod run_control;
mod session_lifecycle;
mod settings;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod dispatcher_tests;
#[cfg(test)]
mod observability_tests;
#[cfg(test)]
mod providers_tests;
#[cfg(test)]
mod run_control_tests;
#[cfg(test)]
mod session_lifecycle_tests;
#[cfg(test)]
mod settings_tests;

use super::{AppState, RpcCommand, RpcResponse};

pub fn handle_command_internal(state: &AppState, cmd: RpcCommand) -> String {
    let id = &cmd.id;
    let cmd_type = &cmd.cmd_type;

    if cmd_type == "get_agent_info" {
        return providers::get_agent_info_response(state, id);
    }
    if cmd_type == "list_models" {
        return providers::list_models_response(
            id,
            &state.model_registry.read(),
            cmd.include_builtin_providers,
        );
    }
    if cmd_type == "list_providers" {
        return providers::list_providers_response(state, id);
    }

    // Credential refresh operates on every session, not one — handle it before
    // resolving a target session (which would needlessly create/load one).
    if cmd_type == "reload_auth" {
        return providers::handle_reload_auth(state, id);
    }

    // ── Config writes (audit item 2): the agent is the sole writer of
    // auth.json / models.json. Each mutation is applied through the agent's
    // own config layer and followed by the same registry rebuild + credential
    // refresh `reload_auth` performs, so clients no longer patch files
    // out-of-band and then paper over the stale in-memory state.
    if cmd_type == "set_auth" {
        return providers::cmd_set_auth(state, id, &cmd);
    }
    if cmd_type == "upsert_provider" {
        return providers::cmd_upsert_provider(state, id, &cmd);
    }
    if cmd_type == "delete_provider" {
        return providers::cmd_delete_provider(state, id, &cmd);
    }

    // Dedicated post-login initialization: synchronously fetch the Future
    // provider's models (warming the cache), then rebuild the registry against
    // that warm cache so the very next `list_models` returns a complete list.
    if cmd_type == "sync_future_models" {
        return providers::handle_sync_future_models(state, id);
    }

    // Persist the onboarding model-picker's choice as the global default model.
    if cmd_type == "set_default_model" {
        return providers::handle_set_default_model(state, &cmd, id);
    }

    // ── Sessionless commands: dispatched WITHOUT resolving a target session.
    match cmd_type.as_str() {
        "shutdown" => return session_lifecycle::cmd_shutdown(state, id),
        "list_sessions" => return session_lifecycle::cmd_list_sessions(state, &cmd, id),
        "list_session_ids" => return session_lifecycle::cmd_list_session_ids(state, id),
        "list_streaming_sessions" => {
            return session_lifecycle::cmd_list_streaming_sessions(state, id)
        }
        "new_session" => return session_lifecycle::cmd_new_session(state, &cmd, id),
        "switch_session" => return session_lifecycle::cmd_switch_session(state, &cmd, id),
        "delete_session" => return session_lifecycle::cmd_delete_session(state, &cmd, id),
        "get_fork_messages" => return session_lifecycle::cmd_get_fork_messages(state, &cmd, id),
        "get_commands" => return session_lifecycle::cmd_get_commands(id),
        // System-wide, no session needed: invalidates the skills discovery cache.
        "refresh_skills" => return providers::cmd_refresh_skills(state, id),
        "set_enabled_models" => {
            // Scoped models are managed entirely by the TUI/client; the agent
            // returns all available models. Kept as a no-op for compatibility.
            return RpcResponse::ok(id, "set_enabled_models", serde_json::json!({}));
        }
        _ => {}
    }

    // ── Session-scoped commands: resolve the target session or fail.
    // No default-session fallback: an empty or unknown session_id is an
    // explicit error, never a silent redirect into another conversation.
    let Some(session) = state.get_session(&cmd.session_id) else {
        return RpcResponse::build_fail(
            id,
            cmd_type,
            "session not found — pass a valid session_id (new_session creates one)",
        );
    };

    match cmd_type.as_str() {
        "prompt" => run_control::handle_prompt(state, &session, &cmd, id),
        "cancel_queued_run" => run_control::handle_cancel_queued_run(&session, &cmd, id),
        "prune_run_events" => run_control::handle_prune_run_events(&session, &cmd, id),
        "abort_session" => run_control::handle_abort_session(&session, id),
        "retry_persistence" => run_control::handle_retry_persistence(&session, id),
        "abort" => run_control::handle_abort(state, &session, &cmd, id),
        "approval_decision" => run_control::handle_approval_decision(state, &cmd, id),
        "get_state" => observability::handle_get_state(state, &cmd, id),
        "get_messages" => observability::handle_get_messages(&session, id),
        "get_events_since" => observability::handle_get_events_since(&session, &cmd, id),
        "get_session_events_since" => {
            observability::handle_get_session_events_since(&session, &cmd, id)
        }
        "set_model" => settings::handle_set_model(&session, &cmd, id),
        "set_thinking_level" => settings::handle_set_thinking_level(&session, &cmd, id),
        "compact" => settings::handle_compact(&session, &cmd, id),
        "set_auto_compaction" => settings::handle_set_auto_compaction(&session, &cmd, id),
        "set_auto_retry" => settings::handle_set_auto_retry(&session, &cmd, id),
        "set_system_prompt" => settings::handle_set_system_prompt(&session, &cmd, id),
        "set_tools" => settings::handle_set_tools(&session, &cmd, id),
        "disable_tools" => settings::handle_disable_tools(&session, id),
        "disable_builtin_tools" => settings::handle_disable_builtin_tools(&session, id),
        "append_system_prompt" => settings::handle_append_system_prompt(&session, &cmd, id),
        "steer" => settings::handle_steer(&session, &cmd, id),
        "set_ephemeral" => settings::handle_set_ephemeral(&session, &cmd, id),
        "shell" => settings::handle_shell(&session, &cmd, id),
        "get_session_stats" => observability::handle_get_session_stats(&session, id),
        "get_runtime_metrics" => observability::handle_get_runtime_metrics(&session, id),
        "fork" => session_lifecycle::cmd_fork(state, &session, &cmd, id),
        "get_session_entries" => session_lifecycle::cmd_get_session_entries(&session, id),
        "get_last_assistant_text" => observability::handle_get_last_assistant_text(&session, id),
        "set_session_name" => settings::handle_set_session_name(&session, &cmd, id),
        "abort_retry" => run_control::handle_abort_retry(&session, id),
        "cycle_model" => settings::handle_cycle_model(state, &session, id),
        "cycle_thinking_level" => settings::handle_cycle_thinking_level(&session, id),
        "clone" => session_lifecycle::cmd_clone(state, &session, id),
        "export_html" => observability::handle_export_html(&session, id),
        "reload_config" => settings::cmd_reload_config(state, &session, id),
        "set_cwd" => settings::handle_set_cwd(&session, &cmd, id),
        "add_session_rule" => settings::handle_add_session_rule(&session, &cmd, id),
        "set_sandbox_policy" => settings::handle_set_sandbox_policy(&session, &cmd, id),
        "set_permission_level" => settings::handle_set_permission_level(&session, &cmd, id),
        _ => RpcResponse::build_fail(id, cmd_type, &format!("unknown command: {}", cmd_type)),
    }
}

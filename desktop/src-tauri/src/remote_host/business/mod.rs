//! Desktop business adapter. No socket, subscription or Tauri AppHandle enters this API.
use crate::remote::protocol::IncomingCmd;
use crate::remote::services::ReplySink;
use serde_json::{json, Value};

mod catalog;
mod compaction;
mod history;
mod prompt;
mod providers;
mod settings;
mod transfers;
mod wire_limits;
pub(crate) use prompt::*;
pub(crate) use wire_limits::*;
pub(crate) async fn execute(cmd: IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "list_sessions" | "list_workspaces" => {
            catalog::execute(&cmd, sink).await;
        }
        "get_messages" | "get_session_entries" | "get_events_since" => {
            history::execute(&cmd, sink).await;
        }
        "upload_init" | "upload_complete" | "upload_cancel" | "list_session_files"
        | "download_prepare" | "download_cancel" => {
            transfers::execute(&cmd, sink).await;
        }
        "prompt" | "get_prompt_receipt" | "abort" | "continue_run" | "approval_decision" => {
            prompt::execute(&cmd, sink).await;
        }
        "compact_context" => {
            compaction::execute(&cmd, sink).await;
        }
        "get_desktop_settings"
        | "update_desktop_settings"
        | "list_settings_models"
        | "list_available_skills"
        | "install_skill"
        | "uninstall_skill"
        | "suggest_skill"
        | "skill_reco_today"
        | "record_skill_reco"
        | "get_state"
        | "list_models"
        | "get_available_models"
        | "list_skills"
        | "set_model"
        | "set_thinking_level"
        | "get_settings"
        | "set_approval_tier" => {
            settings::execute(&cmd, sink).await;
        }
        "generate_session_title"
        | "set_session_name"
        | "set_session_pinned"
        | "delete_session"
        | "set_workspace_pinned"
        | "delete_workspace" => {
            catalog::execute(&cmd, sink).await;
        }
        "list_providers"
        | "update_builtin_provider"
        | "upsert_custom_provider"
        | "delete_custom_provider" => {
            providers::execute(&cmd, sink).await;
        }
        other => {
            reply(
                sink,
                false,
                Value::Null,
                Some(&format!("Unsupported command: {other}")),
            )
            .await;
        }
    }
}

/// A session the Agent answers with `session not found` is a *missing identity*,
/// not a transient failure: the thread is bound to a session id the Agent holds
/// no transcript for — one created but never prompted (the Agent keeps such a
/// row at `revision = -1` and refuses to list it), or one dropped by a restart.
/// The desktop UI already answers such a thread with empty history and lets the
/// prompt path recreate the session on the next send; the remote bridge must
/// answer the phone identically. Forwarding the rejection instead pins the
/// mobile timeline in its "loading messages" state forever, because a failed
/// history page reads as "not loaded yet" and only schedules another retry.
pub(super) fn missing_session(error: &crate::AppError) -> bool {
    error.to_string().contains("session not found")
}

/// The empty display-history page for a session the Agent no longer has, so
/// paging terminates instead of retrying a dead identity.
pub(super) fn empty_entries_page() -> Value {
    json!({"entries": [], "hasMore": false, "nextOffset": 0})
}

pub(super) async fn reply(sink: &dyn ReplySink, success: bool, data: Value, error: Option<&str>) {
    sink.send(success, data, error.map(str::to_owned)).await;
}
pub(super) async fn reply_unit(sink: &dyn ReplySink, result: Result<(), crate::AppError>) {
    match result {
        Ok(()) => reply(sink, true, json!({}), None).await,
        Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
    }
}

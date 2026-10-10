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
mod tasks;
mod transfers;
mod wire_limits;
pub(crate) use prompt::*;
pub(crate) use wire_limits::*;

/// The module that owns an incoming command.
///
/// Routing is a pure step, separate from executing the handler, so the phone's
/// command set can be checked without a store, an Agent or a bridge. The two
/// halves of a command live in different files — the handler match in its own
/// module and this table — and a command routed nowhere is answered
/// "Unsupported command": the client cannot tell that apart from an older
/// desktop, so the feature disappears from the phone while the handler and its
/// unit tests stay green (`set_task_dep`/`remove_task_dep` were exactly that:
/// implemented, tested through `tasks::execute`, advertised as `task_deps_v1`,
/// and unreachable over the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    Catalog,
    History,
    Transfers,
    Prompt,
    Compaction,
    Settings,
    Tasks,
    Providers,
}

/// `None` means no handler owns the command: the bridge answers it with the
/// catch-all "Unsupported command" error.
pub(crate) fn route(cmd_type: &str) -> Option<Route> {
    Some(match cmd_type {
        "list_sessions" | "list_workspaces" => Route::Catalog,
        "get_messages" | "get_session_entries" | "get_events_since" | "get_tool_call_args" => {
            Route::History
        }
        "upload_init" | "upload_complete" | "upload_cancel" | "list_session_files"
        | "download_prepare" | "download_cancel" => Route::Transfers,
        "prompt" | "get_prompt_receipt" | "abort" | "continue_run" | "approval_decision" => {
            Route::Prompt
        }
        "compact_context" => Route::Compaction,
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
        | "set_approval_tier" => Route::Settings,
        "generate_session_title"
        | "set_session_name"
        | "set_session_pinned"
        | "delete_session"
        | "fork_session"
        | "create_workspace"
        | "set_workspace_pinned"
        | "delete_workspace" => Route::Catalog,
        "list_tasks"
        | "get_task"
        | "create_task"
        | "update_task"
        | "delete_task"
        | "set_task_enabled"
        | "run_task"
        | "list_task_runs"
        | "list_task_deps"
        | "set_task_dep"
        | "remove_task_dep"
        | "list_task_revisions"
        | "apply_task_revision" => Route::Tasks,
        "list_providers"
        | "update_builtin_provider"
        | "upsert_custom_provider"
        | "delete_custom_provider" => Route::Providers,
        _ => return None,
    })
}

pub(crate) async fn execute(cmd: IncomingCmd, sink: &dyn ReplySink) {
    let Some(route) = route(&cmd.cmd_type) else {
        reply(
            sink,
            false,
            Value::Null,
            Some(&format!("Unsupported command: {}", cmd.cmd_type)),
        )
        .await;
        return;
    };
    match route {
        Route::Catalog => catalog::execute(&cmd, sink).await,
        Route::History => history::execute(&cmd, sink).await,
        Route::Transfers => transfers::execute(&cmd, sink).await,
        Route::Prompt => prompt::execute(&cmd, sink).await,
        Route::Compaction => compaction::execute(&cmd, sink).await,
        Route::Settings => settings::execute(&cmd, sink).await,
        Route::Tasks => tasks::execute(&cmd, sink).await,
        Route::Providers => providers::execute(&cmd, sink).await,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::test_support::{HomeGuard, RecordingSink};

    /// Every command a paired phone sends, mirrored from the mobile client's
    /// command surface (`mobile/src/remote/useDesktopManagement.ts`,
    /// `useConversationController.ts`, `useTimelineController.ts`,
    /// `useRemoteConnection.ts`, `pairing.ts`, `syncEngine.ts`).
    ///
    /// Three commands the phone also sends are answered outside this table and
    /// are therefore absent here: `get_read_chunk` (reassembled in
    /// `remote_host::read_pages` before dispatch), and the pairing commands
    /// `pair_handshake` / `pair_handshake_confirm` / `get_presence` / `unpair`
    /// (handled by the bridge in `remote/commands.rs`, never reaching a
    /// business handler).
    ///
    /// Adding a command to the phone without routing it here is not a compile
    /// error and not a mobile test failure: the phone ships first and the
    /// desktop answers "Unsupported command" at runtime. This list is the
    /// desktop-side half of that contract.
    const PHONE_COMMANDS: &[&str] = &[
        "abort",
        "apply_task_revision",
        "approval_decision",
        "compact_context",
        "continue_run",
        "create_task",
        "create_workspace",
        "delete_custom_provider",
        "delete_session",
        "delete_task",
        "delete_workspace",
        "download_cancel",
        "download_prepare",
        "fork_session",
        "generate_session_title",
        "get_desktop_settings",
        "get_events_since",
        "get_prompt_receipt",
        "get_session_entries",
        "get_settings",
        "get_state",
        "get_task",
        "get_tool_call_args",
        "install_skill",
        "list_available_skills",
        "list_models",
        "list_providers",
        "list_session_files",
        "list_sessions",
        "list_settings_models",
        "list_skills",
        "list_task_deps",
        "list_task_revisions",
        "list_task_runs",
        "list_tasks",
        "list_workspaces",
        "prompt",
        "record_skill_reco",
        "remove_task_dep",
        "run_task",
        "set_approval_tier",
        "set_model",
        "set_session_name",
        "set_session_pinned",
        "set_task_dep",
        "set_task_enabled",
        "set_thinking_level",
        "set_workspace_pinned",
        "skill_reco_today",
        "suggest_skill",
        "uninstall_skill",
        "update_builtin_provider",
        "update_desktop_settings",
        "update_task",
        "upload_cancel",
        "upload_complete",
        "upload_init",
        "upsert_custom_provider",
    ];

    #[test]
    fn every_command_the_phone_sends_is_routed() {
        for command in PHONE_COMMANDS {
            assert!(
                route(command).is_some(),
                "`{command}` is sent by the mobile client but the dispatcher would answer \
                 \"Unsupported command\""
            );
        }
    }

    /// The table matches on the whole name, so a near-miss is not silently
    /// served by a neighbouring arm.
    #[test]
    fn routing_is_by_exact_command_name() {
        assert_eq!(route("list_tasks"), Some(Route::Tasks));
        for unknown in [
            "",
            " ",
            "list_tasks ",
            "List_Tasks",
            "list_task",
            "set_task_dependent",
            "get_messages_v2",
        ] {
            assert_eq!(route(unknown), None, "`{unknown}` must not be routed");
        }
    }

    /// The regression the routing table exists for: both dependency commands
    /// reach the task handler *through the dispatcher*. `tasks::execute`'s own
    /// test covers the handler, which is why the missing route stayed green
    /// while the phone's dependency editor failed.
    #[tokio::test]
    async fn a_dependency_write_reaches_the_task_handler_through_the_dispatcher() {
        let _home = HomeGuard::new("business-routes-task-deps");
        let store =
            future_tasks::Store::open(&crate::future_home_root()).expect("open the task store");
        let upstream = task("upstream");
        let downstream = task("downstream");
        store.insert_task(&upstream).expect("insert upstream");
        store.insert_task(&downstream).expect("insert downstream");

        let sink = RecordingSink::default();
        execute(
            IncomingCmd {
                cmd_type: "set_task_dep".into(),
                task_id: downstream.id.clone(),
                upstream_task_id: upstream.id.clone(),
                on: "failure".into(),
                ..Default::default()
            },
            &sink,
        )
        .await;
        sink.ok_data();
        let deps = store.list_deps(&downstream.id).expect("list deps");
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].on, future_tasks::DepOn::Failure);

        let sink = RecordingSink::default();
        execute(
            IncomingCmd {
                cmd_type: "remove_task_dep".into(),
                task_id: downstream.id.clone(),
                upstream_task_id: upstream.id.clone(),
                ..Default::default()
            },
            &sink,
        )
        .await;
        sink.ok_data();
        assert!(
            store
                .list_deps(&downstream.id)
                .expect("list deps")
                .is_empty(),
            "the dispatcher removed the edge"
        );
    }

    /// An unrouted command still gets the catch-all the phone already handles,
    /// so the fix above did not turn every name into a handler.
    #[tokio::test]
    async fn an_unknown_command_is_still_unsupported() {
        let sink = RecordingSink::default();
        execute(
            IncomingCmd {
                cmd_type: "definitely_not_a_command".into(),
                ..Default::default()
            },
            &sink,
        )
        .await;
        assert!(sink
            .error_text()
            .contains("Unsupported command: definitely_not_a_command"));
    }

    fn task(name: &str) -> future_tasks::Task {
        let now = 1_000_000;
        future_tasks::Task {
            id: future_tasks::new_task_id(),
            name: name.into(),
            enabled: true,
            prompt: "summarise the week".into(),
            prompt_version: 3,
            cwd: "/tmp/repo".into(),
            model_id: Some("future/gpt-5".into()),
            thinking_level: Some("high".into()),
            session_policy: future_tasks::SessionPolicy::New,
            session_retention: future_tasks::SessionRetention::Keep,
            conversation_mode: future_tasks::ConversationMode::Workspace,
            thread_id: None,
            trigger_kind: future_tasks::TriggerKind::Manual,
            trigger_json: serde_json::Value::Null,
            dep_join: future_tasks::DepJoin::All,
            next_due_at: None,
            pending_request_at: None,
            pending_origin: None,
            pending_actor: None,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        }
    }
}

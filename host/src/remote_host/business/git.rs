//! The phone's Git sheet: the session workspace's history as a lane graph, and
//! one commit's patch.
//!
//! Both commands are read-only and session-rooted: the workspace comes from the
//! session the phone is in, never from the request. The graph itself is built in
//! [`crate::remote_host::git_graph`], which is where the TUI's algorithm lives
//! ported for the host.
use future_remote_protocol::IncomingCmd;
use future_remote_server::services::ReplySink;
use serde_json::Value;

use super::reply;

pub(super) async fn execute(cmd: &IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "list_git_commits" => {
            let session_id = cmd.session_id.clone();
            let result = crate::runtime::run_blocking(move || {
                let workspace = workspace_of(&session_id)?;
                crate::remote_host::git_graph::list(&workspace).map_err(|error| error.to_string())
            })
            .await;
            match result {
                Ok(Ok(data)) => {
                    reply(
                        sink,
                        true,
                        serde_json::to_value(data).unwrap_or(Value::Null),
                        None,
                    )
                    .await
                }
                Ok(Err(error)) => reply(sink, false, Value::Null, Some(&error)).await,
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "get_git_commit" => {
            let session_id = cmd.session_id.clone();
            let git_ref = cmd.git_ref.clone();
            let result = crate::runtime::run_blocking(move || {
                let workspace = workspace_of(&session_id)?;
                crate::remote_host::git_graph::detail(&workspace, &git_ref)
                    .map_err(|error| error.to_string())
            })
            .await;
            match result {
                Ok(Ok(data)) => {
                    reply(
                        sink,
                        true,
                        serde_json::to_value(data).unwrap_or(Value::Null),
                        None,
                    )
                    .await
                }
                Ok(Err(error)) => reply(sink, false, Value::Null, Some(&error)).await,
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        _ => unreachable!("git handler received {}", cmd.cmd_type),
    }
}

/// The directory the session works in. The same resolution
/// `remote_host::session_files` uses, so the sheet and the file browser always
/// describe one workspace.
fn workspace_of(session_id: &str) -> Result<String, String> {
    if session_id.is_empty() {
        return Err("A session is required.".to_string());
    }
    let thread = crate::store::find_thread_by_agent_session(session_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Session workspace is unavailable.".to_string())?;
    crate::agent_bridge::workspace_path_for_thread(&thread.id).map_err(|error| error.to_string())
}

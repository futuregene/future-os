use crate::remote::protocol::IncomingCmd;
use crate::remote::services::ReplySink;
use serde_json::{json, Value};

use super::reply;

pub(super) async fn execute(cmd: &IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "list_sessions" => match crate::remote_host::catalog::sessions("") {
            Some((payload, _)) => reply(sink, true, payload, None).await,
            None => reply(sink, false, Value::Null, Some("catalog_unavailable")).await,
        },
        "list_workspaces" => match crate::remote_host::catalog::workspaces() {
            Some((payload, _)) => reply(sink, true, payload, None).await,
            None => reply(sink, false, Value::Null, Some("catalog_unavailable")).await,
        },
        "generate_session_title" => {
            match crate::agent_bridge::generate_session_title(
                cmd.session_id.clone(),
                cmd.mode.clone(),
            )
            .await
            {
                Ok(data) => reply(sink, true, data, None).await,
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "set_session_name" => {
            match crate::agent_bridge::rename_session(cmd.session_id.clone(), cmd.name.clone())
                .await
            {
                Ok(()) => {
                    if let Ok(Some(thread)) =
                        crate::store::find_thread_by_agent_session(&cmd.session_id)
                    {
                        crate::emit_remote_activity(&thread.id);
                    }
                    reply(sink, true, json!({}), None).await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "set_session_pinned" => {
            match crate::store::pin_thread(crate::store::PinThreadInput {
                thread_id: cmd.thread_id.clone(),
                pinned: cmd.pinned,
            }) {
                Ok(_) => {
                    crate::emit_remote_activity(&cmd.thread_id);
                    reply(sink, true, json!({}), None).await
                }
                Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
            }
        }
        "delete_session" => {
            if cmd.thread_id.is_empty() {
                reply(sink, false, Value::Null, Some("missing thread_id")).await;
            } else {
                match crate::store::delete_thread_with_files(&cmd.thread_id, false) {
                    Ok(_) => {
                        crate::emit_remote_activity(&cmd.thread_id);
                        reply(sink, true, json!({}), None).await
                    }
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                }
            }
        }
        "set_workspace_pinned" => {
            if cmd.workspace_id.is_empty() {
                reply(sink, false, Value::Null, Some("missing workspace_id")).await;
            } else {
                match crate::store::pin_workspace(crate::store::PinWorkspaceInput {
                    workspace_id: cmd.workspace_id.clone(),
                    pinned: cmd.pinned,
                }) {
                    Ok(_) => match crate::remote_host::catalog::workspaces() {
                        Some((payload, _)) => reply(sink, true, payload, None).await,
                        None => reply(sink, false, Value::Null, Some("catalog_unavailable")).await,
                    },
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                }
            }
        }
        "delete_workspace" => {
            if cmd.workspace_id.is_empty() {
                reply(sink, false, Value::Null, Some("missing workspace_id")).await;
            } else {
                match crate::commands::delete_workspace(cmd.workspace_id.clone()).await {
                    Ok(_) => {
                        crate::emit_threads_updated();
                        reply(sink, true, json!({}), None).await
                    }
                    Err(error) => reply(sink, false, Value::Null, Some(&error.to_string())).await,
                }
            }
        }
        _ => unreachable!("catalog handler received {}", cmd.cmd_type),
    }
}

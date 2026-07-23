//! Construction of gRPC `RpcCommand`s and the agent endpoint. This is the thin
//! request-building layer; orchestration and event handling live in the parent
//! module.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tonic::transport::{Channel, Endpoint};

use crate::agent_proto::{Attachment, FutureAgentClient, RpcCommand, RpcResponse};

/// Cap on how long a single connection attempt may take. Without it a hung agent
/// can stall a caller indefinitely — e.g. the GUI's 10s model poll would pile up
/// overlapping calls, and a late failure could clobber fresh state.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// gRPC message-size cap, above tonic's 4MB default (large session responses).
/// Image bytes no longer travel over the wire — the agent reads them from the
/// path — so this need not accommodate base64 payloads. Matches the server.
const MAX_GRPC_MESSAGE_SIZE: usize = 32 * 1024 * 1024;

/// Bare `host:port` the GUI talks to (env override or the default). The single
/// source of the default address, shared with the bundled-agent supervisor.
pub(crate) fn raw_agent_addr() -> String {
    std::env::var("FUTURE_AGENT_GRPC_ADDR").unwrap_or_else(|_| "127.0.0.1:50051".to_string())
}

fn agent_endpoint() -> String {
    let raw = raw_agent_addr();
    if raw.starts_with("http://") || raw.starts_with("https://") {
        raw
    } else {
        format!("http://{raw}")
    }
}

/// Resolve the agent endpoint and open a gRPC client. A connection failure maps
/// to `AppError::AgentUnavailable` so callers can tolerate a down agent (e.g.
/// `abort_run` still cancels the run locally).
pub async fn connect_agent() -> Result<FutureAgentClient<Channel>, crate::AppError> {
    let endpoint = agent_endpoint();
    let unavailable = |error: tonic::transport::Error| {
        crate::AppError::AgentUnavailable(format!(
            "Unable to connect to Future Agent at {endpoint}: {error}"
        ))
    };
    let channel = Endpoint::from_shared(endpoint.clone())
        .map_err(unavailable)?
        .connect_timeout(CONNECT_TIMEOUT)
        .connect()
        .await
        .map_err(unavailable)?;
    // Match the agent server's raised limits: a prompt can carry several
    // base64-encoded images (two ~3MB images ≈ 7MB), which blows past tonic's
    // 4MB default and would otherwise fail the send before the run starts.
    Ok(FutureAgentClient::new(channel)
        .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE)
        .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE))
}

/// Turn a gRPC `RpcResponse` into a `Result`, surfacing the agent's own error
/// message, or `fallback` when the agent reported failure without one.
pub trait RpcResponseExt {
    fn ok_or_rpc_error(self, fallback: &str) -> Result<RpcResponse, crate::AppError>;
}

impl RpcResponseExt for RpcResponse {
    fn ok_or_rpc_error(self, fallback: &str) -> Result<RpcResponse, crate::AppError> {
        if self.success {
            Ok(self)
        } else if self.error.is_empty() {
            Err(fallback.to_string().into())
        } else {
            Err(self.error.into())
        }
    }
}

pub fn get_state_command(session_id: String) -> RpcCommand {
    base_command("get_state", session_id)
}

pub(super) fn fork_command(
    session_id: String,
    entry_id: String,
    parent_session: String,
) -> RpcCommand {
    RpcCommand {
        entry_id,
        parent_session,
        ..base_command("fork", session_id)
    }
}

pub fn delete_session_command(session_id: String) -> RpcCommand {
    base_command("delete_session", session_id)
}

pub fn list_sessions_command() -> RpcCommand {
    base_command("list_sessions", String::new())
}

/// Bulk "who is streaming" query: one RPC returns every streaming session
/// id, so the thread list doesn't fan out one get_state per thread (which
/// also hydrated each polled session on the agent).
pub fn list_streaming_sessions_command() -> RpcCommand {
    base_command("list_streaming_sessions", String::new())
}

pub fn get_session_entries_command(session_id: String) -> RpcCommand {
    base_command("get_session_entries", session_id)
}

pub fn new_session_command(
    session_id: String,
    cwd: String,
    created_by: &str,
    source_meta: serde_json::Value,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> RpcCommand {
    let custom_instructions = serde_json::json!({
        "createdBy": created_by,
        "sourceMeta": source_meta,
    })
    .to_string();
    RpcCommand {
        cwd,
        custom_instructions,
        model_id: model_id.unwrap_or_default(),
        level: thinking_level.unwrap_or_default(),
        ..base_command("new_session", session_id)
    }
}

pub fn set_model_command(model_id: String, session_id: String) -> RpcCommand {
    RpcCommand {
        model_id,
        ..base_command("set_model", session_id)
    }
}

pub fn set_cwd_command(cwd: String, session_id: String) -> RpcCommand {
    RpcCommand {
        cwd,
        ..base_command("set_cwd", session_id)
    }
}

pub fn set_thinking_level_command(level: String, session_id: String) -> RpcCommand {
    RpcCommand {
        level,
        ..base_command("set_thinking_level", session_id)
    }
}

pub fn set_session_name_command(name: String, session_id: String) -> RpcCommand {
    RpcCommand {
        name,
        ..base_command("set_session_name", session_id)
    }
}

pub(super) fn set_permission_level_command(level: String, session_id: String) -> RpcCommand {
    RpcCommand {
        level,
        ..base_command("set_permission_level", session_id)
    }
}

pub(super) fn set_sandbox_policy_command(
    policy: crate::agent_proto::SandboxPolicy,
    session_id: String,
) -> RpcCommand {
    RpcCommand {
        sandbox_policy: Some(policy),
        ..base_command("set_sandbox_policy", session_id)
    }
}

/// Same-run "allow in this workspace/chat" — message = path glob, mode = access.
pub(super) fn add_session_rule_command(
    path: String,
    access: String,
    session_id: String,
) -> RpcCommand {
    RpcCommand {
        message: path,
        mode: access,
        ..base_command("add_session_rule", session_id)
    }
}

/// A file attached to a prompt, as passed from the frontend. Files are
/// referenced by their original absolute path — never copied. Images carry no
/// data here; `encode_attachments` reads the bytes and fills `base64`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInput {
    pub path: String,
    /// "image" | "file".
    pub kind: String,
    pub name: String,
    /// Cached-thumbnail path (images only); carried into the entry meta for reload.
    #[serde(default)]
    pub thumbnail: Option<String>,
}

pub(super) fn prompt_command(
    message: String,
    session_id: String,
    attachments: Vec<AttachmentInput>,
) -> Result<RpcCommand, crate::AppError> {
    // Only paths cross the wire; the agent reads + encodes image bytes itself.
    let attachments = attachments
        .into_iter()
        .map(|item| Attachment {
            path: item.path,
            kind: item.kind,
            name: item.name,
            thumbnail: item.thumbnail.unwrap_or_default(),
        })
        .collect();
    Ok(RpcCommand {
        message,
        attachments,
        ..base_command("prompt", session_id)
    })
}

pub(super) fn approval_decision_command(
    approval_request_id: String,
    status: String,
    note: String,
    session_id: String,
) -> RpcCommand {
    RpcCommand {
        message: note,
        mode: status,
        entry_id: approval_request_id,
        ..base_command("approval_decision", session_id)
    }
}

pub(super) fn base_command(command_type: &str, session_id: String) -> RpcCommand {
    RpcCommand {
        id: command_id(),
        r#type: command_type.to_string(),
        message: String::new(),
        images: vec![],
        attachments: vec![],
        streaming_behavior: String::new(),
        parent_session: String::new(),
        model_id: String::new(),
        level: String::new(),
        mode: String::new(),
        custom_instructions: String::new(),
        enabled: false,
        command: String::new(),
        session_id,
        entry_id: String::new(),
        name: String::new(),
        cwd: String::new(),
        system_prompt: String::new(),
        tools: vec![],
        ephemeral: false,
        enabled_models: vec![],
        run_id: String::new(),
        since_idx: 0,
        sandbox_policy: None,
    }
}

fn command_id() -> String {
    // A monotonic per-process counter makes ids unique even when several
    // commands are issued within the same millisecond (e.g. new_session,
    // set_model, set_thinking_level, prompt during one `agent_prompt`).
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("gui_{millis}_{seq}")
}

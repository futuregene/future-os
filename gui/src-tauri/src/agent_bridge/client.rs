//! Construction of gRPC `RpcCommand`s and the agent endpoint, plus image
//! attachment encoding. This is the thin request-building layer; orchestration
//! and event handling live in the parent module.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use std::time::Duration;
use tonic::transport::{Channel, Endpoint};

use crate::agent_proto::{image_content, FutureAgentClient, ImageContent, RpcCommand, RpcResponse};

/// Cap on how long a single connection attempt may take. Without it a hung agent
/// can stall a caller indefinitely — e.g. the GUI's 10s model poll would pile up
/// overlapping calls, and a late failure could clobber fresh state.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

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
pub(super) async fn connect_agent() -> Result<FutureAgentClient<Channel>, crate::AppError> {
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
    Ok(FutureAgentClient::new(channel))
}

/// Turn a gRPC `RpcResponse` into a `Result`, surfacing the agent's own error
/// message, or `fallback` when the agent reported failure without one.
pub(super) trait RpcResponseExt {
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

pub(super) fn get_state_command(session_id: String) -> RpcCommand {
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

pub(super) fn get_session_entries_command(session_id: String) -> RpcCommand {
    base_command("get_session_entries", session_id)
}

pub(super) fn new_session_command(session_id: String, cwd: String) -> RpcCommand {
    RpcCommand {
        cwd,
        ..base_command("new_session", session_id)
    }
}

pub(super) fn set_model_command(model_id: String, session_id: String) -> RpcCommand {
    RpcCommand {
        model_id,
        ..base_command("set_model", session_id)
    }
}

pub(super) fn set_thinking_level_command(level: String, session_id: String) -> RpcCommand {
    RpcCommand {
        level,
        ..base_command("set_thinking_level", session_id)
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

pub(super) fn prompt_command(
    message: String,
    session_id: String,
    image_paths: Vec<String>,
) -> Result<RpcCommand, crate::AppError> {
    Ok(RpcCommand {
        message,
        images: encode_image_paths(image_paths)?,
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

fn encode_image_paths(paths: Vec<String>) -> Result<Vec<ImageContent>, crate::AppError> {
    paths
        .into_iter()
        .map(|path| {
            let mime_type = image_mime_type(&path)
                .ok_or_else(|| format!("Unsupported image attachment type: {path}"))?;
            let bytes =
                fs::read(&path).map_err(|error| format!("Unable to read image {path}: {error}"))?;
            let data_url = format!("data:{mime_type};base64,{}", BASE64_STANDARD.encode(bytes));
            Ok(ImageContent {
                r#type: "image_base64".to_string(),
                file_path: path,
                content: Some(image_content::Content::Base64(data_url)),
            })
        })
        .collect()
}

fn image_mime_type(path: &str) -> Option<&'static str> {
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase();

    match extension.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "svg" => Some("image/svg+xml"),
        _ => None,
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

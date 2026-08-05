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

/// Process-lifetime runtime that owns the shared agent channel. tonic spawns
/// the h2 connection's driver task on whatever runtime first touches the
/// channel, and several startup callers (session import, run reanimation)
/// `block_on` a throwaway per-thread `Runtime` that is dropped as soon as the
/// thread returns — which used to kill the driver and poison the cached
/// channel for the rest of the process (every later call failed with
/// `Service was not ready: transport error`). Pinning channel creation to a
/// runtime that never shuts down keeps the connection alive regardless of
/// which runtime any caller runs on.
fn agent_channel_runtime() -> tokio::runtime::Handle {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .thread_name("agent-channel")
                .build()
                .expect("build agent channel runtime");
            let handle = runtime.handle().clone();
            std::thread::Builder::new()
                .name("agent-channel-runtime".to_string())
                .spawn(move || {
                    // Park forever: dropping the runtime would kill the shared
                    // connection's driver task.
                    runtime.block_on(std::future::pending::<()>());
                })
                .expect("spawn agent channel runtime thread");
            handle
        })
        .clone()
}

/// Classify an RPC failure from a shared-channel call. With the cached lazy
/// channel, `connect_agent` succeeds even when the agent dies *after* the
/// channel was established — a down agent then surfaces here as a tonic
/// `Unavailable` transport status. Map that to `AppError::AgentUnavailable`
/// so the tolerance sites (`abort_run`'s local cancel, credential reload's
/// "down agent is success") keep working; anything else is an app-level
/// failure and keeps the generic message variant. The agent reports
/// command-level failures inside an OK `RpcResponse` (`success = false`),
/// never as gRPC statuses, so a status error is always transport-level.
pub fn map_rpc_error(context: &str, status: tonic::Status) -> crate::AppError {
    if status.code() == tonic::Code::Unavailable {
        crate::AppError::AgentUnavailable(format!("{context}: {}", status.message()))
    } else {
        crate::AppError::Message(format!("{context}: {status}"))
    }
}

/// Resolve the agent endpoint and open a gRPC client. A connection failure maps
/// to `AppError::AgentUnavailable` so callers can tolerate a down agent (e.g.
/// `abort_run` still cancels the run locally).
pub async fn connect_agent() -> Result<FutureAgentClient<Channel>, crate::AppError> {
    static AGENT_CHANNEL: tokio::sync::OnceCell<Channel> = tokio::sync::OnceCell::const_new();
    let endpoint_str = agent_endpoint();
    let unavailable = |error: tonic::transport::Error| {
        crate::AppError::AgentUnavailable(format!(
            "Unable to connect to Future Agent at {endpoint_str}: {error}"
        ))
    };
    let endpoint = Endpoint::from_shared(endpoint_str.clone())
        .map_err(unavailable)?
        .connect_timeout(CONNECT_TIMEOUT);
    // Shared, lazily-established HTTP/2 channel. Previously every command
    // opened a fresh TCP connection (incl. the per-second status polls), so a
    // fully idle GUI burnt ~0.25 core in backend just on connect/teardown.
    // Cloning a Channel is cheap — every clone shares one underlying h2
    // connection. The channel is created and first used on the pinned
    // process-lifetime runtime (see agent_channel_runtime) so its connection
    // driver outlives any caller's runtime. A one-shot health check validates
    // reachability on first init so callers see a friendly error.
    let channel = agent_channel_runtime()
        .spawn(async move {
            AGENT_CHANNEL
                .get_or_try_init(|| async {
                    let ch = endpoint.connect_lazy();
                    // Validate the lazy channel with a cheap, no-side-effect RPC
                    // so a down agent surfaces the familiar AgentUnavailable
                    // message rather than a raw tonic transport error on the
                    // next real command.
                    let mut client = FutureAgentClient::new(ch.clone())
                        .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE)
                        .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE);
                    client
                        .execute_command(list_streaming_sessions_command())
                        .await
                        .map_err(|status| {
                            crate::AppError::AgentUnavailable(format!(
                                "Unable to connect to Future Agent at {endpoint_str}: {}",
                                status.message()
                            ))
                        })?;
                    Ok::<Channel, crate::AppError>(ch)
                })
                .await
                .cloned()
        })
        .await
        .map_err(|join_error| {
            crate::AppError::AgentUnavailable(format!("Agent channel task failed: {join_error}"))
        })??;
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

pub fn get_run_state_command(session_id: String, run_id: String) -> RpcCommand {
    RpcCommand {
        run_id,
        ..base_command("get_state", session_id)
    }
}

pub fn get_available_models_command() -> RpcCommand {
    base_command("list_models", String::new())
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

pub fn prune_run_events_command(session_id: String, run_id: String) -> RpcCommand {
    RpcCommand {
        run_id,
        ..base_command("prune_run_events", session_id)
    }
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
    RpcCommand {
        cwd,
        // Typed provenance fields (proto created_by/source_meta) — no longer
        // smuggled through custom_instructions, which belongs to compact.
        created_by: created_by.to_string(),
        source_meta: source_meta.to_string(),
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

/// Sessionless: persist the onboarding model-picker's choice as the agent's
/// global default model (settings.json `defaultModel`). No session id.
pub fn set_default_model_command(model_id: String) -> RpcCommand {
    RpcCommand {
        model_id,
        ..base_command("set_default_model", String::new())
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
    requested_run_id: Option<String>,
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
        requested_run_id: requested_run_id.unwrap_or_default(),
        client_request_id: command_id(),
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
        parent_session: String::new(),
        model_id: String::new(),
        level: String::new(),
        mode: String::new(),
        custom_instructions: String::new(),
        created_by: String::new(),
        source_meta: String::new(),
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
        requested_run_id: String::new(),
        client_request_id: String::new(),
        busy_policy: String::new(),
        sandbox_policy: None,
        include_builtin_providers: false,
        auth_update: None,
        provider_config: None,
    }
}

/// `list_models` variant that also asks for the agent's built-in provider
/// catalog summary (`builtinProviders`), so the Providers page can source the
/// catalog from the agent at runtime instead of compiling agent source in.
pub(super) fn list_builtin_providers_command() -> RpcCommand {
    RpcCommand {
        include_builtin_providers: true,
        ..base_command("list_models", String::new())
    }
}

pub(super) fn run_control_command(
    command_type: &str,
    session_id: String,
    run_id: Option<String>,
) -> RpcCommand {
    RpcCommand {
        run_id: run_id.unwrap_or_default(),
        ..base_command(command_type, session_id)
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

#[cfg(test)]
mod tests {
    /// Regression test for the startup channel-poisoning bug: the first
    /// `connect_agent` caller of the process runs on a throwaway per-thread
    /// runtime (mirrors lib.rs's session-import startup thread), which is
    /// dropped as soon as the thread returns. The shared channel must stay
    /// usable afterwards — before the fix, every later call failed with
    /// `Service was not ready: transport error` (surfaced as the run
    /// reanimation failure at startup).
    ///
    /// Requires a live agent on 127.0.0.1:50051: `cargo test -- --ignored`.
    #[test]
    #[ignore = "requires a running future-agent on 127.0.0.1:50051"]
    fn shared_channel_survives_caller_runtime_drop() {
        // First caller on a throwaway runtime, dropped on thread exit.
        std::thread::spawn(|| {
            let rt = tokio::runtime::Runtime::new().expect("runtime 1");
            rt.block_on(async {
                let mut client = super::connect_agent()
                    .await
                    .expect("connect on throwaway runtime");
                client
                    .execute_command(super::list_streaming_sessions_command())
                    .await
                    .expect("first RPC");
            });
        })
        .join()
        .expect("first caller thread");

        // Second caller on a fresh runtime (mirrors the run-reanimation
        // startup thread): with the bug this failed with a transport error.
        let rt = tokio::runtime::Runtime::new().expect("runtime 2");
        rt.block_on(async {
            let mut client = super::connect_agent()
                .await
                .expect("connect after caller runtime drop");
            // Transport-level success is what matters here — an app-level
            // error (unknown session) still proves the connection works.
            client
                .execute_command(super::get_state_command(String::new()))
                .await
                .expect("RPC after caller runtime drop");
        });
    }
}

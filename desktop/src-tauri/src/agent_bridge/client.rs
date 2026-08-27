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

/// Channel cache keyed by endpoint. The GUI talks to a single agent in
/// production, but the test binary hosts more than one in-process mock agent;
/// keying by address lets a changed `FUTURE_AGENT_GRPC_ADDR` reconnect instead
/// of reusing a stale connection to the previous mock.
static AGENT_CHANNEL: tokio::sync::Mutex<Option<(String, Channel)>> =
    tokio::sync::Mutex::const_new(None);

/// Resolve the agent endpoint and open a gRPC client. A connection failure maps
/// to `AppError::AgentUnavailable` so callers can tolerate a down agent (e.g.
/// `abort_run` still cancels the run locally).
pub async fn connect_agent() -> Result<FutureAgentClient<Channel>, crate::AppError> {
    let endpoint_str = agent_endpoint();
    // A cached channel already targeting this endpoint is reused as-is.
    {
        let cached = AGENT_CHANNEL.lock().await;
        if let Some((addr, channel)) = cached.as_ref() {
            if addr == &endpoint_str {
                return Ok(FutureAgentClient::new(channel.clone())
                    .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE)
                    .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE));
            }
        }
    }
    let unavailable = |error: tonic::transport::Error| {
        crate::AppError::AgentUnavailable(format!(
            "Unable to connect to Future Agent at {endpoint_str}: {error}"
        ))
    };
    let endpoint = Endpoint::from_shared(endpoint_str.clone())
        .map_err(unavailable)?
        .connect_timeout(CONNECT_TIMEOUT);
    let health_endpoint = endpoint_str.clone();
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
            let ch = endpoint.connect_lazy();
            let mut client = FutureAgentClient::new(ch.clone())
                .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE)
                .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE);
            health_check(&mut client, &health_endpoint).await?;
            Ok::<Channel, crate::AppError>(ch)
        })
        .await
        // The pinned runtime is process-lifetime (parked forever) and the
        // task body returns every failure as a value — no panic/abort path.
        .expect("agent channel task: pinned runtime outlives the process")?;
    // Remember the freshly connected channel for this endpoint; a later call
    // to a different endpoint overwrites it (and drops the old connection).
    *AGENT_CHANNEL.lock().await = Some((endpoint_str, channel.clone()));
    Ok(FutureAgentClient::new(channel)
        .max_encoding_message_size(MAX_GRPC_MESSAGE_SIZE)
        .max_decoding_message_size(MAX_GRPC_MESSAGE_SIZE))
}

/// One-shot reachability check run when the shared channel is first
/// established: validates the lazy channel with a cheap, no-side-effect RPC
/// so a down agent surfaces the familiar AgentUnavailable message rather
/// than a raw tonic transport error on the next real command.
async fn health_check(
    client: &mut FutureAgentClient<Channel>,
    endpoint_str: &str,
) -> Result<(), crate::AppError> {
    client
        .execute_command(list_streaming_sessions_command())
        .await
        .map_err(|status| {
            crate::AppError::AgentUnavailable(format!(
                "Unable to connect to Future Agent at {endpoint_str}: {}",
                status.message()
            ))
        })?;
    Ok(())
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

pub fn compact_command(session_id: String, instructions: String) -> RpcCommand {
    RpcCommand {
        custom_instructions: instructions,
        ..base_command("compact", session_id)
    }
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

#[cfg(test)]
pub fn prune_run_events_command(session_id: String, run_id: String) -> RpcCommand {
    RpcCommand {
        run_id,
        ..base_command("prune_run_events", session_id)
    }
}

pub fn list_sessions_command() -> RpcCommand {
    base_command("list_sessions", String::new())
}

/// Reconciliation-safe session enumeration: returns only session ids, resolved
/// from the agent's session FILE NAMES (no file contents read). A session whose
/// journal is momentarily unreadable/corrupt is still reported as live, so the
/// orphan-thread cleanup that consumes this can never mistake a transient read
/// failure for a deleted session and hard-delete local threads.
pub fn list_session_ids_command() -> RpcCommand {
    base_command("list_session_ids", String::new())
}

/// Bulk "who is streaming" query: one RPC returns every streaming session
/// id, so the thread list doesn't fan out one get_state per thread (which
/// also hydrated each polled session on the agent).
pub fn list_streaming_sessions_command() -> RpcCommand {
    base_command("list_streaming_sessions", String::new())
}

pub fn get_session_entries_page_command(session_id: String, offset: i64, limit: i64) -> RpcCommand {
    RpcCommand {
        offset: Some(offset),
        limit: Some(limit),
        ..base_command("get_session_entries", session_id)
    }
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
    model_context: String,
    session_id: String,
    attachments: Vec<AttachmentInput>,
    requested_run_id: Option<String>,
) -> RpcCommand {
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
    RpcCommand {
        message,
        model_context,
        attachments,
        requested_run_id: requested_run_id.unwrap_or_default(),
        client_request_id: command_id(),
        ..base_command("prompt", session_id)
    }
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
        model_context: String::new(),
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
        max_events: 0,
        offset: None,
        limit: None,
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
    format!("desktop_{millis}_{seq}")
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{mock_agent, Reply};
    use super::*;

    /// Regression test for the startup channel-poisoning bug: the first
    /// `connect_agent` caller of the process runs on a throwaway per-thread
    /// runtime (mirrors lib.rs's session-import startup thread), which is
    /// dropped as soon as the thread returns. The shared channel must stay
    /// usable afterwards — before the fix, every later call failed with
    /// `Service was not ready: transport error` (surfaced as the run
    /// reanimation failure at startup). Runs against the in-process mock.
    #[test]
    fn shared_channel_survives_caller_runtime_drop() {
        let _mock = mock_agent();
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
            client
                .execute_command(super::get_state_command(String::new()))
                .await
                .expect("RPC after caller runtime drop");
        });
    }

    #[test]
    fn raw_agent_addr_defaults_and_endpoint_adds_scheme() {
        let _mock = mock_agent();
        // The mock sets FUTURE_AGENT_GRPC_ADDR to a bare host:port.
        let raw = raw_agent_addr();
        assert!(!raw.is_empty());
        assert_eq!(agent_endpoint(), format!("http://{raw}"));
    }

    #[test]
    fn agent_endpoint_keeps_an_explicit_scheme() {
        let _mock = mock_agent();
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock sets the addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "https://agent.example:9443");
        assert_eq!(agent_endpoint(), "https://agent.example:9443");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
    }

    #[tokio::test]
    async fn connect_agent_rejects_an_unparseable_endpoint() {
        let _mock = mock_agent();
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock sets the addr");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let result = connect_agent().await;
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        let error = result.expect_err("invalid endpoint must fail");
        assert!(
            matches!(error, crate::AppError::AgentUnavailable(_)),
            "endpoint parse failure maps to AgentUnavailable: {error}"
        );
    }

    #[tokio::test]
    async fn health_check_maps_transport_failure_to_unavailable() {
        let mock = mock_agent();
        // Connect first (the OnceCell init health check consumes the default
        // reply), then script the failure for the explicit health_check call.
        let mut client = connect_agent().await.expect("connect to mock");
        mock.push(
            "list_streaming_sessions",
            Reply::Status(tonic::Code::Unavailable, "mock agent down"),
        );
        let error = health_check(&mut client, "http://mock")
            .await
            .expect_err("health check surfaces the transport failure");
        assert!(matches!(error, crate::AppError::AgentUnavailable(_)));
        let message = error.to_string();
        assert!(message.contains("http://mock"), "message: {message}");
        assert!(message.contains("mock agent down"), "message: {message}");
    }

    #[test]
    fn map_rpc_error_distinguishes_unavailable_from_app_failures() {
        let unavailable = map_rpc_error("ctx", tonic::Status::unavailable("connection refused"));
        assert!(matches!(unavailable, crate::AppError::AgentUnavailable(_)));
        assert_eq!(unavailable.to_string(), "ctx: connection refused");
        let internal = map_rpc_error("ctx", tonic::Status::internal("boom"));
        assert!(matches!(internal, crate::AppError::Message(_)));
        assert!(internal.to_string().starts_with("ctx: "));
    }

    #[test]
    fn ok_or_rpc_error_variants() {
        let ok = RpcResponse {
            success: true,
            ..Default::default()
        };
        assert!(ok.ok_or_rpc_error("fallback").is_ok());

        let with_error = RpcResponse {
            success: false,
            error: "agent said no".to_string(),
            ..Default::default()
        };
        let error = with_error.ok_or_rpc_error("fallback").unwrap_err();
        assert_eq!(error.to_string(), "agent said no");

        let without_error = RpcResponse {
            success: false,
            ..Default::default()
        };
        let error = without_error.ok_or_rpc_error("fallback").unwrap_err();
        assert_eq!(error.to_string(), "fallback");
    }

    #[test]
    fn command_builders_set_type_session_and_ids() {
        let cmd = get_state_command("sess".to_string());
        assert_eq!(cmd.r#type, "get_state");
        assert_eq!(cmd.session_id, "sess");
        assert!(!cmd.id.is_empty(), "command id assigned");

        let cmd = get_run_state_command("sess".to_string(), "run-1".to_string());
        assert_eq!(cmd.r#type, "get_state");
        assert_eq!(cmd.run_id, "run-1");

        assert_eq!(get_available_models_command().r#type, "list_models");
        assert_eq!(get_available_models_command().session_id, "");

        let cmd = fork_command(
            "sess".to_string(),
            "entry-1".to_string(),
            "parent".to_string(),
        );
        assert_eq!(cmd.r#type, "fork");
        assert_eq!(cmd.entry_id, "entry-1");
        assert_eq!(cmd.parent_session, "parent");

        assert_eq!(
            delete_session_command("sess".to_string()).r#type,
            "delete_session"
        );

        let cmd = prune_run_events_command("sess".to_string(), "run-1".to_string());
        assert_eq!(cmd.r#type, "prune_run_events");
        assert_eq!(cmd.run_id, "run-1");

        assert_eq!(list_sessions_command().r#type, "list_sessions");
        assert_eq!(list_session_ids_command().r#type, "list_session_ids");
        assert_eq!(
            list_streaming_sessions_command().r#type,
            "list_streaming_sessions"
        );
        let command = get_session_entries_page_command("sess".to_string(), 25, 50);
        assert_eq!(command.r#type, "get_session_entries");
        assert_eq!(command.offset, Some(25));
        assert_eq!(command.limit, Some(50));
    }

    #[test]
    fn new_session_command_carries_provenance_and_model() {
        let cmd = new_session_command(
            "sess".to_string(),
            "/tmp/ws".to_string(),
            "desktop",
            serde_json::json!({"source": "test"}),
            Some("future/k3".to_string()),
            Some("high".to_string()),
        );
        assert_eq!(cmd.r#type, "new_session");
        assert_eq!(cmd.cwd, "/tmp/ws");
        assert_eq!(cmd.created_by, "desktop");
        assert_eq!(cmd.source_meta, r#"{"source":"test"}"#);
        assert_eq!(cmd.model_id, "future/k3");
        assert_eq!(cmd.level, "high");

        let bare = new_session_command(
            String::new(),
            String::new(),
            "desktop",
            serde_json::Value::Null,
            None,
            None,
        );
        assert_eq!(bare.model_id, "");
        assert_eq!(bare.level, "");
    }

    #[test]
    fn setter_commands_carry_their_payloads() {
        let cmd = set_model_command("m".to_string(), "sess".to_string());
        assert_eq!(
            (cmd.r#type.as_str(), cmd.model_id.as_str()),
            ("set_model", "m")
        );

        let cmd = set_default_model_command("m".to_string());
        assert_eq!(
            (
                cmd.r#type.as_str(),
                cmd.model_id.as_str(),
                cmd.session_id.as_str()
            ),
            ("set_default_model", "m", "")
        );

        let cmd = set_cwd_command("/tmp".to_string(), "sess".to_string());
        assert_eq!((cmd.r#type.as_str(), cmd.cwd.as_str()), ("set_cwd", "/tmp"));

        let cmd = set_thinking_level_command("high".to_string(), "sess".to_string());
        assert_eq!(
            (cmd.r#type.as_str(), cmd.level.as_str()),
            ("set_thinking_level", "high")
        );

        let cmd = set_session_name_command("name".to_string(), "sess".to_string());
        assert_eq!(
            (cmd.r#type.as_str(), cmd.name.as_str()),
            ("set_session_name", "name")
        );

        let cmd = set_permission_level_command("workspace".to_string(), "sess".to_string());
        assert_eq!(
            (cmd.r#type.as_str(), cmd.level.as_str()),
            ("set_permission_level", "workspace")
        );

        let policy = crate::agent_proto::SandboxPolicy {
            tier: "sandbox".to_string(),
        };
        let cmd = set_sandbox_policy_command(policy, "sess".to_string());
        assert_eq!(cmd.r#type, "set_sandbox_policy");
        assert_eq!(cmd.sandbox_policy.expect("policy").tier, "sandbox");

        let cmd = add_session_rule_command(
            "/tmp/**".to_string(),
            "allow".to_string(),
            "sess".to_string(),
        );
        assert_eq!(
            (cmd.r#type.as_str(), cmd.message.as_str(), cmd.mode.as_str()),
            ("add_session_rule", "/tmp/**", "allow")
        );

        let cmd = approval_decision_command(
            "appr-1".to_string(),
            "approved".to_string(),
            "note".to_string(),
            "sess".to_string(),
        );
        assert_eq!(
            (
                cmd.r#type.as_str(),
                cmd.entry_id.as_str(),
                cmd.mode.as_str(),
                cmd.message.as_str()
            ),
            ("approval_decision", "appr-1", "approved", "note")
        );
    }

    #[test]
    fn prompt_command_maps_attachments_and_run_identity() {
        let cmd = prompt_command(
            "hello".to_string(),
            "model-only context".to_string(),
            "sess".to_string(),
            vec![
                AttachmentInput {
                    path: "/tmp/a.png".to_string(),
                    kind: "image".to_string(),
                    name: "a.png".to_string(),
                    thumbnail: Some("/tmp/thumb.png".to_string()),
                },
                AttachmentInput {
                    path: "/tmp/b.txt".to_string(),
                    kind: "file".to_string(),
                    name: "b.txt".to_string(),
                    thumbnail: None,
                },
            ],
            Some("run-1".to_string()),
        );
        assert_eq!(cmd.r#type, "prompt");
        assert_eq!(cmd.message, "hello");
        assert_eq!(cmd.model_context, "model-only context");
        assert_eq!(cmd.requested_run_id, "run-1");
        assert!(cmd.client_request_id.starts_with("desktop_"));
        assert_eq!(cmd.attachments.len(), 2);
        assert_eq!(cmd.attachments[0].thumbnail, "/tmp/thumb.png");
        assert_eq!(
            cmd.attachments[1].thumbnail, "",
            "absent thumbnail defaults"
        );

        let bare = prompt_command(
            "m".to_string(),
            String::new(),
            "s".to_string(),
            vec![],
            None,
        );
        assert_eq!(bare.requested_run_id, "");
    }

    #[test]
    fn list_builtin_providers_command_sets_the_flag() {
        let cmd = list_builtin_providers_command();
        assert_eq!(cmd.r#type, "list_models");
        assert!(cmd.include_builtin_providers);
    }

    #[test]
    fn run_control_command_carries_optional_run_id() {
        let cmd = run_control_command("abort", "sess".to_string(), Some("run-1".to_string()));
        assert_eq!(
            (cmd.r#type.as_str(), cmd.run_id.as_str()),
            ("abort", "run-1")
        );
        let bare = run_control_command("abort", "sess".to_string(), None);
        assert_eq!(bare.run_id, "");
    }

    #[test]
    fn command_ids_are_unique_within_a_millisecond() {
        let first = command_id();
        let second = command_id();
        assert_ne!(first, second, "monotonic sequence separates same-ms ids");
        assert!(first.starts_with("desktop_"));
    }
}

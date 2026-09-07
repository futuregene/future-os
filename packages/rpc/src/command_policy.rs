//! Shared per-command execution policy for every Future Agent RPC client.
//!
//! Transport connection establishment is bounded separately. These deadlines
//! cover only one `ExecuteCommand` request; event streams deliberately do not
//! use this policy because they are governed by handshake and idle watchdogs.

use std::time::Duration;

use tonic::Request;

use crate::proto::RpcCommand;

pub const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
pub const FAST_TIMEOUT: Duration = Duration::from_secs(10);
pub const PROMPT_ACK_TIMEOUT: Duration = Duration::from_secs(15);
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(20);
pub const STORAGE_TIMEOUT: Duration = Duration::from_secs(30);
pub const SHELL_EXECUTION_TIMEOUT: Duration = Duration::from_secs(120);
pub const SHELL_RPC_TIMEOUT: Duration = Duration::from_secs(125);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPolicy {
    /// A read-only request may be retried once within the caller's operation budget.
    SafeRead,
    /// Retry only with the exact same request id/idempotency key.
    SameRequestId,
    /// A timeout leaves the mutation outcome unknown; never retry automatically.
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionKind {
    Immediate,
    Bounded,
    AsyncAck,
    ManagedProcess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandPolicy {
    pub timeout: Duration,
    pub retry: RetryPolicy,
    pub execution: ExecutionKind,
}

const fn policy(timeout: Duration, retry: RetryPolicy, execution: ExecutionKind) -> CommandPolicy {
    CommandPolicy {
        timeout,
        retry,
        execution,
    }
}

/// Every command accepted by the Agent dispatcher. Keep the policy match and
/// its exhaustiveness test aligned with this list when adding a command.
pub const KNOWN_COMMANDS: &[&str] = &[
    "abort",
    "abort_retry",
    "abort_session",
    "add_session_rule",
    "append_system_prompt",
    "approval_decision",
    "cancel_queued_run",
    "clone",
    "compact",
    "cycle_model",
    "cycle_thinking_level",
    "delete_provider",
    "delete_session",
    "disable_builtin_tools",
    "disable_tools",
    "export_html",
    "fork",
    "get_agent_info",
    "get_commands",
    "get_events_since",
    "get_fork_messages",
    "get_last_assistant_text",
    "get_messages",
    "get_runtime_metrics",
    "get_session_entries",
    "get_session_events_since",
    "get_session_stats",
    "get_state",
    "list_models",
    "list_providers",
    "list_session_ids",
    "list_sessions",
    "list_streaming_sessions",
    "new_session",
    "probe_sandbox",
    "probe_windows_sandbox",
    "prompt",
    "prune_run_events",
    "refresh_skills",
    "reload_auth",
    "reload_config",
    "reset_windows_sandbox",
    "retry_persistence",
    "set_auth",
    "set_auto_compaction",
    "set_auto_retry",
    "set_cwd",
    "set_default_model",
    "set_enabled_models",
    "set_ephemeral",
    "set_model",
    "set_permission_level",
    "set_sandbox_policy",
    "set_session_name",
    "set_system_prompt",
    "set_thinking_level",
    "set_tools",
    "shell",
    "shutdown",
    "switch_session",
    "sync_future_models",
    "upsert_provider",
];

pub fn command_policy(command: &str) -> Option<CommandPolicy> {
    let value = match command {
        "abort" | "abort_retry" | "abort_session" | "approval_decision" | "cancel_queued_run"
        | "shutdown" => policy(
            CONTROL_TIMEOUT,
            RetryPolicy::Never,
            ExecutionKind::Immediate,
        ),
        "compact" => policy(
            CONTROL_TIMEOUT,
            RetryPolicy::SameRequestId,
            ExecutionKind::AsyncAck,
        ),
        "get_agent_info"
        | "get_commands"
        | "get_last_assistant_text"
        | "get_runtime_metrics"
        | "get_session_stats"
        | "get_state"
        | "list_models"
        | "list_providers"
        | "list_streaming_sessions" => policy(
            FAST_TIMEOUT,
            RetryPolicy::SafeRead,
            ExecutionKind::Immediate,
        ),
        "prompt" => policy(
            PROMPT_ACK_TIMEOUT,
            RetryPolicy::SameRequestId,
            ExecutionKind::Bounded,
        ),
        "sync_future_models" => policy(NETWORK_TIMEOUT, RetryPolicy::Never, ExecutionKind::Bounded),
        "shell" => policy(
            SHELL_RPC_TIMEOUT,
            RetryPolicy::Never,
            ExecutionKind::ManagedProcess,
        ),
        "get_events_since"
        | "get_fork_messages"
        | "get_messages"
        | "get_session_entries"
        | "get_session_events_since"
        | "list_session_ids"
        | "list_sessions"
        | "probe_sandbox"
        | "probe_windows_sandbox" => policy(
            STORAGE_TIMEOUT,
            RetryPolicy::SafeRead,
            ExecutionKind::Bounded,
        ),
        "add_session_rule"
        | "append_system_prompt"
        | "clone"
        | "cycle_model"
        | "cycle_thinking_level"
        | "delete_provider"
        | "delete_session"
        | "disable_builtin_tools"
        | "disable_tools"
        | "export_html"
        | "fork"
        | "new_session"
        | "prune_run_events"
        | "refresh_skills"
        | "reload_auth"
        | "reload_config"
        | "reset_windows_sandbox"
        | "retry_persistence"
        | "set_auth"
        | "set_auto_compaction"
        | "set_auto_retry"
        | "set_cwd"
        | "set_default_model"
        | "set_enabled_models"
        | "set_ephemeral"
        | "set_model"
        | "set_permission_level"
        | "set_sandbox_policy"
        | "set_session_name"
        | "set_system_prompt"
        | "set_thinking_level"
        | "set_tools"
        | "switch_session"
        | "upsert_provider" => policy(STORAGE_TIMEOUT, RetryPolicy::Never, ExecutionKind::Bounded),
        _ => return None,
    };
    Some(value)
}

/// Wrap one unary command with its command-specific gRPC deadline.
pub fn request_with_timeout(command: RpcCommand) -> Request<RpcCommand> {
    let timeout = command_policy(&command.r#type)
        .map(|policy| policy.timeout)
        // Unknown commands are rejected immediately by the Agent. Keep a
        // finite boundary for malformed/out-of-tree callers nonetheless.
        .unwrap_or(FAST_TIMEOUT);
    let mut request = Request::new(command);
    request.set_timeout(timeout);
    request
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declared_command_has_an_explicit_policy() {
        for command in KNOWN_COMMANDS {
            assert!(
                command_policy(command).is_some(),
                "missing policy for {command}"
            );
        }
    }

    #[test]
    fn long_work_is_not_mistaken_for_a_long_unary_rpc() {
        assert_eq!(command_policy("compact").unwrap().timeout, CONTROL_TIMEOUT);
        assert_eq!(command_policy("shell").unwrap().timeout, SHELL_RPC_TIMEOUT);
        assert!(command_policy("unknown").is_none());
    }

    #[test]
    fn request_deadline_is_encoded_in_grpc_metadata() {
        let request = request_with_timeout(RpcCommand {
            r#type: "compact".to_string(),
            ..Default::default()
        });
        assert!(request.metadata().contains_key("grpc-timeout"));
    }
}

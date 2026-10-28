//! Existing host RPC construction; wire messages remain owned by future-rpc.
use future_rpc::proto::{Attachment, RpcCommand, RpcResponse};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Turn a gRPC `RpcResponse` into a `Result`, surfacing the agent's own error
/// message, or `fallback` when the agent reported failure without one.
pub trait RpcResponseExt {
    fn ok_or_rpc_error(self, fallback: &str) -> Result<RpcResponse, crate::Error>;
}

impl RpcResponseExt for RpcResponse {
    fn ok_or_rpc_error(self, fallback: &str) -> Result<RpcResponse, crate::Error> {
        if self.success {
            Ok(self)
        } else if self.error.is_empty() {
            Err(fallback.to_string().into())
        } else {
            let code = if is_session_not_found(&self) {
                future_rpc::session_errors::SESSION_NOT_FOUND_CODE.into()
            } else {
                self.error_code
            };
            Err(crate::Error::AgentCommand {
                code,
                message: self.error,
            })
        }
    }
}

/// Whether a failed `RpcResponse` is the Agent's definitive answer that it has
/// no such session (`future_rpc::session_errors`), as opposed to a transient
/// failure worth retrying.
///
/// A caller acts on it as "there is nothing there": no session-scoped command
/// can succeed for that id, and neither can waiting for one to answer. The
/// stable error code is matched first, with the pre-code wording as a fallback
/// so an older Agent is still understood.
pub fn is_session_not_found(response: &RpcResponse) -> bool {
    !response.success && is_session_not_found_error(&response.error_code, &response.error)
}

pub fn is_session_not_found_error(code: &str, message: &str) -> bool {
    code == future_rpc::session_errors::SESSION_NOT_FOUND_CODE
        || (code.is_empty()
            && future_rpc::session_errors::legacy_session_not_found_message(message))
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

pub fn fork_command(
    session_id: String,
    entry_id: String,
    parent_session: String,
    creator_id: String,
    request_id: String,
) -> RpcCommand {
    RpcCommand {
        entry_id,
        parent_session,
        mode: "through_turn".to_string(),
        client_request_id: request_id,
        // Declare the GUI as the forking client: the agent propagates this to
        // the forked session's provenance, so the session_created push skips
        // it (the GUI creates its own thread row for the fork).
        created_by: "desktop".to_string(),
        creator_id,
        ..base_command("fork", session_id)
    }
}

#[cfg(test)]
fn delete_session_command(session_id: String) -> RpcCommand {
    base_command("delete_session", session_id)
}

pub fn delete_sessions_command(session_ids: Vec<String>) -> RpcCommand {
    RpcCommand {
        session_ids,
        ..base_command("delete_sessions", String::new())
    }
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

pub fn get_session_entries_before_command(
    session_id: String,
    before: i64,
    user_exchange_limit: i64,
) -> RpcCommand {
    RpcCommand {
        before: Some(before),
        limit: Some(user_exchange_limit),
        ..base_command("get_session_entries", session_id)
    }
}

pub fn new_session_command(
    session_id: String,
    cwd: String,
    created_by: &str,
    creator_id: String,
    source_meta: serde_json::Value,
    model_id: Option<String>,
    thinking_level: Option<String>,
) -> RpcCommand {
    RpcCommand {
        cwd,
        // Typed provenance fields (proto created_by/source_meta) — no longer
        // smuggled through custom_instructions, which belongs to compact.
        created_by: created_by.to_string(),
        creator_id,
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

pub fn set_permission_level_command(level: String, session_id: String) -> RpcCommand {
    RpcCommand {
        level,
        ..base_command("set_permission_level", session_id)
    }
}

pub fn set_sandbox_policy_command(
    policy: future_rpc::proto::SandboxPolicy,
    session_id: String,
) -> RpcCommand {
    RpcCommand {
        sandbox_policy: Some(policy),
        ..base_command("set_sandbox_policy", session_id)
    }
}

/// Same-run "allow in this workspace/chat" — message = path glob, mode = access.
pub fn add_session_rule_command(path: String, access: String, session_id: String) -> RpcCommand {
    RpcCommand {
        message: path,
        mode: access,
        ..base_command("add_session_rule", session_id)
    }
}

/// A file attached to a prompt, as passed from the frontend. Files are
/// referenced by their original absolute path — never copied. Images carry no
/// data here; only attachment paths cross RPC. The agent reads and encodes
/// image bytes as data URIs when it constructs the model request.
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

pub fn prompt_command(
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

pub fn approval_decision_command(
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

pub fn base_command(command_type: &str, session_id: String) -> RpcCommand {
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
        creator_id: String::new(),
        source_meta: String::new(),
        enabled: false,
        command: String::new(),
        shell_timeout_ms: 0,
        session_id,
        session_ids: Vec::new(),
        entry_id: String::new(),
        tool_call_id: None,
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
        before: None,
        fragment_offset: None,
        content_request: None,
        replay_until_idx: None,
        // Cross-session history search bound. The GUI has no UI for it yet, so
        // it stays absent (the Agent then applies its own default scan window).
        max_sessions: None,
        requested_run_id: String::new(),
        client_request_id: String::new(),
        busy_policy: String::new(),
        sandbox_policy: None,
        include_builtin_providers: false,
        auth_update: None,
        provider_config: None,
        suggest_query: String::new(),
        suggest_candidates: vec![],
        skill_id: String::new(),
        skill_version: String::new(),
        // In-place history reset selectors: only `future session reset` sends
        // them, so a desktop-issued command leaves them absent (the Agent then
        // rejects the command for having no selector at all).
        reset_history: None,
    }
}

/// `list_models` variant that also asks for the agent's built-in provider
/// catalog summary (`builtinProviders`), so the Providers page can source the
/// catalog from the agent at runtime instead of compiling agent source in.
pub fn list_builtin_providers_command() -> RpcCommand {
    RpcCommand {
        include_builtin_providers: true,
        ..base_command("list_models", String::new())
    }
}

pub fn run_control_command(
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
    use super::*;
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
    fn is_session_not_found_matches_the_code_then_the_legacy_wording() {
        let failed = |code: &str, error: &str| RpcResponse {
            success: false,
            error_code: code.to_string(),
            error: error.to_string(),
            ..Default::default()
        };

        // The stable code, as the agent sends it, whatever the wording.
        assert!(is_session_not_found(&failed(
            future_rpc::session_errors::SESSION_NOT_FOUND_CODE,
            "session not found — pass a valid session_id (new_session creates one)"
        )));
        // An older agent reports the same condition in the message only.
        assert!(is_session_not_found(&failed("", "session not found")));
        assert!(is_session_not_found(&failed("", "No such session")));
        // A transient failure keeps its retry meaning.
        assert!(!is_session_not_found(&failed(
            "",
            "session_storage_unavailable"
        )));
        assert!(!is_session_not_found(&failed(
            "session_busy",
            "session context compaction is in progress"
        )));
        // Structured storage failures remain errors even when their detail
        // quotes the legacy missing-session wording.
        assert!(!is_session_not_found(&failed(
            "session_storage_unavailable",
            "unable to load session: session not found in recovery archive"
        )));
        // A successful response is never the failure answer, even if a handler
        // echoes the phrasing back.
        assert!(!is_session_not_found(&RpcResponse {
            success: true,
            error: "session not found".to_string(),
            ..Default::default()
        }));
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
            "desktop-test".to_string(),
            "fork-request".to_string(),
        );
        assert_eq!(cmd.r#type, "fork");
        assert_eq!(cmd.entry_id, "entry-1");
        assert_eq!(cmd.parent_session, "parent");
        assert_eq!(cmd.creator_id, "desktop-test");
        assert_eq!(cmd.mode, "through_turn");
        assert_eq!(cmd.client_request_id, "fork-request");

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
        let command = get_session_entries_before_command("sess".to_string(), i64::MAX, 10);
        assert_eq!(command.r#type, "get_session_entries");
        assert_eq!(command.before, Some(i64::MAX));
        assert_eq!(command.limit, Some(10));
    }

    #[test]
    fn new_session_command_carries_provenance_and_model() {
        let cmd = new_session_command(
            "sess".to_string(),
            "/tmp/ws".to_string(),
            "desktop",
            "desktop-test".to_string(),
            serde_json::json!({"source": "test"}),
            Some("future/k3".to_string()),
            Some("high".to_string()),
        );
        assert_eq!(cmd.r#type, "new_session");
        assert_eq!(cmd.cwd, "/tmp/ws");
        assert_eq!(cmd.created_by, "desktop");
        assert_eq!(cmd.creator_id, "desktop-test");
        assert_eq!(cmd.source_meta, r#"{"source":"test"}"#);
        assert_eq!(cmd.model_id, "future/k3");
        assert_eq!(cmd.level, "high");

        let bare = new_session_command(
            String::new(),
            String::new(),
            "desktop",
            String::new(),
            serde_json::Value::Null,
            None,
            None,
        );
        assert_eq!(bare.model_id, "");
        assert_eq!(bare.level, "");
        // A session created without a name stays unnamed: the Agent titles it
        // from its first message, which is what every chat conversation wants.
        // Task runs set the name on the command themselves (see
        // `agent_bridge::session::ensure_named_agent_session`).
        assert_eq!(bare.name, "");
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

        let policy = future_rpc::proto::SandboxPolicy {
            tier: "sandbox".to_string(),
            reviewer: String::new(),
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
        assert!(
            cmd.images.is_empty(),
            "desktop never sends raw Base64 image content"
        );
        assert_eq!(cmd.attachments[0].path, "/tmp/a.png");
        assert_eq!(cmd.attachments[0].kind, "image");
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

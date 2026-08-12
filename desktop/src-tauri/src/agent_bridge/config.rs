//! RPC-first provider/auth config writes (audit item 2).
//!
//! Instead of editing `~/.future/agent/auth.json` / `models.json` directly and
//! then patching the agent's in-memory state with `reload_auth`, the GUI asks
//! the agent to write its own config via `set_auth` / `upsert_provider` /
//! `delete_provider`. The agent applies the change and refreshes its registry
//! and live-session credentials internally, so no follow-up round-trip is
//! needed on this path.
//!
//! Fallback keeps version skew and offline edits working: an unreachable
//! agent, or one that predates these commands (it answers `success = false`
//! with "unknown command"), sends the caller back to the GUI's own file
//! writers plus the best-effort `reload_auth` follow-up — exactly the legacy
//! behavior. An explicit rejection from an agent that DOES know the command
//! (validation failure, e.g. a duplicate provider id) is surfaced as an error
//! instead, never silently re-applied locally.

use crate::agent_proto::{AuthUpdate, RpcCommand};
use crate::AppError;

use super::client::{base_command, connect_agent};

/// Send a config-write command to the agent.
///
/// Returns `Ok(true)` when the agent applied the change (its live state is
/// already refreshed), `Ok(false)` when the caller must fall back to the local
/// file writers (agent unreachable or too old), and `Err` when the agent
/// explicitly rejected the change.
async fn send_config_write(command: RpcCommand, context: &str) -> Result<bool, AppError> {
    let mut client = match connect_agent().await {
        Ok(client) => client,
        Err(error) => {
            eprintln!(
                "FutureOS: agent unavailable for {context} ({error}); falling back to the local config write"
            );
            return Ok(false);
        }
    };
    let response = match client.execute_command(command).await {
        Ok(response) => response.into_inner(),
        Err(status) => {
            eprintln!(
                "FutureOS: {context} failed at transport level ({status}); falling back to the local config write"
            );
            return Ok(false);
        }
    };
    if response.success {
        return Ok(true);
    }
    // A pre-item-2 agent answers success=false "unknown command: <type>".
    // Treat it like an unavailable agent and let the caller fall back.
    if response.error.contains("unknown command") {
        eprintln!(
            "FutureOS: agent does not support {context}; falling back to the local config write"
        );
        return Ok(false);
    }
    let message = if response.error.is_empty() {
        format!("Future Agent rejected {context}.")
    } else {
        response.error
    };
    Err(message.into())
}

fn set_auth_command(update: AuthUpdate) -> RpcCommand {
    RpcCommand {
        auth_update: Some(update),
        ..base_command("set_auth", String::new())
    }
}

/// Set a provider's API key (`auth.json` entry, preserving other fields).
pub(crate) async fn set_provider_key(id: &str, key: &str) -> Result<bool, AppError> {
    send_config_write(
        set_auth_command(AuthUpdate {
            provider: id.to_string(),
            key: key.to_string(),
            ..Default::default()
        }),
        "set_auth",
    )
    .await
}

/// Remove a provider's API key, keeping the rest of its entry.
pub(crate) async fn clear_provider_key(id: &str) -> Result<bool, AppError> {
    send_config_write(
        set_auth_command(AuthUpdate {
            provider: id.to_string(),
            clear_key: true,
            ..Default::default()
        }),
        "set_auth",
    )
    .await
}

/// FutureGene login: store the device-flow key and pin `base_url` (mirrors the
/// CLI's `saveAuth` shape, applied by the agent).
pub(crate) async fn future_login(key: &str, base_url: &str) -> Result<bool, AppError> {
    send_config_write(
        set_auth_command(AuthUpdate {
            provider: crate::auth_store::FUTURE_PROVIDER_ID.to_string(),
            key: key.to_string(),
            base_url: base_url.to_string(),
            ..Default::default()
        }),
        "set_auth",
    )
    .await
}

/// FutureGene logout: drop the key, keep `base_url`.
pub(crate) async fn future_logout() -> Result<bool, AppError> {
    send_config_write(
        set_auth_command(AuthUpdate {
            provider: crate::auth_store::FUTURE_PROVIDER_ID.to_string(),
            clear_key: true,
            ..Default::default()
        }),
        "set_auth",
    )
    .await
}

/// Set (or clear, when `base_url` is empty) a built-in provider's Base URL
/// override in `models.json`.
pub(crate) async fn set_builtin_provider_base_url(
    id: &str,
    base_url: &str,
) -> Result<bool, AppError> {
    send_config_write(
        RpcCommand {
            provider_config: Some(crate::agent_proto::ProviderUpsert {
                id: id.to_string(),
                base_url: base_url.to_string(),
                clear_base_url: base_url.is_empty(),
                ..Default::default()
            }),
            ..base_command("upsert_provider", String::new())
        },
        "upsert_provider",
    )
    .await
}

/// Create/update a `models.json` provider entry (plus optional auth key).
pub(crate) async fn upsert_provider(
    config: crate::agent_proto::ProviderUpsert,
) -> Result<bool, AppError> {
    send_config_write(
        RpcCommand {
            provider_config: Some(config),
            ..base_command("upsert_provider", String::new())
        },
        "upsert_provider",
    )
    .await
}

/// Remove a provider's `models.json` entry AND its `auth.json` entry.
pub(crate) async fn delete_provider(id: &str) -> Result<bool, AppError> {
    send_config_write(
        RpcCommand {
            provider_config: Some(crate::agent_proto::ProviderUpsert {
                id: id.to_string(),
                ..Default::default()
            }),
            ..base_command("delete_provider", String::new())
        },
        "delete_provider",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{mock_agent, Reply};
    use super::*;

    /// Run one closure with a deliberately unparseable agent endpoint so
    /// `connect_agent` fails, then restore the mock's address.
    async fn with_broken_endpoint<F: std::future::Future<Output = Result<bool, AppError>>>(
        call: impl FnOnce() -> F,
    ) -> Result<bool, AppError> {
        let prev = std::env::var("FUTURE_AGENT_GRPC_ADDR").ok();
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let result = call().await;
        if let Some(prev) = prev {
            std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        }
        result
    }

    #[tokio::test]
    async fn applied_rejected_and_fallback_outcomes() {
        let mock = mock_agent();

        // Agent applies the change.
        mock.push("set_auth", Reply::Data("{}".to_string()));
        assert!(set_provider_key("future", "sk-1").await.expect("ok"));
        let request = &mock.requests_of("set_auth")[0];
        let update = request.auth_update.as_ref().expect("auth update");
        assert_eq!(update.provider, "future");
        assert_eq!(update.key, "sk-1");
        assert!(!update.clear_key);

        // Agent knows the command but rejects it (validation failure).
        mock.push(
            "set_auth",
            Reply::Reject("duplicate provider id".to_string()),
        );
        let error = set_provider_key("future", "sk-1")
            .await
            .expect_err("rejected");
        assert_eq!(error.to_string(), "duplicate provider id");

        // Rejection without a message gets a synthesized one.
        mock.push("set_auth", Reply::Reject(String::new()));
        let error = set_provider_key("future", "sk-1")
            .await
            .expect_err("rejected");
        assert_eq!(error.to_string(), "Future Agent rejected set_auth.");

        // Legacy agent: "unknown command" → caller falls back (Ok(false)).
        mock.push(
            "set_auth",
            Reply::Reject("unknown command: set_auth".to_string()),
        );
        assert!(!set_provider_key("future", "sk-1").await.expect("fallback"));

        // Transport-level failure → fall back too.
        mock.push(
            "set_auth",
            Reply::Status(tonic::Code::Unavailable, "connection reset"),
        );
        assert!(!set_provider_key("future", "sk-1").await.expect("fallback"));

        // Agent unreachable at connect time → fall back.
        let applied = with_broken_endpoint(|| set_provider_key("future", "sk-1"))
            .await
            .expect("fallback");
        assert!(!applied);
    }

    #[tokio::test]
    async fn auth_update_variants() {
        let mock = mock_agent();

        mock.push("set_auth", Reply::Data("{}".to_string()));
        assert!(clear_provider_key("future").await.expect("ok"));
        let update = mock.requests_of("set_auth")[0]
            .auth_update
            .clone()
            .expect("auth update");
        assert!(update.clear_key);
        assert_eq!(update.key, "");

        mock.push("set_auth", Reply::Data("{}".to_string()));
        assert!(future_login("fg-key", "https://api.example")
            .await
            .expect("ok"));
        let update = mock.requests_of("set_auth")[1]
            .auth_update
            .clone()
            .expect("auth update");
        assert_eq!(update.provider, crate::auth_store::FUTURE_PROVIDER_ID);
        assert_eq!(update.key, "fg-key");
        assert_eq!(update.base_url, "https://api.example");

        mock.push("set_auth", Reply::Data("{}".to_string()));
        assert!(future_logout().await.expect("ok"));
        let update = mock.requests_of("set_auth")[2]
            .auth_update
            .clone()
            .expect("auth update");
        assert!(update.clear_key);
        assert_eq!(update.provider, crate::auth_store::FUTURE_PROVIDER_ID);
    }

    #[tokio::test]
    async fn provider_config_commands() {
        let mock = mock_agent();

        // Base URL override set.
        mock.push("upsert_provider", Reply::Data("{}".to_string()));
        assert!(
            set_builtin_provider_base_url("future", "https://alt.example")
                .await
                .expect("ok")
        );
        let config = mock.requests_of("upsert_provider")[0]
            .provider_config
            .clone()
            .expect("provider config");
        assert_eq!(config.id, "future");
        assert_eq!(config.base_url, "https://alt.example");
        assert!(!config.clear_base_url);

        // Empty base URL clears the override.
        mock.push("upsert_provider", Reply::Data("{}".to_string()));
        assert!(set_builtin_provider_base_url("future", "")
            .await
            .expect("ok"));
        let config = mock.requests_of("upsert_provider")[1]
            .provider_config
            .clone()
            .expect("provider config");
        assert!(config.clear_base_url);

        // Full upsert.
        mock.push("upsert_provider", Reply::Data("{}".to_string()));
        let upsert = crate::agent_proto::ProviderUpsert {
            id: "custom".to_string(),
            name: "Custom".to_string(),
            ..Default::default()
        };
        assert!(upsert_provider(upsert).await.expect("ok"));
        let config = mock.requests_of("upsert_provider")[2]
            .provider_config
            .clone()
            .expect("provider config");
        assert_eq!(config.name, "Custom");

        // Delete.
        mock.push("delete_provider", Reply::Data("{}".to_string()));
        assert!(delete_provider("custom").await.expect("ok"));
        let config = mock.requests_of("delete_provider")[0]
            .provider_config
            .clone()
            .expect("provider config");
        assert_eq!(config.id, "custom");
    }
}

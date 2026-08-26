//! RPC-first provider/auth config writes (audit item 2).
//!
//! Instead of editing `~/.future/agent/auth.json` / `models.json` directly and
//! then patching the agent's in-memory state with `reload_auth`, the GUI asks
//! the agent to write its own config via `set_auth` / `upsert_provider` /
//! `delete_provider`. The agent applies the change and refreshes its registry
//! and live-session credentials internally, so no follow-up round-trip is
//! needed on this path.
//!
//! There is deliberately no local-write fallback. A successful UI response
//! means the Agent durably committed the mutation and refreshed live state;
//! an unavailable or outdated Agent is an error. This keeps every client and
//! every conversation on one authoritative configuration revision.

use crate::agent_proto::{AuthUpdate, RpcCommand};
use crate::AppError;

use super::client::{base_command, connect_agent};

/// Read the Agent-owned, secret-redacted provider snapshot. This deliberately
/// does not inspect Desktop's filesystem, so a custom Agent endpoint or HOME
/// cannot make the displayed state diverge from the process serving prompts.
pub(crate) async fn list_providers() -> Result<crate::agent_providers::ProvidersView, AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(base_command("list_providers", String::new()))
        .await
        .map_err(|status| {
            AppError::Message(format!("Unable to list Future Agent providers: {status}"))
        })?
        .into_inner();
    if !response.success {
        return Err(if response.error.is_empty() {
            "Future Agent rejected the provider snapshot request."
                .to_string()
                .into()
        } else {
            response.error.into()
        });
    }
    serde_json::from_str(&response.data).map_err(|error| {
        format!("Future Agent returned an invalid provider snapshot: {error}").into()
    })
}

/// Send a config-write command to the agent.
///
/// Returns only after the Agent applied the change and refreshed live state.
async fn send_config_write(command: RpcCommand, context: &str) -> Result<bool, AppError> {
    let mut client = connect_agent().await.map_err(|error| {
        AppError::Message(format!(
            "Future Agent is unavailable; {context} was not saved: {error}"
        ))
    })?;
    let response = client
        .execute_command(command)
        .await
        .map_err(|status| {
            AppError::Message(format!(
                "Future Agent did not complete {context}; nothing was saved: {status}"
            ))
        })?
        .into_inner();
    if response.success {
        return Ok(true);
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

/// Atomically update a built-in provider's optional Base URL and API key.
pub(crate) async fn update_builtin_provider(
    id: &str,
    base_url: Option<&str>,
    api_key: Option<Option<&str>>,
) -> Result<bool, AppError> {
    let mut config = crate::agent_proto::ProviderUpsert {
        id: id.to_string(),
        ..Default::default()
    };
    if let Some(base_url) = base_url {
        config.base_url = base_url.to_string();
        config.clear_base_url = base_url.is_empty();
    }
    if let Some(api_key) = api_key {
        match api_key {
            Some(key) => config.api_key = key.to_string(),
            None => config.clear_api_key = true,
        }
    }
    upsert_provider(config).await
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
    /// `connect_agent` fails, then restore the mock's address. Every caller
    /// holds the mock guard first, so the endpoint env var is always set.
    async fn with_broken_endpoint<F: std::future::Future<Output = Result<bool, AppError>>>(
        call: impl FnOnce() -> F,
    ) -> Result<bool, AppError> {
        let prev =
            std::env::var("FUTURE_AGENT_GRPC_ADDR").expect("mock guard sets the agent endpoint");
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let result = call().await;
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", prev);
        result
    }

    #[tokio::test]
    async fn applied_and_rejected_outcomes_never_fall_back() {
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

        // A legacy Agent cannot become a second authority via local fallback.
        mock.push(
            "set_auth",
            Reply::Reject("unknown command: set_auth".to_string()),
        );
        assert!(set_provider_key("future", "sk-1").await.is_err());

        // Transport failure means nothing is reported as saved.
        mock.push(
            "set_auth",
            Reply::Status(tonic::Code::Unavailable, "connection reset"),
        );
        let error = set_provider_key("future", "sk-1").await.unwrap_err();
        assert!(error.to_string().contains("nothing was saved"));

        // Connect failure also refuses an out-of-band local write.
        let error = with_broken_endpoint(|| set_provider_key("future", "sk-1"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("was not saved"));
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

        // Built-in Base URL + key are one atomic Agent upsert, not two
        // independently observable mutations.
        mock.push("upsert_provider", Reply::Data("{}".to_string()));
        assert!(update_builtin_provider(
            "azure-openai-responses",
            Some("https://tenant.openai.azure.com/openai"),
            Some(Some("azure-key")),
        )
        .await
        .expect("ok"));
        let requests = mock.requests_of("upsert_provider");
        let config = requests.last().unwrap().provider_config.as_ref().unwrap();
        assert_eq!(config.base_url, "https://tenant.openai.azure.com/openai");
        assert_eq!(config.api_key, "azure-key");
        assert!(!config.clear_api_key);

        mock.push("upsert_provider", Reply::Data("{}".to_string()));
        assert!(update_builtin_provider(
            "azure-openai-responses",
            Some("https://other.openai.azure.com/openai"),
            Some(None),
        )
        .await
        .expect("ok"));
        let requests = mock.requests_of("upsert_provider");
        let config = requests.last().unwrap().provider_config.as_ref().unwrap();
        assert_eq!(config.base_url, "https://other.openai.azure.com/openai");
        assert!(config.api_key.is_empty());
        assert!(config.clear_api_key);
    }

    #[tokio::test]
    async fn update_builtin_provider_skips_api_key_when_none() {
        let mock = mock_agent();
        mock.push("upsert_provider", Reply::Data("{}".to_string()));
        assert!(
            update_builtin_provider("future", Some("https://x.example"), None)
                .await
                .expect("ok")
        );
        let config = mock.requests_of("upsert_provider")[0]
            .provider_config
            .clone()
            .expect("provider config");
        assert_eq!(config.base_url, "https://x.example");
        assert!(config.api_key.is_empty());
        assert!(!config.clear_api_key);
    }

    #[tokio::test]
    async fn list_providers_error_paths() {
        let mock = mock_agent();

        // Transport-level failure surfaces through the map_err arm.
        mock.push(
            "list_providers",
            Reply::Status(tonic::Code::Unavailable, "agent down"),
        );
        let error = list_providers().await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Unable to list Future Agent providers"),
            "error: {error}"
        );

        // Rejection without a message gets a synthesized one.
        mock.push("list_providers", Reply::Reject(String::new()));
        let error = list_providers().await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "Future Agent rejected the provider snapshot request."
        );

        // Rejection with a message passes it through.
        mock.push(
            "list_providers",
            Reply::Reject("provider table corrupt".to_string()),
        );
        let error = list_providers().await.unwrap_err();
        assert_eq!(error.to_string(), "provider table corrupt");

        // A successful response that isn't valid provider JSON maps to a
        // deserialization error.
        mock.push("list_providers", Reply::Data("not-json".to_string()));
        let error = list_providers().await.unwrap_err();
        assert!(
            error.to_string().contains("invalid provider snapshot"),
            "error: {error}"
        );
    }
}

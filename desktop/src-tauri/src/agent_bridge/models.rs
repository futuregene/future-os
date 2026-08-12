//! Model catalogue lookup: asks the agent for its available models.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::client::{
    base_command, connect_agent, list_builtin_providers_command, map_rpc_error, RpcResponseExt,
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelOption {
    id: String,
    label: String,
    provider: String,
    #[serde(default)]
    supports_images: bool,
    #[serde(default)]
    thinking_level: Option<String>,
    #[serde(default)]
    context_window: Option<i32>,
    #[serde(default)]
    is_default: bool,
    /// Curated positioning blurb from the Future platform catalog (e.g.
    /// "经济实用版，日常编程和对话任务够用且实惠"). Absent for built-in / user models.
    #[serde(default)]
    description: Option<String>,
    /// English counterpart of `description` (e.g. "Budget-friendly edition, solid
    /// for daily coding and chat"); the GUI shows it when the UI language is not
    /// Chinese. Absent for built-in / user models.
    #[serde(default)]
    description_en: Option<String>,
    /// Future-platform recommendation flag (drives the onboarding model picker).
    #[serde(default)]
    recommended: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentModelsResponse {
    models: Vec<AgentModelOption>,
}

pub async fn list_agent_models() -> Result<Vec<AgentModelOption>, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(base_command("list_models", String::new()))
        .await
        .map_err(|status| map_rpc_error("Unable to load Future Agent models", status))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the model list request.")?;

    let parsed =
        serde_json::from_value::<AgentModelsResponse>(future_rpc::decode::response_data(&response))
            .map_err(|error| format!("Future Agent returned invalid model data: {error}"))?;
    Ok(parsed.models)
}

/// One built-in provider as summarized by the agent's `list_models` response
/// (`builtinProviders` section`): its human-readable display name (the agent is
/// the single source of truth for these — the GUI renders it verbatim), how
/// many catalog models it has, and its catalog base URL (no models.json
/// overrides applied). `name` defaults to empty so a pre-name agent still
/// parses; the caller falls back to the provider id.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinProviderCatalogEntry {
    #[serde(default)]
    pub name: String,
    pub model_count: usize,
    pub base_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuiltinProvidersResponse {
    #[serde(default)]
    builtin_providers: BTreeMap<String, BuiltinProviderCatalogEntry>,
}

/// Fetch the agent's built-in provider catalog via `list_models` with
/// `include_builtin_providers` set. This is the runtime replacement for the
/// former compile-time `#[path]` include of the agent's generated catalog —
/// the agent is the single source of the catalog.
pub async fn list_builtin_providers(
) -> Result<BTreeMap<String, BuiltinProviderCatalogEntry>, crate::AppError> {
    let mut client = connect_agent().await?;
    let response = client
        .execute_command(list_builtin_providers_command())
        .await
        .map_err(|status| map_rpc_error("Unable to load the built-in provider catalog", status))?
        .into_inner()
        .ok_or_rpc_error("Future Agent rejected the provider catalog request.")?;

    let parsed = serde_json::from_value::<BuiltinProvidersResponse>(
        future_rpc::decode::response_data(&response),
    )
    .map_err(|error| format!("Future Agent returned invalid provider catalog data: {error}"))?;
    Ok(parsed.builtin_providers)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{mock_agent, Reply};
    use super::*;

    fn models_payload() -> serde_json::Value {
        serde_json::json!({
            "models": [{
                "id": "future/k3",
                "label": "K3",
                "provider": "future",
                "supportsImages": true,
                "thinkingLevel": "high",
                "contextWindow": 256000,
                "isDefault": true,
                "description": "经济实用版",
                "descriptionEn": "Budget-friendly",
                "recommended": true
            }],
            "defaultModel": "future/k3"
        })
    }

    #[tokio::test]
    async fn list_agent_models_parses_the_catalog() {
        let mock = mock_agent();
        mock.push_data("list_models", models_payload());
        let models = list_agent_models().await.expect("models");
        assert_eq!(models.len(), 1);
        let model = &models[0];
        assert_eq!(model.id, "future/k3");
        assert!(model.supports_images);
        assert_eq!(model.thinking_level.as_deref(), Some("high"));
        assert_eq!(model.context_window, Some(256000));
        assert!(model.is_default);
        assert_eq!(model.description_en.as_deref(), Some("Budget-friendly"));
        assert!(model.recommended);
        assert!(
            !mock.requests_of("list_models")[0].include_builtin_providers,
            "plain model list must not request the provider catalog"
        );
    }

    #[tokio::test]
    async fn list_agent_models_surfaces_rejection_transport_and_parse_errors() {
        let mock = mock_agent();

        mock.push("list_models", Reply::Reject("nope".to_string()));
        let error = list_agent_models().await.expect_err("rejected");
        assert_eq!(error.to_string(), "nope");

        mock.push("list_models",
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        let error = list_agent_models().await.expect_err("transport");
        assert!(
            matches!(error, crate::AppError::AgentUnavailable(_)),
            "transport maps through map_rpc_error: {error}"
        );

        mock.push_data("list_models", serde_json::json!({"models": "not-an-array"}));
        let error = list_agent_models().await.expect_err("invalid data");
        assert!(
            error
                .to_string()
                .contains("Future Agent returned invalid model data"),
            "error: {error}"
        );
    }

    #[tokio::test]
    async fn list_builtin_providers_parses_and_defaults_name() {
        let mock = mock_agent();
        mock.push_data(
            "list_models",
            serde_json::json!({
                "builtinProviders": {
                    "future": {"name": "Future", "modelCount": 3, "baseUrl": "https://api.example"},
                    "legacy": {"modelCount": 1, "baseUrl": "https://old.example"}
                }
            }),
        );
        let providers = list_builtin_providers().await.expect("providers");
        assert_eq!(providers.len(), 2);
        assert_eq!(providers["future"].name, "Future");
        assert_eq!(providers["future"].model_count, 3);
        assert_eq!(
            providers["legacy"].name, "",
            "a pre-name agent leaves the display name empty"
        );
        assert!(
            mock.requests_of("list_models")[0].include_builtin_providers,
            "catalog fetch sets include_builtin_providers"
        );

        mock.push("list_models", Reply::Reject("denied".to_string()));
        let error = list_builtin_providers().await.expect_err("rejected");
        assert_eq!(error.to_string(), "denied");

        mock.push("list_models",
            Reply::Status(tonic::Code::Internal, "boom"),
        );
        let error = list_builtin_providers().await.expect_err("transport");
        assert!(matches!(error, crate::AppError::Message(_)), "{error}");

        mock.push_data(
            "list_models",
            serde_json::json!({"builtinProviders": [1, 2, 3]}),
        );
        let error = list_builtin_providers().await.expect_err("invalid");
        assert!(error.to_string().contains("invalid provider catalog"), "{error}");
    }
}

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
    reasoning: Option<bool>,
    #[serde(default)]
    thinking_level: Option<String>,
    /// The levels this model offers, canonical order (`list_models` sends it
    /// already resolved: `off` alone for a model without thinking controls, the
    /// full six for one that declares nothing). Clients filter their pickers with
    /// it so they never offer a level the agent would clamp away.
    #[serde(default)]
    thinking_levels: Option<Vec<String>>,
    /// The level a conversation starts at on this model, already clamped by the
    /// agent. Clients seed their own default from it instead of carrying a copy
    /// of the snapping rule.
    #[serde(default)]
    default_thinking_level: Option<String>,
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
    /// Listed for everyone, callable only by token-plan subscribers. Only the
    /// Future catalogue sets it; every other provider's models leave it false.
    #[serde(default)]
    requires_subscription: bool,
}

impl AgentModelOption {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// `provider/id`, the reference a client stores and sends back. The id keeps
    /// its own slashes, so this can legitimately contain the provider name twice
    /// (`openrouter` → `openrouter/auto` is `openrouter/openrouter/auto`).
    pub fn reference(&self) -> String {
        if self.provider.is_empty() {
            self.id.clone()
        } else {
            format!("{}/{}", self.provider, self.id)
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentModelsResponse {
    models: Vec<AgentModelOption>,
}

/// Reduce a stored model reference to the one the catalogue names.
///
/// A reference is `provider/id`, and the id is whatever the provider calls the
/// model — slashes included. So a reference cannot be normalized by looking at
/// its string: `openrouter/openrouter/auto` is the *correct* reference for
/// openrouter's `openrouter/auto`, while `omlx/omlx/Qwen3.8-27B-oQ4e-mtp` is a
/// client prefixing an id that already began with its provider. Only the
/// catalogue tells them apart, so this collapses one leading segment at a time
/// and stops at the first level the catalogue recognizes; a level it does not
/// recognize is never guessed at, and a value it cannot place is returned as
/// stored (the Agent's own resolution then applies, as before).
pub fn normalize_model_reference(raw: &str, models: &[AgentModelOption]) -> String {
    let raw = raw.trim();
    if raw.is_empty() || models.is_empty() {
        return raw.to_string();
    }
    if models.iter().any(|model| model.reference() == raw) {
        return raw.to_string();
    }
    let mut candidate = raw.to_string();
    while let Some((provider, rest)) = candidate.split_once('/') {
        // Only a provider this catalogue actually has can be the repeated
        // segment; anything else is part of the model's own id.
        if !models.iter().any(|model| model.provider == provider) {
            break;
        }
        candidate = rest.to_string();
        if models.iter().any(|model| model.reference() == candidate) {
            return candidate;
        }
    }
    raw.to_string()
}

pub async fn list_agent_models() -> Result<Vec<AgentModelOption>, crate::Error> {
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

/// [`normalize_model_reference`] against the Agent's live catalogue.
///
/// A catalogue that cannot be read leaves the value exactly as stored: the run
/// must not fail (or silently switch models) because the picker could not be
/// consulted — the Agent's own resolution is the same fallback it always had.
pub async fn normalize_model_reference_against_catalog(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    match list_agent_models().await {
        Ok(models) => normalize_model_reference(raw, &models),
        Err(error) => {
            tracing::warn!(event = "models", error = %error,
                "FutureOS: could not read the model catalogue to normalize {raw:?}; \
                 using the stored reference: {error}"
            );
            raw.to_string()
        }
    }
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
) -> Result<BTreeMap<String, BuiltinProviderCatalogEntry>, crate::Error> {
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

    /// A catalogue row built the way the Agent sends one: every field private
    /// to this module, so the tests go through the wire shape.
    fn model(provider: &str, id: &str) -> AgentModelOption {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "label": id,
            "provider": provider,
        }))
        .expect("model row")
    }

    /// The catalogue this machine actually has around the case that started
    /// this: `omlx` calls its model `Qwen3.8-27B-oQ4e-mtp`, and openrouter
    /// calls one `openrouter/auto`.
    fn reference_catalog() -> Vec<AgentModelOption> {
        vec![
            model("omlx", "Qwen3.8-27B-oQ4e-mtp"),
            model("openrouter", "openrouter/auto"),
            model("openai", "gpt-5"),
            model("deepseek", "deepseek/deepseek-v4-flash"),
        ]
    }

    fn models_payload() -> serde_json::Value {
        serde_json::json!({
            "models": [{
                "id": "future/k3",
                "label": "K3",
                "provider": "future",
                "supportsImages": true,
                "thinkingLevel": "high",
                "thinkingLevels": ["off", "low", "high"],
                "defaultThinkingLevel": "low",
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
        // The level list and the agent's starting level travel to the desktop:
        // without them every picker would fall back to all six and offer an
        // effort the agent clamps away.
        assert_eq!(
            model.thinking_levels.as_deref(),
            Some(vec!["off".to_string(), "low".to_string(), "high".to_string()].as_slice())
        );
        assert_eq!(model.default_thinking_level.as_deref(), Some("low"));
        assert_eq!(model.context_window, Some(256000));
        assert!(model.is_default);
        assert_eq!(model.description_en.as_deref(), Some("Budget-friendly"));
        assert!(model.recommended);
        assert!(
            !mock.requests_of("list_models")[0].include_builtin_providers,
            "plain model list must not request the provider catalog"
        );
    }

    /// The subscriber-only flag survives the hop to the desktop, and an older
    /// agent that does not send it reads as "not gated" — the direction that
    /// keeps models callable rather than wrongly greying them.
    #[tokio::test]
    async fn list_agent_models_carries_the_subscriber_only_flag() {
        let mock = mock_agent();
        mock.push_data(
            "list_models",
            serde_json::json!({
                "models": [
                    {
                        "id": "future-free",
                        "label": "Free",
                        "provider": "future",
                        "requiresSubscription": true
                    },
                    { "id": "deepseek-flash", "label": "Flash", "provider": "future" }
                ],
                "defaultModel": "future/deepseek-flash"
            }),
        );
        let models = list_agent_models().await.expect("models");
        assert!(models[0].requires_subscription);
        assert!(!models[1].requires_subscription);
        // The webview reads camelCase — the field's name is the contract, not an
        // implementation detail of this struct.
        let wire = serde_json::to_value(&models[0]).expect("serializes");
        assert_eq!(wire["requiresSubscription"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn list_agent_models_surfaces_rejection_transport_and_parse_errors() {
        let mock = mock_agent();

        mock.push("list_models", Reply::Reject("nope".to_string()));
        let error = list_agent_models().await.expect_err("rejected");
        assert_eq!(error.to_string(), "nope");

        mock.push(
            "list_models",
            Reply::Status(tonic::Code::Unavailable, "down"),
        );
        let error = list_agent_models().await.expect_err("transport");
        assert!(
            matches!(error, crate::Error::AgentUnavailable(_)),
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

        mock.push("list_models", Reply::Status(tonic::Code::Internal, "boom"));
        let error = list_builtin_providers().await.expect_err("transport");
        assert!(matches!(error, crate::Error::Message(_)), "{error}");

        mock.push_data(
            "list_models",
            serde_json::json!({"builtinProviders": [1, 2, 3]}),
        );
        let error = list_builtin_providers().await.expect_err("invalid");
        assert!(
            error.to_string().contains("invalid provider catalog"),
            "{error}"
        );
    }

    /// A stored reference the catalogue names exactly is left alone — including
    /// the ones whose *id* repeats the provider, which a string rule would
    /// happily "collapse" into a different model.
    #[test]
    fn a_reference_the_catalogue_names_is_returned_unchanged() {
        let catalog = reference_catalog();
        for raw in [
            "omlx/Qwen3.8-27B-oQ4e-mtp",
            "openrouter/openrouter/auto",
            "openai/gpt-5",
            "deepseek/deepseek/deepseek-v4-flash",
        ] {
            assert_eq!(
                normalize_model_reference(raw, &catalog),
                raw,
                "{raw} is in the catalogue; normalizing must not move it"
            );
        }
    }

    /// The bug behind this: a client prefixed an id that already began with its
    /// provider, so one redundant `provider/` layer is stored. The catalogue
    /// recognizes the inner form, and that is what a run must use.
    #[test]
    fn a_redundant_provider_prefix_is_collapsed_to_the_catalogue_entry() {
        let catalog = reference_catalog();
        assert_eq!(
            normalize_model_reference("omlx/omlx/Qwen3.8-27B-oQ4e-mtp", &catalog),
            "omlx/Qwen3.8-27B-oQ4e-mtp"
        );
        // Three layers collapse to one rather than being stripped blindly: only
        // the levels the catalogue recognizes go away.
        assert_eq!(
            normalize_model_reference("omlx/omlx/omlx/Qwen3.8-27B-oQ4e-mtp", &catalog),
            "omlx/Qwen3.8-27B-oQ4e-mtp"
        );
    }

    /// The counterexample that decides this cannot be a string rule:
    /// `openrouter`'s model really is called `openrouter/auto`, so its correct
    /// reference reads like a doubled prefix. The catalogue keeps it.
    #[test]
    fn a_model_whose_own_id_starts_with_its_provider_is_not_collapsed() {
        let catalog = reference_catalog();
        assert_eq!(
            normalize_model_reference("openrouter/openrouter/auto", &catalog),
            "openrouter/openrouter/auto",
            "this id genuinely repeats its provider"
        );
    }

    /// A value the catalogue cannot place is returned as stored: guessing would
    /// silently swap the model a task names, and the Agent's own resolution is
    /// the fallback that already handles it.
    #[test]
    fn an_unknown_reference_is_returned_as_stored() {
        let catalog = reference_catalog();
        for raw in ["ghost/model", "omlx/not-a-model", "p/q/r", "", "   "] {
            assert_eq!(
                normalize_model_reference(raw, &catalog),
                raw.trim(),
                "{raw:?} is not in the catalogue and must not be rewritten"
            );
        }
        // No catalogue at all (an agent that could not be asked) changes nothing.
        assert_eq!(
            normalize_model_reference("omlx/omlx/Qwen3.8-27B-oQ4e-mtp", &[]),
            "omlx/omlx/Qwen3.8-27B-oQ4e-mtp"
        );
    }

    /// The async entry point reads the live catalogue, so the collapse happens
    /// on what this Agent actually offers — and an unreadable catalogue leaves
    /// the value alone instead of failing the run.
    #[tokio::test]
    async fn the_live_catalogue_normalizes_and_an_unreadable_one_does_not() {
        let mock = mock_agent();
        mock.push_data(
            "list_models",
            serde_json::json!({"models": [
                {"id": "Qwen3.8-27B-oQ4e-mtp", "label": "Qwen", "provider": "omlx"},
                {"id": "openrouter/auto", "label": "auto", "provider": "openrouter"}
            ]}),
        );
        assert_eq!(
            normalize_model_reference_against_catalog("omlx/omlx/Qwen3.8-27B-oQ4e-mtp").await,
            "omlx/Qwen3.8-27B-oQ4e-mtp"
        );
        assert_eq!(
            normalize_model_reference_against_catalog("openrouter/openrouter/auto").await,
            "openrouter/openrouter/auto"
        );

        mock.push("list_models", Reply::Reject("nope".to_string()));
        assert_eq!(
            normalize_model_reference_against_catalog("omlx/omlx/Qwen3.8-27B-oQ4e-mtp").await,
            "omlx/omlx/Qwen3.8-27B-oQ4e-mtp",
            "an unreadable catalogue leaves the stored reference untouched"
        );
    }
}

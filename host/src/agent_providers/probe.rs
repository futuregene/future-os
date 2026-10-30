//! Thinking-level detection for the custom-provider editor.
//!
//! The editor offers a "detect" button that tries each thinking level against
//! the provider being edited and keeps the levels the endpoint answers. The
//! request itself is the Agent's (see `agent::llm::probe`): it is built by the
//! same adapters a real model call uses, so the answer is about the request the
//! Agent will actually send — including per-model wire settings (`compat`)
//! inherited from an already-saved model.
//!
//! One level per call on purpose: the caller reports progress, and the
//! calibration call (`default`) fails independently, which is how a wrong Base
//! URL / key / model id surfaces before any level is judged.

use crate::agent_bridge::config as agent_config;
use crate::Error as AppError;

/// One probe request. Mirrors the editor's live form, not the stored provider:
/// nothing is saved before the probe.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingLevelProbeInput {
    /// Wire protocol: `openai-completions`, `openai-responses` or `anthropic`.
    pub api: String,
    pub base_url: String,
    /// The key typed into the form. Empty means "use the stored key for
    /// `provider_id`" — the field is blank when the user keeps the existing one.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Provider / model id, used to inherit the saved model's wire settings.
    #[serde(default)]
    pub provider_id: Option<String>,
    pub model_id: String,
    /// The level to test; empty asks for the calibration call (no thinking
    /// parameter at all).
    #[serde(default)]
    pub level: Option<String>,
}

/// One probe's verdict, as the editor renders it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingLevelProbeOutcome {
    /// `default` for the calibration call, else the level that was tested.
    pub level: String,
    /// True when the endpoint accepted the request carrying this level.
    pub ok: bool,
    /// The provider's own message when it did not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Probe one thinking level against the provider in the editor.
pub async fn probe_thinking_level(
    input: ThinkingLevelProbeInput,
) -> Result<ThinkingLevelProbeOutcome, AppError> {
    let api = input.api.trim().to_string();
    let base_url = input.base_url.trim().to_string();
    let model_id = input.model_id.trim().to_string();
    if api.is_empty() || base_url.is_empty() || model_id.is_empty() {
        return Err(
            "A Base URL, an API type and a model ID are required to detect thinking levels."
                .to_string()
                .into(),
        );
    }
    agent_config::probe_thinking_level(future_rpc::proto::ThinkingLevelProbe {
        api,
        base_url,
        api_key: input.api_key.unwrap_or_default().trim().to_string(),
        provider_id: input.provider_id.unwrap_or_default().trim().to_string(),
        model_id,
        level: input.level.unwrap_or_default().trim().to_string(),
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(api: &str, base_url: &str, model_id: &str) -> ThinkingLevelProbeInput {
        ThinkingLevelProbeInput {
            api: api.to_string(),
            base_url: base_url.to_string(),
            api_key: None,
            provider_id: None,
            model_id: model_id.to_string(),
            level: Some("low".to_string()),
        }
    }

    #[tokio::test]
    async fn an_incomplete_form_is_rejected_before_the_agent_is_asked() {
        for incomplete in [
            input("", "https://example.test", "m"),
            input("openai-completions", "  ", "m"),
            input("openai-completions", "https://example.test", ""),
        ] {
            let error = probe_thinking_level(incomplete).await.unwrap_err();
            assert!(
                error.to_string().contains("Base URL"),
                "unexpected error: {error}"
            );
        }
    }
}

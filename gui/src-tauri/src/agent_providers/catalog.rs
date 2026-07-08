//! Built-in provider catalog derived from the agent's generated model list, plus
//! the `models.json` / FutureGene-cache path helpers the Providers view needs.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::Value;

use crate::auth_store::{agent_dir, FUTURE_PROVIDER_ID};
use crate::config_io;

// Depth note: this file lives at gui/src-tauri/src/agent_providers/, one level
// deeper than the old flat module, so the include needs one extra `../`.
#[path = "../../../../agent/src/models/generated/mod.rs"]
mod generated_model_catalog;

#[derive(Debug, Clone)]
pub(super) struct CatalogProviderSummary {
    pub(super) name: String,
    pub(super) base_url: String,
    pub(super) model_count: usize,
}

/// Built-in providers summarized from the generated catalog, keyed by id.
///
/// The generated catalog materializes ~900 models, and this is queried up to
/// several times per command, so the derived map is built once and cached; each
/// call returns a clone of the small (≈30-entry) summary map.
pub(super) fn builtin_catalog_providers() -> BTreeMap<String, CatalogProviderSummary> {
    static CACHE: OnceLock<BTreeMap<String, CatalogProviderSummary>> = OnceLock::new();
    CACHE.get_or_init(build_builtin_catalog_providers).clone()
}

fn build_builtin_catalog_providers() -> BTreeMap<String, CatalogProviderSummary> {
    let mut providers = BTreeMap::new();
    for model in generated_model_catalog::init_builtin_models() {
        if model.provider.is_empty() || model.provider == FUTURE_PROVIDER_ID {
            continue;
        }
        let entry =
            providers
                .entry(model.provider.clone())
                .or_insert_with(|| CatalogProviderSummary {
                    name: provider_display_name(&model.provider),
                    base_url: model.base_url.clone(),
                    model_count: 0,
                });
        entry.model_count += 1;
        if entry.base_url.is_empty() && !model.base_url.is_empty() {
            entry.base_url = model.base_url;
        }
    }
    providers
}

fn provider_display_name(id: &str) -> String {
    match id {
        "amazon-bedrock" => "Amazon Bedrock".to_string(),
        "anthropic" => "Anthropic".to_string(),
        "azure-openai-responses" => "Azure OpenAI Responses".to_string(),
        "cerebras" => "Cerebras".to_string(),
        "cloudflare-workers-ai" => "Cloudflare Workers AI".to_string(),
        "deepseek" => "DeepSeek".to_string(),
        "github-copilot" => "GitHub Copilot".to_string(),
        "google" => "Google".to_string(),
        "google-vertex" => "Google Vertex".to_string(),
        "groq" => "Groq".to_string(),
        "huggingface" => "Hugging Face".to_string(),
        "kimi-coding" => "Kimi Coding".to_string(),
        "minimax" => "MiniMax".to_string(),
        "minimax-cn" => "MiniMax CN".to_string(),
        "mistral" => "Mistral".to_string(),
        "moonshotai" => "Moonshot AI".to_string(),
        "moonshotai-cn" => "Moonshot AI CN".to_string(),
        "openai" => "OpenAI".to_string(),
        "openai-codex" => "OpenAI Codex".to_string(),
        "opencode" => "opencode".to_string(),
        "opencode-go" => "opencode Go".to_string(),
        "openrouter" => "OpenRouter".to_string(),
        "vercel-ai-gateway" => "Vercel AI Gateway".to_string(),
        "xai" => "xAI".to_string(),
        "xiaomi" => "Xiaomi".to_string(),
        "xiaomi-token-plan-ams" => "Xiaomi Token Plan AMS".to_string(),
        "xiaomi-token-plan-cn" => "Xiaomi Token Plan CN".to_string(),
        "xiaomi-token-plan-sgp" => "Xiaomi Token Plan SGP".to_string(),
        "zai" => "Z.ai".to_string(),
        "zhipuai" => "ZhipuAI".to_string(),
        _ => id
            .split('-')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

pub(super) fn models_json_path() -> Result<PathBuf, crate::AppError> {
    Ok(agent_dir()?.join("models.json"))
}

pub(super) fn future_models_cache_path() -> Result<PathBuf, crate::AppError> {
    Ok(agent_dir()?.join(".future-models-cache.json"))
}

pub(super) fn future_model_count() -> usize {
    future_models_cache_path()
        .ok()
        .and_then(|path| {
            config_io::read_json_lenient(&path)
                .get("models")
                .and_then(Value::as_array)
                .map(Vec::len)
        })
        .unwrap_or(0)
}

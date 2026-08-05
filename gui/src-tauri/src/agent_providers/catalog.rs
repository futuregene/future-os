//! Built-in provider catalog fetched from the agent at runtime, plus the
//! `models.json` / FutureGene-cache path helpers the Providers view needs.
//!
//! The catalog used to be compiled into the GUI via a `#[path]` include of the
//! agent's generated model catalog — source-level coupling across the agent /
//! GUI boundary. The agent is now the single source of truth: the catalog is
//! fetched over the `list_models` RPC with `include_builtin_providers` set.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::auth_store::{agent_dir, FUTURE_PROVIDER_ID};
use crate::config_io;

#[derive(Debug, Clone)]
pub(crate) struct CatalogProviderSummary {
    pub(super) name: String,
    pub(super) base_url: String,
    pub(super) model_count: usize,
}

/// Built-in providers summarized from the agent's catalog, keyed by id.
///
/// The result is cached for the process lifetime after the first successful
/// fetch — the catalog only changes when the agent binary changes, so there is
/// no need to re-request it on every Providers-page render or write-command
/// validation. When the agent is unreachable and no fetch has succeeded yet,
/// returns an empty map (the Providers page still shows the Future and custom
/// providers; built-in key/URL edits are unavailable until the agent is back).
pub(super) async fn builtin_catalog_providers() -> BTreeMap<String, CatalogProviderSummary> {
    static CACHE: tokio::sync::OnceCell<BTreeMap<String, CatalogProviderSummary>> =
        tokio::sync::OnceCell::const_new();
    match CACHE
        .get_or_try_init(|| async {
            let catalog = crate::agent_bridge::list_builtin_providers().await?;
            Ok::<_, crate::AppError>(
                catalog
                    .into_iter()
                    .filter(|(id, _)| !id.is_empty() && id != FUTURE_PROVIDER_ID)
                    .map(|(id, entry)| {
                        let summary = CatalogProviderSummary {
                            name: provider_display_name(&id),
                            base_url: entry.base_url,
                            model_count: entry.model_count,
                        };
                        (id, summary)
                    })
                    .collect(),
            )
        })
        .await
    {
        Ok(catalog) => catalog.clone(),
        Err(error) => {
            eprintln!("FutureOS: built-in provider catalog unavailable from the agent: {error}");
            BTreeMap::new()
        }
    }
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

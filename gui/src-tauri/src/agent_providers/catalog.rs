//! Built-in provider catalog fetched from the agent at runtime, plus the
//! `models.json` / FutureGene-cache path helpers the Providers view needs.
//!
//! The catalog used to be compiled into the GUI via a `#[path]` include of the
//! agent's generated model catalog — source-level coupling across the agent /
//! GUI boundary. The agent is now the single source of truth: the catalog is
//! fetched over the `list_models` RPC with `include_builtin_providers` set, and
//! that response also carries each provider's human-readable display name, so
//! the GUI has no independent id→name map to keep in sync.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::auth_store::{agent_dir, FUTURE_PROVIDER_ID};
use crate::config_io;

#[derive(Debug, Clone)]
pub(crate) struct CatalogProviderSummary {
    /// Display name as supplied by the agent (single source of truth); falls
    /// back to the provider id for a pre-name agent.
    pub(super) name: String,
    pub(super) base_url: String,
    pub(super) model_count: usize,
}

/// Built-in providers summarized from the agent's catalog, keyed by id.
///
/// The result is cached for the process lifetime after the first successful
/// fetch — the catalog only changes when the agent binary changes, so there is
/// no need to re-request it on every Providers-page render or write-command
/// validation. The built-in catalog is never empty, so an empty map means "the
/// catalog could not be obtained" (agent unreachable and never reached during
/// this process): the Providers page still shows the Future and custom
/// providers, and write-command validation refuses to create a provider it
/// cannot check for reserved-id collisions rather than trusting an empty set.
/// Within a running process, a successful fetch keeps working even after the
/// agent goes away (the `OnceCell` stays populated), so offline creation only
/// degrades on a cold start with the agent already down.
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
                            name: if entry.name.is_empty() {
                                id.clone()
                            } else {
                                entry.name
                            },
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

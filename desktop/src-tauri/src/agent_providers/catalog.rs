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
#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
use serde_json::Value;

#[cfg(test)]
use crate::auth_store::agent_dir;
use crate::auth_store::FUTURE_PROVIDER_ID;
#[cfg(test)]
use crate::config_io;

#[derive(Debug, Clone)]
pub(crate) struct CatalogProviderSummary {
    /// Display name as supplied by the agent (single source of truth); falls
    /// back to the provider id for a pre-name agent.
    pub(super) name: String,
    #[allow(dead_code)]
    pub(super) base_url: String,
    #[allow(dead_code)]
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
    CACHE
        .get_or_try_init(fetch_builtin_catalog)
        .await
        .map_or_else(catalog_unavailable, Clone::clone)
}

/// One catalog fetch: the `list_models` RPC (with `include_builtin_providers`)
/// summarized into the GUI's shape, FutureGene excluded (it is presented
/// separately as the built-in "Future" provider).
async fn fetch_builtin_catalog() -> Result<BTreeMap<String, CatalogProviderSummary>, crate::AppError>
{
    let catalog = crate::agent_bridge::list_builtin_providers().await?;
    Ok(catalog
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
        .collect())
}

/// An unobtainable catalog (agent unreachable) is not an error for the display
/// path: log once per attempt and return an empty map, so the Providers page
/// still shows FutureGene and custom providers, and write-command validation
/// refuses what it cannot check rather than trusting an empty set.
pub(super) fn catalog_unavailable(
    error: crate::AppError,
) -> BTreeMap<String, CatalogProviderSummary> {
    eprintln!("FutureOS: built-in provider catalog unavailable from the agent: {error}");
    BTreeMap::new()
}

#[cfg(test)]
pub(super) fn models_json_path() -> Result<PathBuf, crate::AppError> {
    Ok(agent_dir()?.join("models.json"))
}

#[cfg(test)]
pub(super) fn future_models_cache_path() -> Result<PathBuf, crate::AppError> {
    Ok(agent_dir()?.join(".future-models-cache.json"))
}

#[cfg(test)]
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

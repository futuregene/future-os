//! `future models` — 1:1 port of cli/src/commands/models.ts.
//!
//! Lists models from the running agent via gRPC `list_models`.

use crate::output::Output;
use crate::rpc::{grpc_addr, RunClient};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// `humanContextWindow(tokens)` from models.ts.
fn human_context_window(tokens: i64) -> String {
    if tokens >= 1_000_000 {
        format!("{}M", (tokens as f64 / 1_000_000.0).round() as i64)
    } else if tokens >= 1_000 {
        format!("{}K", (tokens as f64 / 1_000.0).round() as i64)
    } else {
        tokens.to_string()
    }
}

/// One parsed model row from the `list_models` response.
#[derive(Debug, Clone)]
struct Model {
    id: String,
    label: String,
    provider: String,
    supports_images: bool,
    thinking_level: String,
    context_window: i64,
}

/// JSON row — field order matches the TS object literal `{id, label,
/// contextWindow, supportsImages, thinkingLevel, isDefault}` (serde structs
/// serialize fields in declaration order; `serde_json::Map` would sort).
#[derive(serde::Serialize)]
struct ModelRowJson {
    id: String,
    label: String,
    #[serde(rename = "contextWindow")]
    context_window: i64,
    #[serde(rename = "supportsImages")]
    supports_images: bool,
    #[serde(rename = "thinkingLevel")]
    thinking_level: String,
    #[serde(rename = "isDefault")]
    is_default: bool,
}

/// One `"<provider>": [rows]` entry, serialized as a nested JSON map so the
/// provider key order is preserved (TS inserts by_provider entries in the
/// sorted `providers` order).
struct ProviderGroup<'a>(&'a [(String, Vec<ModelRowJson>)]);

impl serde::Serialize for ProviderGroup<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (provider, rows) in self.0 {
            map.serialize_entry(provider, rows)?;
        }
        map.end()
    }
}

/// Top-level JSON — matches the TS object literal `{providers, defaultModel,
/// models, totalModels}` (field order preserved by the manual serializer).
struct ModelsJson {
    providers: Vec<String>,
    default_model: String,
    by_provider: Vec<(String, Vec<ModelRowJson>)>,
    total_models: usize,
}

impl serde::Serialize for ModelsJson {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("providers", &self.providers)?;
        map.serialize_entry("defaultModel", &self.default_model)?;
        map.serialize_entry("models", &ProviderGroup(&self.by_provider))?;
        map.serialize_entry("totalModels", &self.total_models)?;
        map.end()
    }
}

/// `models(args)`.
pub async fn models(args: &[String], out: &Output) -> Result<(), String> {
    // `const jsonFlag = args.includes("--json");`
    let json_flag = args.iter().any(|a| a == "--json");
    // `const nonFlags = args.filter(a => !a.startsWith("--"));`
    let non_flags: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    // `nonFlags[0] ?? process.env.FUTURE_AGENT_GRPC_ADDR ?? "127.0.0.1:50051"`
    let grpc_addr = match non_flags.first() {
        Some(s) => s.as_str().to_string(),
        None => grpc_addr(),
    };

    let client = RunClient::new(&grpc_addr);
    let data = match client.list_models().await {
        Ok(data) => data,
        Err(msg) => {
            // `console.log(JSON.stringify({error: msg}))` / `console.error(...)` + exit(1)
            if json_flag {
                out.log(&json!({ "error": msg }).to_string());
            } else {
                out.log_err(&format!("Error: failed to list models — {msg}"));
            }
            return Err(crate::HANDLED_EXIT.to_string());
        }
    };

    // `data.models.map(m => ...)` — raw rows.
    let raw_models: Vec<Model> = data
        .get("models")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    Some(Model {
                        id: m.get("id")?.as_str()?.to_string(),
                        label: m.get("label")?.as_str()?.to_string(),
                        provider: m.get("provider")?.as_str()?.to_string(),
                        supports_images: m
                            .get("supportsImages")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        thinking_level: m
                            .get("thinkingLevel")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        context_window: m.get("contextWindow").and_then(Value::as_i64).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    // `data.defaultModel`
    let default_model = data
        .get("defaultModel")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if json_flag {
        // `[...new Set(data.models.map(m => m.provider))].sort()`
        let mut providers: Vec<String> = raw_models.iter().map(|m| m.provider.clone()).collect();
        providers.sort();
        providers.dedup();
        // `byProvider[provider] = ...filter(...).sort((a,b) => a.id.localeCompare(b.id))`
        let mut by_provider: Vec<(String, Vec<ModelRowJson>)> = Vec::new();
        for provider in &providers {
            let mut provider_models: Vec<&Model> = raw_models
                .iter()
                .filter(|m| &m.provider == provider)
                .collect();
            provider_models.sort_by(|a, b| a.id.cmp(&b.id));
            let rows: Vec<ModelRowJson> = provider_models
                .iter()
                .map(|m| ModelRowJson {
                    id: m.id.clone(),
                    label: m.label.clone(),
                    context_window: m.context_window,
                    supports_images: m.supports_images,
                    thinking_level: m.thinking_level.clone(),
                    is_default: m.id == default_model,
                })
                .collect();
            by_provider.push((provider.clone(), rows));
        }
        let output = ModelsJson {
            providers,
            default_model,
            by_provider,
            total_models: raw_models.len(),
        };
        out.log(&serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?);
        return Ok(());
    }

    // Text mode: group by provider preserving response order.
    let mut by_provider: BTreeMap<String, Vec<&Model>> = BTreeMap::new();
    for m in &raw_models {
        by_provider.entry(m.provider.clone()).or_default().push(m);
    }
    // `[...byProvider.entries()].sort()` — BTreeMap iterates sorted.
    for (provider, provider_models) in &by_provider {
        out.log(&format!(
            "Provider: {provider}  ({} models)",
            provider_models.len()
        ));
        for m in provider_models {
            let is_default = m.id == default_model;
            let ctx_win = human_context_window(m.context_window);
            let img = if m.supports_images { "  image" } else { "" };
            let thinking = if m.thinking_level != "off" {
                format!("  thinking:{}", m.thinking_level)
            } else {
                String::new()
            };
            let def = if is_default { "  [default]" } else { "" };
            // `m.id.padEnd(28)` / `ctxWin.padStart(5)` — min-width padding,
            // never truncation.
            out.log(&format!(
                "  Model: {:<28} ctx:{:>5}{img}{thinking}{def}",
                m.id, ctx_win
            ));
        }
        out.log("");
    }

    out.log(&format!(
        "{} models, {} providers.  Default model: {default_model}",
        raw_models.len(),
        by_provider.len()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_context_window_behavior() {
        assert_eq!(human_context_window(1_000_000), "1M");
        assert_eq!(human_context_window(1_500_000), "2M");
        assert_eq!(human_context_window(999_999), "1000K");
        assert_eq!(human_context_window(128_000), "128K");
        assert_eq!(human_context_window(1000), "1K");
        assert_eq!(human_context_window(999), "999");
        assert_eq!(human_context_window(0), "0");
    }

    #[tokio::test]
    async fn models_agent_down_error_path() {
        let _guard = crate::test_env::lock_env().await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        // Point at an address that refuses connections quickly (port 1).
        let (out, cap) = Output::memory();
        let args = vec![];
        let result = models(&args, &out).await;
        // The command prints its own error and signals handled-exit.
        assert!(result.is_err());
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("Error: failed to list models —"),
            "stderr: {stderr}"
        );
    }

    #[tokio::test]
    async fn models_json_error_path() {
        let _guard = crate::test_env::lock_env().await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        let (out, cap) = Output::memory();
        let args = vec!["--json".to_string()];
        let result = models(&args, &out).await;
        assert!(result.is_err());
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.starts_with("{\"error\":"), "stdout: {stdout}");
    }
}

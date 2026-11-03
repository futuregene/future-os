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
    /// The levels the model offers, as declared by the agent: `default` first
    /// (it means "send no thinking parameter"), then wire values sent verbatim.
    /// Empty for an agent that predates the field.
    thinking_levels: Vec<String>,
    context_window: i64,
}

/// The reserved level meaning "send no thinking/effort parameter at all".
const DEFAULT_THINKING_LEVEL: &str = "default";

/// The declared levels worth printing: the implicit `default` is not a
/// capability, so only the wire values after it are shown. Empty when the model
/// declares nothing else (a model with only `default`, or an older agent), in
/// which case the row says nothing.
fn declared_levels_label(levels: &[String]) -> String {
    levels
        .iter()
        .map(String::as_str)
        .filter(|level| *level != DEFAULT_THINKING_LEVEL)
        .collect::<Vec<_>>()
        .join(",")
}

/// JSON row — field order matches the TS object literal `{id, label,
/// contextWindow, supportsImages, thinkingLevels, isDefault}` (serde structs
/// serialize fields in declaration order; `serde_json::Map` would sort).
#[derive(serde::Serialize)]
struct ModelRowJson {
    id: String,
    label: String,
    #[serde(rename = "contextWindow")]
    context_window: i64,
    #[serde(rename = "supportsImages")]
    supports_images: bool,
    #[serde(rename = "thinkingLevels")]
    thinking_levels: Vec<String>,
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

/// The Future platform provider id — the catalogue `future models refresh` pulls.
const FUTURE_PROVIDER: &str = "future";

/// `models(args)`.
pub async fn models(args: &[String], out: &Output) -> Result<(), String> {
    // `const jsonFlag = args.includes("--json");`
    let json_flag = args.iter().any(|a| a == "--json");
    // `const nonFlags = args.filter(a => !a.startsWith("--"));`
    let non_flags: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();

    // `future models refresh [<addr>]` — the only subcommand. Every other
    // positional is the explicit agent address the listing accepts.
    if non_flags
        .first()
        .is_some_and(|arg| arg.as_str() == "refresh")
    {
        return refresh(non_flags.get(1).map(|arg| arg.as_str()), json_flag, out).await;
    }

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
                        thinking_levels: m
                            .get("thinkingLevels")
                            .and_then(Value::as_array)
                            .map(|levels| {
                                levels
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default(),
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
                    thinking_levels: m.thinking_levels.clone(),
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
        // Serializing plain JSON values is infallible.
        out.log(&serde_json::to_string_pretty(&output).expect("json serializes"));
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
            // The implicit `default` is not a capability: a model that declares
            // nothing else gets no segment at all.
            let offered = declared_levels_label(&m.thinking_levels);
            let thinking = if offered.is_empty() {
                String::new()
            } else {
                format!("  thinking:{offered}")
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

/// Report a failure the way the listing does — a JSON `{"error": ...}` on stdout
/// under `--json`, a readable line on stderr otherwise — and signal that the
/// exit code is already accounted for.
fn fail(json_flag: bool, out: &Output, message: &str) -> Result<(), String> {
    if json_flag {
        out.log(&json!({ "error": message }).to_string());
    } else {
        out.log_err(&format!("Error: {message}"));
    }
    Err(crate::HANDLED_EXIT.to_string())
}

/// Whether a Future credential is configured (`auth.json` `future` entry).
///
/// Read here only to explain a failed refresh: the agent answers `synced:
/// false` both for "no credential" and for "the platform call failed", and the
/// two need different advice.
fn future_credential_present() -> bool {
    future_agent::AuthStore::load()
        .get(FUTURE_PROVIDER)
        .is_some()
}

/// How many Future models the catalogue currently offers, counted through the
/// same registry injection the Agent uses (so it is the number the pickers
/// would show, not a raw cache-entry count).
fn future_model_count() -> usize {
    future_agent::models::Registry::new()
        .all_models()
        .iter()
        .filter(|model| model.provider == FUTURE_PROVIDER)
        .count()
}

/// `future models refresh [<addr>] [--json]` — pull the Future platform
/// catalogue, the CLI counterpart of the desktop's Settings → Providers →
/// "Update model list" button.
///
/// The agent is asked first: its `sync_future_models` command fetches the
/// platform catalogue, rebuilds the registry and publishes
/// `provider_config_changed`, which is what makes a *running* desktop reload its
/// model list in place — no restart, nothing to copy around. When no agent is
/// reachable (the desktop is closed) the fetch runs here instead and fills the
/// on-disk cache the next Agent start reads.
async fn refresh(address: Option<&str>, json_flag: bool, out: &Output) -> Result<(), String> {
    let addr = address.map(str::to_string).unwrap_or_else(grpc_addr);
    let client = RunClient::new(&addr);
    match client.sync_future_models().await {
        Ok(data) => {
            let synced = data.get("synced").and_then(Value::as_bool).unwrap_or(false);
            let model_count = data.get("modelCount").and_then(Value::as_u64).unwrap_or(0);
            if !synced {
                return fail(json_flag, out, &refresh_failure_reason());
            }
            if json_flag {
                out.log(
                    &json!({
                        "provider": FUTURE_PROVIDER,
                        "source": "agent",
                        "synced": true,
                        "modelCount": model_count,
                    })
                    .to_string(),
                );
            } else {
                out.log(&format!(
                    "Refreshed the Future model catalogue: {model_count} models."
                ));
                out.log("A running Desktop reloads its model list by itself — no restart needed.");
            }
            Ok(())
        }
        Err(agent_error) => {
            // No agent: refresh the catalogue this process owns, so the next
            // Agent (a Desktop start) begins from the fresh cache instead of the
            // stale one. A failed agent call is not fatal on its own — it is
            // reported only if the local fetch cannot make up for it.
            if !future_credential_present() {
                return fail(
                    json_flag,
                    out,
                    &format!(
                        "{agent_error} — and no Future credential is configured; \
                         run `future auth login`."
                    ),
                );
            }
            if !future_agent::models::sync_future_models_cache() {
                return fail(json_flag, out, &refresh_failure_reason());
            }
            let model_count = future_model_count();
            if json_flag {
                out.log(
                    &json!({
                        "provider": FUTURE_PROVIDER,
                        "source": "cache",
                        "synced": true,
                        "modelCount": model_count,
                    })
                    .to_string(),
                );
            } else {
                out.log(&format!(
                    "Refreshed the Future model catalogue on disk: {model_count} models."
                ));
                out.log(&format!(
                    "No agent took the refresh ({agent_error}) — start the Desktop \
                     (or `future agent`) to use them."
                ));
            }
            Ok(())
        }
    }
}

/// Why a refresh did not complete, in the form the user can act on.
fn refresh_failure_reason() -> String {
    if future_credential_present() {
        "the Future platform request failed — check the network, or run `future doctor`."
            .to_string()
    } else {
        "no Future credential is configured — run `future auth login`.".to_string()
    }
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

    /// Mock agent serving two providers' worth of models.
    async fn models_mock() -> (String, crate::test_server::MockAgent) {
        let agent = crate::test_server::MockAgent::respond(
            "list_models",
            "{\"models\":[\
                {\"id\":\"k3\",\"label\":\"Kimi K3\",\"provider\":\"future\",\"supportsImages\":true,\"thinkingLevel\":\"default\",\"thinkingLevels\":[\"default\",\"off\",\"low\",\"high\",\"max\"],\"contextWindow\":262144},\
                {\"id\":\"k2\",\"label\":\"Kimi K2\",\"provider\":\"future\",\"contextWindow\":131072},\
                {\"id\":\"gpt-5\",\"label\":\"GPT-5\",\"provider\":\"openai\",\"supportsImages\":false,\"thinkingLevel\":\"default\",\"thinkingLevels\":[\"default\"],\"contextWindow\":400000}\
            ],\"defaultModel\":\"k3\"}",
        );
        let addr = crate::test_server::spawn_mock(agent.clone()).await;
        (addr, agent)
    }

    #[tokio::test]
    async fn models_text_output_groups_by_provider() {
        let (addr, _agent) = models_mock().await;
        let (out, cap) = Output::memory();
        let args = vec![addr];
        models(&args, &out).await.expect("models");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        // BTreeMap ordering: future before openai.
        let future_pos = stdout
            .find("Provider: future  (2 models)")
            .expect("future group");
        let openai_pos = stdout
            .find("Provider: openai  (1 models)")
            .expect("openai group");
        assert!(future_pos < openai_pos);
        // Padding, context window humanization, flags, default marker.
        assert!(stdout.contains("Model: k3"), "stdout: {stdout}");
        assert!(stdout.contains("ctx: 262K"), "stdout: {stdout}");
        assert!(stdout.contains("  image"), "stdout: {stdout}");
        // The declared levels, with the implicit default left out.
        assert!(
            stdout.contains("  thinking:off,low,high,max"),
            "stdout: {stdout}"
        );
        assert!(stdout.contains("  [default]"), "stdout: {stdout}");
        // A model that declares only `default`, and an agent that declares
        // nothing, render no thinking segment.
        assert!(!stdout.contains("thinking:default"), "stdout: {stdout}");
        assert!(stdout.contains("3 models, 2 providers.  Default model: k3"));
    }

    #[tokio::test]
    async fn models_json_output_shape() {
        let (addr, _agent) = models_mock().await;
        let (out, cap) = Output::memory();
        let args = vec!["--json".to_string(), addr];
        models(&args, &out).await.expect("models");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["providers"], json!(["future", "openai"]));
        assert_eq!(parsed["defaultModel"], "k3");
        assert_eq!(parsed["totalModels"], 3);
        let future_rows = parsed["models"]["future"].as_array().unwrap();
        // Sorted by id within a provider.
        assert_eq!(future_rows[0]["id"], "k2");
        assert_eq!(future_rows[1]["id"], "k3");
        assert_eq!(future_rows[1]["isDefault"], true);
        assert_eq!(future_rows[1]["contextWindow"], 262144);
        assert_eq!(future_rows[1]["supportsImages"], true);
        assert_eq!(
            future_rows[1]["thinkingLevels"],
            json!(["default", "off", "low", "high", "max"])
        );
        // The agent's declaration is echoed as-is; a model that declares only
        // the implicit default carries just that.
        assert_eq!(
            parsed["models"]["openai"][0]["thinkingLevels"],
            json!(["default"])
        );
        assert_eq!(parsed["models"]["openai"][0]["isDefault"], false);
    }

    /// The mock agent that answers `list_models` (above) and a `refresh` mock
    /// that answers with a fixed sync result.
    /// The platform fixture the "no agent, local refresh" tests fetch from: one
    /// model, at the route an `/api` credential base hangs the catalogue off.
    async fn catalogue_platform() -> String {
        crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/api/v1/models",
            200,
            r#"{"data":[{"id":"kimi-k3.1-pro","name":"Kimi K3.1 PRO","context_length":1048576}]}"#,
        )])
        .await
    }

    /// A Future credential whose platform is `base`, so the local catalogue
    /// refresh has somewhere to fetch from. The caller holds the env lock and a
    /// temporary home.
    fn future_credential_pointing_at(base: &str) {
        let auth_path = future_agent::utils::default_config_dir().join("auth.json");
        std::fs::create_dir_all(auth_path.parent().expect("agent dir")).expect("agent dir");
        std::fs::write(
            &auth_path,
            format!(r#"{{"future":{{"type":"api_key","key":"k","base_url":"{base}/api"}}}}"#),
        )
        .expect("auth.json");
    }

    async fn refresh_mock(synced: bool, model_count: u32) -> String {
        let agent = crate::test_server::MockAgent::respond(
            "sync_future_models",
            &format!(r#"{{"synced":{synced},"modelCount":{model_count},"revision":7}}"#),
        );
        crate::test_server::spawn_mock(agent).await
    }

    #[tokio::test]
    async fn refresh_reports_the_agent_sync_and_asks_for_it() {
        let _guard = crate::test_env::lock_env().await;
        // Isolated home: the refresh path reads auth.json to explain failures,
        // and a test must never read (or warm) the developer's real catalogue.
        let _home = crate::test_env::EnvGuard::temp_home();
        let addr = refresh_mock(true, 14).await;

        let (out, cap) = Output::memory();
        models(&["refresh".to_string(), addr.clone()], &out)
            .await
            .expect("refresh");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("Refreshed the Future model catalogue: 14 models."),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("reloads its model list by itself"),
            "the desktop is the point of this command: {stdout}"
        );
        assert!(cap.err.lock().unwrap().is_empty());

        // `--json` carries the same facts, with where the refresh happened.
        let (out, cap) = Output::memory();
        models(&["refresh".to_string(), "--json".to_string(), addr], &out)
            .await
            .expect("refresh json");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["source"], "agent");
        assert_eq!(parsed["provider"], "future");
        assert_eq!(parsed["synced"], true);
        assert_eq!(parsed["modelCount"], 14);
    }

    /// The agent answers `synced: false` both when it has no Future credential
    /// and when the platform call failed; the two need different advice, so the
    /// command reads the credential to pick the message.
    #[tokio::test]
    async fn refresh_explains_a_failed_sync_from_the_local_credential() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        let addr = refresh_mock(false, 0).await;

        let (out, cap) = Output::memory();
        let result = models(&["refresh".to_string(), addr], &out).await;
        assert!(result.is_err(), "a failed refresh must exit non-zero");
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("no Future credential is configured")
                && stderr.contains("future auth login"),
            "stderr: {stderr}"
        );
    }

    /// With no agent (desktop closed) and no credential there is nothing to fetch
    /// with: the command reports the agent error and the login hint rather than
    /// touching the network.
    #[tokio::test]
    async fn refresh_without_an_agent_or_a_credential_reports_both() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();

        let (out, cap) = Output::memory();
        let result = models(&["refresh".to_string(), "127.0.0.1:1".to_string()], &out).await;
        assert!(result.is_err());
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("no Future credential is configured"),
            "stderr: {stderr}"
        );
        assert!(stderr.contains("future auth login"), "stderr: {stderr}");

        // `--json` reports the same failure as a JSON error object.
        let (out, cap) = Output::memory();
        models(
            &[
                "refresh".to_string(),
                "--json".to_string(),
                "127.0.0.1:1".to_string(),
            ],
            &out,
        )
        .await
        .expect_err("handled exit");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(stdout.starts_with("{\"error\":"), "stdout: {stdout}");
    }

    /// With no agent running (the desktop is closed) the command refreshes the
    /// on-disk catalogue itself, so the next Agent start — and the desktop that
    /// starts it — begins from the fresh list.
    ///
    /// `multi_thread` on purpose: the sync path performs the platform fetch on a
    /// native thread and joins it, so the in-test mock server needs a worker of
    /// its own to answer on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refresh_without_an_agent_warms_the_disk_cache() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();

        // A Future credential exists, but only the local catalogue can be
        // refreshed: point it at the mock platform (auth.json stores
        // `{platform}/api`, and the catalogue hangs off it).
        future_credential_pointing_at(&catalogue_platform().await);

        let (out, cap) = Output::memory();
        let result = models(&["refresh".to_string(), "127.0.0.1:1".to_string()], &out).await;
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        result.unwrap_or_else(|_| panic!("local refresh — stdout: {stdout} stderr: {stderr}"));
        assert!(
            stdout.contains("Refreshed the Future model catalogue on disk: 1 models."),
            "stdout: {stdout}"
        );
        assert!(
            stdout.contains("start the Desktop"),
            "the user must know when it takes effect: {stdout}"
        );
        // The catalogue really is on disk for the next Agent start.
        let cache = std::fs::read_to_string(
            future_agent::utils::default_config_dir().join(".future-models-cache.json"),
        )
        .expect("cache written");
        assert!(cache.contains("kimi-k3.1-pro"), "cache: {cache}");
    }

    /// The same local refresh asked for as a document: the same facts, with
    /// `source: "cache"` so a script can tell it apart from an agent's answer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_local_refresh_reports_itself_as_the_cache_in_json() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        future_credential_pointing_at(&catalogue_platform().await);

        let (out, cap) = Output::memory();
        models(
            &[
                "refresh".to_string(),
                "--json".to_string(),
                "127.0.0.1:1".to_string(),
            ],
            &out,
        )
        .await
        .expect("local refresh");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let printed: Value = serde_json::from_str(&stdout).expect("one json document");
        assert_eq!(printed["provider"], "future");
        assert_eq!(printed["source"], "cache");
        assert_eq!(printed["synced"], true);
        assert_eq!(printed["modelCount"], 1);
    }

    /// A credential exists but the platform cannot be reached: the local fetch
    /// fails too, and the command must say *which* of the two reasons applies —
    /// a login hint here would send the user to fix an account that is fine.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_platform_fetch_is_reported_as_the_network_not_the_login() {
        let _guard = crate::test_env::lock_env().await;
        let _home = crate::test_env::EnvGuard::temp_home();
        future_credential_pointing_at("http://127.0.0.1:1");

        let (out, cap) = Output::memory();
        let result = models(&["refresh".to_string(), "127.0.0.1:1".to_string()], &out).await;
        assert!(result.is_err(), "a failed refresh must exit non-zero");
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(
            stderr.contains("the Future platform request failed"),
            "stderr: {stderr}"
        );
        assert!(
            stderr.contains("future doctor") && !stderr.contains("future auth login"),
            "a credential is present, so this is not a login problem: {stderr}"
        );
    }

    #[tokio::test]
    async fn models_missing_fields_and_empty_list() {
        // Malformed rows are filtered; absent models key → empty.
        let agent = crate::test_server::MockAgent::respond(
            "list_models",
            "{\"models\":[{\"id\":\"ok\",\"label\":\"L\",\"provider\":\"p\"},{\"id\":\"broken\"}]}",
        );
        let addr = crate::test_server::spawn_mock(agent).await;
        let (out, cap) = Output::memory();
        models(&["--json".to_string(), addr], &out)
            .await
            .expect("models");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json");
        assert_eq!(parsed["totalModels"], 1);
        // Missing optional fields default.
        assert_eq!(parsed["models"]["p"][0]["thinkingLevels"], json!([]));
        assert_eq!(parsed["models"]["p"][0]["supportsImages"], false);
        assert_eq!(parsed["defaultModel"], "");

        let agent = crate::test_server::MockAgent::respond("list_models", "{}");
        let addr = crate::test_server::spawn_mock(agent).await;
        let (out, cap) = Output::memory();
        models(&[addr], &out).await.expect("models");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert!(
            stdout.contains("0 models, 0 providers."),
            "stdout: {stdout}"
        );
    }
}

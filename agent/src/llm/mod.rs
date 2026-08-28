//! LLM Client — 1:1 compatible with internal/llm/
//!
//! Uses reqwest for HTTP + SSE streaming, matching Go's OpenAI SDK behavior.

mod adapters;
pub mod schema;
mod sse;
use adapters::AdapterRegistry;
use anyhow::{anyhow, Result};
use futures::StreamExt;
use parking_lot::RwLock;
use reqwest::Client as HttpClient;
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;

const DEFAULT_TIMEOUT_SECS: u64 = 1800;
const STREAM_IDLE_TIMEOUT_SECS: u64 = 45;
const UPSTREAM_DISCONNECTED: &str = "[UPSTREAM_DISCONNECTED]";
const MODEL_RESPONSE_ERROR: &str = "[MODEL_RESPONSE_ERROR]";

/// Stream-read idle timeout. Tests override it (a stalled-mock test cannot
/// wait 45 s of real time) via FUTURE_TEST_STREAM_IDLE_SECS.
fn stream_idle_timeout_secs() -> u64 {
    #[cfg(test)]
    if let Some(secs) = std::env::var("FUTURE_TEST_STREAM_IDLE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        return secs;
    }
    STREAM_IDLE_TIMEOUT_SECS
}

/// HTTP request timeout for a single LLM call. Defaults to 30 min (1800 s);
/// override with the FUTURE_LLM_TIMEOUT_SECS env var without rebuilding.
fn llm_timeout_secs() -> u64 {
    std::env::var("FUTURE_LLM_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v >= 60)
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

// ─── LLM Client ────────────────────────────────────────────────────────────

pub struct Client {
    http: HttpClient,
    /// Static targets are used by standalone/test clients. Session clients set
    /// this to `None`: they retain only a provider/model identity plus
    /// session-local generation choices, never a copied provider/model config.
    target: RwLock<Option<schema::ResolvedModelTarget>>,
    generation: RwLock<schema::GenerationConfig>,
    live_model: Option<LiveModelSource>,
    adapters: AdapterRegistry,
}

struct LiveModelSource {
    canonical_model: String,
    registry: std::sync::Arc<parking_lot::RwLock<crate::models::Registry>>,
}

impl Client {
    pub fn from_target(target: schema::ResolvedModelTarget) -> Self {
        Self::from_target_with_registry(target, AdapterRegistry::default())
    }

    /// Construct a client with an explicit adapter registry. This is the
    /// extension seam for embedding additional protocol implementations
    /// without adding provider-specific branches to the agent loop.
    pub fn from_target_with_registry(
        target: schema::ResolvedModelTarget,
        adapters: AdapterRegistry,
    ) -> Self {
        let http = HttpClient::builder()
            .timeout(std::time::Duration::from_secs(llm_timeout_secs()))
            .build()
            .unwrap_or_else(|_| HttpClient::new());
        Self {
            http,
            generation: RwLock::new(target.generation.clone()),
            target: RwLock::new(Some(target)),
            live_model: None,
            adapters,
        }
    }

    /// Build a session client that owns no provider or model configuration.
    /// Every `stream_model` call re-resolves the canonical reference from the
    /// Agent's authoritative Registry snapshot. Even an unresolved historical
    /// identity gets this client, so it can never fall back to a stale template
    /// provider while waiting for a configured replacement.
    pub fn from_live_model(
        canonical_model: String,
        registry: std::sync::Arc<parking_lot::RwLock<crate::models::Registry>>,
    ) -> Self {
        let http = HttpClient::builder()
            .timeout(std::time::Duration::from_secs(llm_timeout_secs()))
            .build()
            .unwrap_or_else(|_| HttpClient::new());
        Self {
            http,
            target: RwLock::new(None),
            generation: RwLock::new(schema::GenerationConfig::default()),
            live_model: Some(LiveModelSource {
                canonical_model,
                registry,
            }),
            adapters: AdapterRegistry::default(),
        }
    }

    fn target_for_request(&self) -> Result<schema::ResolvedModelTarget> {
        let Some(source) = &self.live_model else {
            let mut target = self
                .target
                .read()
                .clone()
                .ok_or_else(|| anyhow::anyhow!("static model target is unavailable"))?;
            target.generation = self.generation.read().clone();
            return Ok(target);
        };

        // One Registry read produces one coherent provider/model snapshot. A
        // concurrent config commit swaps the whole Registry before publishing
        // its revision, so this request sees either the complete old revision
        // or the complete new one, never mixed key/base-url/model metadata.
        let (_resolved_identity, model, api_key) = source
            .registry
            .read()
            .resolve_request_target(&source.canonical_model)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "model `{}` is no longer available and no configured replacement exists",
                    source.canonical_model
                )
            })?;
        let generation = self.generation.read().clone();
        let mut target = schema::ResolvedModelTarget::from_model(
            &model,
            api_key,
            generation.temperature,
            Some(crate::models::effective_max_tokens(&model)),
        )?;
        target.generation.thinking_level = generation.thinking_level;
        target.generation.thinking_budget = generation.thinking_budget;
        Ok(target)
    }

    pub fn with_thinking_level(self, level: &str) -> Self {
        self.generation.write().thinking_level = level.to_string();
        self
    }

    pub fn with_thinking_budget(self, budget: i32) -> Self {
        self.generation.write().thinking_budget = budget;
        self
    }

    pub fn with_thinking_level_map(self, map: HashMap<String, String>) -> Self {
        if let Some(target) = self.target.write().as_mut() {
            target.capabilities.reasoning.levels = map
                .iter()
                .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
                .collect();
        }
        self
    }

    pub fn with_temperature(self, temperature: f32) -> Self {
        self.generation.write().temperature = Some(temperature);
        self
    }

    pub fn with_max_tokens(self, max_tokens: i32) -> Self {
        self.generation.write().max_output_tokens = Some(max_tokens);
        self
    }
}

#[async_trait::async_trait]
impl crate::types::LLMProvider for Client {
    async fn stream_model(
        &self,
        mut request: schema::ModelRequest,
    ) -> Result<ReceiverStream<schema::ModelStreamEvent>> {
        let target = self.target_for_request()?;
        // The session stores a canonical provider/model reference, while the
        // upstream request always uses the latest resolved provider model id.
        request.model = target.model_id.clone();
        // Modality changes are also request-time provider state. Never mutate
        // the durable conversation when a model temporarily loses image input;
        // adapt only this outbound projection.
        if target.capabilities.supports_image_input {
            for message in &mut request.messages {
                let already_has_image = message
                    .content
                    .iter()
                    .any(|block| matches!(block, crate::types::ContentBlock::Image { .. }));
                if already_has_image {
                    continue;
                }
                let image_paths = message
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get("attachments"))
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|attachment| {
                        attachment.get("kind").and_then(serde_json::Value::as_str) == Some("image")
                    })
                    .filter_map(|attachment| {
                        attachment
                            .get("path")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    })
                    .collect::<Vec<_>>();
                for path in image_paths {
                    if let Some(url) = crate::utils::image_data_url_for_model(&path) {
                        message.content.push(crate::types::ContentBlock::image(url));
                    }
                }
            }
        } else {
            for message in &mut request.messages {
                message
                    .content
                    .retain(|block| !matches!(block, crate::types::ContentBlock::Image { .. }));
            }
        }
        let adapter = self.adapters.get(target.protocol.protocol())?;
        let body = adapter.build_body(&target, &request)?;
        let url = format!(
            "{}{}",
            target.route.base_url.trim_end_matches('/'),
            adapter.endpoint_path()
        );
        let mut builder = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .header(
                "User-Agent",
                concat!("future-agent/", env!("FUTURE_VERSION")),
            );
        for (name, value) in &target.route.headers {
            let lower = name.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "authorization" | "x-api-key" | "content-type" | "anthropic-version"
            ) {
                continue;
            }
            builder = builder.header(name, value);
        }
        builder = match target.route.auth {
            schema::AuthScheme::Bearer => {
                builder.header("Authorization", format!("Bearer {}", target.route.api_key))
            }
            schema::AuthScheme::AnthropicApiKey => {
                let version = match &target.protocol {
                    schema::ProtocolConfig::AnthropicMessages(config) => config.version.as_str(),
                    _ => "2023-06-01",
                };
                builder
                    .header("x-api-key", &target.route.api_key)
                    .header("anthropic-version", version)
            }
        };
        let req = builder.json(&body).build()?;
        let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
        info!(
            protocol = target.protocol.protocol().canonical_name(),
            model = %request.model,
            body_kb = body_bytes.len() / 1024,
            "LLM request"
        );
        let resp = self.http.execute(req).await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(normalize_http_error(
                status.as_u16(),
                &text,
                &request.model,
                body_bytes.len(),
            ));
        }

        let (tx, rx) = mpsc::channel(32);
        let mut stream = resp.bytes_stream();
        tokio::spawn(async move {
            let mut decoder = sse::SseDecoder::default();
            let mut state = adapter.new_stream_state();
            let mut terminal = false;
            loop {
                let next = tokio::select! {
                    _ = tx.closed() => return,
                    next = tokio::time::timeout(
                        std::time::Duration::from_secs(stream_idle_timeout_secs()),
                        stream.next(),
                    ) => next,
                };
                let bytes = match next {
                    Ok(Some(Ok(bytes))) => bytes,
                    Ok(Some(Err(error))) => {
                        let _ = tx
                            .send(schema::ModelStreamEvent::Error {
                                message: format!("{UPSTREAM_DISCONNECTED} {error:#}"),
                            })
                            .await;
                        return;
                    }
                    Ok(None) => break,
                    Err(_) => {
                        let _ = tx
                            .send(schema::ModelStreamEvent::Error {
                                message: format!(
                                    "{UPSTREAM_DISCONNECTED} model response stream was idle for {} seconds",
                                    stream_idle_timeout_secs()
                                ),
                            })
                            .await;
                        return;
                    }
                };
                if tx.is_closed() {
                    return;
                }
                let frames = match decoder.push(&bytes) {
                    Ok(frames) => frames,
                    Err(error) => {
                        let _ = tx
                            .send(schema::ModelStreamEvent::Error {
                                message: format!("{MODEL_RESPONSE_ERROR} {error:#}"),
                            })
                            .await;
                        return;
                    }
                };
                for frame in frames {
                    let events = match adapter.decode_frame(&frame, state.as_mut()) {
                        Ok(events) => events,
                        Err(error) => {
                            let _ = tx
                                .send(schema::ModelStreamEvent::Error {
                                    message: format!(
                                        "{MODEL_RESPONSE_ERROR} invalid provider stream event: {error:#}"
                                    ),
                                })
                                .await;
                            return;
                        }
                    };
                    for event in events {
                        terminal |= matches!(
                            event,
                            schema::ModelStreamEvent::Finish { .. }
                                | schema::ModelStreamEvent::Error { .. }
                        );
                        if tx.send(event).await.is_err() {
                            return;
                        }
                    }
                    if terminal {
                        return;
                    }
                }
            }

            let frames = match decoder.finish() {
                Ok(frames) => frames,
                Err(error) => {
                    let _ = tx
                        .send(schema::ModelStreamEvent::Error {
                            message: format!("{MODEL_RESPONSE_ERROR} {error:#}"),
                        })
                        .await;
                    return;
                }
            };
            for frame in frames {
                let events = match adapter.decode_frame(&frame, state.as_mut()) {
                    Ok(events) => events,
                    Err(error) => {
                        let _ = tx
                            .send(schema::ModelStreamEvent::Error {
                                message: format!(
                                    "{MODEL_RESPONSE_ERROR} invalid provider stream event: {error:#}"
                                ),
                            })
                            .await;
                        return;
                    }
                };
                for event in events {
                    terminal |= matches!(
                        event,
                        schema::ModelStreamEvent::Finish { .. }
                            | schema::ModelStreamEvent::Error { .. }
                    );
                    if tx.send(event).await.is_err() {
                        return;
                    }
                }
            }
            if !terminal {
                match adapter.finish_stream(state.as_mut()) {
                    Ok(events) => {
                        for event in events {
                            if tx.send(event).await.is_err() {
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        let _ = tx
                            .send(schema::ModelStreamEvent::Error {
                                message: format!("{MODEL_RESPONSE_ERROR} {error:#}"),
                            })
                            .await;
                    }
                }
            }
        });
        Ok(ReceiverStream::new(rx))
    }

    fn update_thinking(&self, level: &str, budget: i32) {
        let mut generation = self.generation.write();
        generation.thinking_level = level.to_string();
        generation.thinking_budget = budget;
    }
}

fn normalize_http_error(status: u16, text: &str, model: &str, body_bytes: usize) -> anyhow::Error {
    let parsed = serde_json::from_str::<Value>(text).ok();
    let error = parsed
        .as_ref()
        .and_then(|value| value.get("error"))
        .unwrap_or(&Value::Null);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| parsed.as_ref()?.get("message")?.as_str())
        .unwrap_or(text);
    let code = error.get("code").and_then(Value::as_str).unwrap_or("");
    if status == 400
        && (code == "context_length_exceeded"
            || message.contains("maximum context")
            || message.contains("context_length")
            || message.contains("too long"))
    {
        return anyhow!(
            "[CTX_LIMIT] Request exceeds the model context limit for `{model}` ({} KB). {}",
            body_bytes / 1024,
            message
        );
    }
    match status {
        401 | 403 => anyhow!("Authentication failed (HTTP {status}): {message}"),
        429 => anyhow!("Rate limited (HTTP 429): {message}"),
        _ => anyhow!("LLM API request failed (HTTP {status}): {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::LLMProvider;

    static IDLE_TIMEOUT_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // ─── Client construction and single target state ─────────────────────────

    fn chat_target(
        base_url: &str,
        api_key: &str,
        temperature: Option<f32>,
        max_tokens: Option<i32>,
    ) -> schema::ResolvedModelTarget {
        schema::ResolvedModelTarget::openai_chat_compatible(
            "mock",
            base_url,
            api_key,
            temperature,
            max_tokens,
        )
    }

    #[test]
    fn client_from_target_preserves_openai_chat_configuration() {
        let client = Client::from_target(chat_target(
            "https://api.openai.com",
            "sk-test",
            Some(0.7),
            Some(4096),
        ));
        let target = client.target_for_request().unwrap();
        assert_eq!(target.route.base_url, "https://api.openai.com");
        assert_eq!(target.route.api_key, "sk-test");
        assert_eq!(target.generation.temperature, Some(0.7));
        assert_eq!(target.generation.max_output_tokens, Some(4096));
        assert!(matches!(
            target.protocol,
            schema::ProtocolConfig::OpenAiChat(_)
        ));
    }

    #[test]
    fn builders_update_only_the_resolved_target() {
        let mut levels = HashMap::new();
        levels.insert("xhigh".to_string(), "max".to_string());
        let client = Client::from_target(chat_target("https://api.test", "key", None, None))
            .with_thinking_level("medium")
            .with_thinking_budget(8000)
            .with_thinking_level_map(levels)
            .with_temperature(0.3)
            .with_max_tokens(2048);
        let target = client.target_for_request().unwrap();
        assert_eq!(target.generation.thinking_level, "medium");
        assert_eq!(target.generation.thinking_budget, 8000);
        assert_eq!(target.generation.temperature, Some(0.3));
        assert_eq!(target.generation.max_output_tokens, Some(2048));
        assert_eq!(
            target.capabilities.reasoning.levels["xhigh"],
            serde_json::json!("max")
        );
    }

    #[test]
    fn runtime_thinking_setter_updates_generation_controls() {
        let client = Client::from_target(chat_target("https://api.test", "old-key", None, None));
        crate::types::LLMProvider::update_thinking(&client, "high", 16000);
        let target = client.target_for_request().unwrap();
        assert_eq!(target.route.api_key, "old-key");
        assert_eq!(target.route.base_url, "https://api.test");
        assert_eq!(target.generation.thinking_level, "high");
        assert_eq!(target.generation.thinking_budget, 16000);
    }

    #[test]
    fn live_client_resolves_the_latest_complete_provider_model_snapshot_per_request() {
        fn model(base_url: &str, context_window: i32, max_tokens: i32) -> crate::models::Model {
            crate::models::Model {
                id: "m".into(),
                name: "M".into(),
                provider: "provider-a".into(),
                api: "chat".into(),
                base_url: base_url.into(),
                input: vec!["text".into()],
                output: vec!["text".into()],
                context_window,
                max_tokens,
                ..Default::default()
            }
        }

        let first_model = model("https://old.example", 8_000, 1_000);
        let first_registry = crate::models::Registry::from_models_and_auth(
            vec![first_model.clone()],
            r#"{"provider-a":{"type":"api_key","key":"old-key","base_url":"https://old-auth.example"}}"#,
        );
        let registry = std::sync::Arc::new(parking_lot::RwLock::new(first_registry));
        let client = Client::from_live_model("provider-a/m".into(), registry.clone());
        assert!(
            client.target.read().is_none(),
            "session client caches no target"
        );

        let first = client.target_for_request().unwrap();
        assert_eq!(first.route.api_key, "old-key");
        assert_eq!(first.route.base_url, "https://old-auth.example");
        assert_eq!(first.capabilities.context_window, 8_000);
        assert_eq!(first.generation.max_output_tokens, Some(1_000));

        let mut changed = model("https://new.example", 32_000, 4_000);
        changed.input.push("image".into());
        *registry.write() = crate::models::Registry::from_models_and_auth(
            vec![changed],
            r#"{"provider-a":{"type":"api_key","key":"new-key","base_url":"https://new-auth.example"}}"#,
        );

        let latest = client.target_for_request().unwrap();
        assert_eq!(latest.route.api_key, "new-key");
        assert_eq!(latest.route.base_url, "https://new-auth.example");
        assert_eq!(latest.capabilities.context_window, 32_000);
        assert!(latest.capabilities.supports_image_input);
        assert_eq!(latest.generation.max_output_tokens, Some(4_000));
    }

    #[test]
    fn live_client_never_borrows_another_providers_key() {
        let model = crate::models::Model {
            id: "m".into(),
            name: "M".into(),
            provider: "deepseek".into(),
            api: "chat".into(),
            base_url: "https://api.deepseek.com".into(),
            input: vec!["text".into()],
            output: vec!["text".into()],
            context_window: 8_000,
            max_tokens: 1_000,
            ..Default::default()
        };
        let registry = std::sync::Arc::new(parking_lot::RwLock::new(
            crate::models::Registry::from_models_and_auth(
                vec![model.clone()],
                r#"{"future":{"type":"api_key","key":"future-key"}}"#,
            ),
        ));
        let client = Client::from_live_model("deepseek/m".into(), registry);
        let error = client.target_for_request().unwrap_err();
        assert!(error.to_string().contains("no longer available"));
        assert!(!error.to_string().contains("future-key"));
    }

    #[test]
    fn live_client_replaces_a_removed_historical_model_at_request_time() {
        let old = crate::models::Model {
            id: "old".into(),
            name: "Old".into(),
            provider: "provider-a".into(),
            api: "chat".into(),
            base_url: "https://api.example".into(),
            input: vec!["text".into()],
            output: vec!["text".into()],
            context_window: 8_000,
            max_tokens: 1_000,
            ..Default::default()
        };
        let registry = std::sync::Arc::new(parking_lot::RwLock::new(
            crate::models::Registry::from_models_and_auth(
                vec![old.clone()],
                r#"{"provider-a":{"type":"api_key","key":"old-key"}}"#,
            ),
        ));
        let client = Client::from_live_model("provider-a/old".into(), registry.clone());

        let mut replacement = old;
        replacement.id = "new".into();
        replacement.name = "New".into();
        replacement.context_window = 64_000;
        *registry.write() = crate::models::Registry::from_models_and_auth(
            vec![replacement],
            r#"{"provider-a":{"type":"api_key","key":"new-key"}}"#,
        );

        let target = client.target_for_request().unwrap();
        assert_eq!(target.model_id, "new");
        assert_eq!(target.route.api_key, "new-key");
        assert_eq!(target.capabilities.context_window, 64_000);
    }

    #[test]
    fn future_platform_api_root_does_not_override_the_model_v1_route() {
        let model = crate::models::Model {
            id: "deepseek-v4-pro".into(),
            name: "DeepSeek V4 Pro".into(),
            provider: "future".into(),
            api: "openai-completions".into(),
            base_url: "https://test.future-os.cn/api/v1".into(),
            input: vec!["text".into()],
            output: vec!["text".into()],
            context_window: 128_000,
            max_tokens: 16_384,
            ..Default::default()
        };
        let registry = std::sync::Arc::new(parking_lot::RwLock::new(
            crate::models::Registry::from_models_and_auth(
                vec![model],
                r#"{"future":{"type":"api_key","key":"future-key","base_url":"https://test.future-os.cn/api"}}"#,
            ),
        ));
        let client = Client::from_live_model("future/deepseek-v4-pro".into(), registry);

        let target = client.target_for_request().unwrap();
        assert_eq!(target.route.base_url, "https://test.future-os.cn/api/v1");
        assert_eq!(target.route.api_key, "future-key");
    }

    #[test]
    fn normalize_http_error_preserves_retry_and_auth_semantics() {
        assert!(normalize_http_error(
            400,
            r#"{"error":{"code":"context_length_exceeded","message":"too long"}}"#,
            "m",
            1024,
        )
        .to_string()
        .starts_with("[CTX_LIMIT]"));
        assert!(normalize_http_error(401, "nope", "m", 0)
            .to_string()
            .contains("Authentication failed"));
        assert!(normalize_http_error(403, "nope", "m", 0)
            .to_string()
            .contains("Authentication failed"));
        assert!(normalize_http_error(429, "slow down", "m", 0)
            .to_string()
            .contains("Rate limited"));
    }

    #[test]
    fn stream_idle_timeout_defaults_when_override_absent() {
        let _env = IDLE_TIMEOUT_ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::remove_var("FUTURE_TEST_STREAM_IDLE_SECS");
        assert_eq!(stream_idle_timeout_secs(), STREAM_IDLE_TIMEOUT_SECS);
    }

    // ─── mock HTTP server ───────────────────────────────────────────────────
    /// One-shot HTTP server: accepts a single request, records its body, and
    /// replies with a canned (status, content_type, body). Loops so aborted
    /// probe connections don't consume the one real response.
    struct MockServer {
        base_url: String,
        requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        raw_requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    fn mock_server(
        respond: impl Fn(&str) -> (u16, &'static str, String) + Send + 'static,
    ) -> MockServer {
        mock_server_with_timeout(respond, std::time::Duration::from_secs(10))
    }

    /// `mock_server` with a tunable read timeout, so tests can drive the
    /// read-failure arms with silent connections in milliseconds.
    fn mock_server_with_timeout(
        respond: impl Fn(&str) -> (u16, &'static str, String) + Send + 'static,
        read_timeout: std::time::Duration,
    ) -> MockServer {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let raw_requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_raw = raw_requests.clone();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for _ in 0..16 {
                // Blocking accept on a live listener does not error; surplus
                // iterations park here until process exit reaps the thread.
                let (mut stream, _) = listener.accept().expect("mock server accept");
                let _ = stream.set_read_timeout(Some(read_timeout));
                let mut buf: Vec<u8> = Vec::new();
                let mut chunk = [0u8; 8192];
                // Read until end of headers.
                let mut header_end: Option<usize> = None;
                while header_end.is_none() && buf.len() <= 1_000_000 {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            header_end =
                                buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
                        }
                        Err(_) => break,
                    }
                }
                let Some(header_end) = header_end else {
                    continue; // aborted probe connection
                };
                let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
                let content_length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse().ok())
                    })
                    .unwrap_or(0);
                // Read the remaining body bytes.
                while buf.len() < header_end + content_length {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        Err(_) => break,
                    }
                }
                let body = String::from_utf8_lossy(&buf[header_end..]).to_string();
                captured.lock().unwrap().push(body.clone());
                captured_raw
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf).to_string());
                let (status, content_type, response_body) = respond(&body);
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                // Best effort: a client that went away mid-request doesn't
                // consume further accepts (aborted probes are expected).
                let _ = stream
                    .write_all(response.as_bytes())
                    .and_then(|_| stream.flush());
            }
        });
        MockServer {
            base_url: format!("http://127.0.0.1:{port}"),
            requests,
            raw_requests,
        }
    }

    fn protocol_target(
        base_url: &str,
        protocol: schema::ProtocolConfig,
    ) -> schema::ResolvedModelTarget {
        schema::ResolvedModelTarget {
            model_id: "mock".into(),
            route: schema::ProviderRoute {
                provider_id: "fixture".into(),
                base_url: base_url.into(),
                api_key: "secret".into(),
                auth: if matches!(protocol, schema::ProtocolConfig::AnthropicMessages(_)) {
                    schema::AuthScheme::AnthropicApiKey
                } else {
                    schema::AuthScheme::Bearer
                },
                headers: Default::default(),
            },
            protocol,
            capabilities: schema::ModelCapabilities::default(),
            generation: schema::GenerationConfig {
                max_output_tokens: Some(256),
                ..Default::default()
            },
        }
    }

    fn canonical_request() -> schema::ModelRequest {
        schema::ModelRequest {
            model: "mock".into(),
            system_prompt: "system".into(),
            messages: vec![crate::types::AgentMessage::new_user(
                "user",
                serde_json::json!("hello"),
            )],
            tools: Vec::new(),
        }
    }

    #[tokio::test]
    async fn responses_transport_uses_native_endpoint_and_events() {
        let sse = concat!(
            "event: response.output_text.delta\r\n",
            "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_1\",\"delta\":\"hello\"}\r\n\r\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":2,\"output_tokens\":1,\"total_tokens\":3}}}\n\n"
        );
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::from_target(protocol_target(
            &server.base_url,
            schema::ProtocolConfig::OpenAiResponses(schema::OpenAiResponsesConfig::default()),
        ));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        assert!(events.iter().any(|event| matches!(
            event,
            schema::ModelStreamEvent::TextDelta { text, .. } if text == "hello"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            schema::ModelStreamEvent::Finish {
                reason: schema::FinishReason::Stop,
                ..
            }
        )));
        let requests = server.requests.lock().unwrap();
        assert!(requests[0].contains("\"store\":false"));
        let raw_requests = server.raw_requests.lock().unwrap();
        let raw = raw_requests[0].to_ascii_lowercase();
        assert!(raw.starts_with("post /responses "), "{}", raw_requests[0]);
        assert!(raw.contains("authorization: bearer secret"));
    }

    #[tokio::test]
    async fn anthropic_transport_uses_native_headers_and_events() {
        let sse = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":2,\"output_tokens\":0}}}\n\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
            "event: content_block_stop\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::from_target(protocol_target(
            &server.base_url,
            schema::ProtocolConfig::AnthropicMessages(schema::AnthropicMessagesConfig::default()),
        ));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        assert!(events.iter().any(|event| matches!(
            event,
            schema::ModelStreamEvent::TextDelta { text, .. } if text == "hello"
        )));
        let raw_requests = server.raw_requests.lock().unwrap();
        let raw = raw_requests[0].to_ascii_lowercase();
        assert!(raw.starts_with("post /messages "), "{}", raw_requests[0]);
        assert!(raw.contains("x-api-key: secret"));
        assert!(raw.contains("anthropic-version: 2023-06-01"));
        assert!(!raw.contains("authorization:"));
    }

    #[tokio::test]
    async fn canonical_chat_stream_uses_chat_completions_transport() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let server = mock_server(move |_| (200, "text/event-stream", sse.to_string()));
        let client = Client::from_target(chat_target(&server.base_url, "secret", None, None));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        assert!(events
            .iter()
            .any(|event| matches!(event, schema::ModelStreamEvent::TextDelta { text, .. } if text == "hello")));
        assert!(events.iter().any(|event| matches!(
            event,
            schema::ModelStreamEvent::Finish {
                reason: schema::FinishReason::Stop,
                ..
            }
        )));
        let raw_requests = server.raw_requests.lock().unwrap();
        assert!(raw_requests[0]
            .to_ascii_lowercase()
            .starts_with("post /chat/completions "));
    }

    #[tokio::test]
    async fn stream_model_reports_invalid_provider_json_as_an_error_event() {
        let server = mock_server(|_| (200, "text/event-stream", "data: {not json}\n\n".into()));
        let client = Client::from_target(chat_target(&server.base_url, "secret", None, None));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        assert!(matches!(
            events.as_slice(),
            [schema::ModelStreamEvent::Error { message }]
                if message.starts_with(MODEL_RESPONSE_ERROR)
                    && message.contains("invalid provider stream event:")
        ));
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn stream_model_idle_timeout_reports_upstream_disconnect() {
        let _env = IDLE_TIMEOUT_ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::set_var("FUTURE_TEST_STREAM_IDLE_SECS", "1");

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 16 * 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
                )
                .unwrap();
            let frame = b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
            stream
                .write_all(format!("{:X}\r\n", frame.len()).as_bytes())
                .and_then(|_| stream.write_all(frame))
                .and_then(|_| stream.write_all(b"\r\n"))
                .and_then(|_| stream.flush())
                .unwrap();
            std::thread::sleep(std::time::Duration::from_secs(2));
        });

        let client = Client::from_target(chat_target(
            &format!("http://127.0.0.1:{port}"),
            "secret",
            None,
            None,
        ));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        std::env::remove_var("FUTURE_TEST_STREAM_IDLE_SECS");

        assert!(events.iter().any(|event| matches!(
            event,
            schema::ModelStreamEvent::TextDelta { text, .. } if text == "partial"
        )));
        assert!(matches!(
            events.last(),
            Some(schema::ModelStreamEvent::Error { message })
                if message.starts_with(UPSTREAM_DISCONNECTED)
        ));
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn stream_model_keepalives_prevent_idle_timeout() {
        let _env = IDLE_TIMEOUT_ENV_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::set_var("FUTURE_TEST_STREAM_IDLE_SECS", "1");

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 16 * 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
                )
                .unwrap();
            for _ in 0..6 {
                stream
                    .write_all(b"3\r\n:\n\n\r\n")
                    .and_then(|_| stream.flush())
                    .unwrap();
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            let frame = b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
            stream
                .write_all(format!("{:X}\r\n", frame.len()).as_bytes())
                .and_then(|_| stream.write_all(frame))
                .and_then(|_| stream.write_all(b"\r\n0\r\n\r\n"))
                .and_then(|_| stream.flush())
                .unwrap();
        });

        let client = Client::from_target(chat_target(
            &format!("http://127.0.0.1:{port}"),
            "secret",
            None,
            None,
        ));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        std::env::remove_var("FUTURE_TEST_STREAM_IDLE_SECS");

        assert!(matches!(
            events.last(),
            Some(schema::ModelStreamEvent::Finish {
                reason: schema::FinishReason::Stop,
                ..
            })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stream_model_consumer_drop_closes_the_upstream_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let disconnected = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed_disconnect = disconnected.clone();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 16 * 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
                )
                .unwrap();
            for _ in 0..20 {
                if stream
                    .write_all(b"3\r\n:\n\n\r\n")
                    .and_then(|_| stream.flush())
                    .is_err()
                {
                    observed_disconnect.store(true, std::sync::atomic::Ordering::Release);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        });

        let client = Client::from_target(chat_target(
            &format!("http://127.0.0.1:{port}"),
            "secret",
            None,
            None,
        ));
        let stream = client.stream_model(canonical_request()).await.unwrap();
        drop(stream);

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !disconnected.load(std::sync::atomic::Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("consumer drop should close the upstream stream promptly");
    }

    #[tokio::test]
    async fn stream_model_reports_body_read_errors() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 16 * 1024];
            let _ = stream.read(&mut request);
            // Promise more bytes than we send, then close: reqwest reports a
            // body read error while draining the response stream.
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 100\r\nConnection: close\r\n\r\ndata: x",
                )
                .unwrap();
            stream.flush().unwrap();
        });

        let client = Client::from_target(chat_target(
            &format!("http://127.0.0.1:{port}"),
            "secret",
            None,
            None,
        ));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        assert!(matches!(
            events.as_slice(),
            [schema::ModelStreamEvent::Error { .. }]
        ));
    }

    #[tokio::test]
    async fn stream_model_flushes_buffered_frame_on_clean_eof() {
        // No trailing blank line: the final SSE frame is only flushed by
        // `decoder.finish()` once the upstream stream ends without a terminal
        // event, and `finish_stream` then emits an incomplete finish.
        let server = mock_server(|_| {
            (
                200,
                "text/event-stream",
                "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}".into(),
            )
        });
        let client = Client::from_target(chat_target(&server.base_url, "secret", None, None));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        assert!(events.iter().any(|event| matches!(
            event,
            schema::ModelStreamEvent::TextDelta { text, .. } if text == "hi"
        )));
        assert!(matches!(
            events.last(),
            Some(schema::ModelStreamEvent::Finish {
                reason: schema::FinishReason::Incomplete,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn stream_model_flushes_done_marker_without_trailing_newline() {
        // A trailing `[DONE]` without a blank line is only decoded by
        // `decoder.finish()`, and marks the stream terminal so `finish_stream`
        // is skipped.
        let server = mock_server(|_| (200, "text/event-stream", "data: [DONE]".into()));
        let client = Client::from_target(chat_target(&server.base_url, "secret", None, None));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        assert!(matches!(
            events.as_slice(),
            [schema::ModelStreamEvent::Finish {
                reason: schema::FinishReason::Incomplete,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn stream_model_reports_invalid_json_in_buffered_tail() {
        // Invalid JSON flushed by `decoder.finish()` is a malformed model
        // response, not a clean incomplete finish.
        let server = mock_server(|_| (200, "text/event-stream", "data: {not json}".into()));
        let client = Client::from_target(chat_target(&server.base_url, "secret", None, None));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        assert!(matches!(
            events.as_slice(),
            [schema::ModelStreamEvent::Error { message }]
                if message.starts_with(MODEL_RESPONSE_ERROR)
        ));
    }

    #[tokio::test]
    async fn stream_model_reports_invalid_utf8_in_buffered_tail() {
        // Invalid UTF-8 flushed by `decoder.finish()` is a malformed model
        // response, not a clean incomplete finish.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 16 * 1024];
            let _ = stream.read(&mut request);
            let body = [0xffu8, 0xfe];
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            stream.write_all(&body).unwrap();
            stream.flush().unwrap();
        });

        let client = Client::from_target(chat_target(
            &format!("http://127.0.0.1:{port}"),
            "secret",
            None,
            None,
        ));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        assert!(matches!(
            events.as_slice(),
            [schema::ModelStreamEvent::Error { message }]
                if message.starts_with(MODEL_RESPONSE_ERROR)
        ));
    }

    #[tokio::test]
    async fn stream_model_reports_invalid_utf8_in_sse() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 16 * 1024];
            let _ = stream.read(&mut request);
            let body = [0xffu8, 0xfe, b'\n'];
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            stream.write_all(&body).unwrap();
            stream.flush().unwrap();
        });

        let client = Client::from_target(chat_target(
            &format!("http://127.0.0.1:{port}"),
            "secret",
            None,
            None,
        ));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        assert!(matches!(
            events.as_slice(),
            [schema::ModelStreamEvent::Error { message }]
                if message.contains("invalid UTF-8")
        ));
    }

    #[test]
    fn normalize_http_error_ctx_limit_variants_and_generic_failure() {
        // Each short-circuit OR operand that classifies a 400 as CTX_LIMIT.
        assert!(normalize_http_error(400, "maximum context", "m", 0)
            .to_string()
            .starts_with("[CTX_LIMIT]"));
        assert!(normalize_http_error(400, "context_length issue", "m", 0)
            .to_string()
            .starts_with("[CTX_LIMIT]"));
        assert!(normalize_http_error(400, "too long", "m", 0)
            .to_string()
            .starts_with("[CTX_LIMIT]"));
        // Generic failure arm.
        assert!(normalize_http_error(500, "boom", "m", 0)
            .to_string()
            .contains("HTTP 500"));
    }

    #[tokio::test]
    async fn mock_server_survives_aborted_connections() {
        let server = mock_server_with_timeout(
            |_| (200, "text/event-stream", "data: [DONE]\n\n".into()),
            std::time::Duration::from_millis(100),
        );
        let addr = server.base_url.clone();
        let host_port = addr.strip_prefix("http://").unwrap();
        use std::io::Write;

        // (1) Connect then close immediately: EOF during header read (Ok(0)).
        drop(std::net::TcpStream::connect(host_port).unwrap());

        // (2) Partial header then stall past read timeout: Err during header read.
        {
            let mut stream = std::net::TcpStream::connect(host_port).unwrap();
            stream.write_all(b"POST / HTTP/1.1\r\nHost: x\r\n").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        // (3) Full headers + partial body then close: EOF during body read.
        {
            let mut stream = std::net::TcpStream::connect(host_port).unwrap();
            stream
                .write_all(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 50\r\n\r\npartial")
                .unwrap();
        }

        // (4) Full headers then a delayed body then close: Ok(n) + EOF in body read.
        {
            let mut stream = std::net::TcpStream::connect(host_port).unwrap();
            stream
                .write_all(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 50\r\n\r\n")
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(50));
            stream.write_all(b"partial").unwrap();
        }

        // (5) Full headers + partial body then stall: Err during body read.
        {
            let mut stream = std::net::TcpStream::connect(host_port).unwrap();
            stream
                .write_all(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 50\r\n\r\npartial")
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        std::thread::sleep(std::time::Duration::from_millis(300));

        // A real request still succeeds after all the aborted probes.
        let client = Client::from_target(chat_target(&server.base_url, "secret", None, None));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        assert!(!events.is_empty());
    }
}

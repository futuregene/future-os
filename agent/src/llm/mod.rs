//! LLM Client — 1:1 compatible with internal/llm/
//!
//! Uses reqwest for HTTP + SSE streaming, matching Go's OpenAI SDK behavior.

pub(crate) mod adapters;
pub mod schema;
mod sse;
#[cfg(test)]
mod stream_wait_tests;
use adapters::AdapterRegistry;
use anyhow::{anyhow, Result};
use futures::StreamExt;
use parking_lot::RwLock;
use reqwest::Client as HttpClient;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;

const DEFAULT_TIMEOUT_SECS: u64 = 1800;
pub(crate) const UPSTREAM_DISCONNECTED: &str = "[UPSTREAM_DISCONNECTED]";
const MODEL_RESPONSE_ERROR: &str = "[MODEL_RESPONSE_ERROR]";

fn reqwest_stream_error_kind(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_body() {
        "body"
    } else if error.is_decode() {
        "decode"
    } else if error.is_request() {
        "request"
    } else {
        "unknown"
    }
}

fn error_source_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut causes = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        causes.push(cause.to_string());
        source = cause.source();
    }
    if causes.is_empty() {
        "none".to_string()
    } else {
        causes.join(" <- ")
    }
}

fn stream_progress(
    kind: &str,
    chunks: u64,
    bytes: u64,
    frames: u64,
    started_at: Instant,
) -> String {
    format!(
        "kind={kind}, chunks={chunks}, bytes={bytes}, frames={frames}, elapsed_ms={}",
        started_at.elapsed().as_millis()
    )
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

#[derive(Clone)]
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
            request = tokio::task::spawn_blocking(move || {
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
                            attachment.get("kind").and_then(serde_json::Value::as_str)
                                == Some("image")
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
                request
            })
            .await
            .map_err(|error| anyhow::anyhow!("image preparation worker failed: {error}"))?;
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
        let resp = self.http.execute(req).await.map_err(|error| {
            if error.is_timeout() {
                anyhow!("[RESPONSE_TIMEOUT] {error}")
            } else {
                anyhow!(error)
            }
        })?;
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
        let response_status = status.as_u16();
        let protocol = target.protocol.protocol().canonical_name().to_string();
        let model = request.model.clone();
        let mut stream = resp.bytes_stream();
        tokio::spawn(async move {
            let started_at = Instant::now();
            let mut chunks_received = 0_u64;
            let mut bytes_received = 0_u64;
            let mut frames_decoded = 0_u64;
            let mut decoder = sse::SseDecoder::default();
            let mut state = adapter.new_stream_state();
            // The model's protocol-level Finish only ends its text/tool stream.
            // OpenAI-compatible providers can legally send a usage-only frame
            // after that finish_reason, and token accounting would be lost if
            // the byte-stream pump stopped at Finish instead of `[DONE]`.
            let mut protocol_terminal = false;
            let mut transport_error = None;
            loop {
                let next = tokio::select! {
                    _ = tx.closed() => return,
                    // Silence is not evidence of failure: reasoning can pause
                    // without emitting summaries or even transport heartbeats.
                    // reqwest still enforces the whole-request deadline.
                    next = stream.next() => next,
                };
                let bytes = match next {
                    Some(Ok(bytes)) => {
                        chunks_received = chunks_received.saturating_add(1);
                        bytes_received = bytes_received.saturating_add(bytes.len() as u64);
                        bytes
                    }
                    Some(Err(error)) => {
                        // Chat may still be waiting for usage after finish_reason.
                        // Losing that accounting tail must not replay a response
                        // the provider already completed (and its tool calls).
                        if protocol_terminal {
                            tracing::warn!(protocol = %protocol, model = %model, error = %error,
                                "LLM stream disconnected after terminal event");
                            return;
                        }
                        let kind = reqwest_stream_error_kind(&error);
                        let causes = error_source_chain(&error);
                        let progress = stream_progress(
                            kind,
                            chunks_received,
                            bytes_received,
                            frames_decoded,
                            started_at,
                        );
                        tracing::error!(
                            protocol = %protocol,
                            model = %model,
                            status = response_status,
                            error_kind = kind,
                            chunks_received,
                            bytes_received,
                            frames_decoded,
                            elapsed_ms = started_at.elapsed().as_millis() as u64,
                            error = %error,
                            error_debug = ?error,
                            cause_chain = %causes,
                            "LLM response stream disconnected"
                        );
                        let category = if error.is_timeout() {
                            "[RESPONSE_TIMEOUT]"
                        } else {
                            UPSTREAM_DISCONNECTED
                        };
                        // Use the same adapter cleanup as clean EOF before
                        // reporting the disconnect. Otherwise unfinished tool
                        // item ids leak into the retry's request history.
                        transport_error =
                            Some(format!("{category} {error}; {progress}, causes={causes}"));
                        break;
                    }
                    None => break,
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
                frames_decoded = frames_decoded.saturating_add(frames.len() as u64);
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
                        protocol_terminal |= matches!(
                            event,
                            schema::ModelStreamEvent::Finish { .. }
                                | schema::ModelStreamEvent::Error { .. }
                        );
                        if tx.send(event).await.is_err() {
                            return;
                        }
                    }
                    // A logical Finish is not always the wire terminator:
                    // Chat Completions may still owe us a usage-only frame.
                    // Let the adapter decide when no more data is required.
                    if adapter.is_stream_complete(state.as_ref()) {
                        tracing::debug!(protocol = %protocol, model = %model,
                            frames_decoded, bytes_received, "LLM protocol stream terminator received");
                        return;
                    }
                }
            }

            // Only clean EOF can flush an unterminated SSE frame. After a
            // transport error the buffered tail may be cut mid-JSON/UTF-8; do
            // not decode it or turn a retryable disconnect into a parse error.
            let frames = if transport_error.is_some() {
                Vec::new()
            } else {
                match decoder.finish() {
                    Ok(frames) => frames,
                    Err(error) => {
                        let _ = tx
                            .send(schema::ModelStreamEvent::Error {
                                message: format!("{MODEL_RESPONSE_ERROR} {error:#}"),
                            })
                            .await;
                        return;
                    }
                }
            };
            frames_decoded = frames_decoded.saturating_add(frames.len() as u64);
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
                    protocol_terminal |= matches!(
                        event,
                        schema::ModelStreamEvent::Finish { .. }
                            | schema::ModelStreamEvent::Error { .. }
                    );
                    if tx.send(event).await.is_err() {
                        return;
                    }
                }
            }
            tracing::debug!(protocol = %protocol, model = %model, protocol_terminal,
                frames_decoded, bytes_received, "LLM transport reached EOF");
            if !protocol_terminal {
                if transport_error.is_none() {
                    tracing::warn!(
                        protocol = %protocol,
                        model = %model,
                        status = response_status,
                        chunks_received,
                        bytes_received,
                        frames_decoded,
                        elapsed_ms = started_at.elapsed().as_millis() as u64,
                        "LLM response stream reached EOF before a terminal event"
                    );
                }
                match adapter.finish_stream(state.as_mut()) {
                    Ok(events) => {
                        for event in events {
                            // EOF/errors are transport interruptions, not provider-declared
                            // incomplete responses (e.g. an exhausted output budget).
                            // Keep the adapter's closing blocks, but make the missing
                            // terminal frame distinguishable for stream retries.
                            let event = if let schema::ModelStreamEvent::Finish { usage, .. } =
                                event
                            {
                                if let Some(usage) = usage {
                                    if tx
                                        .send(schema::ModelStreamEvent::Usage(usage))
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                                schema::ModelStreamEvent::Error {
                                    message: transport_error.clone().unwrap_or_else(|| format!(
                                        "{UPSTREAM_DISCONNECTED} stream ended before a terminal event; {}",
                                        stream_progress("eof", chunks_received, bytes_received, frames_decoded, started_at),
                                    )),
                                }
                            } else {
                                event
                            };
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

    async fn stream_model_with_output_limit(
        &self,
        request: schema::ModelRequest,
        max_output_tokens: i32,
    ) -> Result<ReceiverStream<schema::ModelStreamEvent>> {
        anyhow::ensure!(
            max_output_tokens > 0,
            "summary output limit must be positive"
        );
        let mut target = self.target_for_request()?;
        let capped = if target.capabilities.max_output_tokens > 0 {
            max_output_tokens.min(target.capabilities.max_output_tokens)
        } else {
            max_output_tokens
        };
        target.generation.max_output_tokens = Some(capped);
        let limited = Self {
            http: self.http.clone(),
            generation: RwLock::new(target.generation.clone()),
            target: RwLock::new(Some(target)),
            live_model: None,
            adapters: self.adapters.clone(),
        };
        limited.stream_model(request).await
    }

    fn snapshot(&self) -> Option<std::sync::Arc<dyn crate::types::LLMProvider>> {
        Some(std::sync::Arc::new(Self {
            http: self.http.clone(),
            target: RwLock::new(self.target.read().clone()),
            generation: RwLock::new(self.generation.read().clone()),
            live_model: self.live_model.clone(),
            adapters: self.adapters.clone(),
        }))
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

    #[tokio::test]
    async fn provider_snapshot_sends_frozen_thinking_on_the_real_http_path() {
        let server = mock_server(|_| {
            (
                200,
                "text/event-stream",
                "data: {\"type\":\"response.completed\",\"response\":{\"output\":[]}}\n\n".into(),
            )
        });
        let mut target = chat_target(&server.base_url, "test-only-key", None, None);
        target.protocol = schema::ProtocolConfig::OpenAiResponses(Default::default());
        target.capabilities.reasoning.supported = true;
        target.generation.thinking_level = "medium".into();
        let client = Client::from_target(target);
        let snapshot = crate::types::LLMProvider::snapshot(&client).unwrap();
        crate::types::LLMProvider::update_thinking(&client, "high", 16000);
        let _events: Vec<_> = snapshot
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        let requests = server.requests.lock().unwrap();
        let body: Value = serde_json::from_str(&requests[0]).unwrap();
        assert_eq!(body["reasoning"]["effort"], "medium");
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
        let snapshot = crate::types::LLMProvider::snapshot(&client).unwrap();
        snapshot.update_thinking("low", 4000);
        let original = client.target_for_request().unwrap();
        assert_eq!(original.generation.thinking_level, "high");
        assert_eq!(original.generation.thinking_budget, 16000);
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

    // ─── transport error classification and the image projection ─────────────

    /// Every arm of `reqwest_stream_error_kind` is reachable from a real socket,
    /// and the classification is what an operator reads to tell "the network went
    /// away" from "the provider sent garbage". Each expectation is paired with the
    /// independent `reqwest` predicate so the test fails if either side drifts.
    #[tokio::test]
    async fn a_transport_error_is_classified_by_the_kind_that_caused_it() {
        use std::io::{Read, Write};

        // Nothing listening: a connect error.
        let closed = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let error = reqwest::Client::new()
            .get(format!("http://{closed}/"))
            .send()
            .await
            .unwrap_err();
        assert!(error.is_connect(), "{error}");
        assert_eq!(reqwest_stream_error_kind(&error), "connect");
        assert!(
            !error_source_chain(&error).is_empty(),
            "a connect error has a source chain to report"
        );

        // A body that stops short of its declared length. `hyper` surfaces this as
        // a *decode* failure (the message is incomplete), which the classifier must
        // report as such rather than claiming the peer closed the connection.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4096\r\n\r\nshort");
            let _ = stream.flush();
        });
        let error = reqwest::Client::new()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap_err();
        assert!(error.is_decode(), "expected a decode error: {error:?}");
        assert_eq!(reqwest_stream_error_kind(&error), "decode");

        // A chunked body whose chunk header is not a number is *also* surfaced as
        // a decode failure by this hyper/reqwest pair (`Invalid chunk size line`),
        // not as `is_body`. Pin both facts: the mapping follows the predicate, and
        // this toolchain never reports a framing failure as a body error.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let _ =
                stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nZZZ\r\n");
            let _ = stream.flush();
        });
        let chunked = reqwest::Client::new()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap_err();
        assert!(chunked.is_decode(), "expected a decode error: {chunked:?}");
        assert!(!chunked.is_body());
        assert_eq!(reqwest_stream_error_kind(&chunked), "decode");

        // A URL the client cannot build a request from is a `Builder` error, which
        // is *outside* every named transport kind — so the catch-all must take it
        // too, and the kind must stay the classifier's own fallback rather than a
        // guess at what the peer did.
        let error = reqwest::Client::new()
            .get("http://this host name has spaces/")
            .send()
            .await
            .unwrap_err();
        assert!(
            !error.is_timeout()
                && !error.is_connect()
                && !error.is_body()
                && !error.is_decode()
                && !error.is_request(),
            "an unbuildable request must not be reported as a transport kind: {error:?}"
        );
        assert_eq!(reqwest_stream_error_kind(&error), "unknown");

        // The table is total and ordered: for any error this crate can hand it, the
        // result is exactly the first matching predicate. Data-driven so the check
        // itself introduces no untested arm.
        for candidate in [&error, &chunked] {
            let expected = [
                (candidate.is_timeout(), "timeout"),
                (candidate.is_connect(), "connect"),
                (candidate.is_body(), "body"),
                (candidate.is_decode(), "decode"),
                (candidate.is_request(), "request"),
            ]
            .into_iter()
            .find_map(|(matches, name)| matches.then_some(name))
            .unwrap_or("unknown");
            assert_eq!(reqwest_stream_error_kind(candidate), expected);
        }

        // An error the classifier does not know must not be mislabelled as a
        // transport kind it is not: an unhandled redirect takes the catch-all.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for _ in 0..8 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                let location = format!("http://{addr}/again");
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n"
                    )
                    .as_bytes(),
                );
                let _ = stream.flush();
            }
        });
        let error = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(1))
            .build()
            .unwrap()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .unwrap_err();
        assert!(
            !error.is_timeout()
                && !error.is_connect()
                && !error.is_body()
                && !error.is_decode()
                && !error.is_request(),
            "the catch-all arm needs an error outside every named kind: {error:?}"
        );
        assert_eq!(reqwest_stream_error_kind(&error), "unknown");
    }

    /// `error_source_chain` reports `none` rather than an empty string when an
    /// error has no source, because the value is embedded in a diagnostic line
    /// where an empty field would read as "the chain was not collected".
    #[test]
    fn an_error_without_a_source_chain_says_none() {
        #[derive(Debug)]
        struct Bare;
        impl std::fmt::Display for Bare {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "bare")
            }
        }
        impl std::error::Error for Bare {}

        assert_eq!(error_source_chain(&Bare), "none");
        let chained = anyhow::Error::new(Bare).context("outer");
        assert_eq!(error_source_chain(chained.as_ref()), "bare");
    }

    /// Providers choose modalities per request: a model that accepts images gets
    /// the session's image attachments, and one that does not must have the image
    /// blocks *stripped* rather than sent (which the provider rejects). A message
    /// that already carries an image block is left alone instead of being
    /// duplicated by its own attachment metadata.
    #[tokio::test]
    async fn the_image_projection_attaches_attachments_strips_for_text_only_and_never_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]))
            .save(&path)
            .unwrap();
        let path = path.to_string_lossy().to_string();

        let attachment = || {
            let mut metadata = serde_json::Map::new();
            metadata.insert(
                "attachments".to_string(),
                serde_json::json!([
                    {"kind": "image", "path": path},
                    {"kind": "file", "path": path}
                ]),
            );
            metadata
        };
        let request = || schema::ModelRequest {
            model: "mock".to_string(),
            system_prompt: "sys".to_string(),
            messages: vec![
                crate::types::AgentMessage {
                    role: "user".to_string(),
                    content: vec![crate::types::ContentBlock::text("look")],
                    metadata: Some(attachment()),
                    ..Default::default()
                },
                // Already has an image, and metadata that would add a second one.
                crate::types::AgentMessage {
                    role: "user".to_string(),
                    content: vec![crate::types::ContentBlock::image(
                        "data:image/png;base64,AAAA",
                    )],
                    metadata: Some(attachment()),
                    ..Default::default()
                },
            ],
            tools: vec![],
        };

        // A vision model sees both attachments (and the text stays first).
        let server = mock_server(|_| {
            (
                200,
                "text/event-stream",
                "data: {\"type\":\"response.completed\",\"response\":{\"output\":[]}}\n\n".into(),
            )
        });
        let mut target = chat_target(&server.base_url, "k", None, None);
        target.capabilities.supports_image_input = true;
        let client = Client::from_target(target);
        let _events: Vec<_> = client
            .stream_model(request())
            .await
            .unwrap()
            .collect()
            .await;
        let body: Value = {
            let requests = server.requests.lock().unwrap();
            serde_json::from_str(&requests[0]).unwrap()
        };
        let images = |body: &Value| body.to_string().matches("data:image/png;base64,").count();
        // Message 1 keeps exactly the image it already carried: the attachment
        // metadata must not add a second one, and the non-image attachment must
        // not become an image either (three would mean a duplicate).
        assert_eq!(
            images(&body),
            2,
            "one image per message, no duplicates: {body}"
        );
        assert!(body.to_string().contains("look"), "text survives: {body}");

        // A text-only model gets no image block at all, not even the one the
        // conversation already carried.
        let server = mock_server(|_| {
            (
                200,
                "text/event-stream",
                "data: {\"type\":\"response.completed\",\"response\":{\"output\":[]}}\n\n".into(),
            )
        });
        let mut target = chat_target(&server.base_url, "k", None, None);
        target.capabilities.supports_image_input = false;
        let client = Client::from_target(target);
        let _events: Vec<_> = client
            .stream_model(request())
            .await
            .unwrap()
            .collect()
            .await;
        let body: Value = {
            let requests = server.requests.lock().unwrap();
            serde_json::from_str(&requests[0]).unwrap()
        };
        let serialized = body.to_string();
        assert!(
            !serialized.contains("base64"),
            "a text-only model must be sent no image payload: {body}"
        );
        assert!(serialized.contains("look"), "the text survives: {body}");
    }

    /// A request that exceeds its HTTP deadline is reported as a *timeout*, not as
    /// a generic request failure: the run loop turns that code into
    /// `request_timeout` so an orchestrator can decide whether to resume.
    #[tokio::test]
    async fn a_request_deadline_is_reported_as_a_response_timeout() {
        // A server that accepts and then says nothing: only the client deadline
        // can end this request, and it must end it as a timeout.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for _ in 0..4 {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                held.push(stream); // keep the connection open, write nothing
            }
        });
        let target = chat_target(&format!("http://{addr}"), "k", None, None);
        let mut client = Client::from_target(target);
        client.http = HttpClient::builder()
            .timeout(std::time::Duration::from_millis(120))
            .build()
            .unwrap();

        let error = client.stream_model(canonical_request()).await.unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("[RESPONSE_TIMEOUT]"),
            "a deadline must be labelled as a timeout, not as a generic failure: {message}"
        );

        // The same code path for a request that fails *without* timing out: the
        // error is forwarded verbatim, because inventing a timeout label there
        // would make a dead provider look like a slow one.
        let closed = {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            probe.local_addr().unwrap()
        };
        let client = Client::from_target(chat_target(&format!("http://{closed}"), "k", None, None));
        let message = client
            .stream_model(canonical_request())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            !message.contains("[RESPONSE_TIMEOUT]"),
            "a refused connection is not a timeout: {message}"
        );
        assert!(
            message.contains("error sending request"),
            "a non-timeout transport failure is forwarded verbatim: {message}"
        );
    }

    /// An SSE frame whose bytes are not valid UTF-8 — a multi-byte character cut
    /// in the middle, which is exactly what a peer- or proxy-truncated stream
    /// produces — must be reported as a model-response error and must not be
    /// silently dropped or decoded into a panic.
    #[tokio::test]
    async fn a_frame_with_a_truncated_utf8_sequence_is_a_model_response_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let mut body: Vec<u8> = br#"data: {"choices":[{"delta":{"content":""#.to_vec();
            // 0xE6 starts a three-byte CJK character; only the first byte is sent.
            body.extend_from_slice(&[0xE6, 0x0A, 0x0A]);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        });

        let client = Client::from_target(chat_target(&format!("http://{addr}"), "k", None, None));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        let mut message = None;
        for event in &events {
            if let schema::ModelStreamEvent::Error { message: text } = event {
                message = Some(text.as_str());
                break;
            }
        }
        let message = message.expect("an undecodable frame must surface a terminal error event");
        assert!(
            message.contains(MODEL_RESPONSE_ERROR),
            "an undecodable frame is a model-response error: {message}"
        );
    }

    /// A stream that stalls long enough to hit the request deadline is a *timeout*
    /// disconnect — the one category that tells an orchestrator the request was
    /// cut short by us rather than by the peer.
    #[tokio::test]
    async fn a_stalled_stream_body_is_reported_as_a_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let mut held = Vec::new();
            for _ in 0..4 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                // Headers and one complete frame, then nothing at all.
                let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
                let frame = "data: {\"choices\":[{\"delta\":{\"content\":\"cut\"}}]}\n\n";
                let chunk = format!("{:x}\r\n{frame}\r\n", frame.len());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(chunk.as_bytes());
                let _ = stream.flush();
                held.push(stream);
            }
        });

        let mut client =
            Client::from_target(chat_target(&format!("http://{addr}"), "k", None, None));
        client.http = HttpClient::builder()
            .timeout(std::time::Duration::from_millis(150))
            .build()
            .unwrap();
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;

        let message = events
            .iter()
            .find_map(|event| match event {
                schema::ModelStreamEvent::Error { message } => Some(message.as_str()),
                _ => None,
            })
            .expect("a stalled body must surface a terminal error event");
        assert!(
            message.contains("[RESPONSE_TIMEOUT]"),
            "a stalled body is a timeout, not a bare disconnect: {message}"
        );
    }

    /// A stream that dies mid-body is reported as an upstream disconnect carrying
    /// the progress it had made, and the buffered tail is *not* decoded: after a
    /// transport error the tail can be cut mid-JSON or mid-UTF-8, and reporting a
    /// parse error there would turn a retryable disconnect into a hard failure.
    #[tokio::test]
    async fn a_truncated_stream_body_is_an_upstream_disconnect_not_a_parse_error() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"par\"}}]}\n\n";
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            // Declare far more than we send, then close: the body is cut.
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                sse.len() + 5000
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(sse.as_bytes());
            let _ = stream.flush();
            drop(stream);
        });

        let target = chat_target(&format!("http://{addr}"), "k", None, None);
        let client = Client::from_target(target);
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;

        let error = events
            .iter()
            .find_map(|event| match event {
                schema::ModelStreamEvent::Error { message } => Some(message.as_str()),
                _ => None,
            })
            .expect("a cut body must surface a terminal error event");
        assert!(
            error.contains(UPSTREAM_DISCONNECTED),
            "a cut body is an upstream disconnect: {error}"
        );
        assert!(
            error.contains("kind=") && error.contains("causes="),
            "the diagnostic must carry the classified kind and the cause chain: {error}"
        );
        assert!(
            !error.contains(MODEL_RESPONSE_ERROR),
            "the cut tail must not be decoded into a parse error: {error}"
        );
        // The partial text the provider did deliver is preserved.
        assert!(events.iter().any(|event| matches!(
            event,
            schema::ModelStreamEvent::TextDelta { text, .. } if text == "par"
        )));
    }

    /// A frame that is valid SSE but not a valid provider payload must be
    /// reported as a model-response error rather than ending the stream quietly:
    /// a gateway that injects an HTML error page or a truncated JSON object into
    /// the event stream would otherwise look like a clean completion.
    #[tokio::test]
    async fn a_frame_whose_payload_is_not_valid_json_is_a_model_response_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for _ in 0..4 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = "data: {\"choices\": [truncated\n\n";
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
                let _ = stream.flush();
            }
        });

        let client = Client::from_target(chat_target(&format!("http://{addr}"), "k", None, None));
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        let mut message = None;
        for event in &events {
            if let schema::ModelStreamEvent::Error { message: text } = event {
                message = Some(text.as_str());
                break;
            }
        }
        let message = message.expect("an undecodable payload must surface a terminal error event");
        assert!(
            message.contains(MODEL_RESPONSE_ERROR),
            "a malformed payload is a model-response error: {message}"
        );
        assert!(
            message.contains("invalid provider stream event"),
            "the diagnostic must say the provider event was invalid: {message}"
        );
    }

    /// The client accepts an injected adapter registry, which is the seam the
    /// protocol-level failure arms need: an adapter that refuses to close a
    /// stream must surface as a model-response error at EOF, and one that
    /// declares the stream complete must end the pump early (its trailing bytes
    /// are intentionally dropped, which is what a logical terminator means).
    #[tokio::test]
    async fn an_adapter_that_fails_or_terminates_controls_the_end_of_the_pump() {
        struct RefusingAdapter;
        impl crate::llm::adapters::ProtocolAdapter for RefusingAdapter {
            fn protocol(&self) -> schema::ApiProtocol {
                schema::ApiProtocol::OpenAiChatCompletions
            }
            fn endpoint_path(&self) -> &'static str {
                "/v1/chat/completions"
            }
            fn build_body(
                &self,
                _: &schema::ResolvedModelTarget,
                request: &schema::ModelRequest,
            ) -> Result<Value> {
                Ok(serde_json::json!({"model": request.model, "stream": true}))
            }
            fn new_stream_state(&self) -> Box<dyn std::any::Any + Send> {
                Box::new(())
            }
            fn decode_frame(
                &self,
                _: &crate::llm::sse::SseFrame,
                _: &mut (dyn std::any::Any + Send),
            ) -> Result<Vec<schema::ModelStreamEvent>> {
                Ok(Vec::new())
            }
            fn finish_stream(
                &self,
                _: &mut (dyn std::any::Any + Send),
            ) -> Result<Vec<schema::ModelStreamEvent>> {
                Err(anyhow::anyhow!("refusing to close this stream"))
            }
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for _ in 0..4 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = "data: {}\n\n";
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
                let _ = stream.flush();
            }
        });

        let mut registry = crate::llm::adapters::AdapterRegistry::default();
        registry.register(RefusingAdapter);
        let client = Client::from_target_with_registry(
            chat_target(&format!("http://{addr}"), "k", None, None),
            registry,
        );
        let events: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        let mut message = None;
        for event in &events {
            if let schema::ModelStreamEvent::Error { message: text } = event {
                message = Some(text.as_str());
                break;
            }
        }
        let message = message.expect("a failed close must surface a terminal error event");
        assert!(
            message.contains(MODEL_RESPONSE_ERROR) && message.contains("refusing to close"),
            "the adapter's close failure must be reported: {message}"
        );
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

    pub(super) fn protocol_target(
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

    pub(super) fn canonical_request() -> schema::ModelRequest {
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
    async fn summary_output_cap_is_request_local_and_sent_on_the_wire() {
        let server = mock_server(|_| {
            (200, "text/event-stream", "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into())
        });
        let mut target = protocol_target(
            &server.base_url,
            schema::ProtocolConfig::OpenAiChat(schema::OpenAiChatConfig::default()),
        );
        target.generation.max_output_tokens = Some(32_000);
        target.capabilities.max_output_tokens = 64_000;
        let client = Client::from_target(target);
        let _: Vec<_> = client
            .stream_model_with_output_limit(canonical_request(), 8192)
            .await
            .unwrap()
            .collect()
            .await;
        let _: Vec<_> = client
            .stream_model(canonical_request())
            .await
            .unwrap()
            .collect()
            .await;
        let requests = server.requests.lock().unwrap();
        let first: serde_json::Value = serde_json::from_str(&requests[0]).unwrap();
        let second: serde_json::Value = serde_json::from_str(&requests[1]).unwrap();
        assert_eq!(first["max_tokens"], 8192);
        assert_eq!(second["max_tokens"], 32_000);
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

    #[tokio::test]
    async fn stream_model_transport_disconnect_reports_progress_and_error_chain() {
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
                .and_then(|_| stream.write_all(b"\r\n20\r\ntruncated"))
                .and_then(|_| stream.flush())
                .unwrap();
            // Drop before completing the declared chunk to force reqwest's
            // response-body stream down its transport-error path.
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

        assert!(events.iter().any(|event| matches!(
            event,
            schema::ModelStreamEvent::TextDelta { text, .. } if text == "partial"
        )));
        let message = match events.last() {
            Some(schema::ModelStreamEvent::Error { message }) => message,
            other => panic!("expected terminal stream error, got {other:?}"),
        };
        assert!(message.starts_with(UPSTREAM_DISCONNECTED), "{message}");
        assert!(message.contains("kind=decode"), "{message}");
        assert!(message.contains("chunks="), "{message}");
        assert!(message.contains("bytes="), "{message}");
        assert!(message.contains("frames="), "{message}");
        assert!(message.contains("elapsed_ms="), "{message}");
        assert!(message.contains("causes="), "{message}");
    }

    #[tokio::test]
    async fn stream_model_processes_usage_frames_after_finish_reason() {
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
            let finish = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\",\"index\":0}],\"usage\":null}\n\n";
            let usage = b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":62,\"completion_tokens\":29,\"total_tokens\":91,\"credit_cost\":\"0.00186\"}}\n\n";
            stream
                .write_all(format!("{:X}\r\n", finish.len()).as_bytes())
                .and_then(|_| stream.write_all(finish))
                .and_then(|_| stream.write_all(b"\r\n"))
                .and_then(|_| stream.write_all(format!("{:X}\r\n", usage.len()).as_bytes()))
                .and_then(|_| stream.write_all(usage))
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

        let usage = events
            .iter()
            .find_map(|event| match event {
                schema::ModelStreamEvent::Usage(usage) => Some(usage),
                _ => None,
            })
            .expect("usage event after finish_reason should be preserved");
        assert_eq!(usage.prompt_tokens, 62);
        assert_eq!(usage.completion_tokens, 29);
        assert_eq!(usage.credit_cost, Some(0.00186));
        assert!(events.iter().any(|event| matches!(
            event,
            schema::ModelStreamEvent::Finish {
                reason: schema::FinishReason::Stop,
                ..
            }
        )));
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
        // event, and the missing terminal frame is reported as a disconnect.
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
            Some(schema::ModelStreamEvent::Error { message })
                if message.starts_with(UPSTREAM_DISCONNECTED) && message.contains("kind=eof")
        ));
    }

    #[tokio::test]
    async fn responses_transport_error_cleans_unfinished_items_before_retry() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, BufReader, Read, Write};
            let mut requests = Vec::new();
            for attempt in 0..2 {
                let (socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                    .unwrap();
                let mut reader = BufReader::new(socket);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                requests.push(serde_json::from_slice::<Value>(&body).unwrap());
                let mut socket = reader.into_inner();
                let body = if attempt == 0 {
                    concat!(
                        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"reasoning\",\"id\":\"rs_done\",\"summary\":[],\"encrypted_content\":\"cipher\"}}\n\n",
                        "data: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"reasoning\",\"id\":\"rs_partial\"}}\n\n",
                        "data: {\"type\":\"response.reasoning_summary_text.delta\",\"output_index\":1,\"item_id\":\"rs_partial\",\"summary_index\":0,\"delta\":\"thinking\"}\n\n",
                        "data: {\"type\":\"response.output_item.added\",\"output_index\":2,\"item\":{\"type\":\"function_call\",\"id\":\"fc_unfinished\",\"call_id\":\"call_1\",\"name\":\"echo\"}}\n\n",
                        "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":2,\"delta\":\"{}\"}\n\n",
                        // Broken buffered SSE must not mask the transport error.
                        "data: {\"type\":",
                    )
                } else {
                    concat!(
                        "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\"done\"}\n\n",
                        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n",
                    )
                };
                // Closing with a short body produces a reqwest transport error,
                // not the clean EOF covered by the other retry integration test.
                let length = body.len() + if attempt == 0 { 100 } else { 0 };
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}").unwrap();
            }
            requests
        });
        let client = Client::from_target(protocol_target(
            &format!("http://127.0.0.1:{port}"),
            schema::ProtocolConfig::OpenAiResponses(Default::default()),
        ));
        let loop_ = crate::agent::Loop::new(std::sync::Arc::new(client), "mock");
        assert_eq!(
            loop_.run_streaming("hi".into(), |_| {}).await.unwrap(),
            "done"
        );
        let requests = server.join().unwrap();
        let input = requests[1]["input"].as_array().unwrap();
        let tool = input
            .iter()
            .find(|item| item["type"] == "function_call")
            .unwrap();
        assert!(tool.get("id").is_none(), "unfinished item replayed: {tool}");
        assert_eq!(tool["call_id"], "call_1");
        assert!(input
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["call_id"] == "call_1"));
        let reasoning: Vec<_> = input
            .iter()
            .filter(|item| item["type"] == "reasoning")
            .collect();
        assert_eq!(reasoning.len(), 1, "only completed reasoning is replayable");
        assert_eq!(reasoning[0]["id"], "rs_done");
        assert_eq!(reasoning[0]["encrypted_content"], "cipher");
        assert!(!loop_
            .stream_incomplete
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn anthropic_retry_projects_partial_tool_arguments_as_an_object() {
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        let server = mock_server(move |_| {
            let body = if attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                concat!(
                    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"echo\",\"input\":{}}}\n\n",
                    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"command\\\":\"}}\n\n",
                )
            } else {
                concat!(
                    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
                    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"done\"}}\n\n",
                    "data: {\"type\":\"message_stop\"}\n\n",
                )
            };
            (200, "text/event-stream", body.to_string())
        });
        let client = Client::from_target(protocol_target(
            &server.base_url,
            schema::ProtocolConfig::AnthropicMessages(Default::default()),
        ));
        let loop_ = crate::agent::Loop::new(std::sync::Arc::new(client), "mock");
        let (text, history) = loop_
            .run_streaming_with_messages(
                canonical_request().messages,
                &crate::agent::StreamContext::default(),
                |_| {},
                |_| {},
                None,
            )
            .await
            .unwrap();
        assert_eq!(text, "done");
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let retry: Value = serde_json::from_str(&requests[1]).unwrap();
        let blocks: Vec<_> = retry["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["content"].as_array().unwrap())
            .collect();
        let tool = blocks
            .iter()
            .find(|block| block["type"] == "tool_use")
            .unwrap();
        assert_eq!(tool["input"], serde_json::json!({}));
        let result = blocks
            .iter()
            .find(|block| block["type"] == "tool_result")
            .unwrap();
        assert_eq!(result["tool_use_id"], tool["id"]);
        assert!(result["content"].as_str().unwrap().contains("not executed"));
        assert!(history.iter().flat_map(|message| &message.content).any(|block| matches!(
            block,
            crate::types::ContentBlock::ToolCall { args, .. } if args == &serde_json::json!("{\"command\":")
        )), "wire normalization must not erase the original partial arguments");
        assert!(!loop_
            .stream_incomplete
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn responses_disconnect_retries_with_partial_history_over_http() {
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        let server = mock_server(move |_| {
            let body = if attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                concat!(
                    "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"reasoning\",\"id\":\"rs_partial\"}}\n\n",
                    "data: {\"type\":\"response.reasoning_summary_text.delta\",\"output_index\":0,\"item_id\":\"rs_partial\",\"summary_index\":0,\"delta\":\"thinking\"}\n\n",
                    "data: {\"type\":\"response.output_text.delta\",\"output_index\":1,\"delta\":\"part \"}\n\n",
                )
            } else {
                concat!(
                    "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\"done\"}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n",
                )
            };
            (200, "text/event-stream", body.to_string())
        });
        let client = Client::from_target(protocol_target(
            &server.base_url,
            schema::ProtocolConfig::OpenAiResponses(Default::default()),
        ));
        let loop_ = crate::agent::Loop::new(std::sync::Arc::new(client), "mock");
        let text = loop_.run_streaming("hi".into(), |_| {}).await.unwrap();
        assert_eq!(text, "part done");
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let retry: Value = serde_json::from_str(&requests[1]).unwrap();
        let input = retry["input"].as_array().unwrap();
        assert!(
            !input.iter().any(|item| item["type"] == "reasoning"),
            "unfinished reasoning must not be replayed as a provider-owned item"
        );
        assert_eq!(input.last().unwrap()["role"], "user");
        let partial = &input[input.len() - 2];
        assert_eq!(partial["role"], "assistant");
        assert_eq!(partial["content"][0]["text"], "part ");
        assert!(!loop_
            .stream_incomplete
            .load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn stream_model_eof_is_retryable_for_every_protocol() {
        for protocol in [
            schema::ProtocolConfig::OpenAiResponses(Default::default()),
            schema::ProtocolConfig::OpenAiChat(Default::default()),
            schema::ProtocolConfig::AnthropicMessages(Default::default()),
        ] {
            let server = mock_server(|_| (200, "text/event-stream", ": heartbeat\n\n".into()));
            let client = Client::from_target(protocol_target(&server.base_url, protocol));
            let events: Vec<_> = client
                .stream_model(canonical_request())
                .await
                .unwrap()
                .collect()
                .await;
            assert!(
                matches!(events.last(), Some(schema::ModelStreamEvent::Error { message })
                if message.starts_with(UPSTREAM_DISCONNECTED))
            );
            assert!(!events
                .iter()
                .any(|event| matches!(event, schema::ModelStreamEvent::Finish { .. })));
        }
    }

    #[tokio::test]
    async fn chat_disconnect_after_finish_does_not_retry_a_completed_response() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, BufReader, Read, Write};
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut reader = BufReader::new(socket);
            let mut length = 0;
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            let mut socket = reader.into_inner();
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\n";
            // HTTP is truncated, but the model's terminal frame arrived intact.
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len() + 100).unwrap();
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
        server.join().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            schema::ModelStreamEvent::Finish {
                reason: schema::FinishReason::Stop,
                ..
            }
        )));
        assert!(!events
            .iter()
            .any(|event| matches!(event, schema::ModelStreamEvent::Error { .. })));
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

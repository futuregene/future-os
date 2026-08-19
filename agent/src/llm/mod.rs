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
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;

const DEFAULT_TIMEOUT_SECS: u64 = 1800;
const STREAM_IDLE_TIMEOUT_SECS: u64 = 45;

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
    target: RwLock<schema::ResolvedModelTarget>,
    adapters: AdapterRegistry,
    #[allow(clippy::type_complexity)]
    on_payload: Option<Arc<dyn Fn(&[u8]) + Send + Sync>>,
    #[allow(clippy::type_complexity)]
    on_response: Option<Arc<dyn Fn(u16, &HashMap<String, String>) + Send + Sync>>,
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
            target: RwLock::new(target),
            adapters,
            on_payload: None,
            on_response: None,
        }
    }

    pub fn with_thinking_level(self, level: &str) -> Self {
        self.target.write().generation.thinking_level = level.to_string();
        self
    }

    pub fn with_thinking_budget(self, budget: i32) -> Self {
        self.target.write().generation.thinking_budget = budget;
        self
    }

    pub fn with_thinking_level_map(self, map: HashMap<String, String>) -> Self {
        self.target.write().capabilities.reasoning.levels = map
            .iter()
            .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
            .collect();
        self
    }

    pub fn with_temperature(self, temperature: f32) -> Self {
        self.target.write().generation.temperature = Some(temperature);
        self
    }

    pub fn with_max_tokens(self, max_tokens: i32) -> Self {
        self.target.write().generation.max_output_tokens = Some(max_tokens);
        self
    }
}

#[async_trait::async_trait]
impl crate::types::LLMProvider for Client {
    async fn stream_model(
        &self,
        request: schema::ModelRequest,
    ) -> Result<ReceiverStream<schema::ModelStreamEvent>> {
        let target = self.target.read().clone();
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
        let headers: HashMap<String, String> = resp
            .headers()
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_str().unwrap_or("").to_string()))
            .collect();
        if let Some(callback) = &self.on_response {
            callback(status.as_u16(), &headers);
        }
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
        let on_payload = self.on_payload.clone();
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
                                message: error.to_string(),
                            })
                            .await;
                        return;
                    }
                    Ok(None) => break,
                    Err(_) => break,
                };
                if tx.is_closed() {
                    return;
                }
                if let Some(callback) = &on_payload {
                    callback(&bytes);
                }
                let frames = match decoder.push(&bytes) {
                    Ok(frames) => frames,
                    Err(error) => {
                        let _ = tx
                            .send(schema::ModelStreamEvent::Error {
                                message: error.to_string(),
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
                                    message: format!("invalid provider stream event: {error}"),
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

            if let Ok(frames) = decoder.finish() {
                for frame in frames {
                    if let Ok(events) = adapter.decode_frame(&frame, state.as_mut()) {
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
                                message: error.to_string(),
                            })
                            .await;
                    }
                }
            }
        });
        Ok(ReceiverStream::new(rx))
    }

    fn set_api_key(&self, api_key: &str) {
        self.target.write().route.api_key = api_key.to_string();
    }

    fn set_base_url(&self, base_url: &str) {
        self.target.write().route.base_url = base_url.to_string();
    }

    fn update_thinking(&self, level: &str, budget: i32) {
        let mut target = self.target.write();
        target.generation.thinking_level = level.to_string();
        target.generation.thinking_budget = budget;
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
        let target = client.target.read();
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
        let target = client.target.read();
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
    fn runtime_setters_update_the_resolved_target() {
        let client = Client::from_target(chat_target("https://api.test", "old-key", None, None));
        crate::types::LLMProvider::set_api_key(&client, "new-key");
        crate::types::LLMProvider::set_base_url(&client, "https://new.test");
        crate::types::LLMProvider::update_thinking(&client, "high", 16000);
        let target = client.target.read();
        assert_eq!(target.route.api_key, "new-key");
        assert_eq!(target.route.base_url, "https://new.test");
        assert_eq!(target.generation.thinking_level, "high");
        assert_eq!(target.generation.thinking_budget, 16000);
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
}

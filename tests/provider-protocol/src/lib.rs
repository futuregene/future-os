use anyhow::{bail, Context, Result};
use future_agent::llm::schema::{
    AnthropicMessagesConfig, AnthropicThinkingMode, AuthScheme, GenerationConfig,
    ModelCapabilities, ModelRequest, ModelStreamEvent, OpenAiResponsesConfig, ProtocolConfig,
    ProviderRoute, ReasoningCapabilities, ResolvedModelTarget,
};
use future_agent::llm::Client;
use future_agent::types::{AgentMessage, LLMProvider};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;

#[derive(Debug, Deserialize)]
pub struct RigCassette {
    pub when: RigWhen,
    pub then: RigThen,
}

#[derive(Debug, Deserialize)]
pub struct RigWhen {
    pub path: String,
    pub method: String,
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct RigThen {
    pub status: u16,
    pub body: String,
}

#[derive(Debug)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

#[derive(Clone)]
pub struct MockResponse {
    pub status: u16,
    pub body: Vec<u8>,
    /// Byte offsets at which an HTTP chunk ends. Offsets may split UTF-8 code points.
    pub chunk_ends: Vec<usize>,
}

impl MockResponse {
    pub fn sse(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            body: body.into(),
            chunk_ends: Vec::new(),
        }
    }

    pub fn chunked_sse(body: impl Into<Vec<u8>>, chunk_ends: Vec<usize>) -> Self {
        Self {
            status: 200,
            body: body.into(),
            chunk_ends,
        }
    }
}

/// Resolve fixture links from this standalone harness, independent of the caller's cwd.
pub fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(relative)
}

pub fn read_rig_cassette(relative: &str) -> Result<RigCassette> {
    let path = fixture_path(relative);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("fixture link is missing or unreadable: {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("invalid Rig cassette: {}", path.display()))
}

pub fn read_fixture(relative: &str) -> Result<Vec<u8>> {
    let path = fixture_path(relative);
    std::fs::read(&path)
        .with_context(|| format!("fixture link is missing or unreadable: {}", path.display()))
}

pub fn responses_target(
    base_url: String,
    model: &str,
    thinking_level: &str,
) -> ResolvedModelTarget {
    ResolvedModelTarget {
        model_id: model.to_string(),
        route: ProviderRoute {
            provider_id: "special-test".into(),
            base_url,
            api_key: "fixture-only".into(),
            auth: AuthScheme::Bearer,
            headers: BTreeMap::new(),
        },
        protocol: ProtocolConfig::OpenAiResponses(OpenAiResponsesConfig::default()),
        capabilities: ModelCapabilities {
            supports_text_input: true,
            supports_tools: true,
            supports_parallel_tools: true,
            reasoning: ReasoningCapabilities {
                supported: true,
                ..Default::default()
            },
            ..Default::default()
        },
        generation: GenerationConfig {
            thinking_level: thinking_level.to_string(),
            ..Default::default()
        },
    }
}

pub fn anthropic_target(base_url: String, model: &str) -> ResolvedModelTarget {
    ResolvedModelTarget {
        model_id: model.to_string(),
        route: ProviderRoute {
            provider_id: "special-test-anthropic".into(),
            base_url,
            api_key: "fixture-only".into(),
            auth: AuthScheme::AnthropicApiKey,
            headers: BTreeMap::new(),
        },
        protocol: ProtocolConfig::AnthropicMessages(AnthropicMessagesConfig {
            version: "2023-06-01".into(),
            thinking_mode: AnthropicThinkingMode::Adaptive,
        }),
        capabilities: ModelCapabilities {
            supports_text_input: true,
            supports_tools: true,
            supports_parallel_tools: true,
            reasoning: ReasoningCapabilities {
                supported: true,
                ..Default::default()
            },
            max_output_tokens: 4096,
            ..Default::default()
        },
        generation: GenerationConfig {
            max_output_tokens: Some(4096),
            thinking_level: "high".into(),
            thinking_budget: 4096,
            ..Default::default()
        },
    }
}

pub fn user_request(model: &str, text: &str) -> ModelRequest {
    ModelRequest {
        model: model.into(),
        system_prompt: String::new(),
        messages: vec![AgentMessage::new_user("user", serde_json::json!(text))],
        tools: Vec::new(),
    }
}

pub async fn stream_events(
    base_url: String,
    model: &str,
    thinking_level: &str,
    request: ModelRequest,
) -> Result<Vec<ModelStreamEvent>> {
    let client = Client::from_target(responses_target(base_url, model, thinking_level));
    let events = client.stream_model(request).await?.collect().await;
    Ok(events)
}

pub async fn stream_anthropic_events(
    base_url: String,
    model: &str,
    request: ModelRequest,
) -> Result<Vec<ModelStreamEvent>> {
    let client = Client::from_target(anthropic_target(base_url, model));
    let events = client.stream_model(request).await?.collect().await;
    Ok(events)
}

pub async fn start_server(
    responses: Vec<MockResponse>,
    base_path: &str,
) -> Result<(String, JoinHandle<Result<Vec<CapturedRequest>>>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let base_url = format!("http://{address}{base_path}");
    let handle = tokio::spawn(async move {
        let mut captured = Vec::with_capacity(responses.len());
        for response in responses {
            let (mut socket, _) = listener.accept().await?;
            captured.push(read_request(&mut socket).await?);
            write_response(&mut socket, &response).await?;
        }
        Ok(captured)
    });
    Ok((base_url, handle))
}

async fn read_request(socket: &mut TcpStream) -> Result<CapturedRequest> {
    let mut raw = Vec::new();
    let header_end = loop {
        if let Some(position) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        let mut buffer = [0_u8; 4096];
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            bail!("client closed before completing HTTP headers");
        }
        raw.extend_from_slice(&buffer[..read]);
    };
    let headers = std::str::from_utf8(&raw[..header_end])?;
    let mut request_line = headers
        .lines()
        .next()
        .context("missing HTTP request line")?
        .split_whitespace();
    let method = request_line
        .next()
        .context("missing HTTP method")?
        .to_string();
    let path = request_line
        .next()
        .context("missing HTTP path")?
        .to_string();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>())
        })
        .transpose()?
        .unwrap_or(0);
    while raw.len() < header_end + content_length {
        let mut buffer = [0_u8; 4096];
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            bail!("client closed before completing HTTP body");
        }
        raw.extend_from_slice(&buffer[..read]);
    }
    Ok(CapturedRequest {
        method,
        path,
        body: raw[header_end..header_end + content_length].to_vec(),
    })
}

async fn write_response(socket: &mut TcpStream, response: &MockResponse) -> Result<()> {
    let reason = if response.status == 200 {
        "OK"
    } else {
        "Fixture Error"
    };
    if response.chunk_ends.is_empty() {
        let headers = format!(
            "HTTP/1.1 {} {}\r\ncontent-type: text/event-stream; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            response.status,
            reason,
            response.body.len()
        );
        socket.write_all(headers.as_bytes()).await?;
        socket.write_all(&response.body).await?;
    } else {
        let headers = format!(
            "HTTP/1.1 {} {}\r\ncontent-type: text/event-stream; charset=utf-8\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n",
            response.status, reason
        );
        socket.write_all(headers.as_bytes()).await?;
        let mut start = 0;
        for end in response
            .chunk_ends
            .iter()
            .copied()
            .chain(std::iter::once(response.body.len()))
        {
            if end <= start || end > response.body.len() {
                continue;
            }
            let chunk = &response.body[start..end];
            socket
                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .await?;
            socket.write_all(chunk).await?;
            socket.write_all(b"\r\n").await?;
            start = end;
        }
        socket.write_all(b"0\r\n\r\n").await?;
    }
    socket.shutdown().await?;
    Ok(())
}

pub fn assert_no_stream_error(events: &[ModelStreamEvent]) -> Result<()> {
    if let Some(message) = events.iter().find_map(|event| match event {
        ModelStreamEvent::Error { message } => Some(message),
        _ => None,
    }) {
        bail!("FutureOS emitted a stream error: {message}");
    }
    Ok(())
}

pub fn request_json(request: &CapturedRequest) -> Result<Value> {
    serde_json::from_slice(&request.body).context("captured request body is not JSON")
}

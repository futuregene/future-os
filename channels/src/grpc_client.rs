//! gRPC client for FutureAgent.
//! Communicates exclusively via gRPC — no direct agent function calls.

use anyhow::{anyhow, Result};
use serde_json::Value;

// Generated proto code (from future.proto)
mod proto {
    tonic::include_proto!("proto");
}

use proto::future_agent_client::FutureAgentClient;
use proto::{RpcCommand, StreamRequest};

/// Event types from the agent event stream.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    TextChunk(String),
    ThinkingStart,
    ThinkingDelta(String),
    ThinkingEnd,
    AgentStart,
    AgentEnd { error: Option<String> },
    ToolStart { tool_id: String, tool_name: String, tool_args: Option<String> },
    ToolDelta { tool_id: String, text: String },
    ToolEnd { tool_id: String, text: Option<String> },
    ApprovalRequest {
        approval_request_id: String,
        tool_id: String,
        tool_name: String,
        kind: String,
        risk_level: String,
        title: String,
        summary: String,
        requested_action: serde_json::Value,
    },
    Error(String),
    Ping,
}

#[derive(Clone)]
pub struct AgentClient {
    inner: FutureAgentClient<tonic::transport::Channel>,
}

impl AgentClient {
    pub async fn connect(addr: &str) -> Result<Self> {
        let addr = format!("http://{}", addr.trim_start_matches("http://").trim_start_matches("https://"));
        let endpoint = tonic::transport::Endpoint::new(addr.clone())?
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60));
        let channel = endpoint.connect().await
            .map_err(|e| anyhow!("Failed to connect to agent at {}: {}", addr, e))?;
        let inner = FutureAgentClient::new(channel);
        Ok(Self { inner })
    }

    /// Execute a command and return the parsed JSON response data.
    async fn call(&mut self, cmd_type: &str, session_id: &str, extra: RpcCommand) -> Result<Value> {
        let request = tonic::Request::new(RpcCommand {
            id: uuid::Uuid::new_v4().to_string(),
            r#type: cmd_type.to_string(),
            session_id: session_id.to_string(),
            session_path: String::new(),
            entry_id: String::new(),
            ..extra
        });

        let response = self.inner.execute_command(request).await
            .map_err(|e| anyhow!("gRPC call '{}' failed: {}", cmd_type, e))?
            .into_inner();

        if !response.success {
            let err = if response.error.is_empty() { "unknown error".to_string() } else { response.error.clone() };
            return Err(anyhow!("Command '{}' failed: {}", cmd_type, err));
        }

        if response.data.is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_str(&response.data)
            .map_err(|e| anyhow!("Failed to parse response data for '{}': {}", cmd_type, e))
    }

    /// Create a new agent session. Returns the session_id.
    pub async fn new_session(&mut self, cwd: &str) -> Result<String> {
        let resp = self.call("new_session", "", RpcCommand {
            cwd: cwd.to_string(),
            ..Default::default()
        }).await?;
        resp["sessionId"].as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("new_session response missing sessionId"))
    }

    /// Send a prompt to the agent. Returns immediately (agent runs in background).
    pub async fn prompt(&mut self, session_id: &str, message: &str, images: Vec<ImageInput>) -> Result<()> {
        let proto_images: Vec<proto::ImageContent> = images.into_iter().map(|img| {
            proto::ImageContent {
                r#type: img.content_type,
                content: Some(match img.data {
                    ImageData::Url(url) => proto::image_content::Content::Url(url),
                    ImageData::Base64(b64) => proto::image_content::Content::Base64(b64),
                }),
                file_path: img.file_path.unwrap_or_default(),
            }
        }).collect();

        self.call("prompt", session_id, RpcCommand {
            message: message.to_string(),
            images: proto_images,
            ..Default::default()
        }).await?;
        Ok(())
    }

    /// Abort current generation.
    pub async fn abort(&mut self, session_id: &str) -> Result<()> {
        self.call("abort", session_id, Default::default()).await?;
        Ok(())
    }

    /// Get session state.
    pub async fn get_state(&mut self, session_id: &str) -> Result<SessionState> {
        let resp = self.call("get_state", session_id, Default::default()).await?;
        Ok(SessionState {
            model: resp["model"].as_str().unwrap_or("?").to_string(),
            image_support: resp["imageSupport"].as_bool().unwrap_or(false),
            thinking_level: resp["thinkingLevel"].as_str().unwrap_or("off").to_string(),
            is_streaming: resp["isStreaming"].as_bool().unwrap_or(false),
            context_tokens: resp["contextTokens"].as_i64().unwrap_or(0),
            context_window: resp["contextWindow"].as_i64().unwrap_or(0),
            tokens_in: resp["tokensIn"].as_i64().unwrap_or(0),
            tokens_out: resp["tokensOut"].as_i64().unwrap_or(0),
            message_count: resp["messageCount"].as_i64().unwrap_or(0) as usize,
            session_id: resp["sessionId"].as_str().unwrap_or("").to_string(),
            session_name: resp["sessionName"].as_str().unwrap_or("").to_string(),
            cwd: resp["cwd"].as_str().unwrap_or("").to_string(),
            auto_compaction: resp["autoCompactionEnabled"].as_bool().unwrap_or(true),
            total_cost: resp["totalCost"].as_f64().unwrap_or(0.0),
            permission_level: resp["permissionLevel"].as_str().unwrap_or("all").to_string(),
        })
    }

    /// Get available models.
    pub async fn get_available_models(&mut self, session_id: &str) -> Result<Vec<ModelInfo>> {
        let resp = self.call("get_available_models", session_id, Default::default()).await?;
        let models = resp["models"].as_array().map(|arr| {
            arr.iter().map(|m| ModelInfo {
                id: m["id"].as_str().unwrap_or("?").to_string(),
                name: m["name"].as_str().unwrap_or("?").to_string(),
                provider: m["provider"].as_str().unwrap_or("").to_string(),
                image: m["image"].as_bool().unwrap_or(false),
                reasoning: m["reasoning"].as_bool().unwrap_or(false),
                context_window: m["contextWindow"].as_i64().unwrap_or(0),
                max_tokens: m["maxTokens"].as_i64().unwrap_or(0),
            }).collect()
        }).unwrap_or_default();
        Ok(models)
    }

    /// Switch to a different model.
    pub async fn set_model(&mut self, session_id: &str, model_id: &str) -> Result<()> {
        self.call("set_model", session_id, RpcCommand {
            model_id: model_id.to_string(),
            ..Default::default()
        }).await?;
        Ok(())
    }

    /// Send approval decision back to the agent.
    pub async fn approval_decision(
        &mut self,
        session_id: &str,
        request_id: &str,
        approved: bool,
        note: &str,
    ) -> Result<()> {
        self.call("approval_decision", session_id, RpcCommand {
            mode: if approved { "approved".to_string() } else { "rejected".to_string() },
            message: note.to_string(),
            entry_id: request_id.to_string(),
            ..Default::default()
        }).await?;
        Ok(())
    }

    /// Compact the current session context.
    pub async fn compact(&mut self, session_id: &str) -> Result<()> {
        self.call("compact", session_id, Default::default()).await?;
        Ok(())
    }

    /// Set thinking level.
    pub async fn set_thinking_level(&mut self, session_id: &str, level: &str) -> Result<()> {
        self.call("set_thinking_level", session_id, RpcCommand {
            level: level.to_string(),
            ..Default::default()
        }).await?;
        Ok(())
    }

    /// Switch to an existing session.
    pub async fn switch_session(&mut self, session_id: &str) -> Result<()> {
        // Note: pass session_id as the second arg to call(), not via extra.
        // Rust struct update syntax (..extra) does NOT override fields
        // that are already explicitly set in the struct literal.
        self.call("switch_session", session_id, RpcCommand {
            ..Default::default()
        }).await?;
        Ok(())
    }

    /// Stream events from the agent for a specific session.
    pub async fn stream_events(&mut self, session_id: &str) -> Result<tonic::Streaming<proto::StreamEvent>> {
        let request = tonic::Request::new(StreamRequest {
            session_id: session_id.to_string(),
            event_types: vec![],
        });
        let stream = self.inner.stream_events(request).await
            .map_err(|e| anyhow!("Failed to subscribe to events: {}", e))?
            .into_inner();
        Ok(stream)
    }

    /// Parse a StreamEvent into an AgentEvent.
    pub fn parse_event(event: proto::StreamEvent) -> Option<AgentEvent> {
        match event.r#type.as_str() {
            "ping" => Some(AgentEvent::Ping),
            "agent_start" => Some(AgentEvent::AgentStart),
            "agent_end" => {
                let error = serde_json::from_str::<Value>(&event.data)
                    .ok()
                    .and_then(|d| d["error"].as_str().map(|s| s.to_string()));
                Some(AgentEvent::AgentEnd { error })
            }
            "text_chunk" => {
                let text = serde_json::from_str::<Value>(&event.data)
                    .ok()
                    .and_then(|d| d["text"].as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Some(AgentEvent::TextChunk(text))
            }
            "thinking_start" => Some(AgentEvent::ThinkingStart),
            "thinking_delta" => {
                let text = serde_json::from_str::<Value>(&event.data)
                    .ok()
                    .and_then(|d| d["text"].as_str().map(|s| s.to_string()))
                    .unwrap_or_default();
                Some(AgentEvent::ThinkingDelta(text))
            }
            "thinking_end" => Some(AgentEvent::ThinkingEnd),
            "tool_start" => {
                let data = serde_json::from_str::<Value>(&event.data).ok()?;
                Some(AgentEvent::ToolStart {
                    tool_id: data["tool_id"].as_str().unwrap_or("").to_string(),
                    tool_name: data["tool_name"].as_str().unwrap_or("").to_string(),
                    tool_args: data["tool_args"].as_str().map(|s| s.to_string()),
                })
            }
            "tool_delta" => {
                let data = serde_json::from_str::<Value>(&event.data).ok()?;
                Some(AgentEvent::ToolDelta {
                    tool_id: data["tool_id"].as_str().unwrap_or("").to_string(),
                    text: data["text"].as_str().unwrap_or("").to_string(),
                })
            }
            "tool_end" => {
                let data = serde_json::from_str::<Value>(&event.data).ok()?;
                Some(AgentEvent::ToolEnd {
                    tool_id: data["tool_id"].as_str().unwrap_or("").to_string(),
                    text: data["text"].as_str().map(|s| s.to_string()),
                })
            }
            "approval_request" => {
                let data = serde_json::from_str::<Value>(&event.data).ok()?;
                Some(AgentEvent::ApprovalRequest {
                    approval_request_id: data["approval_request_id"].as_str().unwrap_or("").to_string(),
                    tool_id: data["tool_id"].as_str().unwrap_or("").to_string(),
                    tool_name: data["tool_name"].as_str().unwrap_or("").to_string(),
                    kind: data["kind"].as_str().unwrap_or("").to_string(),
                    risk_level: data["risk_level"].as_str().unwrap_or("").to_string(),
                    title: data["title"].as_str().unwrap_or("").to_string(),
                    summary: data["summary"].as_str().unwrap_or("").to_string(),
                    requested_action: data["requested_action"].clone(),
                })
            }
            "error" => {
                let msg = serde_json::from_str::<Value>(&event.data)
                    .ok()
                    .and_then(|d| d["error"].as_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| "unknown error".to_string());
                Some(AgentEvent::Error(msg))
            }
            _ => None,
        }
    }
}

// ─── Supporting types ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SessionState {
    pub model: String,
    pub image_support: bool,
    pub thinking_level: String,
    pub is_streaming: bool,
    pub context_tokens: i64,
    pub context_window: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub message_count: usize,
    pub session_id: String,
    pub session_name: String,
    pub cwd: String,
    pub auto_compaction: bool,
    pub total_cost: f64,
    pub permission_level: String,
}

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub image: bool,
    pub reasoning: bool,
    pub context_window: i64,
    pub max_tokens: i64,
}

#[derive(Debug, Clone)]
pub enum ImageData {
    Url(String),
    Base64(String),
}

#[derive(Debug, Clone)]
pub struct ImageInput {
    pub content_type: String,
    pub data: ImageData,
    pub file_path: Option<String>,
}

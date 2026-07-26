//! gRPC client for FutureAgent.
//! Communicates exclusively via gRPC — no direct agent function calls.

use anyhow::{anyhow, Result};
use serde_json::Value;

// Generated proto code (from future.proto) — checked into src/generated/
mod proto {
    include!("generated/proto.rs");
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
    AgentEnd {
        error: Option<String>,
    },
    ToolStart {
        tool_id: String,
        tool_name: String,
        tool_args: Option<String>,
    },
    ToolDelta {
        tool_id: String,
        text: String,
    },
    ToolEnd {
        tool_id: String,
        text: Option<String>,
    },
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
        let addr = format!(
            "http://{}",
            addr.trim_start_matches("http://")
                .trim_start_matches("https://")
        );
        let endpoint = tonic::transport::Endpoint::new(addr.clone())?
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60));
        let channel = endpoint
            .connect()
            .await
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
            entry_id: String::new(),
            ..extra
        });

        let response = self
            .inner
            .execute_command(request)
            .await
            .map_err(|e| anyhow!("gRPC call '{}' failed: {}", cmd_type, e))?
            .into_inner();

        if !response.success {
            let err = if response.error.is_empty() {
                "unknown error".to_string()
            } else {
                response.error.clone()
            };
            return Err(anyhow!("Command '{}' failed: {}", cmd_type, err));
        }

        if response.data.is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_str(&response.data)
            .map_err(|e| anyhow!("Failed to parse response data for '{}': {}", cmd_type, e))
    }

    /// Create a new agent session. Returns the session_id.
    pub async fn new_session(&mut self, cwd: &str, created_by: &str) -> Result<String> {
        let meta = serde_json::json!({ "createdBy": created_by });
        let resp = self
            .call(
                "new_session",
                "",
                RpcCommand {
                    cwd: cwd.to_string(),
                    custom_instructions: meta.to_string(),
                    ..Default::default()
                },
            )
            .await?;
        resp["sessionId"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("new_session response missing sessionId"))
    }

    /// Send a prompt to the agent. Returns immediately (agent runs in background).
    pub async fn prompt(
        &mut self,
        session_id: &str,
        message: &str,
        images: Vec<ImageInput>,
    ) -> Result<()> {
        let proto_images: Vec<proto::ImageContent> = images
            .into_iter()
            .map(|img| proto::ImageContent {
                r#type: img.content_type,
                content: Some(match img.data {
                    ImageData::Url(url) => proto::image_content::Content::Url(url),
                    ImageData::Base64(b64) => proto::image_content::Content::Base64(b64),
                }),
                file_path: img.file_path.unwrap_or_default(),
            })
            .collect();

        self.call(
            "prompt",
            session_id,
            RpcCommand {
                message: message.to_string(),
                images: proto_images,
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    /// Abort current generation.
    pub async fn abort(&mut self, session_id: &str) -> Result<()> {
        self.call("abort", session_id, Default::default()).await?;
        Ok(())
    }

    /// Get session state.
    pub async fn get_state(&mut self, session_id: &str) -> Result<SessionState> {
        let resp = self
            .call("get_state", session_id, Default::default())
            .await?;
        Ok(SessionState {
            model: resp["model"].as_str().unwrap_or("?").to_string(),
            image_support: resp["imageSupport"].as_bool().unwrap_or(false),
            thinking_level: resp["thinkingLevel"].as_str().unwrap_or("off").to_string(),
            is_streaming: resp["isStreaming"].as_bool().unwrap_or(false),
            context_tokens: resp["contextTokens"].as_i64().unwrap_or(0),
            context_window: resp["contextWindow"].as_i64().unwrap_or(0),
            tokens_in: resp["tokensIn"].as_i64().unwrap_or(0),
            tokens_out: resp["tokensOut"].as_i64().unwrap_or(0),
            query_count: resp["queryCount"].as_i64().unwrap_or(0) as usize,
            session_id: resp["sessionId"].as_str().unwrap_or("").to_string(),
            session_name: resp["session_name"].as_str().unwrap_or("").to_string(),
            cwd: resp["cwd"].as_str().unwrap_or("").to_string(),
            auto_compaction: resp["autoCompactionEnabled"].as_bool().unwrap_or(true),
            total_cost: resp["totalCost"].as_f64().unwrap_or(0.0),
            permission_level: resp["permissionLevel"]
                .as_str()
                .unwrap_or("all")
                .to_string(),
        })
    }

    /// Get available models.
    pub async fn get_available_models(&mut self, session_id: &str) -> Result<Vec<ModelInfo>> {
        // Uses list_models (always returns all models; scoping is client-side).
        let resp = self
            .call("list_models", session_id, Default::default())
            .await?;
        let models = resp["models"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|m| ModelInfo {
                        id: m["id"].as_str().unwrap_or("?").to_string(),
                        name: m["label"].as_str().unwrap_or("?").to_string(),
                        provider: m["provider"].as_str().unwrap_or("").to_string(),
                        image: m["supportsImages"].as_bool().unwrap_or(false),
                        reasoning: false, // Not in list_models response
                        context_window: m["contextWindow"].as_i64().unwrap_or(0),
                        max_tokens: 0, // Not in list_models response
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(models)
    }

    /// Switch to a different model.
    pub async fn set_model(&mut self, session_id: &str, model_id: &str) -> Result<()> {
        self.call(
            "set_model",
            session_id,
            RpcCommand {
                model_id: model_id.to_string(),
                ..Default::default()
            },
        )
        .await?;
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
        self.call(
            "approval_decision",
            session_id,
            RpcCommand {
                mode: if approved {
                    "approved".to_string()
                } else {
                    "rejected".to_string()
                },
                message: note.to_string(),
                entry_id: request_id.to_string(),
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    /// Compact the current session context.
    pub async fn compact(&mut self, session_id: &str) -> Result<()> {
        self.call("compact", session_id, Default::default()).await?;
        Ok(())
    }

    /// Set working directory.
    pub async fn set_cwd(&mut self, session_id: &str, cwd: &str) -> Result<()> {
        self.call(
            "set_cwd",
            session_id,
            RpcCommand {
                cwd: cwd.to_string(),
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    /// Set permission level.
    pub async fn set_permission_level(&mut self, session_id: &str, level: &str) -> Result<()> {
        self.call(
            "set_permission_level",
            session_id,
            RpcCommand {
                level: level.to_string(),
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    /// Set thinking level.
    pub async fn set_thinking_level(&mut self, session_id: &str, level: &str) -> Result<()> {
        self.call(
            "set_thinking_level",
            session_id,
            RpcCommand {
                level: level.to_string(),
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    /// Switch to an existing session.
    pub async fn switch_session(&mut self, session_id: &str) -> Result<()> {
        // Note: pass session_id as the second arg to call(), not via extra.
        // Rust struct update syntax (..extra) does NOT override fields
        // that are already explicitly set in the struct literal.
        self.call(
            "switch_session",
            session_id,
            RpcCommand {
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    /// Stream events from the agent for a specific session.
    pub async fn stream_events(
        &mut self,
        session_id: &str,
    ) -> Result<tonic::Streaming<proto::StreamEvent>> {
        let request = tonic::Request::new(StreamRequest {
            session_id: session_id.to_string(),
            event_types: vec![],
        });
        let stream = self
            .inner
            .stream_events(request)
            .await
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
                    approval_request_id: data["approval_request_id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
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
    pub query_count: usize,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(event_type: &str, data: &str) -> proto::StreamEvent {
        proto::StreamEvent {
            r#type: event_type.to_string(),
            data: data.to_string(),
            run_id: "run_1".to_string(),
            idx: 0,
        }
    }

    // ─── parse_event: basic events ───────────────────────────────────────────

    #[test]
    fn parse_ping() {
        let event = make_event("ping", "{}");
        assert!(matches!(
            AgentClient::parse_event(event),
            Some(AgentEvent::Ping)
        ));
    }

    #[test]
    fn parse_agent_start() {
        let event = make_event("agent_start", "{}");
        assert!(matches!(
            AgentClient::parse_event(event),
            Some(AgentEvent::AgentStart)
        ));
    }

    #[test]
    fn parse_agent_end_no_error() {
        let event = make_event("agent_end", "{}");
        match AgentClient::parse_event(event) {
            Some(AgentEvent::AgentEnd { error }) => assert!(error.is_none()),
            other => panic!("expected AgentEnd, got {:?}", other),
        }
    }

    #[test]
    fn parse_agent_end_with_error() {
        let event = make_event("agent_end", r#"{"error":"rate limited"}"#);
        match AgentClient::parse_event(event) {
            Some(AgentEvent::AgentEnd { error }) => {
                assert_eq!(error.as_deref(), Some("rate limited"))
            }
            other => panic!("expected AgentEnd, got {:?}", other),
        }
    }

    // ─── parse_event: text events ────────────────────────────────────────────

    #[test]
    fn parse_text_chunk() {
        let event = make_event("text_chunk", r#"{"text":"Hello world"}"#);
        match AgentClient::parse_event(event) {
            Some(AgentEvent::TextChunk(text)) => assert_eq!(text, "Hello world"),
            other => panic!("expected TextChunk, got {:?}", other),
        }
    }

    #[test]
    fn parse_text_chunk_empty_data() {
        let event = make_event("text_chunk", "{}");
        match AgentClient::parse_event(event) {
            Some(AgentEvent::TextChunk(text)) => assert_eq!(text, ""),
            other => panic!("expected TextChunk, got {:?}", other),
        }
    }

    #[test]
    fn parse_thinking_start() {
        let event = make_event("thinking_start", "{}");
        assert!(matches!(
            AgentClient::parse_event(event),
            Some(AgentEvent::ThinkingStart)
        ));
    }

    #[test]
    fn parse_thinking_delta() {
        let event = make_event("thinking_delta", r#"{"text":"Let me think"}"#);
        match AgentClient::parse_event(event) {
            Some(AgentEvent::ThinkingDelta(text)) => assert_eq!(text, "Let me think"),
            other => panic!("expected ThinkingDelta, got {:?}", other),
        }
    }

    #[test]
    fn parse_thinking_end() {
        let event = make_event("thinking_end", "{}");
        assert!(matches!(
            AgentClient::parse_event(event),
            Some(AgentEvent::ThinkingEnd)
        ));
    }

    // ─── parse_event: tool events ────────────────────────────────────────────

    #[test]
    fn parse_tool_start() {
        let event = make_event(
            "tool_start",
            r#"{"tool_id":"call_1","tool_name":"shell","tool_args":"{\"command\":\"ls\"}"}"#,
        );
        match AgentClient::parse_event(event) {
            Some(AgentEvent::ToolStart {
                tool_id,
                tool_name,
                tool_args,
                ..
            }) => {
                assert_eq!(tool_id, "call_1");
                assert_eq!(tool_name, "shell");
                assert_eq!(tool_args.as_deref(), Some("{\"command\":\"ls\"}"));
            }
            other => panic!("expected ToolStart, got {:?}", other),
        }
    }

    #[test]
    fn parse_tool_start_missing_args() {
        let event = make_event("tool_start", r#"{"tool_id":"call_1","tool_name":"read"}"#);
        match AgentClient::parse_event(event) {
            Some(AgentEvent::ToolStart { tool_args, .. }) => assert!(tool_args.is_none()),
            other => panic!("expected ToolStart, got {:?}", other),
        }
    }

    #[test]
    fn parse_tool_start_invalid_json() {
        let event = make_event("tool_start", "not json");
        assert!(AgentClient::parse_event(event).is_none());
    }

    #[test]
    fn parse_tool_delta() {
        let event = make_event(
            "tool_delta",
            r#"{"tool_id":"call_1","text":"partial output"}"#,
        );
        match AgentClient::parse_event(event) {
            Some(AgentEvent::ToolDelta { tool_id, text }) => {
                assert_eq!(tool_id, "call_1");
                assert_eq!(text, "partial output");
            }
            other => panic!("expected ToolDelta, got {:?}", other),
        }
    }

    #[test]
    fn parse_tool_end() {
        let event = make_event("tool_end", r#"{"tool_id":"call_1","text":"file1.txt"}"#);
        match AgentClient::parse_event(event) {
            Some(AgentEvent::ToolEnd { tool_id, text }) => {
                assert_eq!(tool_id, "call_1");
                assert_eq!(text.as_deref(), Some("file1.txt"));
            }
            other => panic!("expected ToolEnd, got {:?}", other),
        }
    }

    #[test]
    fn parse_tool_end_no_text() {
        let event = make_event("tool_end", r#"{"tool_id":"call_1"}"#);
        match AgentClient::parse_event(event) {
            Some(AgentEvent::ToolEnd { text, .. }) => assert!(text.is_none()),
            other => panic!("expected ToolEnd, got {:?}", other),
        }
    }

    // ─── parse_event: approval & error events ────────────────────────────────

    #[test]
    fn parse_approval_request() {
        let event = make_event(
            "approval_request",
            r#"{
                "approval_request_id": "req_1",
                "tool_id": "call_1",
                "tool_name": "shell",
                "kind": "sandbox",
                "risk_level": "high",
                "title": "Dangerous command",
                "summary": "rm -rf /",
                "requested_action": {"command": "rm -rf /"}
            }"#,
        );
        match AgentClient::parse_event(event) {
            Some(AgentEvent::ApprovalRequest {
                approval_request_id,
                tool_name,
                risk_level,
                title,
                summary,
                ..
            }) => {
                assert_eq!(approval_request_id, "req_1");
                assert_eq!(tool_name, "shell");
                assert_eq!(risk_level, "high");
                assert_eq!(title, "Dangerous command");
                assert_eq!(summary, "rm -rf /");
            }
            other => panic!("expected ApprovalRequest, got {:?}", other),
        }
    }

    #[test]
    fn parse_error_event() {
        let event = make_event("error", r#"{"error":"something went wrong"}"#);
        match AgentClient::parse_event(event) {
            Some(AgentEvent::Error(msg)) => assert_eq!(msg, "something went wrong"),
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[test]
    fn parse_error_event_invalid_json() {
        let event = make_event("error", "not json");
        match AgentClient::parse_event(event) {
            Some(AgentEvent::Error(msg)) => assert_eq!(msg, "unknown error"),
            other => panic!("expected Error, got {:?}", other),
        }
    }

    // ─── parse_event: unknown events ─────────────────────────────────────────

    #[test]
    fn parse_unknown_event_returns_none() {
        let event = make_event("custom_event", "{}");
        assert!(AgentClient::parse_event(event).is_none());
    }

    #[test]
    fn parse_empty_type_returns_none() {
        let event = make_event("", "{}");
        assert!(AgentClient::parse_event(event).is_none());
    }

    // ─── SessionState construction ───────────────────────────────────────────

    #[test]
    fn session_state_fields() {
        let state = SessionState {
            model: "openai/gpt-4o".into(),
            image_support: true,
            thinking_level: "medium".into(),
            is_streaming: false,
            context_tokens: 1500,
            context_window: 128000,
            tokens_in: 500,
            tokens_out: 1000,
            query_count: 3,
            session_id: "sess_1".into(),
            session_name: "test".into(),
            cwd: "/tmp".into(),
            auto_compaction: true,
            total_cost: 0.05,
            permission_level: "all".into(),
        };
        assert_eq!(state.model, "openai/gpt-4o");
        assert!(state.image_support);
        assert_eq!(state.context_tokens, 1500);
    }

    // ─── ImageInput construction ─────────────────────────────────────────────

    #[test]
    fn image_input_base64() {
        let img = ImageInput {
            content_type: "image_url".into(),
            data: ImageData::Base64("data:image/png;base64,abc".into()),
            file_path: Some("/tmp/img.png".into()),
        };
        match &img.data {
            ImageData::Base64(d) => assert!(d.starts_with("data:")),
            _ => panic!("expected Base64"),
        }
    }

    #[test]
    fn image_input_url() {
        let img = ImageInput {
            content_type: "image_url".into(),
            data: ImageData::Url("https://example.com/img.png".into()),
            file_path: None,
        };
        match &img.data {
            ImageData::Url(u) => assert!(u.starts_with("https://")),
            _ => panic!("expected Url"),
        }
    }
}

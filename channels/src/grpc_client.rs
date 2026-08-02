//! gRPC client for FutureAgent.
//! Communicates exclusively via gRPC — no direct agent function calls.

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
        /// Canonical terminal state (`completed` / `cancelled` / `error` /
        /// `incomplete`); lets a bridge tell a cancellation apart from a clean
        /// completion without parsing free-text error strings.
        state: Option<String>,
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
    active_runs: Arc<Mutex<HashMap<String, String>>>,
}

/// Canonical run stream with projection snapshots flattened back into their
/// constituent events. Callers can keep one event-processing path regardless
/// of whether the Agent replayed the bounded ring or returned a compressed
/// projection after the cursor fell behind it.
pub struct AgentEventStream {
    inner: tonic::Streaming<proto::StreamEvent>,
    pending: VecDeque<proto::StreamEvent>,
}

impl AgentEventStream {
    pub async fn message(&mut self) -> Result<Option<proto::StreamEvent>> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }
            let Some(event) = self
                .inner
                .message()
                .await
                .map_err(|error| anyhow!("Agent event stream failed: {error}"))?
            else {
                return Ok(None);
            };
            if !event.projection_snapshot {
                return Ok(Some(event));
            }
            self.pending = expand_projection_snapshot(event);
        }
    }
}

fn expand_projection_snapshot(event: proto::StreamEvent) -> VecDeque<proto::StreamEvent> {
    let run_id = event.run_id;
    let session_id = event.session_id;
    let epoch = event.epoch;
    let run_sequence = event.run_sequence;
    event
        .snapshot_events
        .into_iter()
        .map(|projected| proto::StreamEvent {
            r#type: projected.r#type,
            data: projected.data,
            run_id: run_id.clone(),
            idx: projected.idx,
            projection_snapshot: false,
            snapshot_events: Vec::new(),
            snapshot_cursor: 0,
            session_id: session_id.clone(),
            epoch,
            event_id: String::new(),
            timestamp: String::new(),
            session_idx: -1,
            run_sequence,
        })
        .collect()
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
        Ok(Self {
            inner,
            active_runs: Arc::new(Mutex::new(HashMap::new())),
        })
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
    ) -> Result<String> {
        self.prompt_with_policy(session_id, message, images, "reject_if_busy")
            .await
    }

    pub async fn prompt_superseding(
        &mut self,
        session_id: &str,
        message: &str,
        images: Vec<ImageInput>,
    ) -> Result<String> {
        self.prompt_with_policy(session_id, message, images, "supersede_session")
            .await
    }

    async fn prompt_with_policy(
        &mut self,
        session_id: &str,
        message: &str,
        images: Vec<ImageInput>,
        busy_policy: &str,
    ) -> Result<String> {
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

        let ack = self
            .call(
                "prompt",
                session_id,
                RpcCommand {
                    message: message.to_string(),
                    images: proto_images,
                    client_request_id: format!("request_{}", uuid::Uuid::new_v4().simple()),
                    requested_run_id: format!("run_{}", uuid::Uuid::new_v4().simple()),
                    busy_policy: busy_policy.to_string(),
                    ..Default::default()
                },
            )
            .await?;
        let run_id = ack["run_id"]
            .as_str()
            .or_else(|| ack["runId"].as_str())
            .ok_or_else(|| anyhow!("prompt response missing canonical run id"))?
            .to_string();
        if let Ok(mut active_runs) = self.active_runs.lock() {
            active_runs.insert(session_id.to_string(), run_id.clone());
        }
        Ok(run_id)
    }

    pub async fn wait_until_run_active(
        &mut self,
        session_id: &str,
        run_id: &str,
        timeout: Duration,
    ) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let state = self
                .call("get_state", session_id, Default::default())
                .await?;
            if state
                .get("activeRun")
                .and_then(|run| run.get("runId"))
                .and_then(|value| value.as_str())
                == Some(run_id)
            {
                return Ok(());
            }
            let still_queued = state
                .get("queuedRuns")
                .and_then(|runs| runs.as_array())
                .is_some_and(|runs| {
                    runs.iter().any(|run| {
                        run.get("runId").and_then(|value| value.as_str()) == Some(run_id)
                    })
                });
            if !still_queued {
                return Err(anyhow!(
                    "superseding run {run_id} was cancelled before start"
                ));
            }
            if Instant::now() >= deadline {
                return Err(anyhow!("timed out waiting for run {run_id} to start"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Abort current generation.
    pub async fn abort(&mut self, session_id: &str) -> Result<()> {
        let run_id = self
            .active_runs
            .lock()
            .ok()
            .and_then(|active_runs| active_runs.get(session_id).cloned())
            .unwrap_or_default();
        self.call(
            "abort",
            session_id,
            RpcCommand {
                run_id,
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    /// Poll `get_state` until the session has no active run, so a prompt issued
    /// right after [`abort`](Self::abort) isn't rejected by the Agent's run
    /// state machine (the old `is_streaming` flag flipped to idle on abort; the
    /// new state machine keeps the session busy through Cancelling/Finalizing).
    /// Returns as soon as the run clears. Transient `get_state` errors are
    /// retried within the same deadline; a stuck state or timeout is returned as
    /// an explicit error so callers do not immediately issue a prompt that the
    /// Agent will reject as busy.
    pub async fn wait_until_idle(&mut self, session_id: &str, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut delay = Duration::from_millis(100);
        loop {
            let wait_error = match self.call("get_state", session_id, Default::default()).await {
                Ok(resp) => {
                    match resp.get("activeRun").and_then(|v| v.as_object()) {
                        None => return Ok(()),
                        Some(active) => {
                            let state = active.get("state").and_then(|v| v.as_str()).unwrap_or("");
                            if state == "cancellation_stuck" || state == "persistence_degraded" {
                                return Err(anyhow!(
                                    "session {session_id} cannot accept a new prompt while run state is {state}"
                                ));
                            }
                        }
                    }
                    None
                }
                Err(error) => Some(error),
            };
            if Instant::now() >= deadline {
                return Err(wait_error.unwrap_or_else(|| {
                    anyhow!("timed out waiting for session {session_id} to become idle")
                }));
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_millis(500));
        }
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

    /// Atomically attach to one canonical run from its beginning.
    ///
    /// The Agent registers the live receiver and snapshots the replay tail under
    /// one lock, closing the prompt-ack -> subscribe loss window. If the bounded
    /// ring has already rolled over, [`AgentEventStream`] transparently expands
    /// the returned projection snapshot.
    pub async fn stream_run_events(
        &mut self,
        session_id: &str,
        run_id: &str,
    ) -> Result<AgentEventStream> {
        let request = tonic::Request::new(StreamRequest {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            event_types: vec![],
            after_idx: -1,
            atomic_attach: true,
        });
        let inner = self
            .inner
            .stream_events(request)
            .await
            .map_err(|e| anyhow!("Failed to attach to run {run_id}: {e}"))?
            .into_inner();
        Ok(AgentEventStream {
            inner,
            pending: VecDeque::new(),
        })
    }

    /// Parse a StreamEvent into an AgentEvent, paired with the event's canonical
    /// `run_id` so callers can drop events that belong to a different run on the
    /// same session (another client, or a stale tail after a supersede) instead
    /// of letting a foreign `agent_end` finalize their reply.
    pub fn parse_event(event: proto::StreamEvent) -> Option<(String, AgentEvent)> {
        let parsed: Option<AgentEvent> = match event.r#type.as_str() {
            "ping" => Some(AgentEvent::Ping),
            "agent_start" => Some(AgentEvent::AgentStart),
            "agent_end" => {
                let data = serde_json::from_str::<Value>(&event.data).ok();
                let error = data
                    .as_ref()
                    .and_then(|d| d["error"].as_str().map(|s| s.to_string()));
                let state = data
                    .as_ref()
                    .and_then(|d| d["state"].as_str().map(|s| s.to_string()));
                Some(AgentEvent::AgentEnd { error, state })
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
        };
        parsed.map(|ev| (event.run_id.clone(), ev))
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
            ..Default::default()
        }
    }

    /// `parse_event` also yields the event's run_id; the unit tests below only
    /// care about the decoded variant, so strip the id here.
    fn parsed(event: proto::StreamEvent) -> Option<AgentEvent> {
        AgentClient::parse_event(event).map(|(_, ev)| ev)
    }

    // ─── parse_event: basic events ───────────────────────────────────────────

    #[test]
    fn parse_event_carries_run_id() {
        let event = make_event("text_chunk", r#"{"text":"hi"}"#);
        let (run_id, ev) = AgentClient::parse_event(event).expect("parsed");
        assert_eq!(run_id, "run_1");
        assert!(matches!(ev, AgentEvent::TextChunk(t) if t == "hi"));
    }

    #[test]
    fn projection_snapshot_expands_in_order_with_canonical_envelope() {
        let snapshot = proto::StreamEvent {
            r#type: "run_snapshot".to_string(),
            run_id: "run_snapshot_1".to_string(),
            projection_snapshot: true,
            snapshot_events: vec![
                proto::ProjectedRunEvent {
                    r#type: "text_chunk".to_string(),
                    data: r#"{"text":"hello"}"#.to_string(),
                    idx: 4,
                },
                proto::ProjectedRunEvent {
                    r#type: "agent_end".to_string(),
                    data: r#"{"state":"completed"}"#.to_string(),
                    idx: 5,
                },
            ],
            session_id: "session_1".to_string(),
            epoch: 7,
            ..Default::default()
        };

        let expanded: Vec<_> = expand_projection_snapshot(snapshot).into_iter().collect();
        assert_eq!(expanded.len(), 2);
        assert_eq!(expanded[0].r#type, "text_chunk");
        assert_eq!(expanded[0].idx, 4);
        assert_eq!(expanded[1].r#type, "agent_end");
        assert_eq!(expanded[1].idx, 5);
        assert!(expanded.iter().all(|event| {
            event.run_id == "run_snapshot_1"
                && event.session_id == "session_1"
                && event.epoch == 7
                && !event.projection_snapshot
        }));
    }

    #[test]
    fn parse_agent_end_state() {
        let event = make_event("agent_end", r#"{"state":"cancelled"}"#);
        match parsed(event) {
            Some(AgentEvent::AgentEnd { state, error }) => {
                assert_eq!(state.as_deref(), Some("cancelled"));
                assert!(error.is_none());
            }
            other => panic!("expected AgentEnd, got {:?}", other),
        }
    }

    #[test]
    fn parse_ping() {
        let event = make_event("ping", "{}");
        assert!(matches!(parsed(event), Some(AgentEvent::Ping)));
    }

    #[test]
    fn parse_agent_start() {
        let event = make_event("agent_start", "{}");
        assert!(matches!(parsed(event), Some(AgentEvent::AgentStart)));
    }

    #[test]
    fn parse_agent_end_no_error() {
        let event = make_event("agent_end", "{}");
        match parsed(event) {
            Some(AgentEvent::AgentEnd { error, .. }) => assert!(error.is_none()),
            other => panic!("expected AgentEnd, got {:?}", other),
        }
    }

    #[test]
    fn parse_agent_end_with_error() {
        let event = make_event("agent_end", r#"{"error":"rate limited"}"#);
        match parsed(event) {
            Some(AgentEvent::AgentEnd { error, .. }) => {
                assert_eq!(error.as_deref(), Some("rate limited"))
            }
            other => panic!("expected AgentEnd, got {:?}", other),
        }
    }

    // ─── parse_event: text events ────────────────────────────────────────────

    #[test]
    fn parse_text_chunk() {
        let event = make_event("text_chunk", r#"{"text":"Hello world"}"#);
        match parsed(event) {
            Some(AgentEvent::TextChunk(text)) => assert_eq!(text, "Hello world"),
            other => panic!("expected TextChunk, got {:?}", other),
        }
    }

    #[test]
    fn parse_text_chunk_empty_data() {
        let event = make_event("text_chunk", "{}");
        match parsed(event) {
            Some(AgentEvent::TextChunk(text)) => assert_eq!(text, ""),
            other => panic!("expected TextChunk, got {:?}", other),
        }
    }

    #[test]
    fn parse_thinking_start() {
        let event = make_event("thinking_start", "{}");
        assert!(matches!(parsed(event), Some(AgentEvent::ThinkingStart)));
    }

    #[test]
    fn parse_thinking_delta() {
        let event = make_event("thinking_delta", r#"{"text":"Let me think"}"#);
        match parsed(event) {
            Some(AgentEvent::ThinkingDelta(text)) => assert_eq!(text, "Let me think"),
            other => panic!("expected ThinkingDelta, got {:?}", other),
        }
    }

    #[test]
    fn parse_thinking_end() {
        let event = make_event("thinking_end", "{}");
        assert!(matches!(parsed(event), Some(AgentEvent::ThinkingEnd)));
    }

    // ─── parse_event: tool events ────────────────────────────────────────────

    #[test]
    fn parse_tool_start() {
        let event = make_event(
            "tool_start",
            r#"{"tool_id":"call_1","tool_name":"shell","tool_args":"{\"command\":\"ls\"}"}"#,
        );
        match parsed(event) {
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
        match parsed(event) {
            Some(AgentEvent::ToolStart { tool_args, .. }) => assert!(tool_args.is_none()),
            other => panic!("expected ToolStart, got {:?}", other),
        }
    }

    #[test]
    fn parse_tool_start_invalid_json() {
        let event = make_event("tool_start", "not json");
        assert!(parsed(event).is_none());
    }

    #[test]
    fn parse_tool_delta() {
        let event = make_event(
            "tool_delta",
            r#"{"tool_id":"call_1","text":"partial output"}"#,
        );
        match parsed(event) {
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
        match parsed(event) {
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
        match parsed(event) {
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
        match parsed(event) {
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
        match parsed(event) {
            Some(AgentEvent::Error(msg)) => assert_eq!(msg, "something went wrong"),
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[test]
    fn parse_error_event_invalid_json() {
        let event = make_event("error", "not json");
        match parsed(event) {
            Some(AgentEvent::Error(msg)) => assert_eq!(msg, "unknown error"),
            other => panic!("expected Error, got {:?}", other),
        }
    }

    // ─── parse_event: unknown events ─────────────────────────────────────────

    #[test]
    fn parse_unknown_event_returns_none() {
        let event = make_event("custom_event", "{}");
        assert!(parsed(event).is_none());
    }

    #[test]
    fn parse_empty_type_returns_none() {
        let event = make_event("", "{}");
        assert!(parsed(event).is_none());
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

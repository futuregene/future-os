//! gRPC client for FutureAgent.
//! Communicates exclusively via gRPC — no direct agent function calls.

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Generated proto code lives in the future-rpc crate (single codegen owner;
// typed-RPC milestone). Re-exported under the historical module name so call
// sites keep their `proto::...` paths.
use future_rpc::proto;

// The wire-contract event vocabulary and its parser live in future-rpc so
// every client decodes events the same way.
pub use future_rpc::events::AgentEvent;

use proto::future_agent_client::FutureAgentClient;
use proto::{RpcCommand, StreamRequest};

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
            // Keep the typed payload so expanded events decode through the
            // same path as live ones.
            payload: projected.payload,
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
            // entry_id comes from `extra` (approval_decision passes the
            // request id through it) — an explicit field here would shadow it.
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

        // Typed payload first, JSON `data` fallback (old agent / untyped
        // commands) — the shared decode keeps every client consistent.
        Ok(future_rpc::decode::response_data(&response))
    }

    /// Create a new agent session. Returns the session_id.
    pub async fn new_session(&mut self, cwd: &str, created_by: &str) -> Result<String> {
        let resp = self
            .call(
                "new_session",
                "",
                RpcCommand {
                    cwd: cwd.to_string(),
                    created_by: created_by.to_string(),
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
            // Canonical `sessionName` since audit item 1; fall back to the
            // legacy `session_name` emitted by older agents.
            session_name: resp["sessionName"]
                .as_str()
                .or_else(|| resp["session_name"].as_str())
                .unwrap_or("")
                .to_string(),
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
    ///
    /// Delegates to the shared future-rpc parser (typed payload first, JSON
    /// `data` fallback).
    pub fn parse_event(event: proto::StreamEvent) -> Option<(String, AgentEvent)> {
        future_rpc::events::parse_agent_event(&event)
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

    // Event-parsing coverage lives with the shared parser in future-rpc
    // (`events::tests`), where every case runs against both the JSON-data
    // twin (old agent) and the typed-payload twin (new agent).

    #[test]
    fn parse_event_delegates_to_shared_parser() {
        // JSON-data path.
        let event = proto::StreamEvent {
            r#type: "text_chunk".to_string(),
            data: r#"{"text":"hi"}"#.to_string(),
            run_id: "run_1".to_string(),
            ..Default::default()
        };
        let (run_id, ev) = AgentClient::parse_event(event).expect("parsed");
        assert_eq!(run_id, "run_1");
        assert!(matches!(ev, AgentEvent::TextChunk(t) if t == "hi"));

        // Typed-payload path.
        let payload = future_rpc::encode::event_payload("text_chunk", r#"{"text":"yo"}"#)
            .expect("text_chunk encodes");
        let typed = proto::StreamEvent {
            r#type: "text_chunk".to_string(),
            data: String::new(),
            run_id: "run_2".to_string(),
            payload: Some(payload),
            ..Default::default()
        };
        let (run_id, ev) = AgentClient::parse_event(typed).expect("parsed");
        assert_eq!(run_id, "run_2");
        assert!(matches!(ev, AgentEvent::TextChunk(t) if t == "yo"));
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
                    payload: None,
                },
                proto::ProjectedRunEvent {
                    r#type: "agent_end".to_string(),
                    data: r#"{"state":"completed"}"#.to_string(),
                    idx: 5,
                    payload: None,
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
    fn projection_expansion_keeps_typed_payloads() {
        let payload = future_rpc::encode::event_payload("text_chunk", r#"{"text":"hi"}"#)
            .expect("text_chunk encodes");
        let snapshot = proto::StreamEvent {
            r#type: "run_snapshot".to_string(),
            run_id: "run_snapshot_1".to_string(),
            projection_snapshot: true,
            snapshot_events: vec![proto::ProjectedRunEvent {
                r#type: "text_chunk".to_string(),
                data: r#"{"text":"hi"}"#.to_string(),
                idx: 4,
                payload: Some(payload),
            }],
            session_id: "session_1".to_string(),
            ..Default::default()
        };
        let expanded: Vec<_> = expand_projection_snapshot(snapshot).into_iter().collect();
        assert!(
            expanded[0].payload.is_some(),
            "expanded events must carry the typed payload"
        );
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
        assert!(matches!(&img.data, ImageData::Base64(d) if d.starts_with("data:")));
    }

    #[test]
    fn image_input_url() {
        let img = ImageInput {
            content_type: "image_url".into(),
            data: ImageData::Url("https://example.com/img.png".into()),
            file_path: None,
        };
        assert!(matches!(&img.data, ImageData::Url(u) if u.starts_with("https://")));
    }

    // ─── Live calls against the mock gRPC agent ──────────────────────────────

    use crate::test_support::{self as ts, MockState};

    async fn connect_to(state: MockState) -> (AgentClient, ts::SharedState) {
        let (addr, shared) = ts::spawn_mock_grpc(state).await;
        let client = AgentClient::connect(&addr).await.expect("connect");
        (client, shared)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connect_variants() {
        // Plain host:port works.
        let (addr, _) = ts::spawn_mock_grpc(MockState::default()).await;
        assert!(AgentClient::connect(&addr).await.is_ok());
        // http:// prefix is stripped.
        assert!(AgentClient::connect(&format!("http://{}", addr)).await.is_ok());
        // Unreachable → error mentioning the address.
        let err = AgentClient::connect("127.0.0.1:1").await.err().unwrap();
        assert!(err.to_string().contains("127.0.0.1:1"), "{err}");
        // Garbage URI → endpoint parse error.
        assert!(AgentClient::connect("%%not a uri%%").await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn call_error_variants() {
        // success=false with a message
        let mut state = MockState::default();
        state.fail_commands.insert("compact".to_string());
        state.fail_silent.insert("abort".to_string());
        state.status_error.insert("set_cwd".to_string());
        let (mut c, _) = connect_to(state).await;

        let err = c.compact("s").await.unwrap_err();
        assert!(err.to_string().contains("mock failure: compact"), "{err}");
        let err = c.abort("s").await.unwrap_err();
        assert!(err.to_string().contains("unknown error"), "{err}");
        let err = c.set_cwd("s", "/x").await.unwrap_err();
        assert!(err.to_string().contains("gRPC call 'set_cwd' failed"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn new_session_ok_and_missing_id() {
        let (mut c, _) = connect_to(MockState::default()).await;
        let sid = c.new_session("/tmp", "test").await.unwrap();
        assert_eq!(sid, "mock-session-1");

        let mut state = MockState::default();
        state.responses.insert("new_session".into(), "{}".into());
        let (mut c, _) = connect_to(state).await;
        let err = c.new_session("/tmp", "test").await.unwrap_err();
        assert!(err.to_string().contains("missing sessionId"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_tracks_run_and_maps_images() {
        let (mut c, shared) = connect_to(MockState::default()).await;
        let images = vec![
            ImageInput {
                content_type: "image_url".into(),
                data: ImageData::Url("https://x/img.png".into()),
                file_path: None,
            },
            ImageInput {
                content_type: "image_url".into(),
                data: ImageData::Base64("data:image/png;base64,AA==".into()),
                file_path: Some("/tmp/i.png".into()),
            },
        ];
        let run = c.prompt("sess", "hello", images).await.unwrap();
        assert_eq!(run, "mock-run-1");
        // prompt_superseding uses the busy_policy override.
        let run2 = c.prompt_superseding("sess", "again", vec![]).await.unwrap();
        assert_eq!(run2, "mock-run-2");

        let prompts = ts::recorded_of(&shared, "prompt");
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0].busy_policy, "reject_if_busy");
        assert_eq!(prompts[1].busy_policy, "supersede_session");
        assert_eq!(prompts[0].images.len(), 2);
        assert!(matches!(
            prompts[0].images[0].content,
            Some(proto::image_content::Content::Url(_))
        ));
        assert!(matches!(
            prompts[0].images[1].content,
            Some(proto::image_content::Content::Base64(_))
        ));
        assert_eq!(prompts[0].images[1].file_path, "/tmp/i.png");
        assert!(!prompts[0].client_request_id.is_empty());
        assert!(!prompts[0].requested_run_id.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prompt_run_id_variants() {
        // camelCase runId fallback.
        let mut state = MockState::default();
        state.responses.insert("prompt".into(), r#"{"runId":"r-camel"}"#.into());
        let (mut c, _) = connect_to(state).await;
        assert_eq!(c.prompt("s", "m", vec![]).await.unwrap(), "r-camel");

        // Missing run id entirely → error.
        let mut state = MockState::default();
        state.responses.insert("prompt".into(), "{}".into());
        let (mut c, _) = connect_to(state).await;
        let err = c.prompt("s", "m", vec![]).await.unwrap_err();
        assert!(err.to_string().contains("missing canonical run id"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wait_until_run_active_paths() {
        // Already active (mock default: prompt registers the active run).
        let (mut c, _) = connect_to(MockState::default()).await;
        let run = c.prompt("sess", "m", vec![]).await.unwrap();
        c.wait_until_run_active("sess", &run, Duration::from_secs(2))
            .await
            .unwrap();

        // Queued first, then active.
        let mut state = MockState::default();
        state.sequences.insert(
            "get_state".into(),
            vec![
                r#"{"queuedRuns":[{"runId":"r-q"}]}"#.to_string(),
                r#"{"activeRun":{"runId":"r-q","state":"running"}}"#.to_string(),
            ],
        );
        let (mut c, _) = connect_to(state).await;
        c.wait_until_run_active("sess", "r-q", Duration::from_secs(2))
            .await
            .unwrap();

        // Neither active nor queued → superseded run cancelled before start.
        let mut state = MockState::default();
        state
            .responses
            .insert("get_state".into(), r#"{"queuedRuns":[]}"#.into());
        let (mut c, _) = connect_to(state).await;
        let err = c
            .wait_until_run_active("sess", "r-gone", Duration::from_secs(2))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cancelled before start"), "{err}");

        // Always queued → times out.
        let mut state = MockState::default();
        state.responses.insert(
            "get_state".into(),
            r#"{"queuedRuns":[{"runId":"r-slow"}]}"#.into(),
        );
        let (mut c, _) = connect_to(state).await;
        let err = c
            .wait_until_run_active("sess", "r-slow", Duration::from_millis(250))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn abort_sends_tracked_run_id() {
        let (mut c, shared) = connect_to(MockState::default()).await;
        // No prompt yet → empty run_id.
        c.abort("sess").await.unwrap();
        let run = c.prompt("sess", "m", vec![]).await.unwrap();
        c.abort("sess").await.unwrap();
        let aborts = ts::recorded_of(&shared, "abort");
        assert_eq!(aborts.len(), 2);
        assert_eq!(aborts[0].run_id, "");
        assert_eq!(aborts[1].run_id, run);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wait_until_idle_paths() {
        // No active run → immediately idle.
        let (mut c, _) = connect_to(MockState::default()).await;
        c.wait_until_idle("sess", Duration::from_secs(2)).await.unwrap();

        // Stuck states → explicit error.
        for stuck in ["cancellation_stuck", "persistence_degraded"] {
            let mut state = MockState::default();
            state.responses.insert(
                "get_state".into(),
                format!(r#"{{"activeRun":{{"runId":"r","state":"{stuck}"}}}}"#),
            );
            let (mut c, _) = connect_to(state).await;
            let err = c
                .wait_until_idle("sess", Duration::from_secs(2))
                .await
                .unwrap_err();
            assert!(err.to_string().contains(stuck), "{err}");
        }

        // Busy forever → timeout error.
        let mut state = MockState::default();
        state.responses.insert(
            "get_state".into(),
            r#"{"activeRun":{"runId":"r","state":"running"}}"#.into(),
        );
        let (mut c, _) = connect_to(state).await;
        let err = c
            .wait_until_idle("sess", Duration::from_millis(250))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");

        // Transient transport failure, then idle → retries succeed.
        let mut state = MockState::default();
        state.fail_times.insert("get_state".into(), 1);
        let (mut c, _) = connect_to(state).await;
        c.wait_until_idle("sess", Duration::from_secs(2)).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_state_parses_all_fields() {
        let (mut c, _) = connect_to(MockState::default()).await;
        let s = c.get_state("sess").await.unwrap();
        assert_eq!(s.model, "future/k3");
        assert!(s.image_support);
        assert_eq!(s.thinking_level, "high");
        assert_eq!(s.context_tokens, 100);
        assert_eq!(s.context_window, 1000);
        assert_eq!(s.tokens_in, 10);
        assert_eq!(s.tokens_out, 20);
        assert_eq!(s.query_count, 3);
        assert_eq!(s.session_id, "sess");
        assert_eq!(s.cwd, "/tmp");
        assert!(s.auto_compaction);
        assert!((s.total_cost - 0.01).abs() < 1e-9);
        assert_eq!(s.permission_level, "all");

        // Legacy session_name fallback + field defaults on an empty payload.
        let mut state = MockState::default();
        state
            .responses
            .insert("get_state".into(), r#"{"session_name":"legacy"}"#.into());
        let (mut c, _) = connect_to(state).await;
        let s = c.get_state("x").await.unwrap();
        assert_eq!(s.model, "?");
        assert_eq!(s.session_name, "legacy");
        assert_eq!(s.thinking_level, "off");
        assert_eq!(s.permission_level, "all");

        // Canonical sessionName wins over the legacy key.
        let mut state = MockState::default();
        state.responses.insert(
            "get_state".into(),
            r#"{"sessionName":"canon","session_name":"legacy"}"#.into(),
        );
        let (mut c, _) = connect_to(state).await;
        assert_eq!(c.get_state("x").await.unwrap().session_name, "canon");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_available_models_ok_and_empty() {
        let (mut c, _) = connect_to(MockState::default()).await;
        let models = c.get_available_models("sess").await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "future/k3");
        assert_eq!(models[0].name, "K3");
        assert_eq!(models[0].provider, "future");
        assert!(models[0].image);
        assert_eq!(models[0].context_window, 256000);
        assert_eq!(models[0].max_tokens, 0);
        assert!(!models[0].reasoning);

        // No models key → empty vec.
        let mut state = MockState::default();
        state
            .responses
            .insert("list_models".into(), "{}".into());
        let (mut c, _) = connect_to(state).await;
        assert!(c.get_available_models("sess").await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn simple_command_wrappers_send_right_fields() {
        let (mut c, shared) = connect_to(MockState::default()).await;
        c.set_model("s", "future/k3").await.unwrap();
        c.set_thinking_level("s", "high").await.unwrap();
        c.set_permission_level("s", "workspace").await.unwrap();
        c.set_cwd("s", "/work").await.unwrap();
        c.switch_session("s").await.unwrap();
        c.approval_decision("s", "req_1", true, "ok via card").await.unwrap();
        c.approval_decision("s", "req_2", false, "no").await.unwrap();

        let set_model = ts::recorded_of(&shared, "set_model");
        assert_eq!(set_model[0].model_id, "future/k3");
        let thinking = ts::recorded_of(&shared, "set_thinking_level");
        assert_eq!(thinking[0].level, "high");
        let perm = ts::recorded_of(&shared, "set_permission_level");
        assert_eq!(perm[0].level, "workspace");
        let cwd = ts::recorded_of(&shared, "set_cwd");
        assert_eq!(cwd[0].cwd, "/work");
        let switch = ts::recorded_of(&shared, "switch_session");
        assert_eq!(switch[0].session_id, "s");
        let approvals = ts::recorded_of(&shared, "approval_decision");
        assert_eq!(approvals[0].mode, "approved");
        assert_eq!(approvals[0].entry_id, "req_1");
        assert_eq!(approvals[0].message, "ok via card");
        assert_eq!(approvals[1].mode, "rejected");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stream_run_events_ok_and_attach_failure() {
        // Attach failure surfaces as an error.
        let mut state = MockState::default();
        state.stream_status_error = true;
        let (mut c, _) = connect_to(state).await;
        let err = c.stream_run_events("sess", "r").await.err().unwrap();
        assert!(err.to_string().contains("Failed to attach"), "{err}");

        // Happy path: events flow, stream ends with None.
        let mut state = MockState::default();
        state.events = vec![
            ts::ev("r", 0, "agent_start", "{}"),
            ts::ev("r", 1, "text_chunk", r#"{"text":"hi"}"#),
        ];
        let (mut c, _) = connect_to(state).await;
        let mut stream = c.stream_run_events("sess", "r").await.unwrap();
        let e1 = stream.message().await.unwrap().expect("event 1");
        assert_eq!(e1.r#type, "agent_start");
        let e2 = stream.message().await.unwrap().expect("event 2");
        assert_eq!(e2.r#type, "text_chunk");
        assert!(stream.message().await.unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stream_expands_projection_snapshots_in_place() {
        let mut snapshot = ts::ev("r", 0, "run_snapshot", "");
        snapshot.projection_snapshot = true;
        snapshot.snapshot_events = vec![
            proto::ProjectedRunEvent {
                r#type: "text_chunk".into(),
                data: r#"{"text":"a"}"#.into(),
                idx: 7,
                payload: None,
            },
            proto::ProjectedRunEvent {
                r#type: "text_chunk".into(),
                data: r#"{"text":"b"}"#.into(),
                idx: 8,
                payload: None,
            },
        ];
        let mut state = MockState::default();
        state.events = vec![ts::ev("r", 0, "agent_start", "{}"), snapshot];
        let (mut c, _) = connect_to(state).await;
        let mut stream = c.stream_run_events("sess", "r").await.unwrap();
        assert_eq!(stream.message().await.unwrap().unwrap().r#type, "agent_start");
        let a = stream.message().await.unwrap().unwrap();
        let b = stream.message().await.unwrap().unwrap();
        assert_eq!((a.r#type.as_str(), a.idx), ("text_chunk", 7));
        assert_eq!((b.r#type.as_str(), b.idx), ("text_chunk", 8));
        assert!(stream.message().await.unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stream_mid_error_surfaces() {
        let mut state = MockState::default();
        state.events = vec![ts::ev("r", 0, "agent_start", "{}")];
        state.stream_mid_error_after = Some(1);
        let (mut c, _) = connect_to(state).await;
        let mut stream = c.stream_run_events("sess", "r").await.unwrap();
        assert!(stream.message().await.unwrap().is_some());
        let err = stream.message().await.unwrap_err();
        assert!(err.to_string().contains("event stream failed"), "{err}");
    }
}

//! Thin gRPC client for FutureAgent — the ONLY surface this control plane
//! uses to drive the agent. No direct agent function calls (same contract as
//! `channels/src/grpc_client.rs`).
//!
//! Responsibilities here are purely transport: build RpcCommands, parse
//! responses, subscribe to the event stream. All loop *policy* lives in
//! `loop_engine.rs` / `state.rs` — this module must stay policy-free.

use anyhow::{anyhow, Result};
use serde_json::Value;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/proto.rs"));
}

use proto::future_agent_client::FutureAgentClient;
use proto::{RpcCommand, StreamEvent, StreamRequest};

/// What the agent reported at the end of one bounded turn (agent_end).
#[derive(Debug, Clone)]
pub struct RunSummary {
    pub run_id: String,
    /// Canonical terminal state: `completed` / `cancelled` / `error` /
    /// `incomplete` (mirrors the agent's agent_end `state` field).
    pub terminal_state: String,
    pub error: Option<String>,
    /// Tool names invoked this turn, in order.
    pub tools: Vec<String>,
    /// Concatenated assistant text this turn (bounded to a few KB).
    pub text: String,
    /// usage payload from agent_end, when present.
    pub usage: Option<Value>,
    pub duration_ms: Option<i64>,
}

/// Cumulative token/cost accounting for a session (from get_state).
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionTotals {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
}

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
            .timeout(std::time::Duration::from_secs(120));
        let channel = endpoint
            .connect()
            .await
            .map_err(|e| anyhow!("Failed to connect to agent at {addr}: {e}"))?;
        Ok(Self {
            inner: FutureAgentClient::new(channel),
        })
    }

    async fn call(&mut self, cmd_type: &str, session_id: &str, extra: RpcCommand) -> Result<Value> {
        let request = tonic::Request::new(RpcCommand {
            id: uuid::Uuid::new_v4().to_string(),
            r#type: cmd_type.to_string(),
            session_id: session_id.to_string(),
            ..extra
        });
        let response = self
            .inner
            .execute_command(request)
            .await
            .map_err(|e| anyhow!("gRPC '{cmd_type}' failed: {e}"))?
            .into_inner();
        if !response.success {
            let code = if response.error_code.is_empty() {
                "unknown".to_string()
            } else {
                response.error_code.clone()
            };
            return Err(anyhow!(
                "Command '{cmd_type}' failed [{code}]: {}",
                if response.error.is_empty() {
                    "unknown error"
                } else {
                    &response.error
                }
            ));
        }
        if response.data.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&response.data)
            .map_err(|e| anyhow!("'{cmd_type}' returned invalid JSON: {e}"))
    }

    /// List models available from the agent (from auth.json / models.json,
    /// merged with the built-in catalog). Returns the raw list_models payload.
    pub async fn list_models(&mut self) -> Result<Value> {
        self.call("list_models", "", Default::default()).await
    }

    /// Create a fresh, isolated session for this goal. One goal = one session
    /// (LoopX: durable identity is the goal, not a chat thread).
    pub async fn new_session(&mut self, cwd: &str) -> Result<String> {
        let resp = self
            .call(
                "new_session",
                "",
                RpcCommand {
                    cwd: cwd.to_string(),
                    ..Default::default()
                },
            )
            .await?;
        resp["sessionId"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("new_session response missing sessionId: {resp}"))
    }

    /// Append the stable goal boundary (objective + acceptance + rules) to the
    /// built-in system prompt. Done once per session: set_system_prompt would
    /// REPLACE the built-in identity/tool instructions, which is not what we
    /// want — the agent stays a normal agent, just governed.
    pub async fn append_system_prompt(&mut self, session_id: &str, text: &str) -> Result<()> {
        self.call(
            "append_system_prompt",
            session_id,
            RpcCommand {
                system_prompt: text.to_string(),
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    /// Select the model for this session (e.g. "future/deepseek-v4-flash").
    pub async fn set_model(&mut self, session_id: &str, model: &str) -> Result<()> {
        self.call(
            "set_model",
            session_id,
            RpcCommand {
                model_id: model.to_string(),
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    /// Set the thinking level: "off" | "minimal" | "low" | "medium" | "high" | "xhigh".
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

    /// Enqueue one bounded turn. Returns the canonical run_id.
    ///
    /// Idempotency: the caller owns `client_request_id`; re-sending the same
    /// key must NOT double-execute (the agent dedups via knows_request).
    pub async fn prompt(
        &mut self,
        session_id: &str,
        message: &str,
        client_request_id: &str,
    ) -> Result<String> {
        let resp = self
            .call(
                "prompt",
                session_id,
                RpcCommand {
                    message: message.to_string(),
                    client_request_id: client_request_id.to_string(),
                    busy_policy: "reject_if_busy".to_string(),
                    ..Default::default()
                },
            )
            .await?;
        resp["run_id"]
            .as_str()
            .or_else(|| resp["runId"].as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("prompt response missing run_id: {resp}"))
    }

    /// Abort the currently active run — the orchestrator's timeout / no-progress
    /// termination handle. Not exercised by this demo (kept as the one missing
    /// control surface a real loop needs).
    #[allow(dead_code)]
    pub async fn abort(&mut self, session_id: &str) -> Result<()> {
        self.call("abort", session_id, Default::default()).await?;
        Ok(())
    }

    /// Cumulative session token/cost totals (for per-turn accounting we take
    /// the delta between two calls).
    pub async fn session_totals(&mut self, session_id: &str) -> Result<SessionTotals> {
        let s = self
            .call("get_state", session_id, Default::default())
            .await?;
        Ok(SessionTotals {
            tokens_in: s["tokensIn"].as_u64().unwrap_or(0),
            tokens_out: s["tokensOut"].as_u64().unwrap_or(0),
            cost: s["totalCost"].as_f64().unwrap_or(0.0),
        })
    }

    /// Subscribe to one canonical run from its beginning (atomic attach closes
    /// the prompt-ack -> subscribe loss window) and collect events until
    /// `agent_end` (or the stream closes / errors).
    pub async fn run_turn(&mut self, session_id: &str, run_id: &str) -> Result<RunSummary> {
        let request = tonic::Request::new(StreamRequest {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            event_types: vec![],
            after_idx: -1,
            atomic_attach: true,
        });
        let mut stream = self
            .inner
            .stream_events(request)
            .await
            .map_err(|e| anyhow!("Failed to attach to run {run_id}: {e}"))?
            .into_inner();

        let mut summary = RunSummary {
            run_id: run_id.to_string(),
            terminal_state: "incomplete".to_string(),
            error: None,
            tools: vec![],
            text: String::new(),
            usage: None,
            duration_ms: None,
        };

        use tokio_stream::StreamExt;
        while let Some(ev) = stream.next().await {
            let ev = ev.map_err(|e| anyhow!("stream error on run {run_id}: {e}"))?;
            if ev.run_id != run_id {
                // Stale tail from a previous run on the same session (or a
                // supersede) — ignore foreign events.
                continue;
            }
            let Some(data) = parse_data(&ev) else {
                continue;
            };
            match ev.r#type.as_str() {
                "tool_start" => {
                    if let Some(name) = data.get("tool_name").and_then(|v| v.as_str()) {
                        summary.tools.push(name.to_string());
                    }
                }
                "text_chunk" => {
                    if let Some(text) = data.get("text").and_then(|v| v.as_str()) {
                        summary.text.push_str(text);
                        if summary.text.len() > 8_000 {
                            summary.text.truncate(8_000);
                        }
                    }
                }
                "agent_end" => {
                    summary.error = data.get("error").and_then(|v| v.as_str()).map(String::from);
                    summary.terminal_state = data
                        .get("state")
                        .and_then(|v| v.as_str())
                        .unwrap_or("completed")
                        .to_string();
                    summary.usage = data.get("usage").cloned();
                    summary.duration_ms = data.get("duration_ms").and_then(|v| v.as_i64());
                    break;
                }
                "error" => {
                    let msg = data
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown error");
                    summary.error = Some(msg.to_string());
                    summary.terminal_state = "error".to_string();
                    break;
                }
                _ => {}
            }
        }
        Ok(summary)
    }
}

fn parse_data(ev: &StreamEvent) -> Option<Value> {
    if ev.data.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(&ev.data).ok()
}

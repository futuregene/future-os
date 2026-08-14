//! Thin gRPC client for FutureAgent — the ONLY surface this control plane
//! uses to drive the agent. No direct agent function calls (same contract as
//! `channels/src/grpc_client.rs`).
//!
//! Responsibilities here are purely transport: build RpcCommands, parse
//! responses, subscribe to the event stream. All loop *policy* lives in
//! `loop_engine.rs` / `state.rs` — this module must stay policy-free.

use anyhow::{anyhow, Result};
use future_rpc::proto::future_agent_client::FutureAgentClient;
use future_rpc::proto::{RpcCommand, StreamEvent, StreamRequest};
use serde_json::Value;

/// Agent gRPC address (`host:port`). Defaults to the local agent; overridable
/// via FUTURE_LOOP_AGENT_ADDR so tests can point the control plane at a mock
/// server (mirrors the BROWSER_LAUNCHER_OVERRIDE test-hook precedent in cli).
pub fn agent_addr() -> String {
    std::env::var("FUTURE_LOOP_AGENT_ADDR").unwrap_or_else(|_| "127.0.0.1:50051".to_string())
}

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

/// O3: write-class tools — a `tool_start` of any of these resets the idle
/// clock. Everything else (read, grep, todo, …) is observation-only.
pub fn is_write_class_tool(name: &str) -> bool {
    matches!(name, "write" | "edit" | "shell")
}

/// O3: per-turn progress signals observed on the event stream. Shared between
/// `run_turn` (writer) and the run loop (reader) so the budget-truncation
/// path can still evaluate progress after dropping the turn future. Atomics:
/// written from the stream task, read from the loop task.
#[derive(Debug)]
pub struct TurnProgressTracker {
    /// Wall-clock epoch secs at turn start (the idle baseline when no
    /// write-class tool ever starts).
    turn_start_at: std::sync::atomic::AtomicU64,
    /// Wall-clock epoch secs of the last write-class tool start; 0 = none.
    last_write_tool_at: std::sync::atomic::AtomicU64,
    /// Total tool_start events observed (all classes).
    tool_calls_total: std::sync::atomic::AtomicU32,
}

/// Immutable snapshot of [`TurnProgressTracker`] for turn-end evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnProgressSnapshot {
    pub turn_start_at: u64,
    /// `None` when no write-class tool started this turn.
    pub last_write_tool_at: Option<u64>,
    pub tool_calls_total: u32,
}

impl TurnProgressTracker {
    pub fn new(turn_start_at: u64) -> Self {
        Self {
            turn_start_at: std::sync::atomic::AtomicU64::new(turn_start_at),
            last_write_tool_at: std::sync::atomic::AtomicU64::new(0),
            tool_calls_total: std::sync::atomic::AtomicU32::new(0),
        }
    }

    /// Record one `tool_start` event (the live-log wall_ts doubles as the
    /// observation clock, same as the streamed events).
    pub fn observe_tool_start(&self, tool: &str, at: u64) {
        use std::sync::atomic::Ordering;
        self.tool_calls_total.fetch_add(1, Ordering::Relaxed);
        if is_write_class_tool(tool) {
            self.last_write_tool_at.store(at, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> TurnProgressSnapshot {
        use std::sync::atomic::Ordering;
        let last = self.last_write_tool_at.load(Ordering::Relaxed);
        TurnProgressSnapshot {
            turn_start_at: self.turn_start_at.load(Ordering::Relaxed),
            last_write_tool_at: (last != 0).then_some(last),
            tool_calls_total: self.tool_calls_total.load(Ordering::Relaxed),
        }
    }
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
            let code = Some(response.error_code.as_str())
                .filter(|c| !c.is_empty())
                .unwrap_or("unknown");
            let msg = Some(response.error.as_str())
                .filter(|e| !e.is_empty())
                .unwrap_or("unknown error");
            return Err(anyhow!("Command '{cmd_type}' failed [{code}]: {msg}"));
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

    /// Queue a mid-turn steering note (drained by the running turn at its next
    /// LLM step, unlike append_system_prompt which applies from the next run).
    pub async fn steer(&mut self, session_id: &str, text: &str) -> Result<()> {
        self.call(
            "steer",
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

    /// Delete the agent session backing this run — closes its persistence
    /// journal and reclaims the on-disk session state (~/.future/agent/sessions/).
    /// The agent session is a per-run scratch workspace (context is replayed
    /// via the turn envelope), so `run` deletes it when the run ends.
    /// Fails with a retryable error if the session still has an active run.
    pub async fn delete_session(&mut self, session_id: &str) -> Result<()> {
        self.call("delete_session", session_id, Default::default())
            .await?;
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
    /// `live_log`: when set, every streamed event is teed to this JSONL file
    /// so operators can watch a long turn live (`loop status` shows one line
    /// otherwise).
    ///
    /// O5: when the stream terminates with a gap-class error (the agent's
    /// `DataLoss` "event stream gap" — its replay ring lagged), reconnect
    /// once after a 2s backoff on the same session and resume from the last
    /// observed idx (atomic attach replays everything after the cursor, so
    /// nothing double-counts). A second consecutive gap-class failure —
    /// including a failed reconnect — terminates the turn carrying the
    /// original error.
    pub async fn run_turn(
        &mut self,
        session_id: &str,
        run_id: &str,
        live_log: Option<&std::path::Path>,
        progress: Option<&TurnProgressTracker>,
    ) -> Result<RunSummary> {
        let mut summary = RunSummary {
            run_id: run_id.to_string(),
            terminal_state: "incomplete".to_string(),
            error: None,
            tools: vec![],
            text: String::new(),
            usage: None,
            duration_ms: None,
        };
        // O5: resume cursor (last event idx actually observed) and the
        // original gap error (carried if the retry also fails).
        let mut after_idx: i64 = -1;
        let mut first_gap_error: Option<String> = None;
        loop {
            let request = tonic::Request::new(StreamRequest {
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                event_types: vec![],
                after_idx,
                atomic_attach: true,
            });
            let mut stream = match self.inner.stream_events(request).await {
                Ok(resp) => resp.into_inner(),
                Err(e) => {
                    let attach_err = format!("Failed to attach to run {run_id}: {e}");
                    return match first_gap_error {
                        // The reconnect itself failed: a second consecutive
                        // failure — terminate the turn with the ORIGINAL
                        // gap error attached (O5).
                        Some(original) => {
                            Err(anyhow!("{original} (reconnect also failed: {attach_err})"))
                        }
                        None => Err(anyhow!("{attach_err}")),
                    };
                }
            };
            match consume_run_stream(&mut stream, run_id, &mut summary, live_log, progress).await {
                StreamOutcome::Done | StreamOutcome::Closed => return Ok(summary),
                StreamOutcome::Failed { status, last_idx } => {
                    after_idx = last_idx;
                    let gap_err = format!("stream error on run {run_id}: {status}");
                    if !is_stream_gap(&status) {
                        return Err(anyhow!("{gap_err}"));
                    }
                    if let Some(original) = first_gap_error.take() {
                        // Second consecutive gap — terminate the turn with
                        // the original error (O5: 把原错误带上).
                        return Err(anyhow!("{original} (retry also failed: {gap_err})"));
                    }
                    first_gap_error = Some(gap_err);
                    tokio::time::sleep(STREAM_GAP_RETRY_BACKOFF).await;
                }
            }
        }
    }
}

/// O5: backoff between a stream-gap disconnect and the reconnect attempt.
const STREAM_GAP_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_secs(2);

/// O5: gap-class stream errors. The agent terminates the event stream with
/// `DataLoss` when its per-run replay ring lagged (message: "event stream
/// gap … reconnect with atomic attach"). Those are recoverable by reconnect
/// + cursor resume; anything else fails the turn immediately.
fn is_stream_gap(status: &tonic::Status) -> bool {
    status.code() == tonic::Code::DataLoss
        || status
            .message()
            .to_ascii_lowercase()
            .contains("event stream gap")
}

/// How one stream subscription ended (O5).
enum StreamOutcome {
    /// A terminal event (`agent_end` / run `error` event) was consumed.
    Done,
    /// The subscription closed cleanly without a terminal event.
    Closed,
    /// Transport failure — the `status` plus the last event idx actually
    /// observed (the resume cursor for a gap retry).
    Failed {
        status: tonic::Status,
        last_idx: i64,
    },
}

/// Consume one subscription until it terminates; `summary` accumulates
/// across reconnect attempts (each attempt replays only idx > last cursor,
/// so nothing double-counts).
async fn consume_run_stream(
    stream: &mut tonic::Streaming<StreamEvent>,
    run_id: &str,
    summary: &mut RunSummary,
    live_log: Option<&std::path::Path>,
    progress: Option<&TurnProgressTracker>,
) -> StreamOutcome {
    use tokio_stream::StreamExt;
    let mut last_idx: i64 = -1;
    while let Some(ev) = stream.next().await {
        let ev = match ev {
            Ok(ev) => ev,
            Err(status) => return StreamOutcome::Failed { status, last_idx },
        };
        if ev.run_id != run_id {
            // Stale tail from a previous run on the same session (or a
            // supersede) — ignore foreign events.
            continue;
        }
        // Resume cursor: the last event of THIS run, even when its payload
        // is malformed (attach replays strictly idx > cursor).
        last_idx = ev.idx;
        let Some(data) = parse_data(&ev) else {
            continue;
        };
        if let Some(path) = live_log {
            let wall_ts = crate::state::now_epoch();
            let mut line = serde_json::json!({
                "type": ev.r#type.as_str(),
                "idx": ev.idx,
                "wall_ts": wall_ts,
            });
            if ev.r#type == "tool_start" {
                if let Some(n) = data.get("tool_name").and_then(|v| v.as_str()) {
                    line["tool"] = serde_json::Value::String(n.to_string());
                }
            }
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map(|mut f| {
                    use std::io::Write;
                    let _ = writeln!(f, "{}", line);
                });
        }
        // O3: fold tool_start into the progress tracker (write-class
        // starts reset the idle clock; all starts count).
        if ev.r#type == "tool_start" {
            if let (Some(progress), Some(name)) =
                (progress, data.get("tool_name").and_then(|v| v.as_str()))
            {
                progress.observe_tool_start(name, crate::state::now_epoch());
            }
        }
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
                        // truncate at a UTF-8 char boundary — str::truncate panics mid-char
                        let boundary = summary.text.floor_char_boundary(8_000);
                        summary.text.truncate(boundary);
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
                return StreamOutcome::Done;
            }
            "error" => {
                let msg = data
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                summary.error = Some(msg.to_string());
                summary.terminal_state = "error".to_string();
                return StreamOutcome::Done;
            }
            _ => {}
        }
    }
    StreamOutcome::Closed
}

fn parse_data(ev: &StreamEvent) -> Option<Value> {
    if ev.data.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(&ev.data).ok()
}

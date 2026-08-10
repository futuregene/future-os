//! gRPC client — port of `cli/src/rpc/grpc-client.ts` `RunClient`.
//!
//! P1 ports the methods the P1 commands use: get_agent_info, list_models,
//! get_state, list_sessions, get_session_entries, set_session_name (rename),
//! delete_session. P2 adds the `run` orchestration: session resolution
//! (fork/session/continue/fresh), config application, streaming, and the
//! fire-and-forget `notifyAgentRefreshSkills`.
//!
//! Error surface: like the TS client, transport failures and `success:false`
//! responses surface as plain `String` messages; the exact bytes of
//! transport errors differ from grpc-js (network-stack dependent), which the
//! golden diff tests accept for remote commands.

use crate::output::Output;
use future_rpc::proto::future_agent_client::FutureAgentClient;
use future_rpc::proto::{RpcCommand, StreamEvent, StreamRequest};
use serde_json::{json, Map, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `grpcAddr()` from agent.ts/session.ts — env override, then localhost default.
pub fn grpc_addr() -> String {
    std::env::var("FUTURE_AGENT_GRPC_ADDR").unwrap_or_else(|_| "127.0.0.1:50051".to_string())
}

/// `String(Date.now())` — millisecond epoch, used as the request correlation id.
fn now_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

/// Port of `RunClient` — fire-and-forget one-shot gRPC calls.
pub struct RunClient {
    addr: String,
}

impl RunClient {
    pub fn new(addr: &str) -> Self {
        Self {
            addr: addr.to_string(),
        }
    }

    /// Low-level `executeCommand(type, cmd, sessionId?, timeoutSecs=10)`.
    ///
    /// The TS client reuses one channel per command instance; this port
    /// connects per call — equivalent for one-shot CLI usage and keeps the
    /// client usable across `.await` points without interior mutability.
    async fn execute_command(
        &self,
        r#type: &str,
        mut cmd: RpcCommand,
        session_id: Option<&str>,
        timeout_secs: u64,
    ) -> Result<Value, String> {
        cmd.id = now_id();
        cmd.r#type = r#type.to_string();
        if let Some(sid) = session_id {
            cmd.session_id = sid.to_string();
        }

        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{}", self.addr))
            .map_err(|e| e.to_string())?
            .timeout(Duration::from_secs(timeout_secs));
        let channel = endpoint.connect().await.map_err(|e| e.to_string())?;
        let mut client = FutureAgentClient::new(channel);
        let response = client
            .execute_command(cmd)
            .await
            .map_err(|status| {
                let msg = status.message();
                if msg.is_empty() {
                    status.to_string()
                } else {
                    msg.to_string()
                }
            })?
            .into_inner();

        if !response.success {
            return Err(if response.error.is_empty() {
                "unknown error".to_string()
            } else {
                response.error
            });
        }

        // `response.data` is a JSON string; try to parse it, else pass the
        // raw string through (mirrors the TS try/parse/catch).
        if response.data.is_empty() {
            return Ok(Value::Null);
        }
        match serde_json::from_str::<Value>(&response.data) {
            Ok(value) => Ok(value),
            Err(_) => Ok(Value::String(response.data)),
        }
    }

    /// `getAgentInfo()` — `get_agent_info` → `{version, skillsCount}`.
    pub async fn get_agent_info(&self) -> Result<Value, String> {
        self.execute_command("get_agent_info", RpcCommand::default(), None, 5)
            .await
    }

    /// `listModels()` — `list_models` → `{models, defaultModel}`.
    pub async fn list_models(&self) -> Result<Value, String> {
        self.execute_command("list_models", RpcCommand::default(), None, 5)
            .await
    }

    /// `getState(sessionId?)` — `get_state` → SessionState JSON.
    pub async fn get_state(&self, session_id: Option<&str>) -> Result<Value, String> {
        self.execute_command("get_state", RpcCommand::default(), session_id, 5)
            .await
    }

    /// `listSessions()` — `list_sessions` → `{sessions: [...]}`.
    pub async fn list_sessions(&self) -> Result<Value, String> {
        self.execute_command("list_sessions", RpcCommand::default(), None, 5)
            .await
    }

    /// `getSessionEntries(sessionId)` — `get_session_entries` → `{entries}`.
    pub async fn get_session_entries(&self, session_id: &str) -> Result<Value, String> {
        self.execute_command(
            "get_session_entries",
            RpcCommand::default(),
            Some(session_id),
            5,
        )
        .await
    }

    /// `renameSession(sessionId, name)` — `set_session_name`; errors on failure.
    pub async fn rename_session(&self, session_id: &str, name: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            name: name.to_string(),
            ..Default::default()
        };
        self.execute_command("set_session_name", cmd, Some(session_id), 5)
            .await?;
        Ok(())
    }

    /// `deleteSession(sessionId)` — `delete_session` → `{deleted: bool}`.
    pub async fn delete_session(&self, session_id: &str) -> Result<Value, String> {
        self.execute_command("delete_session", RpcCommand::default(), Some(session_id), 5)
            .await
    }

    // ─── Run orchestration (P2 — port of `RunClient.run`) ─────────────────

    /// `switchSession(sessionId)` — `switch_session` → `{cancelled}`.
    pub async fn switch_session(&self, session_id: &str) -> Result<Value, String> {
        self.execute_command("switch_session", RpcCommand::default(), Some(session_id), 5)
            .await
    }

    /// `fork(entryId, sessionId?)` — `fork` → `{cancelled, sessionId?}`.
    pub async fn fork(&self, entry_id: &str, session_id: &str) -> Result<Value, String> {
        let cmd = RpcCommand {
            entry_id: entry_id.to_string(),
            ..Default::default()
        };
        self.execute_command("fork", cmd, Some(session_id), 5).await
    }

    /// `newSession(cwd)` — `new_session` with `createdBy: "cli"`.
    pub async fn new_session(&self, cwd: &str) -> Result<Value, String> {
        let cmd = RpcCommand {
            cwd: cwd.to_string(),
            created_by: "cli".to_string(),
            ..Default::default()
        };
        self.execute_command("new_session", cmd, None, 5).await
    }

    /// `setModel(modelId, sessionId?)` — `set_model`.
    pub async fn set_model(&self, model_id: &str, session_id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            model_id: model_id.to_string(),
            ..Default::default()
        };
        self.execute_command("set_model", cmd, Some(session_id), 5)
            .await?;
        Ok(())
    }

    /// `setThinkingLevel(level, sessionId?)` — `set_thinking_level`.
    pub async fn set_thinking_level(&self, level: &str, session_id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            level: level.to_string(),
            ..Default::default()
        };
        self.execute_command("set_thinking_level", cmd, Some(session_id), 5)
            .await?;
        Ok(())
    }

    /// `setTools(toolNames, sessionId?)` — `set_tools`.
    pub async fn set_tools(&self, tool_names: &[String], session_id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            tools: tool_names.to_vec(),
            ..Default::default()
        };
        self.execute_command("set_tools", cmd, Some(session_id), 5)
            .await?;
        Ok(())
    }

    /// `disableTools(sessionId?)` — `disable_tools`.
    pub async fn disable_tools(&self, session_id: &str) -> Result<(), String> {
        self.execute_command("disable_tools", RpcCommand::default(), Some(session_id), 5)
            .await?;
        Ok(())
    }

    /// `disableBuiltinTools(sessionId?)` — `disable_builtin_tools`.
    pub async fn disable_builtin_tools(&self, session_id: &str) -> Result<(), String> {
        self.execute_command(
            "disable_builtin_tools",
            RpcCommand::default(),
            Some(session_id),
            5,
        )
        .await?;
        Ok(())
    }

    /// `setSystemPrompt(prompt, sessionId?)` — `set_system_prompt`.
    pub async fn set_system_prompt(&self, prompt: &str, session_id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            system_prompt: prompt.to_string(),
            ..Default::default()
        };
        self.execute_command("set_system_prompt", cmd, Some(session_id), 5)
            .await?;
        Ok(())
    }

    /// `appendSystemPrompt(prompt, sessionId?)` — `append_system_prompt`.
    pub async fn append_system_prompt(&self, prompt: &str, session_id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            system_prompt: prompt.to_string(),
            ..Default::default()
        };
        self.execute_command("append_system_prompt", cmd, Some(session_id), 5)
            .await?;
        Ok(())
    }

    /// `setEphemeral(ephemeral, sessionId?)` — `set_ephemeral`.
    pub async fn set_ephemeral(&self, ephemeral: bool, session_id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            ephemeral,
            ..Default::default()
        };
        self.execute_command("set_ephemeral", cmd, Some(session_id), 5)
            .await?;
        Ok(())
    }

    /// `setPermissionLevel(level, sessionId?)` — `set_permission_level`.
    pub async fn set_permission_level(&self, level: &str, session_id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            level: level.to_string(),
            ..Default::default()
        };
        self.execute_command("set_permission_level", cmd, Some(session_id), 5)
            .await?;
        Ok(())
    }

    /// `setCwd(cwd, sessionId?)` — `set_cwd`.
    pub async fn set_cwd(&self, cwd: &str, session_id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            cwd: cwd.to_string(),
            ..Default::default()
        };
        self.execute_command("set_cwd", cmd, Some(session_id), 5)
            .await?;
        Ok(())
    }

    /// `prompt(message, sessionId?)` — `prompt` with a 30 s timeout.
    pub async fn prompt(&self, message: &str, session_id: &str) -> Result<(), String> {
        let cmd = RpcCommand {
            message: message.to_string(),
            ..Default::default()
        };
        self.execute_command("prompt", cmd, Some(session_id), 30)
            .await?;
        Ok(())
    }

    /// `streamEvents(sessionId, onText?, verbose)` — subscribe to the event
    /// stream for a session and accumulate text/events until `agent_end`,
    /// the server closes the stream, or the 5-minute wall clock expires.
    ///
    /// The TS client cancels the stream on `agent_end` / timeout and resolves
    /// with what it has; stream errors after the run started are treated the
    /// same way (best-effort parity — the deadline dominates in practice).
    #[allow(clippy::type_complexity)]
    pub async fn stream_events(
        &self,
        session_id: &str,
        on_text: Option<Box<dyn Fn(&str) + Send>>,
        verbose: bool,
        out: &Output,
    ) -> Result<(Vec<Value>, String), String> {
        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{}", self.addr))
            .map_err(|e| e.to_string())?
            // TS: 5-minute setTimeout — the whole stream is bounded by it.
            .timeout(Duration::from_secs(300));
        let channel = endpoint.connect().await.map_err(|e| e.to_string())?;
        let mut client = FutureAgentClient::new(channel);
        let request = StreamRequest {
            session_id: session_id.to_string(),
            ..Default::default()
        };
        let mut stream = client
            .stream_events(request)
            .await
            .map_err(|status| {
                let msg = status.message();
                if msg.is_empty() {
                    status.to_string()
                } else {
                    msg.to_string()
                }
            })?
            .into_inner();

        let mut events: Vec<Value> = Vec::new();
        let mut text = String::new();
        loop {
            let message = match stream.message().await {
                Ok(Some(event)) => event,
                // Stream end (Ok(None)) or a mid-stream error: the TS resolves
                // with accumulated events in both cases (deadline / end).
                Ok(None) => break,
                Err(_) => break,
            };
            // `raw_data`: parsed `data` payload (empty object when absent).
            let raw_data: Map<String, Value> = if message.data.is_empty() {
                Map::new()
            } else {
                match serde_json::from_str::<Value>(&message.data) {
                    Ok(Value::Object(map)) => map,
                    // TS: parse errors inside the data handler are swallowed
                    // and the whole event is dropped.
                    _ => continue,
                }
            };
            let Some(event_json) = parse_stream_event(&message, &raw_data) else {
                continue;
            };

            if message.r#type == "text_chunk" {
                let chunk = raw_data
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                text.push_str(&chunk);
                if let Some(cb) = &on_text {
                    cb(&chunk);
                }
            } else if message.r#type == "tool_start" && verbose {
                // `\x1b[2m⚙ ${toolName}${inputStr ? " " + inputStr.slice(0, 80) : ""}\x1b[0m\n`
                let tool_name = raw_data
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .or_else(|| raw_data.get("name").and_then(Value::as_str))
                    .unwrap_or("unknown");
                let tool_input = raw_data
                    .get("tool_args")
                    .or_else(|| raw_data.get("input"))
                    .cloned()
                    .unwrap_or_else(|| Value::String(String::new()));
                let input_str = match tool_input {
                    Value::String(s) => s,
                    other => serde_json::to_string(&other).unwrap_or_default(),
                };
                let suffix = if input_str.is_empty() {
                    String::new()
                } else {
                    // JS `inputStr.slice(0, 80)` — first 80 chars, safe at
                    // UTF-8 char boundaries.
                    let clipped: String = input_str.chars().take(80).collect();
                    format!(" {clipped}")
                };
                out.write_err(&format!("\x1b[2m⚙ {tool_name}{suffix}\x1b[0m\n"));
            } else if message.r#type == "error" {
                // `\x1b[31mError: ${rawData?.error || "unknown"}\x1b[0m\n`
                let error = raw_data
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                out.write_err(&format!("\x1b[31mError: {error}\x1b[0m\n"));
            }
            events.push(event_json);
            if message.r#type == "agent_end" {
                break;
            }
        }
        Ok((events, text))
    }

    /// High-level `run(config)` — port of `RunClient.run`.
    ///
    /// Resolve the target session (fork → session → continue → fresh), apply
    /// configuration, start streaming, send the prompt, and await completion.
    pub async fn run(&self, config: &RunConfig, out: &Output) -> Result<RunResult, String> {
        let verbose = config.verbose;

        // 1. Establish session
        if verbose {
            out.write_err(&format!("Connecting to {}...\n", self.addr));
        }

        let mut session_id: String;
        if let Some(fork_entry) = &config.fork {
            // Fork needs an explicit parent session. Without --session, fork
            // from the most recently updated session.
            let mut parent_id = config.session.clone();
            if parent_id.is_none() {
                let sessions = self.list_sessions().await?;
                let list = sessions
                    .get("sessions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if list.is_empty() {
                    return Err("No previous session to fork from.".to_string());
                }
                let mut rows = list
                    .iter()
                    .filter_map(|s| {
                        let obj = s.as_object()?;
                        let id = obj.get("id")?.as_str()?.to_string();
                        let updated = obj
                            .get("updated_at")
                            .and_then(Value::as_str)
                            .map(parse_updated_at)
                            .unwrap_or(0);
                        Some((updated, id))
                    })
                    .collect::<Vec<_>>();
                rows.sort_by_key(|row| std::cmp::Reverse(row.0));
                parent_id = rows.first().map(|(_, id)| id.clone());
                if parent_id.is_none() {
                    return Err("No previous session to fork from.".to_string());
                }
            }
            self.switch_session(parent_id.as_deref().unwrap_or_default())
                .await?;
            session_id = parent_id.unwrap_or_default();
            if verbose {
                out.write_err(&format!("Forking from entry {fork_entry}...\n"));
            }
            let result = self.fork(fork_entry, &session_id).await?;
            if result
                .get("cancelled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return Err("Fork was cancelled".to_string());
            }
            if let Some(sid) = result.get("sessionId").and_then(Value::as_str) {
                session_id = sid.to_string();
            }
        } else if let Some(sid) = &config.session {
            self.switch_session(sid).await?;
            session_id = sid.clone();
            if verbose {
                out.write_err(&format!("Switched to session {sid}\n"));
            }
        } else if config.continue_last {
            let sessions = self.list_sessions().await?;
            let list = sessions
                .get("sessions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if !list.is_empty() {
                let mut rows = list
                    .iter()
                    .filter_map(|s| {
                        let obj = s.as_object()?;
                        let id = obj.get("id")?.as_str()?.to_string();
                        let name = obj
                            .get("session_name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let updated = obj
                            .get("updated_at")
                            .and_then(Value::as_str)
                            .map(parse_updated_at)
                            .unwrap_or(0);
                        Some((updated, id, name))
                    })
                    .collect::<Vec<_>>();
                rows.sort_by_key(|row| std::cmp::Reverse(row.0));
                if let Some((_, id, name)) = rows.first() {
                    self.switch_session(id).await?;
                    session_id = id.clone();
                    if verbose {
                        let label = if name.is_empty() {
                            id.clone()
                        } else {
                            name.clone()
                        };
                        out.write_err(&format!("Continuing session {label}...\n"));
                    }
                } else {
                    return Err(
                        "No previous session to continue; run without --continue to start a new one."
                            .to_string(),
                    );
                }
            } else {
                return Err(
                    "No previous session to continue; run without --continue to start a new one."
                        .to_string(),
                );
            }
        } else {
            // Fresh session for every standalone run — isolates model/thinking/
            // tool changes so they never bleed into subsequent invocations.
            let new_session = self.new_session(&config.cwd).await?;
            session_id = new_session
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or_else(|| "new_session returned no sessionId".to_string())?
                .to_string();
            if config.no_session {
                self.set_ephemeral(true, &session_id).await?;
                if verbose {
                    out.write_err(&format!("Created ephemeral session {session_id}\n"));
                }
            } else if verbose {
                out.write_err(&format!("Created session {session_id}\n"));
            }
        }

        // 3. Apply configuration options (all scoped to this run's session)
        if let Some(model) = &config.model {
            if verbose {
                out.write_err(&format!("Model: {model}\n"));
            }
            self.set_model(model, &session_id).await?;
        }
        if let Some(thinking) = &config.thinking {
            if verbose {
                out.write_err(&format!("Thinking: {thinking}\n"));
            }
            self.set_thinking_level(thinking, &session_id).await?;
        }
        if let Some(tools) = &config.tools {
            if !tools.is_empty() {
                self.set_tools(tools, &session_id).await?;
            }
        } else if config.no_tools {
            self.disable_tools(&session_id).await?;
        }
        if config.no_builtin_tools {
            self.disable_builtin_tools(&session_id).await?;
        }
        if let Some(system_prompt) = &config.system_prompt {
            self.set_system_prompt(system_prompt, &session_id).await?;
        }
        if let Some(append_system_prompt) = &config.append_system_prompt {
            self.append_system_prompt(append_system_prompt, &session_id)
                .await?;
        }
        if let Some(permission) = &config.permission {
            if verbose {
                out.write_err(&format!("Permission: {permission}\n"));
            }
            self.set_permission_level(permission, &session_id).await?;
        }
        if !config.cwd.is_empty() {
            self.set_cwd(&config.cwd, &session_id).await?;
        }

        // 4. Start streaming events BEFORE sending prompt
        if verbose {
            out.write_err("Running...\n");
        }
        let stream_session = session_id.clone();
        let stream_addr = self.addr.clone();
        let out_stream = out.clone();
        #[allow(clippy::type_complexity)]
        let on_text: Option<Box<dyn Fn(&str) + Send>> = if config.mode == "text" {
            let out_text = out.clone();
            Some(Box::new(move |chunk: &str| out_text.write_out(chunk)))
        } else {
            None
        };
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<(Vec<Value>, String), String>>();
        let handle = tokio::spawn(async move {
            let client = RunClient::new(&stream_addr);
            let result = client
                .stream_events(&stream_session, on_text, verbose, &out_stream)
                .await;
            let _ = tx.send(result);
        });

        // 5. Send prompt (must target the same session as streamEvents)
        self.prompt(&config.message, &session_id).await?;

        // 6. Wait for events to complete
        let (events, text) = match rx.await {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => return Err(err),
            Err(_) => {
                handle.abort();
                return Err("Event stream task was dropped".to_string());
            }
        };

        // 7. Get final state for model info (query the run's own session)
        let mut model: Option<String> = None;
        let mut thinking_level: Option<String> = None;
        if let Ok(final_state) = self.get_state(Some(&session_id)).await {
            model = final_state
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string);
            thinking_level = final_state
                .get("thinkingLevel")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        // (get_state failure is ignored — non-critical, exactly like TS)

        // 8. Output (for text mode, already streamed to stdout)
        if config.mode == "json" {
            let mut result = Map::new();
            result.insert("sessionId".to_string(), Value::String(session_id.clone()));
            if let Some(model) = &model {
                result.insert("model".to_string(), Value::String(model.clone()));
            }
            if let Some(level) = &thinking_level {
                result.insert("thinkingLevel".to_string(), Value::String(level.clone()));
            }
            result.insert("text".to_string(), Value::String(text.clone()));
            result.insert("messages".to_string(), Value::Array(events.clone()));
            out.write_out(&format!(
                "{}\n",
                serde_json::to_string_pretty(&Value::Object(result)).unwrap_or_default()
            ));
        } else {
            // Add trailing newline if text doesn't already end with one
            if !text.is_empty() && !text.ends_with('\n') {
                out.write_out("\n");
            }
        }

        Ok(RunResult {
            session_id,
            text,
            events,
            model,
            thinking_level,
        })
    }
}

/// `parse_updated_at(s)` — comparable timestamp for `updated_at` sorting
/// (the TS uses `new Date(...).getTime()`; this mirrors the ordering, not
/// the exact epoch). Accepts RFC3339 and `"YYYY-MM-DD HH:MM:SS"`.
fn parse_updated_at(s: &str) -> i64 {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp_millis();
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return dt.and_utc().timestamp_millis();
    }
    0
}

/// Build the event JSON the TS client pushes: the envelope keys in order,
/// then the parsed `data` spread over them (data wins on key collisions,
/// envelope keys keep their position).
fn parse_stream_event(event: &StreamEvent, raw_data: &Map<String, Value>) -> Option<Value> {
    let mut obj = Map::new();
    obj.insert(
        "type".to_string(),
        Value::String(if event.r#type.is_empty() {
            "message".to_string()
        } else {
            event.r#type.clone()
        }),
    );
    obj.insert(
        "sessionId".to_string(),
        Value::String(event.session_id.clone()),
    );
    obj.insert("runId".to_string(), Value::String(event.run_id.clone()));
    obj.insert("epoch".to_string(), json!(event.epoch));
    obj.insert("idx".to_string(), json!(event.idx));
    obj.insert("eventId".to_string(), Value::String(event.event_id.clone()));
    obj.insert(
        "timestamp".to_string(),
        Value::String(event.timestamp.clone()),
    );
    obj.insert(
        "projectionSnapshot".to_string(),
        Value::Bool(event.projection_snapshot),
    );
    obj.insert("snapshotCursor".to_string(), json!(event.snapshot_cursor));
    obj.insert(
        "snapshotEvents".to_string(),
        Value::Array(
            event
                .snapshot_events
                .iter()
                .map(|e| {
                    json!({
                        "type": e.r#type,
                        "data": e.data,
                        "idx": e.idx,
                    })
                })
                .collect(),
        ),
    );
    for (k, v) in raw_data {
        obj.insert(k.clone(), v.clone());
    }
    Some(Value::Object(obj))
}

/// `RunConfig` from grpc-client.ts.
#[derive(Debug, Clone, Default)]
pub struct RunConfig {
    pub fork: Option<String>,
    pub session: Option<String>,
    pub continue_last: bool,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub tools: Option<Vec<String>>,
    pub no_tools: bool,
    pub no_builtin_tools: bool,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub permission: Option<String>,
    pub no_session: bool,
    pub mode: String,
    pub cwd: String,
    pub verbose: bool,
    pub message: String,
}

/// `RunResult` from grpc-client.ts.
#[derive(Debug)]
pub struct RunResult {
    pub session_id: String,
    pub text: String,
    pub events: Vec<Value>,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
}

/// `notifyAgentRefreshSkills(grpcAddr?)` — best-effort 1-second refresh RPC
/// after skills are installed/removed, so the agent drops its skills cache.
/// Errors are silently dropped (agent often not running when skills are
/// installed from a bare shell; the TTL-based refresh picks it up).
pub async fn notify_agent_refresh_skills() {
    let client = RunClient::new(&grpc_addr());
    let _ = client
        .execute_command("refresh_skills", RpcCommand::default(), None, 1)
        .await;
}

/// `process.env.FUTURE_AGENT_GRPC_ADDR ?? "127.0.0.1:50051"` (doctor.ts).
pub fn grpc_addr_env() -> String {
    grpc_addr()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{spawn_mock, stream_event, MockAgent};

    // ── addr / helpers ──────────────────────────────────────────────

    #[tokio::test]
    async fn grpc_addr_default_and_env_override() {
        let _guard = crate::test_env::lock_env().await;
        let _env = crate::test_env::EnvGuard::remove(&["FUTURE_AGENT_GRPC_ADDR"]);
        assert_eq!(grpc_addr(), "127.0.0.1:50051");
        assert_eq!(grpc_addr_env(), "127.0.0.1:50051");
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("10.0.0.1:1234"),
        )]);
        assert_eq!(grpc_addr(), "10.0.0.1:1234");
        assert_eq!(grpc_addr_env(), "10.0.0.1:1234");
    }

    #[test]
    fn now_id_is_millis() {
        let id = now_id();
        let millis: u128 = id.parse().expect("numeric id");
        assert!(millis > 1_000_000_000_000); // past 2001-09-09
    }

    #[test]
    fn parse_updated_at_formats() {
        assert_eq!(parse_updated_at("2026-08-06T12:00:00Z"), 1786017600000);
        assert_eq!(
            parse_updated_at("2026-08-06 12:00:00"),
            1786017600000
        );
        assert_eq!(parse_updated_at("garbage"), 0);
    }

    // ── execute_command surface ─────────────────────────────────────

    #[tokio::test]
    async fn one_shot_methods_roundtrip() {
        let mut agent = MockAgent::default();
        agent
            .responses
            .insert("get_agent_info".into(), "{\"version\":\"1.0\"}".into());
        agent
            .responses
            .insert("list_models".into(), "{\"models\":[]}".into());
        agent.responses.insert(
            "get_state".into(),
            "{\"model\":\"m1\",\"thinkingLevel\":\"high\"}".into(),
        );
        agent.responses.insert(
            "list_sessions".into(),
            "{\"sessions\":[{\"id\":\"s1\"}]}".into(),
        );
        agent
            .responses
            .insert("get_session_entries".into(), "{\"entries\":[]}".into());
        agent
            .responses
            .insert("delete_session".into(), "{\"deleted\":true}".into());
        agent
            .responses
            .insert("switch_session".into(), "{\"cancelled\":false}".into());
        agent
            .responses
            .insert("fork".into(), "{\"cancelled\":false}".into());
        agent
            .responses
            .insert("new_session".into(), "{\"sessionId\":\"s9\"}".into());
        agent
            .responses
            .insert("notify".into(), "not json at all".into());
        let addr = spawn_mock(agent.clone()).await;
        let client = RunClient::new(&addr);

        assert_eq!(client.get_agent_info().await.unwrap()["version"], "1.0");
        assert!(client.list_models().await.unwrap()["models"].is_array());
        let state = client.get_state(Some("s1")).await.unwrap();
        assert_eq!(state["model"], "m1");
        let state = client.get_state(None).await.unwrap();
        assert_eq!(state["thinkingLevel"], "high");
        assert_eq!(
            client.list_sessions().await.unwrap()["sessions"][0]["id"],
            "s1"
        );
        assert!(client.get_session_entries("s1").await.unwrap()["entries"].is_array());
        assert_eq!(client.delete_session("s1").await.unwrap()["deleted"], true);
        assert_eq!(client.switch_session("s1").await.unwrap()["cancelled"], false);
        assert!(client.fork("e1", "s1").await.unwrap()["cancelled"].is_boolean());
        assert_eq!(client.new_session("/tmp").await.unwrap()["sessionId"], "s9");

        // Field-carrying commands land on the wire.
        client.rename_session("s1", "hello").await.unwrap();
        client.set_model("m2", "s1").await.unwrap();
        client.set_thinking_level("low", "s1").await.unwrap();
        client
            .set_tools(&["a".to_string(), "b".to_string()], "s1")
            .await
            .unwrap();
        client.disable_tools("s1").await.unwrap();
        client.disable_builtin_tools("s1").await.unwrap();
        client.set_system_prompt("p", "s1").await.unwrap();
        client.append_system_prompt("q", "s1").await.unwrap();
        client.set_ephemeral(true, "s1").await.unwrap();
        client.set_permission_level("full", "s1").await.unwrap();
        client.set_cwd("/work", "s1").await.unwrap();
        client.prompt("hi", "s1").await.unwrap();

        let seen = agent.seen.lock().expect("seen");
        let by_type = |t: &str| seen.iter().find(|c| c.r#type == t).expect(t).clone();
        assert_eq!(by_type("set_session_name").name, "hello");
        assert_eq!(by_type("set_session_name").session_id, "s1");
        assert_eq!(by_type("set_model").model_id, "m2");
        assert_eq!(by_type("set_thinking_level").level, "low");
        assert_eq!(by_type("set_tools").tools, vec!["a", "b"]);
        assert_eq!(by_type("set_system_prompt").system_prompt, "p");
        assert_eq!(by_type("append_system_prompt").system_prompt, "q");
        assert!(by_type("set_ephemeral").ephemeral);
        assert_eq!(by_type("set_permission_level").level, "full");
        assert_eq!(by_type("set_cwd").cwd, "/work");
        assert_eq!(by_type("prompt").message, "hi");
        assert_eq!(by_type("fork").entry_id, "e1");
        assert_eq!(by_type("new_session").created_by, "cli");
        assert_eq!(by_type("get_session_entries").session_id, "s1");
        // get_state without a session leaves the field empty.
        let no_session = seen
            .iter()
            .filter(|c| c.r#type == "get_state")
            .find(|c| c.session_id.is_empty());
        assert!(no_session.is_some());
        // Every command got a millis id assigned.
        assert!(seen.iter().all(|c| !c.id.is_empty()));
        drop(seen);

        // Non-JSON data passes through as a string; empty data → Null.
        // (Unknown command types return the default "{}" → Null object.)
        let raw = client
            .execute_command("notify", RpcCommand::default(), None, 5)
            .await
            .unwrap();
        assert_eq!(raw, Value::String("not json at all".to_string()));
        let empty = client
            .execute_command("unknown_type", RpcCommand::default(), None, 5)
            .await
            .unwrap();
        assert_eq!(empty, json!({}));
    }

    #[tokio::test]
    async fn empty_data_yields_null() {
        let agent = MockAgent::respond("get_agent_info", "");
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        assert_eq!(client.get_agent_info().await.unwrap(), Value::Null);
    }

    #[tokio::test]
    async fn error_surface_variants() {
        let mut agent = MockAgent::default();
        agent.fail_types.insert("list_models".into());
        agent.fail_silent_types.insert("get_state".into());
        agent.status_empty_types.insert("list_sessions".into());
        agent.status_message_types.insert("delete_session".into());
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);

        // success=false with an error string surfaces it.
        assert_eq!(client.list_models().await.unwrap_err(), "boom");
        // success=false with empty error → "unknown error".
        assert_eq!(client.get_state(None).await.unwrap_err(), "unknown error");
        // tonic Status without message → Status Display fallback.
        let err = client.list_sessions().await.unwrap_err();
        assert!(err.contains("Unknown"), "err: {err}");
        // tonic Status with a message surfaces the message.
        assert_eq!(client.delete_session("s1").await.unwrap_err(), "transport down");
    }

    #[tokio::test]
    async fn connect_failure_is_err() {
        let client = RunClient::new("127.0.0.1:1");
        assert!(client.get_agent_info().await.is_err());
        // Endpoint parse failure (garbage addr) also surfaces as Err.
        let client = RunClient::new("not a valid addr %%");
        assert!(client.get_agent_info().await.is_err());
    }

    // ── streaming ───────────────────────────────────────────────────

    #[tokio::test]
    async fn stream_events_accumulates_text_and_events() {
        let agent = MockAgent {
            events: vec![
                stream_event("text_chunk", "{\"text\":\"hel\"}"),
                stream_event("text_chunk", "{\"text\":\"lo\"}"),
                stream_event("tool_start", "{\"tool_name\":\"bash\",\"tool_args\":{\"cmd\":\"ls\"}}"),
                stream_event("error", "{\"error\":\"nope\"}"),
                stream_event("agent_end", "{}"),
                // Never reached: agent_end breaks the loop.
                stream_event("text_chunk", "{\"text\":\"late\"}"),
            ],
            ..Default::default()
        };
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, cap) = Output::memory();
        let seen_text = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let sink = seen_text.clone();
        let (events, text) = client
            .stream_events(
                "s1",
                Some(Box::new(move |c: &str| sink.lock().unwrap().push_str(c))),
                true,
                &out,
            )
            .await
            .unwrap();
        assert_eq!(text, "hello");
        assert_eq!(*seen_text.lock().unwrap(), "hello");
        // text chunks + tool_start + error + agent_end (the late event dropped).
        assert_eq!(events.len(), 5);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("⚙ bash {\"cmd\":\"ls\"}"), "stderr: {stderr}");
        assert!(stderr.contains("Error: nope"), "stderr: {stderr}");
        // Envelope fields on every event.
        assert_eq!(events[0]["type"], "text_chunk");
        assert_eq!(events[0]["text"], "hel");
        assert!(events[0].get("projectionSnapshot").is_some());
    }

    #[tokio::test]
    async fn stream_events_edge_inputs() {
        let agent = MockAgent {
            events: vec![
                // Unparseable data → event dropped.
                StreamEvent {
                    r#type: "text_chunk".into(),
                    data: "{bad".into(),
                    ..Default::default()
                },
                // Non-object data → dropped.
                stream_event("text_chunk", "[1,2]"),
                // Empty data → event kept with empty object payload.
                stream_event("lifecycle", ""),
                // Empty type → "message".
                StreamEvent {
                    r#type: "".into(),
                    data: "{}".into(),
                    ..Default::default()
                },
                // tool_start fallbacks: `name`/`input` fields, empty input.
                stream_event("tool_start", "{\"name\":\"read\"}"),
                // tool_start with string input, clipped at 80 chars.
                stream_event(
                    "tool_start",
                    &format!("{{\"tool_name\":\"big\",\"tool_args\":\"{}\"}}", "x".repeat(100)),
                ),
                // error without payload → "unknown".
                stream_event("error", "{}"),
            ],
            ..Default::default()
        };
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, cap) = Output::memory();
        let (events, text) = client
            .stream_events("s1", None, true, &out)
            .await
            .unwrap();
        assert_eq!(text, "");
        // Dropped: bad JSON + array. Kept: lifecycle, message, 2 tool_start, error.
        assert_eq!(events.len(), 5);
        assert_eq!(events[0]["type"], "lifecycle");
        assert_eq!(events[1]["type"], "message");
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("⚙ read"), "stderr: {stderr}");
        assert!(stderr.contains(&"x".repeat(80)), "clip at 80: {stderr}");
        assert!(!stderr.contains(&"x".repeat(81)), "clip at 80: {stderr}");
        assert!(stderr.contains("Error: unknown"), "stderr: {stderr}");
    }

    #[tokio::test]
    async fn stream_events_not_verbose_hides_tool_lines() {
        let agent = MockAgent {
            events: vec![stream_event("tool_start", "{\"tool_name\":\"bash\"}")],
            ..Default::default()
        };
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, cap) = Output::memory();
        // Stream end (Ok(None)) terminates without agent_end.
        let (events, _) = client.stream_events("s1", None, false, &out).await.unwrap();
        assert_eq!(events.len(), 1);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.is_empty());
    }

    #[tokio::test]
    async fn stream_events_mid_stream_error_resolves_partial() {
        let agent = MockAgent {
            events: vec![stream_event("text_chunk", "{\"text\":\"a\"}")],
            stream_error_after: true,
            ..Default::default()
        };
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let (events, text) = client.stream_events("s1", None, false, &out).await.unwrap();
        assert_eq!(text, "a");
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn stream_events_rpc_failures() {
        // stream_events RPC rejected with a message-bearing Status.
        let agent = MockAgent {
            stream_status_error: Some(tonic::Status::unavailable("stream down")),
            ..Default::default()
        };
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        assert_eq!(
            client.stream_events("s1", None, false, &out).await.unwrap_err(),
            "stream down"
        );
        // Message-less Status → Display fallback; and connect failure.
        let agent = MockAgent {
            stream_status_error: Some(tonic::Status::new(tonic::Code::Unknown, "")),
            ..Default::default()
        };
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let err = client.stream_events("s1", None, false, &out).await.unwrap_err();
        assert!(err.contains("Unknown"), "err: {err}");
        let client = RunClient::new("127.0.0.1:1");
        assert!(client.stream_events("s1", None, false, &out).await.is_err());
        let client = RunClient::new("garbage addr %%");
        assert!(client.stream_events("s1", None, false, &out).await.is_err());
    }

    #[tokio::test]
    async fn stream_event_envelope_includes_snapshot_events() {
        let agent = MockAgent {
            events: vec![StreamEvent {
                r#type: "agent_end".into(),
                data: "{}".into(),
                session_id: "s1".into(),
                run_id: "r1".into(),
                epoch: 2,
                idx: 3,
                event_id: "e1".into(),
                timestamp: "2026-08-10T00:00:00Z".into(),
                projection_snapshot: true,
                snapshot_cursor: 7,
                snapshot_events: vec![future_rpc::proto::ProjectedRunEvent {
                    r#type: "text_chunk".into(),
                    data: "{\"text\":\"x\"}".into(),
                    idx: 1,
                    payload: None,
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let (events, _) = client.stream_events("s1", None, false, &out).await.unwrap();
        let event = &events[0];
        assert_eq!(event["sessionId"], "s1");
        assert_eq!(event["runId"], "r1");
        assert_eq!(event["epoch"], 2);
        assert_eq!(event["idx"], 3);
        assert_eq!(event["eventId"], "e1");
        assert_eq!(event["timestamp"], "2026-08-10T00:00:00Z");
        assert_eq!(event["projectionSnapshot"], true);
        assert_eq!(event["snapshotCursor"], 7);
        assert_eq!(event["snapshotEvents"][0]["type"], "text_chunk");
        assert_eq!(event["snapshotEvents"][0]["idx"], 1);
    }

    // ── run() orchestration ─────────────────────────────────────────

    fn run_config(message: &str) -> RunConfig {
        RunConfig {
            message: message.to_string(),
            cwd: "/tmp".to_string(),
            ..Default::default()
        }
    }

    /// Mock pre-loaded for a fresh-session run: new_session + agent_end event.
    fn fresh_run_agent() -> MockAgent {
        let mut agent = MockAgent::default();
        agent
            .responses
            .insert("new_session".into(), "{\"sessionId\":\"s-new\"}".into());
        agent.events = vec![
            stream_event("text_chunk", "{\"text\":\"answer\"}"),
            stream_event("agent_end", "{}"),
        ];
        agent
    }

    #[tokio::test]
    async fn run_fresh_session_text_mode() {
        let agent = fresh_run_agent();
        let addr = spawn_mock(agent.clone()).await;
        let client = RunClient::new(&addr);
        let (out, cap) = Output::memory();
        let mut config = run_config("hello");
        config.verbose = true;
        config.mode = "text".to_string();
        let result = client.run(&config, &out).await.expect("run");
        assert_eq!(result.session_id, "s-new");
        assert_eq!(result.text, "answer");
        assert_eq!(result.events.len(), 2);
        assert!(result.model.is_none());
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        // Text streamed live + trailing newline added.
        assert_eq!(stdout, "answer\n");
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Connecting to"), "stderr: {stderr}");
        assert!(stderr.contains("Created session s-new"), "stderr: {stderr}");
        assert!(stderr.contains("Running..."), "stderr: {stderr}");
        // set_cwd fires because cwd is non-empty.
        assert_eq!(agent.seen_of("set_cwd").len(), 1);
        assert_eq!(agent.seen_of("prompt")[0].message, "hello");
        assert_eq!(agent.seen_of("prompt")[0].session_id, "s-new");
    }

    #[tokio::test]
    async fn run_fresh_session_json_mode_with_final_state() {
        let mut agent = fresh_run_agent();
        agent.responses.insert(
            "get_state".into(),
            "{\"model\":\"m1\",\"thinkingLevel\":\"high\"}".into(),
        );
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, cap) = Output::memory();
        let mut config = run_config("hello");
        config.mode = "json".to_string();
        let result = client.run(&config, &out).await.expect("run");
        assert_eq!(result.model.as_deref(), Some("m1"));
        assert_eq!(result.thinking_level.as_deref(), Some("high"));
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        let parsed: Value = serde_json::from_str(&stdout).expect("json out");
        assert_eq!(parsed["sessionId"], "s-new");
        assert_eq!(parsed["model"], "m1");
        assert_eq!(parsed["thinkingLevel"], "high");
        assert_eq!(parsed["text"], "answer");
        assert_eq!(parsed["messages"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn run_no_session_sets_ephemeral() {
        let agent = fresh_run_agent();
        let addr = spawn_mock(agent.clone()).await;
        let client = RunClient::new(&addr);
        let (out, cap) = Output::memory();
        let mut config = run_config("hi");
        config.no_session = true;
        config.verbose = true;
        client.run(&config, &out).await.expect("run");
        let seen = agent.seen_of("set_ephemeral");
        assert_eq!(seen.len(), 1);
        assert!(seen[0].ephemeral);
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Created ephemeral session s-new"));
    }

    #[tokio::test]
    async fn run_applies_full_configuration() {
        let agent = fresh_run_agent();
        let addr = spawn_mock(agent.clone()).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let mut config = run_config("hi");
        config.verbose = true;
        config.model = Some("m-x".to_string());
        config.thinking = Some("max".to_string());
        config.tools = Some(vec!["bash".to_string()]);
        config.no_builtin_tools = true;
        config.system_prompt = Some("sys".to_string());
        config.append_system_prompt = Some("app".to_string());
        config.permission = Some("strict".to_string());
        client.run(&config, &out).await.expect("run");
        for t in [
            "set_model",
            "set_thinking_level",
            "set_tools",
            "disable_builtin_tools",
            "set_system_prompt",
            "append_system_prompt",
            "set_permission_level",
            "set_cwd",
        ] {
            assert_eq!(agent.seen_of(t).len(), 1, "missing {t}");
        }
        let stderr_config = agent.seen_of("set_model");
        assert_eq!(stderr_config[0].model_id, "m-x");
        // tools Some(empty) → no set_tools call; no_tools → disable_tools.
        let agent = fresh_run_agent();
        let addr = spawn_mock(agent.clone()).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let mut config = run_config("hi");
        config.tools = Some(vec![]);
        client.run(&config, &out).await.expect("run");
        assert!(agent.seen_of("set_tools").is_empty());

        let agent = fresh_run_agent();
        let addr = spawn_mock(agent.clone()).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let mut config = run_config("hi");
        config.no_tools = true;
        client.run(&config, &out).await.expect("run");
        assert_eq!(agent.seen_of("disable_tools").len(), 1);
    }

    #[tokio::test]
    async fn run_explicit_session_switch() {
        let agent = fresh_run_agent();
        let addr = spawn_mock(agent.clone()).await;
        let client = RunClient::new(&addr);
        let (out, cap) = Output::memory();
        let mut config = run_config("hi");
        config.session = Some("s-explicit".to_string());
        config.verbose = true;
        let result = client.run(&config, &out).await.expect("run");
        assert_eq!(result.session_id, "s-explicit");
        assert_eq!(agent.seen_of("switch_session")[0].session_id, "s-explicit");
        // No new_session needed.
        assert!(agent.seen_of("new_session").is_empty());
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Switched to session s-explicit"));
    }

    #[tokio::test]
    async fn run_continue_last_picks_most_recent() {
        let mut agent = fresh_run_agent();
        agent.responses.insert(
            "list_sessions".into(),
            "{\"sessions\":[\
                {\"id\":\"old\",\"updated_at\":\"2026-08-01T00:00:00Z\"},\
                {\"id\":\"new\",\"session_name\":\"Latest\",\"updated_at\":\"2026-08-09T00:00:00Z\"},\
                {\"bogus\":true},\
                {\"id\":\"naive\",\"updated_at\":\"2026-08-08 12:00:00\"}\
            ]}"
            .into(),
        );
        let addr = spawn_mock(agent.clone()).await;
        let client = RunClient::new(&addr);
        let (out, cap) = Output::memory();
        let mut config = run_config("hi");
        config.continue_last = true;
        config.verbose = true;
        let result = client.run(&config, &out).await.expect("run");
        assert_eq!(result.session_id, "new");
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Continuing session Latest..."), "stderr: {stderr}");
    }

    #[tokio::test]
    async fn run_continue_without_sessions_errors() {
        let mut agent = fresh_run_agent();
        agent
            .responses
            .insert("list_sessions".into(), "{\"sessions\":[]}".into());
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let mut config = run_config("hi");
        config.continue_last = true;
        let err = client.run(&config, &out).await.unwrap_err();
        assert!(err.contains("No previous session to continue"), "err: {err}");
    }

    #[tokio::test]
    async fn run_fork_with_explicit_parent() {
        let mut agent = fresh_run_agent();
        agent.responses.insert(
            "fork".into(),
            "{\"cancelled\":false,\"sessionId\":\"s-forked\"}".into(),
        );
        let addr = spawn_mock(agent.clone()).await;
        let client = RunClient::new(&addr);
        let (out, cap) = Output::memory();
        let mut config = run_config("hi");
        config.fork = Some("entry-1".to_string());
        config.session = Some("s-parent".to_string());
        config.verbose = true;
        let result = client.run(&config, &out).await.expect("run");
        assert_eq!(result.session_id, "s-forked");
        assert_eq!(agent.seen_of("fork")[0].entry_id, "entry-1");
        assert_eq!(agent.seen_of("fork")[0].session_id, "s-parent");
        let stderr = String::from_utf8(cap.err.lock().unwrap().clone()).unwrap();
        assert!(stderr.contains("Forking from entry entry-1..."));
    }

    #[tokio::test]
    async fn run_fork_without_session_picks_latest() {
        let mut agent = fresh_run_agent();
        agent.responses.insert(
            "list_sessions".into(),
            "{\"sessions\":[{\"id\":\"s-latest\",\"updated_at\":\"2026-08-09T00:00:00Z\"}]}".into(),
        );
        agent
            .responses
            .insert("fork".into(), "{\"cancelled\":false}".into());
        let addr = spawn_mock(agent.clone()).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let mut config = run_config("hi");
        config.fork = Some("e9".to_string());
        let result = client.run(&config, &out).await.expect("run");
        // No sessionId in fork response → stays on the parent.
        assert_eq!(result.session_id, "s-latest");
        assert_eq!(agent.seen_of("fork")[0].session_id, "s-latest");
    }

    #[tokio::test]
    async fn run_fork_cancelled_errors() {
        let mut agent = fresh_run_agent();
        agent.responses.insert(
            "list_sessions".into(),
            "{\"sessions\":[{\"id\":\"s1\",\"updated_at\":\"2026-08-09T00:00:00Z\"}]}".into(),
        );
        agent
            .responses
            .insert("fork".into(), "{\"cancelled\":true}".into());
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let mut config = run_config("hi");
        config.fork = Some("e9".to_string());
        assert_eq!(client.run(&config, &out).await.unwrap_err(), "Fork was cancelled");
    }

    #[tokio::test]
    async fn run_fork_without_any_session_errors() {
        let mut agent = fresh_run_agent();
        agent
            .responses
            .insert("list_sessions".into(), "{\"sessions\":[]}".into());
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let mut config = run_config("hi");
        config.fork = Some("e9".to_string());
        let err = client.run(&config, &out).await.unwrap_err();
        assert_eq!(err, "No previous session to fork from.");
        // Sessions present but all unparseable → same error.
        let mut agent = fresh_run_agent();
        agent
            .responses
            .insert("list_sessions".into(), "{\"sessions\":[{\"bogus\":1}]}".into());
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let err = client.run(&config, &out).await.unwrap_err();
        assert_eq!(err, "No previous session to fork from.");
    }

    #[tokio::test]
    async fn run_new_session_missing_id_errors() {
        let mut agent = MockAgent::default();
        agent.responses.insert("new_session".into(), "{}".into());
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let config = run_config("hi");
        assert_eq!(
            client.run(&config, &out).await.unwrap_err(),
            "new_session returned no sessionId"
        );
    }

    #[tokio::test]
    async fn run_prompt_failure_propagates() {
        let mut agent = fresh_run_agent();
        agent.fail_types.insert("prompt".into());
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let config = run_config("hi");
        assert_eq!(client.run(&config, &out).await.unwrap_err(), "boom");
    }

    #[tokio::test]
    async fn run_get_state_failure_is_ignored() {
        let mut agent = fresh_run_agent();
        agent.fail_types.insert("get_state".into());
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, _) = Output::memory();
        let mut config = run_config("hi");
        config.mode = "json".to_string();
        let result = client.run(&config, &out).await.expect("run");
        assert!(result.model.is_none());
        assert!(result.thinking_level.is_none());
    }

    #[tokio::test]
    async fn run_text_mode_trailing_newline_rules() {
        // Text already ending in \n → no extra newline.
        let mut agent = fresh_run_agent();
        agent.events = vec![stream_event("text_chunk", "{\"text\":\"done\\n\"}")];
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, cap) = Output::memory();
        let mut config = run_config("hi");
        config.mode = "text".to_string();
        client.run(&config, &out).await.expect("run");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "done\n");

        // Empty text → nothing at all.
        let mut agent = fresh_run_agent();
        agent.events = vec![stream_event("agent_end", "{}")];
        let addr = spawn_mock(agent).await;
        let client = RunClient::new(&addr);
        let (out, cap) = Output::memory();
        let mut config = run_config("hi");
        config.mode = "text".to_string();
        client.run(&config, &out).await.expect("run");
        let stdout = String::from_utf8(cap.out.lock().unwrap().clone()).unwrap();
        assert_eq!(stdout, "");
    }

    #[tokio::test]
    async fn notify_agent_refresh_skills_best_effort() {
        let _guard = crate::test_env::lock_env().await;
        // Against a live mock: covers the success path.
        let agent = MockAgent::default();
        let addr = spawn_mock(agent.clone()).await;
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from(addr),
        )]);
        notify_agent_refresh_skills().await;
        // Wait for the fire-and-forget call to land.
        for _ in 0..40 {
            if !agent.seen_of("refresh_skills").is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(agent.seen_of("refresh_skills").len(), 1);
        // Against a dead port: errors are swallowed.
        let _env = crate::test_env::EnvGuard::set(&[(
            "FUTURE_AGENT_GRPC_ADDR",
            std::ffi::OsString::from("127.0.0.1:1"),
        )]);
        notify_agent_refresh_skills().await;
    }
}

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

use crate::generated::proto::future_agent_client::FutureAgentClient;
use crate::generated::proto::{RpcCommand, StreamEvent, StreamRequest};
use crate::output::Output;
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

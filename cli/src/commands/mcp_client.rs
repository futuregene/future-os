//! A minimal stdio MCP client — the execution half of `future mcp`.
//!
//! FutureOS has no MCP client inside the agent (its tool set is the four
//! built-ins), so the CLI is the one place an MCP server can be reached. The
//! JSON-RPC layer below is deliberately generic over its reader and writer:
//! framing, id matching, notification skipping, cursor paging and the per-frame
//! cap are all exercised on an in-memory duplex, and process spawning is a thin
//! adapter on top of it.
//!
//! Transport is stdio only. The server is spawned with piped stdio and speaks
//! newline-delimited JSON (the MCP stdio framing — no `Content-Length` header).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// Protocol revision this client asks for. A server may answer with a different
/// revision; the client keeps using what it was given.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Largest single JSON-RPC frame accepted from a server.
///
/// A real server can answer `tools/list` with a few megabytes (REA: 3.1 MB for
/// 207 tools), which must still fit. The cap is the backstop behind
/// `future mcp call --max-bytes`: that flag bounds what is *printed*, this
/// bounds what can be read into memory at all.
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

/// Ceiling on `tools/list` pages followed, so a server that repeats the same
/// `nextCursor` cannot spin forever.
const MAX_TOOL_PAGES: usize = 200;

/// How long a well-behaved server gets to exit after its stdin closes.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// Default per-request timeout (`--timeout` overrides it).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Lines of a server's stderr kept for diagnostics.
const STDERR_TAIL_LINES: usize = 40;

/// One configured MCP server, as `~/.future/mcp.json` describes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerSpec {
    /// The executable to spawn. Resolved through `PATH`.
    pub command: String,
    /// Arguments passed verbatim.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables. The parent environment is inherited.
    #[serde(default)]
    pub env: Map<String, Value>,
    /// Working directory for the server process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Disabled servers stay in the file but are refused by name.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl ServerSpec {
    /// The command line, for error messages (`rea --mcp`).
    pub fn command_line(&self) -> String {
        std::iter::once(self.command.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

// ── The JSON-RPC session ───────────────────────────────────────────────────

/// One MCP session over a line-delimited JSON channel.
pub struct Session<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> {
    reader: BufReader<R>,
    writer: W,
    next_id: u64,
    timeout: Duration,
}

impl<R, W> Session<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    pub fn new(reader: R, writer: W, timeout: Duration) -> Self {
        Self {
            reader: BufReader::new(reader),
            writer,
            next_id: 1,
            timeout,
        }
    }

    /// `initialize` followed by the `notifications/initialized` ack.
    pub async fn initialize(&mut self) -> Result<Value, String> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "future", "version": crate::version::VERSION },
        });
        let result = self.request("initialize", params).await?;
        self.notify("notifications/initialized", json!({})).await?;
        Ok(result)
    }

    /// Fire-and-forget notification (no id, no response).
    pub async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.send(&body).await
    }

    /// One request, waiting for the response carrying this id.
    ///
    /// Frames the server sends on its own (notifications, logging) are skipped:
    /// the client declares no server-directed capabilities, so anything that
    /// arrives before the answer is not ours to act on.
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.send(&body).await?;

        let timeout = self.timeout;
        match tokio::time::timeout(timeout, self.await_response(id, method)).await {
            Ok(result) => result,
            Err(_) => Err(format!(
                "MCP '{method}' timed out after {}s (raise --timeout for a slow server)",
                timeout.as_secs()
            )),
        }
    }

    /// Every tool the server advertises, following `nextCursor` to the end.
    pub async fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_TOOL_PAGES {
            let mut params = Map::new();
            if let Some(cursor) = cursor.as_ref() {
                params.insert("cursor".to_string(), Value::String(cursor.clone()));
            }
            let result = self.request("tools/list", Value::Object(params)).await?;
            if let Some(page) = result.get("tools").and_then(Value::as_array) {
                tools.extend(page.iter().cloned());
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|cursor| !cursor.is_empty())
                .map(str::to_string);
            if cursor.is_none() {
                return Ok(tools);
            }
        }
        Err(format!(
            "MCP 'tools/list' did not finish after {MAX_TOOL_PAGES} pages — the server keeps \
             returning a nextCursor"
        ))
    }

    /// One `tools/call`. The returned value is the MCP `result` envelope
    /// (`content`, `isError`, …), not the tool's own payload.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let params = json!({ "name": name, "arguments": arguments });
        self.request("tools/call", params).await
    }

    async fn send(&mut self, body: &Value) -> Result<(), String> {
        let mut line =
            serde_json::to_vec(body).map_err(|error| format!("encode MCP request: {error}"))?;
        line.push(b'\n');
        self.writer
            .write_all(&line)
            .await
            .map_err(|error| format!("write to the MCP server failed: {error}"))?;
        self.writer
            .flush()
            .await
            .map_err(|error| format!("write to the MCP server failed: {error}"))
    }

    async fn await_response(&mut self, id: u64, method: &str) -> Result<Value, String> {
        loop {
            let Some(frame) = read_frame(&mut self.reader, MAX_FRAME_BYTES).await? else {
                return Err(format!(
                    "the MCP server closed its output before answering '{method}'"
                ));
            };
            if frame.is_empty() {
                continue;
            }
            let message: Value = match serde_json::from_slice(&frame) {
                Ok(message) => message,
                Err(error) => {
                    return Err(format!(
                        "the MCP server sent an unreadable frame while answering '{method}': \
                         {error}"
                    ))
                }
            };
            // Notifications and other answers carry no matching id.
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(format!("MCP '{method}' failed: {}", describe_error(error)));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

fn describe_error(error: &Value) -> String {
    let code = match error.get("code") {
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::String(text)) => text.clone(),
        _ => "unknown".to_string(),
    };
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("no message");
    let data = error
        .get("data")
        .filter(|data| !data.is_null())
        .map(|data| format!(" — {data}"))
        .unwrap_or_default();
    format!("code={code}, message={message}{data}")
}

/// Read one newline-delimited frame, refusing anything past `limit` bytes.
///
/// `Ok(None)` means the server closed its output.
async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    limit: usize,
) -> Result<Option<Vec<u8>>, String> {
    let mut frame = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .await
            .map_err(|error| format!("read from the MCP server failed: {error}"))?;
        if available.is_empty() {
            return Ok(if frame.is_empty() { None } else { Some(frame) });
        }
        if let Some(end) = available.iter().position(|byte| *byte == b'\n') {
            frame.extend_from_slice(&available[..end]);
            reader.consume(end + 1);
            return Ok(Some(frame));
        }
        let taken = available.len();
        frame.extend_from_slice(available);
        reader.consume(taken);
        if frame.len() > limit {
            return Err(format!(
                "the MCP server sent a frame larger than {limit} bytes"
            ));
        }
    }
}

// ── The spawned server ─────────────────────────────────────────────────────

/// A spawned MCP server and its session.
///
/// Dropping this kills the child (`kill_on_drop`), so an error path cannot
/// leave an orphan behind; [`shutdown`](Self::shutdown) closes stdin first and
/// only kills a server that ignores that.
pub struct ServerProcess {
    child: tokio::process::Child,
    session: Session<tokio::process::ChildStdout, tokio::process::ChildStdin>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
}

impl ServerProcess {
    /// Spawn `name` (the configured server) and complete the MCP handshake.
    pub async fn start(name: &str, spec: &ServerSpec, timeout: Duration) -> Result<Self, String> {
        let mut command = tokio::process::Command::new(&spec.command);
        command
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for (key, value) in &spec.env {
            // A non-string value is a configuration error, not something to
            // stringify silently: JSON allows it, a process environment does not.
            let value = value.as_str().ok_or_else(|| {
                format!(
                    "server '{name}': environment variable {key} must be a string, not {}",
                    json_kind(value)
                )
            })?;
            command.env(key, value);
        }
        if let Some(cwd) = spec.cwd.as_ref() {
            command.current_dir(cwd);
        }

        let mut child = command.spawn().map_err(|error| {
            format!(
                "could not start the MCP server '{name}' ({}): {error}",
                spec.command_line()
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("server '{name}': stdin was not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("server '{name}': stdout was not piped"))?;

        let stderr_tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(stderr) = child.stderr.take() {
            let tail = Arc::clone(&stderr_tail);
            tokio::spawn(async move { drain_stderr(stderr, tail).await });
        }

        let mut session = Session::new(stdout, stdin, timeout);
        if let Err(error) = session.initialize().await {
            let mut message = format!("the MCP server '{name}' failed its handshake: {error}");
            let tail = tail_of(&stderr_tail);
            if !tail.trim().is_empty() {
                message.push_str("\n--- server stderr (last lines) ---\n");
                message.push_str(tail.trim_end());
            }
            return Err(message);
        }
        Ok(Self {
            child,
            session,
            stderr_tail,
        })
    }

    pub fn session(
        &mut self,
    ) -> &mut Session<tokio::process::ChildStdout, tokio::process::ChildStdin> {
        &mut self.session
    }

    /// The server's recent stderr, for attaching to a failure message.
    pub fn stderr_tail(&self) -> String {
        tail_of(&self.stderr_tail)
    }

    /// Close stdin, give the server a moment to exit, then kill whatever is left.
    pub async fn shutdown(mut self) {
        drop(self.session);
        if tokio::time::timeout(SHUTDOWN_GRACE, self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
    }
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn tail_of(tail: &Arc<Mutex<VecDeque<String>>>) -> String {
    tail.lock()
        .map(|lines| lines.iter().cloned().collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

/// Drain stderr continuously: a full pipe would block the server process, and
/// the last lines are what a failure needs to explain itself.
async fn drain_stderr<R: AsyncRead + Unpin>(stderr: R, tail: Arc<Mutex<VecDeque<String>>>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Ok(mut tail) = tail.lock() {
            if tail.len() == STDERR_TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    struct FakeServer {
        session: Session<tokio::io::DuplexStream, tokio::io::DuplexStream>,
        seen: Arc<Mutex<Vec<Value>>>,
    }

    /// Start a fake server on an in-memory duplex and return a client session
    /// wired to it. `pages` are the successive `tools/list` answers.
    fn fake_server(pages: Vec<Value>) -> FakeServer {
        let (client_out, server_in) = tokio::io::duplex(256 * 1024);
        let (server_out, client_in) = tokio::io::duplex(256 * 1024);
        let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

        let seen_server = Arc::clone(&seen);
        tokio::spawn(async move {
            let mut lines = BufReader::new(server_in).lines();
            let mut output = server_out;
            let mut page = 0usize;
            while let Ok(Some(line)) = lines.next_line().await {
                let message: Value = serde_json::from_str(&line).unwrap();
                seen_server.lock().unwrap().push(message.clone());
                if message.get("id").is_none() {
                    continue;
                }
                let method = message["method"].as_str().unwrap_or_default().to_string();

                // An unsolicited notification before every answer: the client
                // must skip it rather than mistake it for the response.
                let chatter = json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/message",
                    "params": { "level": "info", "data": "warming up" },
                });
                let mut chatter = serde_json::to_vec(&chatter).unwrap();
                chatter.push(b'\n');
                output.write_all(&chatter).await.unwrap();

                let answer = match method.as_str() {
                    "initialize" => Ok(json!({ "protocolVersion": PROTOCOL_VERSION })),
                    "tools/list" => {
                        let page_value = pages
                            .get(page)
                            .cloned()
                            .unwrap_or_else(|| json!({ "tools": [] }));
                        page += 1;
                        Ok(page_value)
                    }
                    "tools/call" => Ok(json!({
                        "content": [{ "type": "text", "text": "called" }],
                    })),
                    other => Err(json!({ "code": -32601, "message": other })),
                };
                let mut body = json!({ "jsonrpc": "2.0", "id": message["id"].clone() });
                match answer {
                    Ok(result) => body["result"] = result,
                    Err(error) => body["error"] = error,
                }
                let mut bytes = serde_json::to_vec(&body).unwrap();
                bytes.push(b'\n');
                output.write_all(&bytes).await.unwrap();
                output.flush().await.unwrap();
            }
        });

        FakeServer {
            session: Session::new(client_in, client_out, Duration::from_secs(5)),
            seen,
        }
    }

    #[tokio::test]
    async fn handshake_sends_initialize_then_the_initialized_notification() {
        let mut server = fake_server(vec![json!({ "tools": [] })]);
        server.session.initialize().await.unwrap();
        // A second round trip: the fake server reads its input in order, so once
        // this answer is back the notification before it has been recorded too
        // (asserting straight after `initialize` would race the server task).
        server.session.list_tools().await.unwrap();

        let seen = server.seen.lock().unwrap();
        assert_eq!(seen[0]["method"], "initialize");
        assert_eq!(seen[0]["params"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(seen[0]["params"]["clientInfo"]["name"], "future");
        assert_eq!(seen[1]["method"], "notifications/initialized");
        assert!(seen[1].get("id").is_none(), "a notification carries no id");
    }

    #[tokio::test]
    async fn list_tools_follows_the_cursor_and_ignores_unsolicited_frames() {
        let mut server = fake_server(vec![
            json!({
                "tools": [{ "name": "first", "description": "one" }],
                "nextCursor": "page-2",
            }),
            json!({ "tools": [{ "name": "second", "description": "two" }] }),
        ]);

        let tools = server.session.list_tools().await.unwrap();
        let names: Vec<&str> = tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["first", "second"]);

        let seen = server.seen.lock().unwrap();
        let pages: Vec<&Value> = seen
            .iter()
            .filter(|message| message["method"] == "tools/list")
            .collect();
        assert_eq!(pages.len(), 2, "the second page must be requested");
        assert_eq!(pages[1]["params"]["cursor"], "page-2");
    }

    #[tokio::test]
    async fn a_tool_error_is_reported_with_its_code_and_message() {
        let mut server = fake_server(vec![]);
        // `tools/call` is answered normally by the fake, so ask for a method it
        // rejects instead.
        let error = server
            .session
            .request("resources/list", json!({}))
            .await
            .unwrap_err();
        assert!(error.contains("-32601"), "{error}");
        assert!(error.contains("resources/list"), "{error}");
    }

    #[tokio::test]
    async fn a_closed_output_is_reported_instead_of_hanging() {
        let (client_out, _server_in) = tokio::io::duplex(4096);
        let (server_out, client_in) = tokio::io::duplex(4096);
        // The server's write half is gone, so the answer can never arrive.
        drop(server_out);

        let mut session = Session::new(client_in, client_out, Duration::from_secs(5));
        let error = session.request("initialize", json!({})).await.unwrap_err();
        assert!(error.contains("closed its output"), "{error}");
    }

    #[tokio::test]
    async fn a_frame_past_the_cap_is_refused() {
        let (client_out, server_in) = tokio::io::duplex(4096);
        let (mut server_out, client_in) = tokio::io::duplex(4096);
        drop(client_out);
        drop(server_in);
        // One frame with no newline in it, longer than the cap under test.
        server_out.write_all(&[b'x'; 200]).await.unwrap();

        let mut reader = BufReader::new(client_in);
        let error = read_frame(&mut reader, 64).await.unwrap_err();
        assert!(error.contains("larger than 64 bytes"), "{error}");
    }
}

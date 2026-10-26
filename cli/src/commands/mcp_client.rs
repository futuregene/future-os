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

/// Default limit on one JSON-RPC frame accepted from a server.
///
/// A real server can answer `tools/list` with a few megabytes (REA: 3.1 MB for
/// 207 tools), which must still fit. This bounds what is *read into memory*,
/// which is a different budget from `future mcp call --max-bytes` (what is
/// *printed*): the print cap cannot help when the frame itself will not fit.
/// Raise it with `--max-frame-bytes` or `FUTURE_MCP_MAX_FRAME_BYTES`.
pub const DEFAULT_MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

/// Environment variable that overrides [`DEFAULT_MAX_FRAME_BYTES`].
pub const MAX_FRAME_BYTES_ENV: &str = "FUTURE_MCP_MAX_FRAME_BYTES";

/// The frame cap in effect: `FUTURE_MCP_MAX_FRAME_BYTES` when it parses, the
/// default otherwise.
///
/// A malformed value is ignored rather than fatal: this is a read budget, and
/// refusing to run because it is misspelled would be worse than using it.
pub fn max_frame_bytes_from_env() -> usize {
    std::env::var(MAX_FRAME_BYTES_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|bytes| *bytes > 0)
        .unwrap_or(DEFAULT_MAX_FRAME_BYTES)
}

/// Ceiling on `tools/list` pages followed, so a server that repeats the same
/// `nextCursor` cannot spin forever.
const MAX_TOOL_PAGES: usize = 200;

/// How long a well-behaved server gets to exit after its stdin closes.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// Default per-request timeout (`--timeout` overrides it).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Lines of a server's stderr kept for diagnostics.
const STDERR_TAIL_LINES: usize = 40;

/// How long a failed handshake waits for the server's stderr to reach EOF
/// before reporting without it: the wait ends at the server's own exit, and the
/// bound is a backstop for a server that closed stdout while keeping stderr
/// open (or left it open behind a child of its own).
const STDERR_DRAIN_GRACE: Duration = Duration::from_secs(2);

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
    max_frame_bytes: usize,
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
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
        }
    }

    /// Override the frame cap (see [`DEFAULT_MAX_FRAME_BYTES`]).
    pub fn with_max_frame_bytes(mut self, bytes: usize) -> Self {
        self.max_frame_bytes = bytes;
        self
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
            let Some(frame) = read_frame(&mut self.reader, self.max_frame_bytes, method).await?
            else {
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
/// `Ok(None)` means the server closed its output. `method` names what was being
/// awaited, and the over-limit error explains both budgets, because the print
/// cap (`call --max-bytes`) cannot rescue a frame that will not fit in memory.
async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    limit: usize,
    method: &str,
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
            // The cap has to be checked here too: a frame that arrives together
            // with its newline is otherwise never measured, and the limit would
            // depend on how the pipe happened to chunk it.
            if frame.len() + end > limit {
                return Err(over_cap_error(limit, method));
            }
            frame.extend_from_slice(&available[..end]);
            reader.consume(end + 1);
            return Ok(Some(frame));
        }
        let taken = available.len();
        frame.extend_from_slice(available);
        reader.consume(taken);
        if frame.len() > limit {
            return Err(over_cap_error(limit, method));
        }
    }
}

/// The over-cap message: it names what was being awaited and both budgets, so
/// the reader is not left with only `--max-bytes`, which cannot help.
fn over_cap_error(limit: usize, method: &str) -> String {
    format!(
        "the MCP server sent a frame larger than {limit} bytes while answering '{method}'\n\
         This is the read budget (what is held in memory), which `--max-bytes` does not affect. \
         Raise it with --max-frame-bytes <bytes> or {MAX_FRAME_BYTES_ENV}, or make the server \
         answer with less — many servers have their own response budget."
    )
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
        Self::start_with_frame_cap(name, spec, timeout, max_frame_bytes_from_env()).await
    }

    /// [`start`](Self::start) with an explicit frame cap (see
    /// [`DEFAULT_MAX_FRAME_BYTES`]).
    pub async fn start_with_frame_cap(
        name: &str,
        spec: &ServerSpec,
        timeout: Duration,
        max_frame_bytes: usize,
    ) -> Result<Self, String> {
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
        let stderr_drain = child.stderr.take().map(|stderr| {
            let tail = Arc::clone(&stderr_tail);
            tokio::spawn(async move { drain_stderr(stderr, tail).await })
        });

        let mut session =
            Session::new(stdout, stdin, timeout).with_max_frame_bytes(max_frame_bytes);
        if let Err(error) = session.initialize().await {
            // The tail is written by a task of its own, so nothing orders it
            // before this read: a server that dies during its handshake is
            // exactly the case where its own stderr is the explanation, and on a
            // busy host this error path can run before the drain task has ever
            // been polled. Join it first: it ends when the server's stderr
            // reaches EOF, which the server's exit produces (the bound is a
            // backstop for a server that closed stdout but kept stderr open, or
            // left it open behind a child of its own).
            if let Some(drain) = stderr_drain {
                let _ = tokio::time::timeout(STDERR_DRAIN_GRACE, drain).await;
            }
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
        task: tokio::task::JoinHandle<()>,
    }

    /// Let a fake server's task run to its own end: dropping the client half
    /// makes its read loop see EOF, so it finishes instead of being cancelled by
    /// the runtime at teardown. Awaiting it is what makes the loop's last line an
    /// executed line rather than a span the runtime aborts mid-await; it is also
    /// how a leak would surface (a task that never ends fails the test instead of
    /// vanishing quietly).
    async fn finish(server: FakeServer) {
        let FakeServer { session, task, .. } = server;
        drop(session);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the fake server must end when the client hangs up")
            .expect("the fake server task panics rather than ends");
    }

    /// A snapshot of the requests the fake server has seen, cloned out of the
    /// lock inside this call. Tests assert on the snapshot rather than holding a
    /// guard, so no `MutexGuard` can be live at the `await` in `finish` — which
    /// is what `clippy::await_holding_lock` (a syntactic lint: it does not see an
    /// explicit `drop`) demands.
    fn seen_messages(server: &FakeServer) -> Vec<Value> {
        server.seen.lock().unwrap().clone()
    }

    /// Start a fake server on an in-memory duplex and return a client session
    /// wired to it. `pages` are the successive `tools/list` answers.
    fn fake_server(pages: Vec<Value>) -> FakeServer {
        let (client_out, server_in) = tokio::io::duplex(256 * 1024);
        let (server_out, client_in) = tokio::io::duplex(256 * 1024);
        let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

        let seen_server = Arc::clone(&seen);
        let task = tokio::spawn(async move {
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
            task,
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

        let seen = seen_messages(&server);
        assert_eq!(seen[0]["method"], "initialize");
        assert_eq!(seen[0]["params"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(seen[0]["params"]["clientInfo"]["name"], "future");
        assert_eq!(seen[1]["method"], "notifications/initialized");
        assert!(seen[1].get("id").is_none(), "a notification carries no id");
        finish(server).await;
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

        let seen = seen_messages(&server);
        let pages: Vec<&Value> = seen
            .iter()
            .filter(|message| message["method"] == "tools/list")
            .collect();
        assert_eq!(pages.len(), 2, "the second page must be requested");
        assert_eq!(pages[1]["params"]["cursor"], "page-2");
        finish(server).await;
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
        finish(server).await;
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
        let error = read_frame(&mut reader, 64, "tools/call").await.unwrap_err();
        assert!(error.contains("larger than 64 bytes"), "{error}");
    }

    /// The over-cap error must be actionable: name the method being awaited,
    /// distinguish the read budget from the print budget, and give both knobs.
    #[tokio::test]
    async fn an_over_cap_frame_says_how_to_raise_the_budget() {
        let (client_out, server_in) = tokio::io::duplex(4096);
        let (mut server_out, client_in) = tokio::io::duplex(4096);
        drop(client_out);
        drop(server_in);
        server_out.write_all(&[b'x'; 200]).await.unwrap();

        let mut reader = BufReader::new(client_in);
        let error = read_frame(&mut reader, 64, "tools/call").await.unwrap_err();
        assert!(error.contains("answering 'tools/call'"), "{error}");
        assert!(error.contains("--max-frame-bytes"), "{error}");
        assert!(error.contains(MAX_FRAME_BYTES_ENV), "{error}");
        assert!(error.contains("--max-bytes"), "{error}");
    }

    #[tokio::test]
    async fn the_frame_cap_can_be_raised_for_one_session() {
        // A frame that arrives together with its newline must still be measured,
        // not accepted because of how the pipe chunked it.
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "tools": [], "padding": "y".repeat(20_000) },
        });
        let mut bytes = serde_json::to_vec(&payload).unwrap();
        bytes.push(b'\n');

        // The server ends stay alive: dropping the read end would turn the
        // client's *write* into a broken pipe, and the cap would never be read.
        let (client_out, _server_in) = tokio::io::duplex(64 * 1024);
        let (mut server_out, client_in) = tokio::io::duplex(64 * 1024);
        server_out.write_all(&bytes).await.unwrap();
        server_out.flush().await.unwrap();

        let mut strict =
            Session::new(client_in, client_out, Duration::from_secs(5)).with_max_frame_bytes(1024);
        let error = strict.request("tools/list", json!({})).await.unwrap_err();
        assert!(error.contains("larger than 1024 bytes"), "{error}");

        // The same bytes pass under a raised cap: that is what the knob buys.
        let (client_out, _server_in) = tokio::io::duplex(64 * 1024);
        let (mut server_out, client_in) = tokio::io::duplex(64 * 1024);
        server_out.write_all(&bytes).await.unwrap();
        server_out.flush().await.unwrap();
        let mut relaxed = Session::new(client_in, client_out, Duration::from_secs(5))
            .with_max_frame_bytes(64 * 1024);
        let result = relaxed.request("tools/list", json!({})).await.unwrap();
        assert_eq!(result["tools"], json!([]));
    }

    #[tokio::test]
    async fn a_malformed_frame_budget_falls_back_to_the_default() {
        // Env vars are process-global and cargo runs tests in threads, so this
        // holds the shared lock (see `crate::test_env`).
        let _lock = crate::test_env::lock_env().await;
        let _guard = crate::test_env::EnvGuard::remove(&[MAX_FRAME_BYTES_ENV]);
        assert_eq!(max_frame_bytes_from_env(), DEFAULT_MAX_FRAME_BYTES);

        // Not fatal: this is a read budget, and a typo should not stop the CLI.
        for raw in ["", "  ", "abc", "0", "-5", "12.5"] {
            std::env::set_var(MAX_FRAME_BYTES_ENV, raw);
            assert_eq!(
                max_frame_bytes_from_env(),
                DEFAULT_MAX_FRAME_BYTES,
                "raw={raw:?}"
            );
        }
        std::env::set_var(MAX_FRAME_BYTES_ENV, "1048576");
        assert_eq!(max_frame_bytes_from_env(), 1_048_576);
    }

    // ── The failure shapes the JSON fake cannot express ────────────────────

    /// A server that writes `payload` verbatim once the request has arrived and
    /// then holds the channel open until the client hangs up. Staying open is the
    /// point: closing the write end is the *EOF* failure, a different arm from
    /// "no answer came" — and unlike a `sleep`, the read loop gives the task a
    /// real end, which `close` below waits for.
    ///
    /// Returns the session plus its task, so a test can hang up and await the end
    /// instead of leaving the runtime to cancel it at teardown.
    fn raw_server(
        payload: String,
        timeout: Duration,
    ) -> (
        Session<tokio::io::DuplexStream, tokio::io::DuplexStream>,
        tokio::task::JoinHandle<()>,
    ) {
        let (client_out, server_in) = tokio::io::duplex(64 * 1024);
        let (mut server_out, client_in) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            let mut lines = BufReader::new(server_in).lines();
            let _ = lines.next_line().await;
            server_out.write_all(payload.as_bytes()).await.unwrap();
            server_out.flush().await.unwrap();
            while let Ok(Some(_)) = lines.next_line().await {}
        });
        (Session::new(client_in, client_out, timeout), task)
    }

    /// Hang up a [`raw_server`] session and wait for its task to end.
    async fn close<W>(
        session: Session<tokio::io::DuplexStream, W>,
        task: tokio::task::JoinHandle<()>,
    ) where
        W: AsyncWrite + Unpin,
    {
        drop(session);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the fixture server must end when the client hangs up")
            .expect("the fixture server task panics rather than ends");
    }

    /// A server every one of whose pages carries another cursor. Its read loop
    /// ends when the client hangs up, so `close` can await a real end.
    fn endless_cursor_server() -> (
        Session<tokio::io::DuplexStream, tokio::io::DuplexStream>,
        tokio::task::JoinHandle<()>,
    ) {
        let (client_out, server_in) = tokio::io::duplex(256 * 1024);
        let (mut server_out, client_in) = tokio::io::duplex(256 * 1024);
        let task = tokio::spawn(async move {
            let mut lines = BufReader::new(server_in).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let message: Value = serde_json::from_str(&line).unwrap();
                let body = json!({
                    "jsonrpc": "2.0",
                    "id": message["id"].clone(),
                    "result": { "tools": [], "nextCursor": "always-more" },
                });
                let mut bytes = serde_json::to_vec(&body).unwrap();
                bytes.push(b'\n');
                server_out.write_all(&bytes).await.unwrap();
                server_out.flush().await.unwrap();
            }
        });
        (
            Session::new(client_in, client_out, Duration::from_secs(30)),
            task,
        )
    }

    /// A request that is never answered must fail at its own deadline, and the
    /// message must name the budget the caller can raise.
    #[tokio::test]
    async fn a_silent_server_times_out_with_the_budget_it_was_given() {
        let (mut session, task) = raw_server(String::new(), Duration::from_secs(1));
        let error = session.request("tools/call", json!({})).await.unwrap_err();
        assert!(error.contains("'tools/call' timed out after 1s"), "{error}");
        assert!(error.contains("raise --timeout"), "{error}");
        close(session, task).await;
    }

    /// A server that keeps handing back a cursor must not spin forever: the
    /// page cap ends it, with an error that says so.
    #[tokio::test]
    async fn list_tools_gives_up_when_the_pages_never_end() {
        let (mut session, task) = endless_cursor_server();
        let error = session.list_tools().await.unwrap_err();
        assert!(error.contains("did not finish after 200 pages"), "{error}");
        assert!(error.contains("nextCursor"), "{error}");
        close(session, task).await;
    }

    /// A bare newline is a frame with nothing in it, which is skipped; a frame
    /// that is not JSON at all is an error naming the method being awaited.
    #[tokio::test]
    async fn an_empty_frame_is_skipped_and_an_unreadable_frame_is_reported() {
        let answer = json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } });
        let (mut session, task) = raw_server(format!("\n{answer}\n"), Duration::from_secs(5));
        let result = session.request("tools/list", json!({})).await.unwrap();
        assert_eq!(result["ok"], json!(true));
        close(session, task).await;

        let (mut session, task) =
            raw_server("not json at all\n".to_string(), Duration::from_secs(5));
        let error = session.request("tools/list", json!({})).await.unwrap_err();
        assert!(error.contains("unreadable frame"), "{error}");
        assert!(error.contains("answering 'tools/list'"), "{error}");
        close(session, task).await;
    }

    /// Every shape an `error` member can take is described rather than dropped:
    /// a numeric code, a string code, no code, no message, with data, and with
    /// an explicit `null` where the data would be.
    #[test]
    fn every_error_code_shape_is_described() {
        assert_eq!(
            describe_error(&json!({ "code": -32601, "message": "no such method" })),
            "code=-32601, message=no such method"
        );
        assert_eq!(
            describe_error(&json!({ "code": "EPARSE", "message": "bad" })),
            "code=EPARSE, message=bad"
        );
        assert_eq!(
            describe_error(&json!({ "message": "bad" })),
            "code=unknown, message=bad"
        );
        assert_eq!(
            describe_error(&json!({ "code": 1 })),
            "code=1, message=no message"
        );
        assert_eq!(
            describe_error(&json!({ "code": 1, "data": { "hint": "x" } })),
            "code=1, message=no message — {\"hint\":\"x\"}"
        );
        assert_eq!(
            describe_error(&json!({ "code": 1, "message": "m", "data": Value::Null })),
            "code=1, message=m"
        );
    }

    #[tokio::test]
    async fn call_tool_sends_the_name_and_arguments_it_was_given() {
        let mut server = fake_server(vec![]);
        let result = server
            .session
            .call_tool("plain", json!({ "path": "/bin/ls" }))
            .await
            .unwrap();
        assert_eq!(result["content"][0]["text"], "called");

        let seen = seen_messages(&server);
        let call = seen
            .iter()
            .find(|message| message["method"] == "tools/call")
            .expect("a tools/call request");
        assert_eq!(call["params"]["name"], "plain");
        assert_eq!(call["params"]["arguments"]["path"], "/bin/ls");
        finish(server).await;
    }

    #[test]
    fn every_json_kind_is_named_in_the_environment_error() {
        assert_eq!(json_kind(&Value::Null), "null");
        assert_eq!(json_kind(&json!(true)), "a boolean");
        assert_eq!(json_kind(&json!(7)), "a number");
        assert_eq!(json_kind(&json!("text")), "a string");
        assert_eq!(json_kind(&json!([1])), "an array");
        assert_eq!(json_kind(&json!({ "a": 1 })), "an object");
    }

    /// The environment is validated *before* anything is spawned: a JSON
    /// number is a configuration mistake, not something to stringify.
    #[tokio::test]
    async fn a_non_string_environment_value_is_refused_before_spawning() {
        let mut spec = ServerSpec {
            command: "definitely-not-a-real-program".to_string(),
            args: Vec::new(),
            env: Map::new(),
            cwd: None,
            enabled: true,
        };
        spec.env.insert("COUNT".to_string(), json!(5));
        // Not `unwrap_err`: the `Ok` type is not `Debug`, and the point is that
        // a started process would make this assertion unobservable.
        let error = ServerProcess::start("counted", &spec, Duration::from_secs(5))
            .await
            .err()
            .expect("a non-string environment value must be refused before spawning");
        assert_eq!(
            error,
            "server 'counted': environment variable COUNT must be a string, not a number"
        );
    }

    /// A minimal POSIX MCP server: answers `initialize` and `tools/list`, and
    /// exits when its stdin closes.
    ///
    /// The request id is taken with shell parameter expansion, never by piping
    /// through `sed`: the child inherits this process's environment, and other
    /// tests in the same binary call `std::env::set_var` while it is being
    /// spawned (libtest runs them in parallel, and `set_var` is `unsafe` in Rust
    /// 2024 for exactly this reason). When the copy of `environ` that `spawn`
    /// makes came out torn, the inherited `PATH` lost `/bin`, and this server
    /// died with `/bin/sh: line 8: sed: command not found` — measured twice on a
    /// 32-core host, always
    /// `the_stderr_tail_keeps_the_last_lines_and_drops_the_oldest`, which then
    /// failed its handshake assertion. Builtins alone cannot be affected.
    #[cfg(unix)]
    const SERVE_ONCE: &str = r#"
while IFS= read -r line; do
  id=${line#*\"id\":}
  id=${id%%,*}
  case "$line" in
    *notifications/initialized*) ;;
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id" ;;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[]}}\n' "$id" ;;
  esac
done
"#;

    #[cfg(unix)]
    mod process {
        use super::*;

        fn sh_spec(script: String, env: Map<String, Value>, cwd: Option<String>) -> ServerSpec {
            ServerSpec {
                command: "/bin/sh".to_string(),
                args: vec!["-c".to_string(), script],
                env,
                cwd,
                enabled: true,
            }
        }

        /// The configured environment and working directory must reach the
        /// child: a server that cannot read its own settings is unusable, and a
        /// silently dropped one is impossible to debug.
        #[tokio::test]
        async fn the_configured_environment_and_working_directory_reach_the_child() {
            let work = tempfile::tempdir().unwrap();
            let log = tempfile::tempdir().unwrap();
            let log_file = log.path().join("probe.log");
            let mut env = Map::new();
            env.insert("MCP_PROBE".to_string(), json!("value"));
            env.insert(
                "MCP_PROBE_LOG".to_string(),
                json!(log_file.display().to_string()),
            );
            let spec = sh_spec(
                r#"printf '%s|%s' "$MCP_PROBE" "$(pwd -P)" > "$MCP_PROBE_LOG""#.to_string(),
                env,
                Some(work.path().display().to_string()),
            );

            // The script records its settings and exits without answering, so
            // the handshake fails: what is under test was on disk before that.
            let error = ServerProcess::start("probe", &spec, Duration::from_secs(5))
                .await
                .err()
                .expect("a script that never answers cannot complete a handshake");
            assert!(error.contains("failed its handshake"), "{error}");

            let canonical = std::fs::canonicalize(work.path()).unwrap();
            assert_eq!(
                std::fs::read_to_string(&log_file).unwrap(),
                format!("value|{}", canonical.display())
            );
        }

        /// Closing stdin is not enough for a server that ignores it: `shutdown`
        /// waits out its grace period and then kills what is left.
        #[tokio::test]
        async fn shutdown_kills_a_server_that_ignores_its_closed_stdin() {
            let spec = sh_spec(format!("{SERVE_ONCE}\nsleep 30"), Map::new(), None);
            let process = ServerProcess::start("stubborn", &spec, Duration::from_secs(10))
                .await
                .expect("the handshake is a normal one");

            let started = std::time::Instant::now();
            process.shutdown().await;
            let elapsed = started.elapsed();
            assert!(
                elapsed >= SHUTDOWN_GRACE,
                "the grace period must be waited out: {elapsed:?}"
            );
            assert!(
                elapsed < Duration::from_secs(20),
                "the kill must end it: {elapsed:?}"
            );
        }

        /// Only the last lines of stderr are kept: the oldest are dropped, and
        /// the newest are what explains a failure.
        #[tokio::test]
        async fn the_stderr_tail_keeps_the_last_lines_and_drops_the_oldest() {
            const CHATTY: &str = r#"
i=1
while [ $i -le 45 ]; do
  printf 'stderr-%02d\n' "$i" >&2
  i=$((i + 1))
done
"#;
            let spec = sh_spec(format!("{CHATTY}{SERVE_ONCE}"), Map::new(), None);
            let process = ServerProcess::start("chatty", &spec, Duration::from_secs(10))
                .await
                .expect("the handshake is a normal one");

            // The drain is a background task: wait (bounded) for the tail to
            // fill rather than guessing how long it needs.
            crate::test_env::wait_for(|| {
                process.stderr_tail().lines().count() == STDERR_TAIL_LINES
            })
            .await;

            let tail = process.stderr_tail();
            let lines: Vec<&str> = tail.lines().collect();
            assert_eq!(lines.len(), STDERR_TAIL_LINES, "{tail}");
            assert_eq!(
                lines[0], "stderr-06",
                "the oldest lines are dropped: {tail}"
            );
            assert_eq!(lines[STDERR_TAIL_LINES - 1], "stderr-45", "{tail}");
            process.shutdown().await;
        }

        /// A poisoned tail is a state both of its users must survive. Nothing in
        /// this crate panics while holding that lock — `drain_stderr` guards its
        /// own lock and `tail_of` falls back — so the only way to produce the
        /// state is to poison it deliberately, and that is what this does.
        ///
        /// The server writes a stderr line on every `tools/call`, so lines keep
        /// arriving *after* the poison: the drain must skip them instead of
        /// panicking (a panic there would stop reading the pipe and block the
        /// child on a full stderr buffer), and the caller-visible tail must stay
        /// readable rather than panicking a `future mcp status`.
        #[tokio::test]
        async fn a_poisoned_stderr_tail_is_tolerated_by_the_drain_and_the_reader() {
            const ON_DEMAND_STDERR: &str = r#"
while IFS= read -r line; do
  id=${line#*\"id\":}
  id=${id%%,*}
  case "$line" in
    *notifications/initialized*) ;;
    *'"initialize"'*)
      printf 'first\n' >&2
      printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id" ;;
    *'"tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[]}}\n' "$id" ;;
    *'"tools/call"'*)
      printf 'later\n' >&2
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[]}}\n' "$id" ;;
  esac
done
"#;
            let spec = sh_spec(ON_DEMAND_STDERR.to_string(), Map::new(), None);
            let mut process = ServerProcess::start("gated", &spec, Duration::from_secs(10))
                .await
                .expect("the handshake is a normal one");

            // Before the poison the drain keeps what the server wrote, which is
            // what makes the assertions below about a real tail rather than an
            // empty one. (No `{:?}` argument here: an assertion message is only
            // evaluated on failure, so it would be a line no passing test can
            // cover — the same class as module-cli.md §6.1's `auth.rs:988`.)
            let kept = crate::test_env::wait_for(|| process.stderr_tail().contains("first")).await;
            assert!(kept, "the drain must keep the handshake's stderr");

            // Poison it the way a panicking reader would: hold the lock, unwind.
            let tail = Arc::clone(&process.stderr_tail);
            let poisoned = std::thread::spawn(move || {
                let _held = tail.lock().expect("a fresh lock");
                panic!("poison the stderr tail deliberately");
            })
            .join();
            assert!(poisoned.is_err(), "unwinding while holding it poisons it");

            // Lines arriving after the poison are dropped, not fatal, and the
            // session is unaffected. Several calls, because the drain task is
            // woken by each write — the point is that a lock *failure* never
            // stops the drain, which a single line could leave to timing.
            for _ in 0..3 {
                let result = process
                    .session()
                    .call_tool("probe", json!({}))
                    .await
                    .expect("the session does not depend on the diagnostics lock");
                assert_eq!(result["content"], json!([]));
            }

            // What the caller sees: a readable (empty) tail, not a panic.
            assert_eq!(process.stderr_tail(), "");
            process.shutdown().await;
        }
    }
}

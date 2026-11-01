//! The Streamable HTTP transport for `future mcp`.
//!
//! An MCP server can be a local process (see [`super::mcp_client`]) or an HTTP
//! endpoint, and this is the second one: a single URL the client POSTs JSON-RPC
//! messages to. Three properties of that transport matter for the code below.
//!
//! - **The answer is either JSON or a stream.** A POST is answered with one
//!   `application/json` body or with `text/event-stream`, where each event's
//!   `data:` line is one JSON-RPC message. Both are handled here, and an event
//!   stream is read until *our* answer arrives rather than until it closes:
//!   a server may send notifications on the same stream and may keep it open.
//! - **The session is named by a response header.** `initialize` answers with
//!   `Mcp-Session-Id`, and every later request has to echo it back; the negotiated
//!   protocol revision travels as `MCP-Protocol-Version`. A server that offers no
//!   session id is legal, and then no header is sent.
//! - **A notification is answered with 202 and no body.** Nothing is awaited
//!   beyond the status, because there is nothing to await.
//!
//! Everything a command does with a session — the handshake, paging the tool
//! list, calling a tool, describing an error — is shared with the stdio
//! transport through [`Rpc`](super::mcp_client::Rpc), so the two behave the same.
//!
//! The budget knobs are shared too: `--timeout` bounds each request and
//! `--max-frame-bytes` bounds what one answer may read into memory.

use super::mcp_client::{describe_error, over_cap_error, Rpc, ServerSpec, PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::time::Duration;

/// The header a server names its session in, and the client echoes back.
const SESSION_HEADER: &str = "mcp-session-id";

/// The header carrying the negotiated protocol revision after `initialize`.
const PROTOCOL_HEADER: &str = "MCP-Protocol-Version";

/// What a POST says it can read: the spec requires both, and a server may
/// answer either way regardless of what the first one suggests.
const ACCEPT: &str = "application/json, text/event-stream";

/// How long ending a session may take. It is a courtesy (the server expires the
/// session regardless), so it must not hold a command open.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Bytes of a server's error body quoted in a failed request's message.
const ERROR_BODY_CHARS: usize = 300;

/// One MCP session over Streamable HTTP.
pub struct HttpSession {
    client: reqwest::Client,
    /// The configured name, for error messages.
    name: String,
    url: String,
    /// Headers from the configuration (credentials, mostly), sent on every
    /// request.
    headers: Vec<(reqwest::header::HeaderName, reqwest::header::HeaderValue)>,
    /// `Mcp-Session-Id`, once the server has named one.
    session_id: Option<String>,
    /// The revision the server negotiated, sent as `MCP-Protocol-Version`.
    protocol_version: Option<String>,
    timeout: Duration,
    max_frame_bytes: usize,
    next_id: u64,
}

impl HttpSession {
    /// Open `name` (the configured endpoint) and complete the MCP handshake.
    pub async fn start(
        name: &str,
        spec: &ServerSpec,
        timeout: Duration,
        max_frame_bytes: usize,
    ) -> Result<Self, String> {
        let Some(url) = spec.url.clone() else {
            return Err(format!(
                "server '{name}' has no \"url\" — it is configured as a local process"
            ));
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!(
                "server '{name}': \"url\" must start with http:// or https://, got {url}"
            ));
        }
        let mut headers = Vec::new();
        for (key, value) in &spec.headers {
            let header =
                reqwest::header::HeaderName::from_bytes(key.as_bytes()).map_err(|error| {
                    format!("server '{name}': header name '{key}' is not valid: {error}")
                })?;
            let header_value = reqwest::header::HeaderValue::from_str(value).map_err(|error| {
                format!("server '{name}': the value of header '{key}' is not valid: {error}")
            })?;
            headers.push((header, header_value));
        }

        let mut session = Self {
            client: crate::dns::client(),
            name: name.to_string(),
            url,
            headers,
            session_id: None,
            protocol_version: None,
            timeout,
            max_frame_bytes,
            next_id: 1,
        };
        session.initialize().await?;
        Ok(session)
    }

    /// `initialize`, then the `notifications/initialized` ack.
    ///
    /// The session id comes back in a response header, which is why this is not
    /// the shared handshake the stdio session uses: `exchange` records it, and
    /// the revision the server answered with becomes the header every later
    /// request carries.
    pub async fn initialize(&mut self) -> Result<Value, String> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "future", "version": crate::version::VERSION },
        });
        let result = Rpc::request(self, "initialize", params).await?;
        self.protocol_version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .map(str::to_string);
        Rpc::notify(self, "notifications/initialized", json!({})).await?;
        Ok(result)
    }

    /// Every tool the endpoint advertises, following `nextCursor`.
    pub async fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        super::mcp_client::list_tools(self).await
    }

    /// One `tools/call` (the MCP `result` envelope).
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        super::mcp_client::call_tool(self, name, arguments).await
    }

    /// End the session, if the server named one.
    ///
    /// A server that does not implement `DELETE` answers 405 and the session
    /// expires on its own; nothing here is worth failing a command over, so the
    /// errors are dropped and the wait is short.
    pub async fn shutdown(self) {
        let Some(session_id) = self.session_id.clone() else {
            return;
        };
        let mut request = self
            .client
            .delete(&self.url)
            .header(SESSION_HEADER, session_id);
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(version) = self.protocol_version.as_deref() {
            request = request.header(PROTOCOL_HEADER, version);
        }
        let _ = tokio::time::timeout(SHUTDOWN_GRACE, request.send()).await;
    }

    /// POST one message and wait for its answer, if it has one.
    async fn exchange(
        &mut self,
        id: Option<u64>,
        method: &str,
        body: &Value,
    ) -> Result<Option<Value>, String> {
        let payload =
            serde_json::to_vec(body).map_err(|error| format!("encode MCP request: {error}"))?;
        let mut request = self
            .client
            .post(&self.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, ACCEPT)
            .body(payload);
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(session_id) = self.session_id.as_deref() {
            request = request.header(SESSION_HEADER, session_id);
        }
        if let Some(version) = self.protocol_version.as_deref() {
            request = request.header(PROTOCOL_HEADER, version);
        }

        let response = request.send().await.map_err(|error| {
            format!(
                "could not reach the MCP server '{}' ({}): {}",
                self.name,
                self.url,
                crate::dns::describe(&error)
            )
        })?;

        // The server may name its session on any response, and a rotating id is
        // reported the same way.
        if let Some(session_id) = response
            .headers()
            .get(SESSION_HEADER)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
        {
            self.session_id = Some(session_id.to_string());
        }

        let status = response.status();
        let Some(id) = id else {
            // A notification is acknowledged with 202 and no body. Nothing is
            // awaited, and the body is dropped rather than read: a server that
            // answers a notification with a stream would otherwise hold this
            // open.
            return if status.is_success() {
                Ok(None)
            } else {
                Err(self.status_error(response, status.as_u16(), method).await)
            };
        };

        if !status.is_success() {
            return Err(self.status_error(response, status.as_u16(), method).await);
        }

        let streaming = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.contains("text/event-stream"))
            .unwrap_or(false);
        if streaming {
            self.read_event_stream(response, id, method).await
        } else {
            read_json_answer(response, id, self.max_frame_bytes, method)
                .await
                .map(Some)
        }
    }

    /// The message for a response that is not a usable answer.
    async fn status_error(&self, response: reqwest::Response, status: u16, method: &str) -> String {
        let detail = read_body(response, self.max_frame_bytes, method)
            .await
            .unwrap_or_default();
        let detail = detail.trim();
        let mut message = format!(
            "the MCP server '{}' answered HTTP {status} to '{method}'",
            self.name
        );
        if !detail.is_empty() {
            message.push_str(": ");
            message.push_str(&shorten(detail, ERROR_BODY_CHARS));
        }
        message
    }

    /// Read one event stream until the answer to `id` arrives.
    ///
    /// Events that are not that answer — notifications, logging — are skipped;
    /// the stream is dropped as soon as the answer is in hand, so a server that
    /// keeps it open for further notifications is not waited on.
    async fn read_event_stream(
        &mut self,
        mut response: reqwest::Response,
        id: u64,
        method: &str,
    ) -> Result<Option<Value>, String> {
        let mut decoder = EventStreamDecoder::default();
        loop {
            let chunk = response
                .chunk()
                .await
                .map_err(|error| format!("read from the MCP server failed: {error}"))?;
            let Some(chunk) = chunk else {
                break;
            };
            for event in decoder.feed(&chunk, self.max_frame_bytes, method)? {
                if let Some(message) = answered(&event, id, method)? {
                    return Ok(Some(message));
                }
            }
        }
        // A stream that ends without the blank line that would have dispatched
        // its last event still carried that event: read it rather than calling
        // an answer that arrived "no answer".
        if let Some(event) = decoder.finish() {
            if let Some(message) = answered(&event, id, method)? {
                return Ok(Some(message));
            }
        }
        Err(format!(
            "the MCP server closed its response before answering '{method}'"
        ))
    }
}

impl Rpc for HttpSession {
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let timeout = self.timeout;
        let answer =
            match tokio::time::timeout(timeout, self.exchange(Some(id), method, &body)).await {
                Ok(answer) => answer?,
                Err(_) => {
                    return Err(format!(
                        "MCP '{method}' timed out after {}s (raise --timeout for a slow server)",
                        timeout.as_secs()
                    ))
                }
            };
        let Some(message) = answer else {
            return Err(format!(
                "the MCP server '{}' acknowledged '{method}' without answering it",
                self.name
            ));
        };
        if let Some(error) = message.get("error") {
            return Err(format!("MCP '{method}' failed: {}", describe_error(error)));
        }
        Ok(message.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let timeout = self.timeout;
        match tokio::time::timeout(timeout, self.exchange(None, method, &body)).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(format!(
                "MCP '{method}' timed out after {}s (raise --timeout for a slow server)",
                timeout.as_secs()
            )),
        }
    }
}

/// The JSON-RPC message an event's payload is, when it answers `id`.
///
/// `Ok(None)` means "not this answer" — a notification carries no id, and the
/// error arm is reserved for a payload that is not JSON at all.
fn answered(payload: &str, id: u64, method: &str) -> Result<Option<Value>, String> {
    let message: Value = serde_json::from_str(payload).map_err(|error| {
        format!("the MCP server sent an unreadable frame while answering '{method}': {error}")
    })?;
    if message.get("id").and_then(Value::as_u64) == Some(id) {
        Ok(Some(message))
    } else {
        Ok(None)
    }
}

/// Read a whole body as one JSON-RPC answer, within the read budget.
async fn read_json_answer(
    response: reqwest::Response,
    id: u64,
    limit: usize,
    method: &str,
) -> Result<Value, String> {
    let body = read_body(response, limit, method).await?;
    let message: Value = serde_json::from_str(&body).map_err(|error| {
        format!("the MCP server sent an unreadable frame while answering '{method}': {error}")
    })?;
    // A batch is legal; the answer to this request is the element carrying its
    // id, and anything else in the array is not ours to act on.
    if let Value::Array(items) = message {
        return items
            .into_iter()
            .find(|item| item.get("id").and_then(Value::as_u64) == Some(id))
            .ok_or_else(|| {
                format!("the MCP server sent no answer to '{method}' in its batch response")
            });
    }
    Ok(message)
}

/// Read a body into memory, refusing anything past `limit` bytes.
async fn read_body(
    mut response: reqwest::Response,
    limit: usize,
    method: &str,
) -> Result<String, String> {
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("read from the MCP server failed: {error}"))?
    {
        if body.len() + chunk.len() > limit {
            return Err(over_cap_error(limit, method));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Collapse a body to one quotable line.
fn shorten(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        return one_line;
    }
    let head: String = one_line.chars().take(max).collect();
    format!("{head}…")
}

/// An incremental `text/event-stream` decoder.
///
/// Only `data:` matters here: every other field (`event`, `id`, `retry`) and a
/// comment line are ignored, which is what the spec allows a client that only
/// wants the messages to do. A chunk boundary may fall anywhere — inside a line,
/// between the `data:` and its value, between the two newlines that end an event
/// — so state has to be carried between chunks.
#[derive(Default)]
struct EventStreamDecoder {
    /// The bytes of a line that has not reached its newline yet.
    partial: Vec<u8>,
    /// The `data` lines collected for the event being read.
    data: String,
}

impl EventStreamDecoder {
    /// Feed one chunk; return the payload of every event it completed.
    fn feed(&mut self, chunk: &[u8], limit: usize, method: &str) -> Result<Vec<String>, String> {
        let mut events = Vec::new();
        for byte in chunk {
            if *byte != b'\n' {
                self.partial.push(*byte);
                // A single line longer than the budget is already a payload no
                // answer could fit in; stop holding it.
                if self.partial.len() > limit {
                    return Err(over_cap_error(limit, method));
                }
                continue;
            }
            let mut line = std::mem::take(&mut self.partial);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.is_empty() {
                // A blank line ends the event.
                if !self.data.is_empty() {
                    events.push(std::mem::take(&mut self.data));
                }
                continue;
            }
            let text = String::from_utf8_lossy(&line);
            let Some(value) = text.strip_prefix("data:") else {
                continue;
            };
            let value = value.strip_prefix(' ').unwrap_or(value);
            if !self.data.is_empty() {
                self.data.push('\n');
            }
            self.data.push_str(value);
            if self.data.len() > limit {
                return Err(over_cap_error(limit, method));
            }
        }
        Ok(events)
    }

    /// The event left buffered when the stream ends without a blank line.
    fn finish(&mut self) -> Option<String> {
        if self.data.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.data))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_in_chunks(chunks: &[&str]) -> Vec<String> {
        let mut decoder = EventStreamDecoder::default();
        let mut events = Vec::new();
        for chunk in chunks {
            events.extend(
                decoder
                    .feed(chunk.as_bytes(), 1 << 20, "tools/call")
                    .unwrap(),
            );
        }
        events
    }

    #[test]
    fn an_event_stream_is_decoded_whatever_the_chunk_boundaries() {
        let whole = "event: message\ndata: {\"a\":1}\n\n";
        assert_eq!(feed_in_chunks(&[whole]), vec!["{\"a\":1}"]);

        // Split inside the field name, inside the value, and between the two
        // newlines that end the event: three places a naive splitter breaks.
        assert_eq!(
            feed_in_chunks(&["event: mess", "age\ndata: {\"a\"", ":1}\n", "\n"]),
            vec!["{\"a\":1}"]
        );

        // CRLF line endings, and a comment line, which carries nothing.
        assert_eq!(
            feed_in_chunks(&[": keep-alive\r\ndata: {\"a\":2}\r\n\r\n"]),
            vec!["{\"a\":2}"]
        );

        // Two events in one chunk, one in the next.
        assert_eq!(
            feed_in_chunks(&["data: one\n\ndata: two\n\n", "data: three\n\n"]),
            vec!["one", "two", "three"]
        );

        // Several data lines in one event are joined by newlines.
        assert_eq!(
            feed_in_chunks(&["data: first\ndata: second\n\n"]),
            vec!["first\nsecond"]
        );

        // No trailing blank line: nothing is dispatched by `feed`, because the
        // event is not finished yet — `finish` is what a stream's end reads.
        assert!(feed_in_chunks(&["data: dangling\n"]).is_empty());
        let mut decoder = EventStreamDecoder::default();
        decoder
            .feed(b"data: dangling\n", 1 << 20, "tools/call")
            .unwrap();
        assert_eq!(decoder.finish().as_deref(), Some("dangling"));
        assert_eq!(decoder.finish(), None, "the buffer is taken, not copied");
    }

    #[test]
    fn an_event_over_the_budget_is_refused() {
        let mut decoder = EventStreamDecoder::default();
        let error = decoder
            .feed(b"data: 0123456789\n\n", 4, "tools/call")
            .unwrap_err();
        assert!(error.contains("larger than 4 bytes"), "{error}");
        assert!(error.contains("--max-frame-bytes"), "{error}");

        // A line that never ends is refused as it grows, not only at dispatch.
        let mut decoder = EventStreamDecoder::default();
        let error = decoder
            .feed(b"data: 0123456789", 4, "tools/call")
            .unwrap_err();
        assert!(error.contains("larger than 4 bytes"), "{error}");
    }

    #[test]
    fn only_the_answer_with_our_id_is_returned() {
        let notification = json!({ "jsonrpc": "2.0", "method": "notifications/message" });
        assert!(answered(&notification.to_string(), 7, "tools/call")
            .unwrap()
            .is_none());

        let other = json!({ "jsonrpc": "2.0", "id": 6, "result": {} });
        assert!(answered(&other.to_string(), 7, "tools/call")
            .unwrap()
            .is_none());

        let ours = json!({ "jsonrpc": "2.0", "id": 7, "result": { "ok": true } });
        assert_eq!(
            answered(&ours.to_string(), 7, "tools/call").unwrap(),
            Some(ours)
        );

        let error = answered("not json", 7, "tools/call").unwrap_err();
        assert!(error.contains("unreadable frame"), "{error}");
        assert!(error.contains("answering 'tools/call'"), "{error}");
    }

    #[test]
    fn a_quoted_error_body_is_one_shortened_line() {
        assert_eq!(shorten("a\nb   c", 40), "a b c");
        assert_eq!(shorten("abcdef", 3), "abc…");
        assert_eq!(shorten("abc", 3), "abc");
    }

    /// A bare server name with no URL is refused by name, before any connection.
    #[tokio::test]
    async fn a_session_needs_an_http_url() {
        let mut spec = super::super::mcp_client::ServerSpec {
            command: None,
            args: Vec::new(),
            env: serde_json::Map::new(),
            cwd: None,
            url: Some("ftp://example.invalid/mcp".to_string()),
            headers: Default::default(),
            enabled: true,
        };
        let error = HttpSession::start("odd", &spec, Duration::from_secs(5), 1 << 20)
            .await
            .err()
            .expect("a non-HTTP URL is refused");
        assert!(error.contains("must start with http://"), "{error}");

        spec.url = None;
        let error = HttpSession::start("odd", &spec, Duration::from_secs(5), 1 << 20)
            .await
            .err()
            .expect("a server with no url is refused");
        assert!(error.contains("no \"url\""), "{error}");
    }

    /// A header that cannot be sent is a configuration error, and it is named.
    #[tokio::test]
    async fn a_malformed_header_is_refused_with_its_name() {
        let mut headers = std::collections::BTreeMap::new();
        headers.insert("bad header".to_string(), "value".to_string());
        let spec = super::super::mcp_client::ServerSpec {
            command: None,
            args: Vec::new(),
            env: serde_json::Map::new(),
            cwd: None,
            url: Some("http://127.0.0.1:1/mcp".to_string()),
            headers,
            enabled: true,
        };
        let error = HttpSession::start("odd", &spec, Duration::from_secs(5), 1 << 20)
            .await
            .err()
            .expect("an invalid header name is refused");
        assert!(error.contains("header name 'bad header'"), "{error}");

        let mut headers = std::collections::BTreeMap::new();
        headers.insert("x-key".to_string(), "bad\nvalue".to_string());
        let spec = super::super::mcp_client::ServerSpec {
            command: None,
            args: Vec::new(),
            env: serde_json::Map::new(),
            cwd: None,
            url: Some("http://127.0.0.1:1/mcp".to_string()),
            headers,
            enabled: true,
        };
        let error = HttpSession::start("odd", &spec, Duration::from_secs(5), 1 << 20)
            .await
            .err()
            .expect("an invalid header value is refused");
        assert!(error.contains("header 'x-key'"), "{error}");
    }

    // ── a whole session, against a real HTTP server ─────────────────────────

    /// The configuration for a server that is an endpoint, as `mcp_http` takes it.
    fn spec_for(url: &str) -> super::super::mcp_client::ServerSpec {
        super::super::mcp_client::ServerSpec {
            command: None,
            args: Vec::new(),
            env: serde_json::Map::new(),
            cwd: None,
            url: Some(url.to_string()),
            headers: Default::default(),
            enabled: true,
        }
    }

    fn answer(id: u64, result: Value) -> String {
        json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
    }

    /// The plain transport: one JSON body per POST, a 202 for the notification,
    /// and a tool call whose answer is the MCP envelope rather than the payload.
    #[tokio::test]
    async fn a_json_answered_session_handshakes_lists_and_calls() {
        let initialize = answer(
            1,
            json!({ "protocolVersion": PROTOCOL_VERSION, "capabilities": {} }),
        );
        let tools = answer(
            2,
            json!({ "tools": [{ "name": "echo" }, { "name": "other" }] }),
        );
        let called = answer(3, json!({ "content": [{ "type": "text", "text": "hi" }] }));
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sequence(
            "/mcp",
            vec![
                (200, initialize.as_str()),
                // A notification is acknowledged with no body at all.
                (202, ""),
                (200, tools.as_str()),
                (200, called.as_str()),
            ],
        )])
        .await;

        let mut session = HttpSession::start(
            "fake",
            &spec_for(&format!("{base}/mcp")),
            Duration::from_secs(5),
            1 << 20,
        )
        .await
        .expect("the handshake completes");
        let listed = session.list_tools().await.expect("tools/list answers");
        assert_eq!(listed.len(), 2, "{listed:?}");
        assert_eq!(listed[0]["name"], "echo");
        let result = session
            .call_tool("echo", json!({ "text": "hi" }))
            .await
            .expect("tools/call answers");
        assert_eq!(result["content"][0]["text"], "hi");
        // No session was named, so there is nothing to end: the drop is silent.
        session.shutdown().await;
    }

    /// The streaming transport: the session id arrives in a response header and
    /// every later request echoes it back.
    #[tokio::test]
    async fn an_sse_answered_session_keeps_the_header_the_server_named() {
        let initialize = format!(
            "data: {}\n\n",
            answer(1, json!({ "protocolVersion": PROTOCOL_VERSION }))
        );
        let called = format!(
            "data: {}\n\n",
            answer(
                2,
                json!({ "content": [{ "type": "text", "text": "streamed" }] })
            )
        );
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let base = crate::test_server::spawn_http_recording(
            vec![crate::test_server::HttpRoute::sse_sequence(
                "/mcp",
                vec![
                    (initialize.as_str(), Some("sess-7")),
                    ("", None),
                    (called.as_str(), None),
                ],
            )],
            Some(requests.clone()),
        )
        .await;

        let mut session = HttpSession::start(
            "fake",
            &spec_for(&format!("{base}/mcp")),
            Duration::from_secs(5),
            1 << 20,
        )
        .await
        .expect("the handshake completes over a stream");
        let result = session
            .call_tool("echo", json!({}))
            .await
            .expect("the streamed answer is read");
        assert_eq!(result["content"][0]["text"], "streamed");
        let posed = requests.lock().unwrap();
        assert!(
            posed
                .iter()
                .any(|text| text.to_ascii_lowercase().contains("mcp-session-id: sess-7")),
            "the named session is echoed on later requests: {posed:?}"
        );
        drop(posed);
        session.shutdown().await;
    }

    /// A stream that closes without ever carrying our answer is an error that
    /// says so, rather than an answer of `null`.
    #[tokio::test]
    async fn a_stream_that_never_carries_the_answer_says_so() {
        let notification = json!({ "jsonrpc": "2.0", "method": "notifications/message" });
        let body = format!("data: {notification}\n\n");
        let base =
            crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sse_sequence(
                "/mcp",
                vec![(body.as_str(), None)],
            )])
            .await;

        let error = HttpSession::start(
            "fake",
            &spec_for(&format!("{base}/mcp")),
            Duration::from_secs(5),
            1 << 20,
        )
        .await
        .err()
        .expect("a stream with only a notification answers nothing");
        assert!(
            error.contains("closed its response before answering 'initialize'"),
            "{error}"
        );
    }

    /// A stream that ends mid-event still carried that event: it is read, and
    /// since it is not the answer, the failure is the same one.
    #[tokio::test]
    async fn a_stream_that_ends_mid_event_reads_its_last_event() {
        let notification = json!({ "jsonrpc": "2.0", "method": "notifications/message" });
        // No trailing blank line: the event is finished only by the stream's end.
        let body = format!("data: {notification}");
        let base =
            crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sse_sequence(
                "/mcp",
                vec![(body.as_str(), None)],
            )])
            .await;

        let error = HttpSession::start(
            "fake",
            &spec_for(&format!("{base}/mcp")),
            Duration::from_secs(5),
            1 << 20,
        )
        .await
        .err()
        .expect("a dangling notification is not the answer");
        assert!(
            error.contains("closed its response before answering 'initialize'"),
            "{error}"
        );
    }

    /// The error object a server answers with is described, not printed raw.
    #[tokio::test]
    async fn an_error_answer_names_the_method_and_the_jsonrpc_error() {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32601, "message": "no such method" },
        })
        .to_string();
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/mcp", 200, &body,
        )])
        .await;

        let error = HttpSession::start(
            "fake",
            &spec_for(&format!("{base}/mcp")),
            Duration::from_secs(5),
            1 << 20,
        )
        .await
        .err()
        .expect("an error envelope is not a result");
        assert!(error.contains("MCP 'initialize' failed"), "{error}");
        assert!(error.contains("code=-32601"), "{error}");
        assert!(error.contains("no such method"), "{error}");
    }

    /// A body that is not JSON at all is an unreadable frame, and it names the
    /// method it was answering.
    #[tokio::test]
    async fn an_answer_that_is_not_json_is_an_unreadable_frame() {
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/mcp", 200, "not json",
        )])
        .await;

        let error = HttpSession::start(
            "fake",
            &spec_for(&format!("{base}/mcp")),
            Duration::from_secs(5),
            1 << 20,
        )
        .await
        .err()
        .expect("a non-JSON body answers nothing");
        assert!(error.contains("unreadable frame"), "{error}");
        assert!(error.contains("answering 'initialize'"), "{error}");
    }

    /// A batch is legal: the answer is the element carrying our id, and a batch
    /// without it is an error rather than a guess at which one was meant.
    #[tokio::test]
    async fn a_batch_answer_is_searched_for_our_id() {
        let batch = json!([
            { "jsonrpc": "2.0", "id": 99, "result": { "not": "ours" } },
            { "jsonrpc": "2.0", "id": 1, "result": { "protocolVersion": PROTOCOL_VERSION } },
        ])
        .to_string();
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/mcp", 200, &batch,
        )])
        .await;
        HttpSession::start(
            "fake",
            &spec_for(&format!("{base}/mcp")),
            Duration::from_secs(5),
            1 << 20,
        )
        .await
        .expect("the element with our id is the answer");

        let without = json!([{ "jsonrpc": "2.0", "id": 99, "result": {} }]).to_string();
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/mcp", 200, &without,
        )])
        .await;
        let error = HttpSession::start(
            "fake",
            &spec_for(&format!("{base}/mcp")),
            Duration::from_secs(5),
            1 << 20,
        )
        .await
        .err()
        .expect("a batch without our id answers nothing");
        assert!(
            error.contains("no answer to 'initialize' in its batch response"),
            "{error}"
        );
    }

    /// The read budget is the one the caller set, and the error says what to
    /// raise — the print cap cannot rescue a frame that will not fit in memory.
    #[tokio::test]
    async fn a_body_over_the_read_budget_is_refused() {
        let body = answer(
            1,
            json!({ "protocolVersion": PROTOCOL_VERSION, "padding": "x".repeat(64) }),
        );
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::json(
            "/mcp", 200, &body,
        )])
        .await;

        let error = HttpSession::start(
            "fake",
            &spec_for(&format!("{base}/mcp")),
            Duration::from_secs(5),
            8,
        )
        .await
        .err()
        .expect("an answer past the budget is refused");
        assert!(error.contains("8 bytes"), "{error}");
        assert!(error.contains("--max-frame-bytes"), "{error}");
    }

    /// A server that refuses the notification fails the handshake with its own
    /// status, because a session nobody acknowledged was not opened.
    #[tokio::test]
    async fn a_refused_notification_fails_the_handshake() {
        let initialize = answer(1, json!({ "protocolVersion": PROTOCOL_VERSION }));
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::sequence(
            "/mcp",
            vec![(200, initialize.as_str()), (500, "boom")],
        )])
        .await;

        let error = HttpSession::start(
            "fake",
            &spec_for(&format!("{base}/mcp")),
            Duration::from_secs(5),
            1 << 20,
        )
        .await
        .err()
        .expect("HTTP 500 to a notification is an error");
        assert!(
            error.contains("answered HTTP 500 to 'notifications/initialized'"),
            "{error}"
        );
        assert!(error.contains("boom"), "the body is quoted: {error}");
    }

    /// A request budget is per request: a server slower than it fails with the
    /// budget named, for a request and for a notification alike.
    #[tokio::test]
    async fn a_slow_server_is_reported_as_a_timeout_with_the_budget() {
        let base = crate::test_server::spawn_http(vec![crate::test_server::HttpRoute::slow(
            "/mcp",
            Duration::from_secs(2),
        )])
        .await;
        let error = HttpSession::start(
            "fake",
            &spec_for(&format!("{base}/mcp")),
            Duration::from_millis(100),
            1 << 20,
        )
        .await
        .err()
        .expect("a slow initialize is a timeout");
        assert!(error.contains("MCP 'initialize' timed out"), "{error}");
        assert!(error.contains("--timeout"), "{error}");

        // The same budget applies to the notification behind the handshake: the
        // answer is immediate, the acknowledgement is not.
        let initialize = answer(1, json!({ "protocolVersion": PROTOCOL_VERSION }));
        let mut route = crate::test_server::HttpRoute::sequence(
            "/mcp",
            vec![(200, initialize.as_str()), (202, "")],
        );
        route.responses[1].delay = Duration::from_secs(2);
        let base = crate::test_server::spawn_http(vec![route]).await;
        let error = HttpSession::start(
            "fake",
            &spec_for(&format!("{base}/mcp")),
            Duration::from_millis(100),
            1 << 20,
        )
        .await
        .err()
        .expect("a slow acknowledgement is a timeout too");
        assert!(
            error.contains("MCP 'notifications/initialized' timed out"),
            "{error}"
        );
    }

    /// A decoder that holds several data lines already reads the running total
    /// against the budget, so an event assembled from under-limit lines is still
    /// refused before it is handed on.
    #[test]
    fn an_event_assembled_from_small_lines_is_still_bounded() {
        let mut decoder = EventStreamDecoder::default();
        let error = decoder
            .feed(b"data: 1234\ndata: 1234\ndata: 1234\n\n", 10, "tools/call")
            .unwrap_err();
        assert!(error.contains("larger than 10 bytes"), "{error}");
    }

    /// Nothing listens there: the failure names the endpoint and the cause.
    #[tokio::test]
    async fn an_unreachable_endpoint_is_reported_with_its_url() {
        let spec = super::super::mcp_client::ServerSpec {
            command: None,
            args: Vec::new(),
            env: serde_json::Map::new(),
            cwd: None,
            url: Some("http://127.0.0.1:1/mcp".to_string()),
            headers: Default::default(),
            enabled: true,
        };
        let error = HttpSession::start("dead", &spec, Duration::from_secs(5), 1 << 20)
            .await
            .err()
            .expect("a closed port cannot complete a handshake");
        assert!(
            error.contains("could not reach the MCP server 'dead'"),
            "{error}"
        );
        assert!(error.contains("http://127.0.0.1:1/mcp"), "{error}");
    }
}

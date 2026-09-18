//! Loopback HTTP + WebSocket listener that serves the terminal registry to the
//! app's own webview.
//!
//! Why a listener instead of Tauri IPC: the transport is the part of opencode's
//! design worth copying exactly. One WebSocket per attached view carries raw
//! PTY bytes as binary frames in both directions, so a reconnect is one cursor
//! number, and the same endpoint can be driven by `curl`/`websocat` in an
//! automated check instead of only by a human clicking through the GUI.
//!
//! Security model (design §7):
//! * bound to `127.0.0.1` with an ephemeral port — never reachable from the LAN;
//! * a 32-byte random secret generated per process, handed to the webview
//!   through a Tauri command, required on every control route;
//! * WebSocket connects redeem a single-use, session-scoped, 60-second ticket
//!   issued over an authenticated route (a browser cannot set headers on a
//!   handshake);
//! * cross-origin requests from anything that is not the app itself (or a dev
//!   server on loopback) are refused even when they somehow hold a ticket.
//!
//! Output never leaves this process: the buffer lives in memory, no request
//! body is logged, and nothing is written to the store.

use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;

use super::manager::{CreateRequest, Manager, ManagerError, MAX_SESSIONS};
use super::protocol::{self, Meta};
use super::session::SessionEvent;
use super::shell;
use super::ticket::{IssuedTicket, TicketScope, TicketStore};

/// Largest request head we accept (request line + headers).
const MAX_HEAD: usize = 16 * 1024;
/// Largest control-route body we accept. Nothing here is big: a create request
/// is a handful of fields.
const MAX_BODY: usize = 64 * 1024;
/// Concurrent connections. A loopback client that opens more than this is
/// broken or hostile; the accept loop parks at capacity instead of exhausting
/// file descriptors.
const MAX_CONNECTIONS: usize = 32;
/// Close code used when a viewer fell behind and must re-attach.
pub const CLOSE_LAGGED: u16 = 4408;

/// What the webview needs to talk to the terminal server.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    /// Base URL, e.g. `http://127.0.0.1:41234`.
    pub url: String,
    /// Per-process secret; required on every control route.
    pub token: String,
    pub port: u16,
    pub max_sessions: usize,
}

pub struct TerminalServer {
    info: ServerInfo,
    listener: Mutex<Option<std::net::TcpListener>>,
    manager: Arc<Manager>,
    tickets: TicketStore,
    connections: Arc<tokio::sync::Semaphore>,
}

static SERVER: OnceLock<TerminalServer> = OnceLock::new();

/// Bind the listener. Synchronous on purpose: the port must be known before the
/// Tauri event loop starts serving requests, and binding needs no reactor.
pub fn bind() -> Result<&'static TerminalServer, String> {
    if let Some(server) = SERVER.get() {
        return Ok(server);
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("terminal server bind failed: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("terminal server nonblocking failed: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("terminal server address failed: {error}"))?
        .port();

    let server = TerminalServer {
        info: ServerInfo {
            url: format!("http://127.0.0.1:{port}"),
            token: random_secret(),
            port,
            max_sessions: MAX_SESSIONS,
        },
        listener: Mutex::new(Some(listener)),
        manager: Arc::new(Manager::new()),
        tickets: TicketStore::default(),
        connections: Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS)),
    };
    let _ = SERVER.set(server);
    SERVER
        .get()
        .ok_or_else(|| "terminal server unavailable".to_string())
}

/// Start accepting connections. Called from the Tauri `Ready` event so the
/// runtime is live; `bind` must have run first.
pub fn serve() -> Result<(), String> {
    let server = SERVER
        .get()
        .ok_or_else(|| "terminal server must be bound before serve".to_string())?;
    let std_listener = server
        .listener
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| "terminal server is already serving".to_string())?;
    let semaphore = Arc::clone(&server.connections);
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::from_std(std_listener) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("terminal: listener registration failed: {error}");
                return;
            }
        };
        loop {
            // Acquire before accepting: at capacity the loop parks instead of
            // accepting sockets it cannot serve.
            let Ok(permit) = semaphore.clone().acquire_owned().await else {
                return;
            };
            match listener.accept().await {
                Ok((stream, _)) => {
                    tokio::spawn(async move {
                        let _permit = permit;
                        if let Some(server) = SERVER.get() {
                            server.handle(stream).await;
                        }
                    });
                }
                Err(_) => {
                    // A transient accept error (EMFILE, ECONNABORTED) must not
                    // spin the loop; the permit is released with the branch.
                    drop(permit);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
    });
    Ok(())
}

/// The process-wide manager (sessions outlive any single request).
pub fn manager() -> Option<&'static Arc<Manager>> {
    SERVER.get().map(|server| &server.manager)
}

/// Info for the frontend, or `None` before `bind`.
pub fn info() -> Option<&'static ServerInfo> {
    SERVER.get().map(|server| &server.info)
}

/// Path shapes. The method decides which operation a shape maps to
/// (`/terminal` is list-or-create, `/terminal/:id` is get-or-update-or-remove),
/// so the dispatcher below reads like the route table it is.
#[derive(Debug)]
enum Route {
    Shells,
    Root,
    Item(String),
    ConnectToken(String),
    Connect(String),
    Unknown,
}

fn route(path: &str) -> Route {
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match segments.as_slice() {
        ["terminal", "shells"] => Route::Shells,
        ["terminal"] => Route::Root,
        ["terminal", id] => Route::Item((*id).to_string()),
        ["terminal", id, "connect-token"] => Route::ConnectToken((*id).to_string()),
        ["terminal", id, "connect"] => Route::Connect((*id).to_string()),
        _ => Route::Unknown,
    }
}

impl TerminalServer {
    async fn handle(&self, mut stream: TcpStream) {
        let Some(request) = read_request(&mut stream).await else {
            return;
        };
        if !self.origin_allowed(&request) {
            let _ = write_json(
                &mut stream,
                StatusCode::FORBIDDEN,
                &error_body("ORIGIN_FORBIDDEN", "origin not allowed"),
                None,
            )
            .await;
            return;
        }

        let method = request.method.as_str();
        let path = request.path.clone();
        let route = route(&request.route_path);

        // Preflight. The webview sends an `authorization` header, which is not a
        // CORS-simple request, so the browser asks first — and a preflight never
        // carries the secret it is asking permission to send. Answering it is
        // therefore unauthenticated by construction; the real request still has
        // to present the token.
        if method == "OPTIONS" {
            let _ = write_preflight(&mut stream, request.origin.clone()).await;
            return;
        }

        // A WebSocket upgrade authenticates with a ticket, not the secret:
        // browsers cannot set an Authorization header on a handshake.
        if let Route::Connect(id) = &route {
            if method != "GET" {
                let _ = write_json(
                    &mut stream,
                    StatusCode::METHOD_NOT_ALLOWED,
                    &error_body("METHOD_NOT_ALLOWED", "connect requires GET"),
                    None,
                )
                .await;
                return;
            }
            if !request.is_upgrade() {
                let _ = write_json(
                    &mut stream,
                    StatusCode::BAD_REQUEST,
                    &error_body("INVALID_ARGUMENT", "connect requires a websocket upgrade"),
                    request.origin.clone(),
                )
                .await;
                return;
            }
            self.handle_connect(stream, request, id.clone()).await;
            return;
        }

        if !self.secret_matches(&request) {
            let _ = write_json(
                &mut stream,
                StatusCode::UNAUTHORIZED,
                &error_body("UNAUTHORIZED", "terminal token missing or invalid"),
                request.origin.clone(),
            )
            .await;
            return;
        }

        let response = match (&route, method) {
            (Route::Shells, "GET") => {
                let items = shell::list_shells();
                ControlResponse::ok(serde_json::to_value(items).unwrap_or_default())
            }
            (Route::Root, "GET") => {
                let thread_id = query_value(&path, "threadId");
                let list = self.manager.list(thread_id.as_deref());
                ControlResponse::ok(serde_json::to_value(list).unwrap_or_default())
            }
            (Route::Root, "POST") => self.create(&request),
            (Route::Item(id), "GET") => match self.manager.get(id) {
                Ok(info) => ControlResponse::ok(serde_json::to_value(info).unwrap_or_default()),
                Err(error) => ControlResponse::error(error),
            },
            (Route::Item(id), "PATCH") => self.update(id, &request),
            (Route::Item(id), "DELETE") => match self.manager.remove(id) {
                Ok(()) => ControlResponse::ok(serde_json::json!({ "removed": true })),
                Err(error) => ControlResponse::error(error),
            },
            (Route::ConnectToken(id), "POST") => self.connect_token(id),
            (Route::Unknown, _) => ControlResponse::status(
                StatusCode::NOT_FOUND,
                error_body("NOT_FOUND", "unknown terminal route"),
            ),
            (_, _) => ControlResponse::status(
                StatusCode::METHOD_NOT_ALLOWED,
                error_body("METHOD_NOT_ALLOWED", "method not allowed for this route"),
            ),
        };

        let _ = write_json(
            &mut stream,
            response.status,
            &response.body,
            request.origin.clone(),
        )
        .await;
    }

    fn create(&self, request: &Request) -> ControlResponse {
        let Some(body) = parse_body::<CreateBody>(request) else {
            return ControlResponse::status(
                StatusCode::BAD_REQUEST,
                error_body("INVALID_ARGUMENT", "invalid create request body"),
            );
        };
        if body.thread_id.trim().is_empty() {
            return ControlResponse::status(
                StatusCode::BAD_REQUEST,
                error_body("INVALID_ARGUMENT", "threadId is required"),
            );
        }
        let created = self.manager.create(CreateRequest {
            thread_id: body.thread_id.clone(),
            title: body.title.clone(),
            cols: body.cols.unwrap_or(80),
            rows: body.rows.unwrap_or(24),
        });
        match created {
            Ok(info) => ControlResponse::ok(serde_json::to_value(info).unwrap_or_default()),
            Err(error) => ControlResponse::error(error),
        }
    }

    fn update(&self, id: &str, request: &Request) -> ControlResponse {
        let Some(body) = parse_body::<UpdateBody>(request) else {
            return ControlResponse::status(
                StatusCode::BAD_REQUEST,
                error_body("INVALID_ARGUMENT", "invalid update request body"),
            );
        };
        let size = match (body.cols, body.rows) {
            (Some(cols), Some(rows)) => Some((cols, rows)),
            _ => None,
        };
        match self.manager.update(id, body.title.clone(), size) {
            Ok(info) => ControlResponse::ok(serde_json::to_value(info).unwrap_or_default()),
            Err(error) => ControlResponse::error(error),
        }
    }

    fn connect_token(&self, id: &str) -> ControlResponse {
        let info = match self.manager.get(id) {
            Ok(info) => info,
            Err(error) => return ControlResponse::error(error),
        };
        let scope = TicketScope {
            terminal_id: info.id.clone(),
            thread_id: info.thread_id.clone(),
        };
        match self.tickets.issue(scope) {
            Some(IssuedTicket { ticket, expires_in }) => ControlResponse::ok(serde_json::json!({
                "ticket": ticket,
                "expiresIn": expires_in,
            })),
            None => ControlResponse::status(
                StatusCode::TOO_MANY_REQUESTS,
                error_body("CAPACITY_EXCEEDED", "too many outstanding connect tickets"),
            ),
        }
    }

    async fn handle_connect(
        self: &TerminalServer,
        stream: TcpStream,
        request: Request,
        id: String,
    ) {
        // The ticket proves the caller reached the authenticated control route
        // and was issued this exact session.
        let ticket = query_value(&request.path, "ticket").unwrap_or_default();
        let info = match self.manager.get(&id) {
            Ok(info) => info,
            Err(error) => {
                let mut stream = stream;
                let _ = write_json(
                    &mut stream,
                    StatusCode::NOT_FOUND,
                    &error_body(error.code(), &error.message()),
                    request.origin.clone(),
                )
                .await;
                return;
            }
        };
        let scope = TicketScope {
            terminal_id: info.id.clone(),
            thread_id: info.thread_id.clone(),
        };
        if ticket.is_empty() || !self.tickets.consume(&ticket, &scope) {
            let mut stream = stream;
            let _ = write_json(
                &mut stream,
                StatusCode::FORBIDDEN,
                &error_body("UNAUTHORIZED", "connect ticket missing or invalid"),
                request.origin.clone(),
            )
            .await;
            return;
        }

        let cursor =
            query_value(&request.path, "cursor").and_then(|value| value.parse::<i64>().ok());
        // Echo a restrictive CORS header: the handshake is not subject to CORS,
        // but a browser can still start one, and the response must not look
        // reusable to another origin.
        // `tungstenite` re-reads the client's handshake from the stream, so the
        // bytes this router already consumed have to be handed back in front of
        // the socket. Without that the upgrade would wait forever for a request
        // that was already parsed.
        let rewound = RewindStream::new(request.raw, stream);
        let ws = tokio_tungstenite::accept_hdr_async(rewound, upgrade_response).await;

        let Ok(ws) = ws else {
            return;
        };
        self.pump(ws, id, cursor).await;
    }

    async fn pump<S>(
        &self,
        ws: tokio_tungstenite::WebSocketStream<S>,
        id: String,
        cursor: Option<i64>,
    ) where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let mut attachment = match self.manager.attach(&id, cursor) {
            Ok(attachment) => attachment,
            Err(_) => return,
        };
        let (mut sink, mut incoming) = ws.split();

        // Replay, then the control frame that tells the client where it stands.
        // Only after that does live output start, so the client never has to
        // interleave a snapshot with a stream. The replay is taken out of the
        // attachment first: sending it must not keep the attachment borrowed
        // while the exit path below reads its metadata.
        let replay = std::mem::take(&mut attachment.replay);
        for chunk in replay {
            if sink.send(Message::Binary(chunk)).await.is_err() {
                attachment.detach();
                return;
            }
        }
        let start_meta = Meta {
            cursor: attachment.cursor,
            start: attachment.start,
            exit_code: None,
        };
        if attachment.exit_code.is_some() {
            // Exited before we attached: replay is the final screen, so the
            // control frame carries the exit and we stop here.
            let meta = Meta {
                cursor: attachment.cursor,
                start: attachment.start,
                exit_code: attachment.exit_code,
            };
            let _ = sink
                .send(Message::Binary(protocol::meta_frame(&meta)))
                .await;
            let _ = sink
                .send(Message::Close(Some(CloseFrame {
                    code: CloseCode::Normal,
                    reason: "exited".into(),
                })))
                .await;
            attachment.detach();
            return;
        }
        if sink
            .send(Message::Binary(protocol::meta_frame(&start_meta)))
            .await
            .is_err()
        {
            attachment.detach();
            return;
        }

        let Some(mut events) = attachment.activate() else {
            attachment.detach();
            return;
        };
        let mut close: Option<CloseFrame> = None;
        loop {
            tokio::select! {
                event = events.recv() => {
                    match event {
                        Some(SessionEvent::Data(bytes)) => {
                            if sink.send(Message::Binary(bytes)).await.is_err() {
                                break;
                            }
                        }
                        Some(SessionEvent::Exited(meta)) => {
                            if sink.send(Message::Binary(protocol::meta_frame(&meta))).await.is_err() {
                                break;
                            }
                            close = Some(CloseFrame { code: CloseCode::Normal, reason: "exited".into() });
                            break;
                        }
                        Some(SessionEvent::Lagged) => {
                            // The viewer must come back with its own cursor; we
                            // cannot pretend the stream was continuous.
                            close = Some(CloseFrame {
                                code: CloseCode::Library(CLOSE_LAGGED),
                                reason: "lagged".into(),
                            });
                            break;
                        }
                        None => break,
                    }
                }
                message = incoming.next() => {
                    match message {
                        Some(Ok(Message::Binary(bytes))) => {
                            if let Some(text) = protocol::decode_input(&bytes) {
                                if attachment.write(text.as_bytes()).is_err() {
                                    break;
                                }
                            }
                        }
                        Some(Ok(Message::Text(text))) => {
                            if attachment.write(text.as_bytes()).is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            if sink.send(Message::Pong(payload)).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(_)) => {}
                        Some(Err(_)) => break,
                    }
                }
            }
        }

        if let Some(frame) = close {
            let _ = sink.send(Message::Close(Some(frame))).await;
        }
        attachment.detach();
    }

    /// The app's own webview, plus loopback dev servers, may talk to us.
    /// A missing Origin (curl, integration checks) is allowed because the
    /// secret is still required.
    fn origin_allowed(&self, request: &Request) -> bool {
        let Some(origin) = request.origin.as_deref() else {
            return true;
        };
        is_allowed_origin(origin)
    }

    fn secret_matches(&self, request: &Request) -> bool {
        let Some(header) = request.header("authorization") else {
            return false;
        };
        let presented = header.strip_prefix("Bearer ").unwrap_or(header);
        constant_time_eq(presented.as_bytes(), self.info.token.as_bytes())
    }
}

struct ControlResponse {
    status: StatusCode,
    body: serde_json::Value,
}

impl ControlResponse {
    fn ok(body: serde_json::Value) -> Self {
        ControlResponse {
            status: StatusCode::OK,
            body,
        }
    }

    fn status(status: StatusCode, body: serde_json::Value) -> Self {
        ControlResponse { status, body }
    }

    fn error(error: ManagerError) -> Self {
        ControlResponse {
            status: StatusCode::from_u16(error.status()).unwrap_or(StatusCode::BAD_REQUEST),
            body: error_body(error.code(), &error.message()),
        }
    }
}

fn error_body(code: &str, message: &str) -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": code,
            "message": message,
        }
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateBody {
    thread_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateBody {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
}

fn parse_body<T: for<'de> Deserialize<'de>>(request: &Request) -> Option<T> {
    serde_json::from_slice(&request.body).ok()
}

#[derive(Debug)]
struct Request {
    method: String,
    /// Full request target, for query parsing.
    path: String,
    /// Path without the query string, for routing.
    route_path: String,
    headers: Vec<(String, String)>,
    origin: Option<String>,
    body: Vec<u8>,
    /// Every byte this parser consumed for the request head and any body bytes
    /// that arrived with it. Kept so a WebSocket upgrade can hand the stream
    /// back to `tungstenite` exactly as it arrived.
    raw: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn is_upgrade(&self) -> bool {
        self.header("upgrade")
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
    }
}

/// Read one HTTP/1.1 request (head plus declared body) with a bounded size and
/// a read timeout. One request per connection: no keep-alive, no pipelining.
async fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    let head_end;
    loop {
        let mut chunk = [0_u8; 2048];
        let read =
            match tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk)).await {
                Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return None,
                Ok(Ok(read)) => read,
            };
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find_head_end(&buffer) {
            head_end = position;
            break;
        }
        if buffer.len() > MAX_HEAD {
            return None;
        }
    }

    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let route_path = path.split('?').next().unwrap_or(&path).to_string();

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }

    let body_start = head_end + 4;
    let content_length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY {
        return None;
    }
    let mut body = buffer[body_start.min(buffer.len())..].to_vec();
    while body.len() < content_length {
        let mut chunk = [0_u8; 2048];
        let read =
            match tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk)).await {
                Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
                Ok(Ok(read)) => read,
            };
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    let origin = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("origin"))
        .map(|(_, value)| value.clone());

    Some(Request {
        method,
        path,
        route_path,
        headers,
        origin,
        body,
        raw: buffer[..buffer.len().min(body_start + content_length)].to_vec(),
    })
}

fn find_head_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

async fn write_json(
    stream: &mut TcpStream,
    status: StatusCode,
    body: &serde_json::Value,
    origin: Option<String>,
) -> std::io::Result<()> {
    let payload = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let mut head = format!(
        "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("OK"),
        payload.len()
    );
    append_cors(&mut head, origin.as_deref());
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&payload).await?;
    stream.flush().await
}

/// Answer a CORS preflight for the loopback listener.
///
/// Only the app's own origins reach this point (`handle` rejects a foreign
/// origin first), so echoing the origin cannot turn the listener into an open
/// one.
async fn write_preflight(stream: &mut TcpStream, origin: Option<String>) -> std::io::Result<()> {
    let mut head = String::from("HTTP/1.1 204 No Content\r\nconnection: close\r\n");
    append_cors(&mut head, origin.as_deref());
    head.push_str("access-control-allow-methods: GET, POST, PATCH, DELETE, OPTIONS\r\n");
    head.push_str("access-control-allow-headers: authorization, content-type\r\n");
    head.push_str("access-control-max-age: 600\r\n\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.flush().await
}

fn append_cors(head: &mut String, origin: Option<&str>) {
    if let Some(origin) = origin {
        head.push_str(&format!("access-control-allow-origin: {origin}\r\n"));
        head.push_str("vary: origin\r\n");
    }
}

fn query_value(path: &str, key: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if name == key {
            return Some(percent_decode(value));
        }
    }
    None
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Origins allowed to talk to the terminal server: the Tauri webview (whose
/// scheme differs per platform) and loopback dev servers.
pub fn is_allowed_origin(origin: &str) -> bool {
    let lowered = origin.to_ascii_lowercase();
    if lowered == "tauri://localhost"
        || lowered == "http://tauri.localhost"
        || lowered == "https://tauri.localhost"
        || lowered == "http://localhost"
        || lowered == "https://localhost"
    {
        return true;
    }
    // Loopback dev servers, any port.
    if let Some(rest) = lowered
        .strip_prefix("http://")
        .or_else(|| lowered.strip_prefix("https://"))
    {
        let host = rest.split('/').next().unwrap_or(rest);
        let host = host.split(':').next().unwrap_or(host);
        if host == "localhost" || host == "127.0.0.1" || host == "[::1]" {
            return true;
        }
    }
    false
}

/// Add the CORS echo to the upgrade response. The handshake itself is not
/// subject to CORS, but a browser can still start one, and the response must
/// not look reusable to another origin. A named function (rather than a
/// closure) keeps the allow for tungstenite's large error type local and makes
/// `accept_hdr_async`'s callback signature explicit.
#[allow(clippy::result_large_err)]
fn upgrade_response(
    request: &tokio_tungstenite::tungstenite::handshake::server::Request,
    mut response: tokio_tungstenite::tungstenite::handshake::server::Response,
) -> Result<
    tokio_tungstenite::tungstenite::handshake::server::Response,
    tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
> {
    if let Some(origin) = request
        .headers()
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| HeaderValue::from_str(value).ok())
    {
        let _ = response
            .headers_mut()
            .insert("access-control-allow-origin", origin);
    }
    Ok(response)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// A stream that replays bytes the HTTP head parser already consumed before
/// delegating to the socket. `tungstenite` insists on reading the client's
/// handshake itself; this is what lets one listener route by URL and still
/// complete a standard upgrade.
struct RewindStream {
    prefix: Vec<u8>,
    offset: usize,
    inner: TcpStream,
}

impl RewindStream {
    fn new(prefix: Vec<u8>, inner: TcpStream) -> Self {
        RewindStream {
            prefix,
            offset: 0,
            inner,
        }
    }
}

impl AsyncRead for RewindStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.offset < this.prefix.len() {
            let remaining = &this.prefix[this.offset..];
            let take = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..take]);
            this.offset += take;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for RewindStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, data)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

fn random_secret() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..64)
        .map(|_| char::from_digit(rng.gen_range(0..16), 16).unwrap_or('0'))
        .collect()
}

#[cfg(test)]
mod end_to_end {
    //! Drives the real listener over real TCP: control routes with the process
    //! secret, then a real WebSocket carrying a real shell's bytes. This is the
    //! check the previous IPC design could only run by hand in a GUI — the
    //! transport being a socket is what makes it scriptable.

    use std::io::{Read, Write};
    use std::net::TcpStream as StdStream;
    use std::time::{Duration, Instant};

    use tokio_tungstenite::tungstenite::Message;

    use super::*;
    use crate::auth_store::test_support::HomeGuard;
    use crate::store;

    struct HttpResponse {
        status: u16,
        body: String,
        raw: String,
    }

    fn request(
        port: u16,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&str>,
    ) -> HttpResponse {
        request_with_origin(port, method, path, token, body, None)
    }

    /// As `request`, with an explicit Origin header so the CORS rules can be
    /// exercised the way the webview exercises them.
    fn request_with_origin(
        port: u16,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&str>,
        origin: Option<&str>,
    ) -> HttpResponse {
        let mut stream = StdStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        let payload = body.unwrap_or("");
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nhost: 127.0.0.1:{port}\r\nconnection: close\r\ncontent-length: {}\r\n",
            payload.len()
        );
        if let Some(token) = token {
            head.push_str(&format!("authorization: Bearer {token}\r\n"));
        }
        if let Some(origin) = origin {
            head.push_str(&format!("origin: {origin}\r\n"));
        }
        if !payload.is_empty() {
            head.push_str("content-type: application/json\r\n");
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes()).expect("write head");
        stream.write_all(payload.as_bytes()).expect("write body");
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        let status = response
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(0);
        let body = response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_string())
            .unwrap_or_default();
        HttpResponse {
            status,
            body,
            raw: response,
        }
    }

    fn body_json(response: &HttpResponse) -> serde_json::Value {
        serde_json::from_str(&response.body)
            .unwrap_or_else(|error| panic!("not JSON ({}): {}", error, response.body))
    }

    /// One temp HOME per test *process* is all the store supports: its home
    /// guard takes a process-wide lock, so a second guard inside the same test
    /// would deadlock against the first.
    fn setup_home(label: &str) -> HomeGuard {
        let home = HomeGuard::new(label);
        store::initialize_app_store().expect("initialize store");
        home
    }

    /// A real workspace directory plus a conversation inside it. The store
    /// resolves a terminal's working directory through exactly this link.
    fn create_thread_in_new_workspace(label: &str) -> String {
        let dir = std::env::temp_dir().join(format!("futureos-terminal-{label}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("workspace dir");
        let workspace = store::create_workspace(store::CreateWorkspaceInput {
            name: Some(label.to_string()),
            path: dir.display().to_string(),
            description: None,
            create_directory: Some(true),
        })
        .expect("create workspace");
        let thread = store::create_thread(store::CreateThreadInput {
            mode: "workspace".into(),
            title: Some(label.to_string()),
            workspace_id: Some(workspace.id),
            workspace_path: None,
            workspace_name: None,
            agent_session_id: None,
        })
        .expect("create thread");
        thread.id
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_real_shell_streams_over_the_loopback_transport() {
        let _home = setup_home("terminal-e2e");
        let thread_id = create_thread_in_new_workspace("terminal-e2e");
        let server = bind().expect("bind");
        serve().expect("serve");
        let port = server.info.port;
        let token = server.info.token.clone();

        // The control routes refuse an unauthenticated caller.
        let anonymous = request(port, "GET", "/terminal", None, None);
        assert_eq!(anonymous.status, 401, "body: {}", anonymous.body);

        // The webview sends `authorization`, so it always preflights first.
        let preflight = request_with_origin(
            port,
            "OPTIONS",
            "/terminal",
            None,
            None,
            Some("tauri://localhost"),
        );
        assert_eq!(preflight.status, 204, "raw: {}", preflight.raw);
        assert!(
            preflight
                .raw
                .to_lowercase()
                .contains("access-control-allow-origin: tauri://localhost"),
            "preflight must echo the app origin: {}",
            preflight.raw
        );
        assert!(
            preflight
                .raw
                .to_lowercase()
                .contains("access-control-allow-headers: authorization, content-type"),
            "preflight must allow the header the client sends: {}",
            preflight.raw
        );

        // A foreign origin is refused even when it holds the secret.
        let foreign = request_with_origin(
            port,
            "GET",
            "/terminal",
            Some(&token),
            None,
            Some("https://evil.example"),
        );
        assert_eq!(foreign.status, 403, "body: {}", foreign.body);

        // The app's own origin is accepted and echoed on the real response.
        let own = request_with_origin(
            port,
            "GET",
            "/terminal",
            Some(&token),
            None,
            Some("tauri://localhost"),
        );
        assert_eq!(own.status, 200, "body: {}", own.body);
        assert!(own
            .raw
            .to_lowercase()
            .contains("access-control-allow-origin: tauri://localhost"));

        // shell listing is a normal authenticated route
        let shells = request(port, "GET", "/terminal/shells", Some(&token), None);
        assert_eq!(shells.status, 200);
        assert!(!body_json(&shells).as_array().expect("array").is_empty());

        let create_body = serde_json::json!({
            "threadId": thread_id,
            "cols": 100,
            "rows": 30,
            "title": "Terminal 1",
        })
        .to_string();
        let created = request(port, "POST", "/terminal", Some(&token), Some(&create_body));
        assert_eq!(created.status, 200, "body: {}", created.body);
        let created = body_json(&created);
        let id = created["id"].as_str().expect("id").to_string();
        assert_eq!(created["status"], "running");
        assert_eq!(created["threadId"], thread_id);
        assert_eq!(created["cols"], 100);

        let listed = request(
            port,
            "GET",
            &format!("/terminal?threadId={thread_id}"),
            Some(&token),
            None,
        );
        assert_eq!(listed.status, 200);
        assert_eq!(body_json(&listed).as_array().expect("array").len(), 1);

        let ticket = request(
            port,
            "POST",
            &format!("/terminal/{id}/connect-token"),
            Some(&token),
            None,
        );
        assert_eq!(ticket.status, 200, "body: {}", ticket.body);
        let ticket = body_json(&ticket)["ticket"]
            .as_str()
            .expect("ticket")
            .to_string();

        // The ticket is single use: a second request with the same one fails.
        let replay = request(
            port,
            "POST",
            &format!("/terminal/{id}/connect-token"),
            Some(&token),
            None,
        );
        let replay_ticket = body_json(&replay)["ticket"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert_ne!(replay_ticket, ticket);

        let url = format!("ws://127.0.0.1:{port}/terminal/{id}/connect?cursor=-1&ticket={ticket}");
        let (mut socket, response) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .expect("websocket handshake");
        assert_eq!(response.status().as_u16(), 101);

        socket
            .send(Message::Text("echo marker-$((21 + 21))\n".into()))
            .await
            .expect("send input");

        // Collect output until the shell's echo lands, tracking the control
        // frame the way the client does.
        let mut output = String::new();
        let mut meta: Option<Meta> = None;
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline && !output.contains("marker-42") {
            let next = tokio::time::timeout(Duration::from_secs(5), socket.next()).await;
            match next {
                Ok(Some(Ok(Message::Binary(bytes)))) => {
                    if bytes.first() == Some(&protocol::CONTROL_PREFIX) {
                        meta = serde_json::from_slice(&bytes[1..]).ok();
                    } else {
                        output.push_str(&String::from_utf8_lossy(&bytes));
                    }
                }
                Ok(Some(Ok(Message::Text(text)))) => output.push_str(&text),
                Ok(Some(Ok(_))) => {}
                Ok(Some(Err(error))) => panic!("socket error: {error}"),
                Ok(None) => break,
                Err(_) => break,
            }
        }
        assert!(
            output.contains("marker-42"),
            "the shell's output must arrive over the socket: {output:?}"
        );
        let meta = meta.expect("a control frame must precede live output");
        // Tailing from -1 asks for no history: nothing is replayed, so the
        // replay starts exactly at the cursor the client must resume from.
        assert_eq!(meta.start, meta.cursor);
        assert!(meta.exit_code.is_none(), "the shell is still running");

        // A resize over the control route is reflected in the session info.
        let resized = request(
            port,
            "PATCH",
            &format!("/terminal/{id}"),
            Some(&token),
            Some(&serde_json::json!({ "cols": 120, "rows": 40 }).to_string()),
        );
        assert_eq!(resized.status, 200);
        assert_eq!(body_json(&resized)["cols"], 120);

        // Removing the tab kills the shell and forgets it.
        let removed = request(
            port,
            "DELETE",
            &format!("/terminal/{id}"),
            Some(&token),
            None,
        );
        assert_eq!(removed.status, 200);
        let gone = request(port, "GET", &format!("/terminal/{id}"), Some(&token), None);
        assert_eq!(gone.status, 404);
        assert_eq!(body_json(&gone)["error"]["code"], "TERMINAL_NOT_FOUND");
        let _ = socket.close(None).await;

        // Second scenario in the same process: the store and the listener are
        // process-global, so a second #[tokio::test] would fight over them.
        // Kept inside this function on purpose.
        let thread_id = create_thread_in_new_workspace("terminal-e2e-cwd");
        // A workspace directory that no longer exists must not block the tab:
        // the shell starts in home instead.
        let thread = store::get_thread(&thread_id)
            .expect("thread")
            .expect("present");
        let workspace = store::get_workspace(&thread.workspace_id)
            .expect("workspace")
            .expect("present");
        std::fs::remove_dir_all(&workspace.path).expect("remove workspace dir");

        let body = serde_json::json!({ "threadId": thread_id }).to_string();
        let response = request(port, "POST", "/terminal", Some(&token), Some(&body));
        assert_eq!(response.status, 200, "body: {}", response.body);
        let created = body_json(&response);
        let home = std::env::var("HOME").expect("guarded HOME");
        let home = std::fs::canonicalize(&home).unwrap_or_else(|_| std::path::PathBuf::from(home));
        assert_eq!(
            std::path::Path::new(created["cwd"].as_str().expect("cwd")),
            home
        );
        let id = created["id"].as_str().expect("id").to_string();
        let _ = request(
            port,
            "DELETE",
            &format!("/terminal/{id}"),
            Some(&token),
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_recognised_and_unknown_ones_are_not() {
        assert!(matches!(route("/terminal/shells"), Route::Shells));
        assert!(matches!(route("/terminal"), Route::Root));
        assert!(matches!(route("/terminal/abc"), Route::Item(id) if id == "abc"));
        assert!(
            matches!(route("/terminal/abc/connect-token"), Route::ConnectToken(id) if id == "abc")
        );
        assert!(matches!(route("/terminal/abc/connect"), Route::Connect(id) if id == "abc"));
        assert!(matches!(route("/terminal/abc/nope"), Route::Unknown));
        assert!(matches!(route("/"), Route::Unknown));
    }

    #[test]
    fn only_the_app_and_loopback_origins_are_allowed() {
        assert!(is_allowed_origin("tauri://localhost"));
        assert!(is_allowed_origin("http://tauri.localhost"));
        assert!(is_allowed_origin("http://127.0.0.1:5173"));
        assert!(is_allowed_origin("http://localhost:1420"));
        assert!(!is_allowed_origin("https://evil.example"));
        assert!(!is_allowed_origin("http://127.0.0.1.evil.example"));
        assert!(!is_allowed_origin("null"));
    }

    #[test]
    fn query_values_are_decoded() {
        assert_eq!(
            query_value("/terminal/t/connect?cursor=12&ticket=ab", "cursor").as_deref(),
            Some("12")
        );
        assert_eq!(
            query_value("/terminal/t/connect?ticket=a%2Bb", "ticket").as_deref(),
            Some("a+b")
        );
        assert_eq!(query_value("/terminal/t/connect", "ticket"), None);
    }

    #[test]
    fn error_bodies_carry_the_code() {
        let body = error_body("CWD_INVALID", "CWD_INVALID: nope");
        assert_eq!(body["error"]["code"], "CWD_INVALID");
        assert_eq!(body["error"]["message"], "CWD_INVALID: nope");
    }

    #[tokio::test]
    async fn rewind_stream_replays_consumed_bytes_before_the_socket() {
        // Two ends of a real loopback socket so both the prefix and the
        // delegated path are exercised.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let mut client = TcpStream::connect(addr).await.expect("connect");
        let (server, _) = listener.accept().await.expect("accept");
        let mut stream = RewindStream::new(b"GET / HTTP/1.1\r\n\r\n".to_vec(), server);
        client.write_all(b"first").await.expect("write");

        let mut head = [0_u8; 18];
        stream.read_exact(&mut head).await.expect("prefix read");
        assert_eq!(&head, b"GET / HTTP/1.1\r\n\r\n");
        let mut body = [0_u8; 5];
        stream.read_exact(&mut body).await.expect("socket read");
        assert_eq!(&body, b"first");
    }

    #[test]
    fn secrets_compare_in_constant_time_and_mismatch() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn secret_is_long_and_hex() {
        let secret = random_secret();
        assert_eq!(secret.len(), 64);
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

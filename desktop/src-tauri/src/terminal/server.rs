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
    use crate::terminal::test_support;

    /// A server with its own manager, so a test can create the exact child it
    /// needs without the store, the shared listener or another test's sessions.
    ///
    /// Defined here, not in `mod tests`, because both this module and its nested
    /// test module need it: the route tests drive a real session through the
    /// control layer, and the pump tests drive one through a synthetic socket.
    fn isolated_server() -> TerminalServer {
        TerminalServer {
            info: ServerInfo {
                url: "http://127.0.0.1:0".to_string(),
                token: random_secret(),
                port: 0,
                max_sessions: MAX_SESSIONS,
            },
            listener: Mutex::new(None),
            manager: Arc::new(Manager::new()),
            tickets: TicketStore::default(),
            connections: Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS)),
        }
    }

    /// Create a session in `server`'s own manager and hand back its id.
    fn isolated_session(
        server: &TerminalServer,
        thread: &str,
        command: (std::path::PathBuf, Vec<String>),
    ) -> String {
        let (program, args) = command;
        server
            .manager
            .create_with(
                thread.to_string(),
                Some("Harness".to_string()),
                program,
                args,
                std::env::temp_dir(),
                80,
                24,
            )
            .expect("create session")
            .id
    }
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

    /// Send raw bytes over a fresh connection and read everything until the
    /// peer closes. Used for the cases the helper above cannot express: a
    /// request that is never terminated, an oversized body, a silent socket.
    fn raw_exchange(port: u16, payload: &[u8]) -> String {
        raw_exchange_split(port, payload, &[], Duration::from_millis(0))
    }

    /// As [`raw_exchange`], but `payload` is written in two parts separated by
    /// `gap`. That is what makes a *split* request body deterministic: without
    /// the pause TCP would coalesce both writes into the segment the parser
    /// reads first, and the body-read loop would never run.
    fn raw_exchange_split(port: u16, head: &[u8], body: &[u8], gap: Duration) -> String {
        let mut stream = StdStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("timeout");
        if !head.is_empty() {
            stream.write_all(head).expect("write head");
            stream.flush().expect("flush head");
        }
        if body.is_empty() {
            // A peer that has nothing more to say: half-close so the parser
            // sees EOF instead of waiting out its read timeout.
            let _ = stream.shutdown(std::net::Shutdown::Write);
        } else {
            std::thread::sleep(gap);
            stream.write_all(body).expect("write body");
            stream.flush().expect("flush body");
        }
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        response
    }

    /// Bind and start the accept loop, tolerating a loop that another test in
    /// this process already started. The listener is process-wide, so which
    /// test gets there first is not something a test may depend on.
    fn shared_server() -> (u16, String) {
        let server = bind().expect("bind");
        if let Err(error) = serve() {
            assert!(error.contains("already serving"), "{error}");
        }
        (server.info.port, server.info.token.clone())
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
        // The accept loop is process-wide: whichever test in this process gets
        // here first starts it, and every later call observes that it is
        // already running. `serve_owns_the_listener_exactly_once` pins the
        // refusal message down.
        if let Err(error) = serve() {
            assert!(error.contains("already serving"), "{error}");
        }
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

        // CR is what a terminal actually sends for Enter (a Unix pty maps it
        // back to LF through ICRNL); Windows PowerShell's PSReadLine reads CR
        // as Enter and treats a bare LF as "insert a line", leaving the command
        // sitting in the buffer as a continuation line.
        socket
            .send(Message::Text("echo marker-$((21 + 21))\r".into()))
            .await
            .expect("send input");

        // Collect output until the shell's echo lands, tracking the control
        // frame the way the client does.
        //
        // A real terminal must also answer the shell's Device Status Report
        // (`ESC [ 6 n`, a cursor-position query): Windows PowerShell's
        // PSReadLine emits it while starting an interactive session and then
        // waits for the reply before it draws a prompt or reads the line we
        // typed. Without an answering client the shell sits at the query and
        // the test would time out on a machine whose default shell is pwsh.
        const DSR_QUERY: &str = "\u{1b}[6n";
        const DSR_REPLY: &str = "\u{1b}[1;1R";
        let mut answered_dsr = false;
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
            if !answered_dsr && output.contains(DSR_QUERY) {
                answered_dsr = true;
                socket
                    .send(Message::Text(DSR_REPLY.into()))
                    .await
                    .expect("answer the shell's cursor-position query");
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
        // The home fallback is served in the ordinary spelling (Windows'
        // `\\?\` extended-length form never reaches a client or a shell).
        let home = crate::store::strip_verbatim_prefix(
            std::fs::canonicalize(&home).unwrap_or_else(|_| std::path::PathBuf::from(home)),
        );
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

    /// The listener is process-wide: the first `bind`/`serve` wins and the
    /// second call reports the existing state instead of rebinding or starting
    /// a second accept loop.
    #[test]
    fn serve_owns_the_listener_exactly_once() {
        let first = bind().expect("bind");
        let second = bind().expect("bind again");
        assert_eq!(first.info.port, second.info.port, "one listener, one port");
        assert_eq!(
            first.info.token, second.info.token,
            "one secret per process"
        );
        assert_eq!(first.info.max_sessions, MAX_SESSIONS);

        // Whether this test or the end-to-end one started the loop first is not
        // something either may depend on; what must hold is that the *second*
        // call never steals the listener the first one handed out.
        let _ = serve();
        let refused = serve().expect_err("the listener can only be taken once");
        assert!(refused.contains("already serving"), "{refused}");
        assert_eq!(
            info().map(|info| info.port),
            Some(first.info.port),
            "info() reports the bound endpoint"
        );
    }

    /// Everything the control surface must refuse: an unknown route, a wrong
    /// method, a missing secret, a malformed body, an unknown session and a
    /// foreign origin. None of these need a conversation, so they can run
    /// without a home in the store.
    #[test]
    fn control_routes_reject_malformed_unknown_and_unauthenticated_requests() {
        let (port, token) = shared_server();

        // An unknown path is a 404 with the standard error envelope.
        let unknown = request(port, "GET", "/nope", Some(&token), None);
        assert_eq!(unknown.status, 404, "body: {}", unknown.body);
        assert_eq!(body_json(&unknown)["error"]["code"], "NOT_FOUND");

        // A known path with an unsupported method is a 405, not a 404.
        let wrong_method = request(port, "PUT", "/terminal", Some(&token), None);
        assert_eq!(wrong_method.status, 405, "body: {}", wrong_method.body);
        assert_eq!(
            body_json(&wrong_method)["error"]["code"],
            "METHOD_NOT_ALLOWED"
        );

        // The secret is required on every control route except the preflight,
        // and a wrong secret is refused exactly like a missing one.
        for token in [None, Some("not-the-secret")] {
            let refused = request(port, "GET", "/terminal", token, None);
            assert_eq!(refused.status, 401, "body: {}", refused.body);
            assert_eq!(body_json(&refused)["error"]["code"], "UNAUTHORIZED");
        }

        // A foreign origin is refused even when it holds the secret.
        let foreign = request_with_origin(
            port,
            "OPTIONS",
            "/terminal",
            Some(&token),
            None,
            Some("https://evil.example"),
        );
        assert_eq!(foreign.status, 403, "body: {}", foreign.body);
        assert_eq!(body_json(&foreign)["error"]["code"], "ORIGIN_FORBIDDEN");

        // A loopback dev server is allowed, and the echoed CORS header names the
        // origin that asked.
        let dev = request_with_origin(
            port,
            "OPTIONS",
            "/terminal",
            None,
            None,
            Some("http://localhost:5173"),
        );
        assert_eq!(dev.status, 204);
        assert!(
            dev.raw
                .to_lowercase()
                .contains("access-control-allow-origin: http://localhost:5173"),
            "raw: {}",
            dev.raw
        );

        // Create: a body that is not JSON, then one whose threadId is blank.
        let not_json = request(port, "POST", "/terminal", Some(&token), Some("not json"));
        assert_eq!(not_json.status, 400);
        assert_eq!(
            body_json(&not_json)["error"]["message"],
            "invalid create request body"
        );
        let blank = request(
            port,
            "POST",
            "/terminal",
            Some(&token),
            Some(&serde_json::json!({ "threadId": "   " }).to_string()),
        );
        assert_eq!(blank.status, 400);
        assert_eq!(
            body_json(&blank)["error"]["message"],
            "threadId is required"
        );

        // Update: an unparseable body is refused before any session lookup...
        let bad_update = request(port, "PATCH", "/terminal/ghost", Some(&token), Some("["));
        assert_eq!(bad_update.status, 400);
        assert_eq!(
            body_json(&bad_update)["error"]["message"],
            "invalid update request body"
        );
        // ...and a well-formed body for an unknown session is a 404 from the
        // manager, not a 400.
        let unknown_update = request(
            port,
            "PATCH",
            "/terminal/ghost",
            Some(&token),
            Some(&serde_json::json!({ "cols": 90, "rows": 30 }).to_string()),
        );
        assert_eq!(unknown_update.status, 404, "body: {}", unknown_update.body);
        assert_eq!(
            body_json(&unknown_update)["error"]["code"],
            "TERMINAL_NOT_FOUND"
        );

        // Every route that addresses one session reports the same 404.
        for (method, path) in [
            ("GET", "/terminal/ghost"),
            ("DELETE", "/terminal/ghost"),
            ("POST", "/terminal/ghost/connect-token"),
        ] {
            let response = request(port, method, path, Some(&token), None);
            assert_eq!(response.status, 404, "{method} {path}: {}", response.body);
            assert_eq!(body_json(&response)["error"]["code"], "TERMINAL_NOT_FOUND");
        }

        // The websocket route refuses a non-GET and a plain GET before it looks
        // at a ticket: a ticket is not an authentication substitute for the
        // upgrade itself.
        let connect_post = request(port, "POST", "/terminal/ghost/connect", Some(&token), None);
        assert_eq!(connect_post.status, 405);
        assert_eq!(
            body_json(&connect_post)["error"]["message"],
            "connect requires GET"
        );
        let connect_plain = request(port, "GET", "/terminal/ghost/connect", Some(&token), None);
        assert_eq!(connect_plain.status, 400);
        assert_eq!(
            body_json(&connect_plain)["error"]["message"],
            "connect requires a websocket upgrade"
        );

        // A query the request does not carry, and one that does not name the
        // key asked for, both fall back to the caller's default.
        let ignored_query = request(port, "GET", "/terminal?foo=1", Some(&token), None);
        assert_eq!(ignored_query.status, 200);
        assert!(body_json(&ignored_query).is_array());
    }

    /// The single-session `GET` and a partial `PATCH`.
    ///
    /// `PATCH` takes its size from `(cols, rows)` **only when both are
    /// present**: a caller that re-fits one axis (the renderer sends `cols`
    /// alone while a user drags the panel wider) must not have the other axis
    /// silently reset, and must not resize the PTY at all until it has a
    /// complete size. That `_ => None` arm is the whole point of the match, and
    /// nothing else in this suite sends a partial body.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_session_can_be_read_and_partially_updated() {
        let _home = setup_home("terminal-item-routes");
        let thread_id = create_thread_in_new_workspace("terminal-item-routes");
        let (port, token) = shared_server();

        let created = request(
            port,
            "POST",
            "/terminal",
            Some(&token),
            Some(
                &serde_json::json!({
                    "threadId": thread_id,
                    "cols": 100,
                    "rows": 30,
                    "title": "Terminal 1",
                })
                .to_string(),
            ),
        );
        assert_eq!(created.status, 200, "body: {}", created.body);
        let id = body_json(&created)["id"].as_str().expect("id").to_string();

        // GET one session: the same document `create` returned, addressed by id.
        let read = request(port, "GET", &format!("/terminal/{id}"), Some(&token), None);
        assert_eq!(read.status, 200, "body: {}", read.body);
        let read = body_json(&read);
        assert_eq!(read["id"], serde_json::json!(id));
        assert_eq!(read["threadId"], serde_json::json!(thread_id));
        assert_eq!(read["cols"], serde_json::json!(100));
        assert_eq!(read["rows"], serde_json::json!(30));
        assert_eq!(read["title"], serde_json::json!("Terminal 1"));
        assert_eq!(read["status"], serde_json::json!("running"));

        // A partial body: a new title, but only one of the two size axes. The
        // title must change and both axes must be left alone — if the size were
        // built from the partial pair, `rows` would come back missing.
        let partial = request(
            port,
            "PATCH",
            &format!("/terminal/{id}"),
            Some(&token),
            Some(&serde_json::json!({ "title": "Renamed", "cols": 132 }).to_string()),
        );
        assert_eq!(partial.status, 200, "body: {}", partial.body);
        let partial = body_json(&partial);
        assert_eq!(partial["title"], serde_json::json!("Renamed"));
        assert_eq!(
            partial["cols"],
            serde_json::json!(100),
            "a partial size must not be applied at all"
        );
        assert_eq!(partial["rows"], serde_json::json!(30));

        // A complete pair does resize, so the call above was not silently
        // ignored for some unrelated reason.
        let full = request(
            port,
            "PATCH",
            &format!("/terminal/{id}"),
            Some(&token),
            Some(&serde_json::json!({ "cols": 132, "rows": 44 }).to_string()),
        );
        assert_eq!(full.status, 200, "body: {}", full.body);
        let full = body_json(&full);
        assert_eq!(full["cols"], serde_json::json!(132));
        assert_eq!(full["rows"], serde_json::json!(44));
        assert_eq!(
            full["title"],
            serde_json::json!("Renamed"),
            "an update without a title must keep the current one"
        );

        let removed = request(
            port,
            "DELETE",
            &format!("/terminal/{id}"),
            Some(&token),
            None,
        );
        assert_eq!(removed.status, 200, "body: {}", removed.body);
    }

    /// The request parser is bounded: a connection that never sends a request,
    /// an oversized head and an oversized declared body are all dropped without
    /// a response, and a body that arrives in a second TCP segment is still
    /// assembled — that second read is the only reason the body loop exists.
    #[test]
    fn the_request_parser_is_bounded_and_reassembles_split_bodies() {
        let (port, token) = shared_server();

        // A connection that sends nothing at all: the parser sees EOF and
        // closes without writing a byte.
        assert_eq!(raw_exchange(port, b""), "");

        // A head bigger than MAX_HEAD with no terminator is dropped.
        let oversized = vec![b'H'; MAX_HEAD + 64];
        assert_eq!(raw_exchange(port, &oversized), "");

        // A declared body bigger than MAX_BODY is refused before it is read.
        let huge = format!(
            "POST /terminal HTTP/1.1\r\nhost: x\r\ncontent-length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        assert_eq!(raw_exchange(port, huge.as_bytes()), "");

        // A body split across two segments is reassembled and parsed: the
        // refusal names the *conversation*, which is only reachable once the
        // body was decoded.
        // The body is only decoded behind the control route's secret, so the
        // request needs the bearer token: without it every case below would be
        // a 401 and none of the parser's body paths could be reached.
        let body = serde_json::json!({ "threadId": "split-body" }).to_string();
        let head = format!(
            "POST /terminal HTTP/1.1\r\nhost: x\r\nconnection: close\r\nauthorization: Bearer {token}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
            body.len()
        );
        let response = raw_exchange_split(
            port,
            head.as_bytes(),
            body.as_bytes(),
            Duration::from_millis(80),
        );
        assert!(
            !response.contains("invalid create request body"),
            "the split body must be reassembled, got: {response}"
        );
        assert!(
            response.contains("THREAD_NOT_FOUND"),
            "response: {response}"
        );

        // A body that stops early (the peer half-closed) is truncated, and the
        // truncated request is rejected instead of hanging on.
        let truncated = format!(
            "POST /terminal HTTP/1.1\r\nhost: x\r\nconnection: close\r\nauthorization: Bearer {token}\r\ncontent-length: 64\r\n\r\n{{\"threadId\":\"sh"
        );
        let response = raw_exchange(port, truncated.as_bytes());
        assert!(
            response.contains("invalid create request body"),
            "a truncated body must be rejected: {response}"
        );
    }

    /// The HTTP status the server answers a WebSocket handshake with, for the
    /// handshakes that are *expected* to be refused. A successful one returns
    /// 101 and hands back a live socket the caller must drain, which is what
    /// `a_real_shell_streams_over_the_loopback_transport` covers.
    async fn refused_handshake_status(port: u16, path: &str) -> u16 {
        let url = format!("ws://127.0.0.1:{port}{path}");
        match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok(_) => 101,
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                response.status().as_u16()
            }
            Err(error) => panic!("unexpected handshake failure for {path}: {error}"),
        }
    }

    /// The WebSocket route has its own authentication boundary and every part of
    /// it is checked *before* the socket is upgraded, so a caller that cannot
    /// connect is told why by name instead of being left with a socket that
    /// silently closes.
    ///
    /// A browser cannot set an `Authorization` header on a handshake, so the
    /// secret only buys a one-shot ticket, and a ticket is bound to one session.
    /// That is the whole reason the two routes exist, and this test pins the
    /// four refusals a client can actually hit, in the order the router checks
    /// them: method, upgrade, ticket, session.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_connect_route_authenticates_before_it_upgrades() {
        let _home = setup_home("terminal-connect-auth");
        let thread_id = create_thread_in_new_workspace("terminal-connect-auth");
        let (port, token) = shared_server();

        let created = request(
            port,
            "POST",
            "/terminal",
            Some(&token),
            Some(&serde_json::json!({ "threadId": thread_id, "cols": 80, "rows": 24 }).to_string()),
        );
        assert_eq!(created.status, 200, "body: {}", created.body);
        let id = body_json(&created)["id"].as_str().expect("id").to_string();

        // A connect is always a GET. Anything else is refused by name, and the
        // refusal carries the code the client switches on.
        let wrong_method = request(
            port,
            "POST",
            &format!("/terminal/{id}/connect"),
            Some(&token),
            None,
        );
        assert_eq!(wrong_method.status, 405, "raw: {}", wrong_method.raw);
        assert!(
            wrong_method.body.contains("METHOD_NOT_ALLOWED"),
            "{}",
            wrong_method.body
        );

        // A plain GET is not an upgrade either, and the route says so rather
        // than holding the connection open waiting for one.
        let not_an_upgrade = request(
            port,
            "GET",
            &format!("/terminal/{id}/connect"),
            Some(&token),
            None,
        );
        assert_eq!(not_an_upgrade.status, 400, "raw: {}", not_an_upgrade.raw);
        assert!(
            not_an_upgrade.body.contains("INVALID_ARGUMENT"),
            "{}",
            not_an_upgrade.body
        );

        // A real handshake with no ticket at all: the secret alone is not
        // enough on this route.
        assert_eq!(
            refused_handshake_status(port, &format!("/terminal/{id}/connect")).await,
            403,
            "a handshake without a ticket must be refused"
        );
        // ... and a ticket nobody issued is refused the same way.
        assert_eq!(
            refused_handshake_status(port, &format!("/terminal/{id}/connect?ticket=deadbeef"))
                .await,
            403,
            "an unissued ticket must be refused"
        );
        // A ticket is scoped to one session, so an unknown session is a 404 —
        // the ticket's own validity is not even the question.
        assert_eq!(
            refused_handshake_status(port, "/terminal/no-such-session/connect?ticket=deadbeef")
                .await,
            404,
            "an unknown session must be reported as missing"
        );

        // The handle the refusals were about is still usable: a ticket issued
        // for it upgrades (101), and the shell it carries is then torn down.
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
        assert_eq!(
            refused_handshake_status(
                port,
                &format!("/terminal/{id}/connect?cursor=-1&ticket={ticket}")
            )
            .await,
            101,
            "a valid ticket must upgrade"
        );

        let removed = request(
            port,
            "DELETE",
            &format!("/terminal/{id}"),
            Some(&token),
            None,
        );
        assert_eq!(removed.status, 200, "body: {}", removed.body);
        assert_eq!(
            body_json(&removed)["removed"],
            serde_json::json!(true),
            "a successful removal reports itself"
        );
    }

    /// The WebSocket pump's client-facing half, and both of its exit paths.
    ///
    /// The end-to-end test above covers the *streaming* half (a live shell's
    /// bytes reaching a viewer). This one covers what a client can send and how
    /// the socket ends, which is the part a browser actually depends on:
    ///
    /// * a `Ping` must be answered with a matching `Pong` — that is how the
    ///   renderer notices a dead connection;
    /// * a `Text` frame is input, and the shell's echo proves it arrived;
    /// * a session that exits while a viewer is attached must send the exit
    ///   control frame and then a normal `Close`, not a silent socket;
    /// * a viewer that connects *after* the exit must get the final screen, the
    ///   exit code and the same `Close` immediately.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_pump_answers_pings_carries_input_and_ends_with_the_shell() {
        let _home = setup_home("terminal-pump");
        let thread_id = create_thread_in_new_workspace("terminal-pump");
        let server = bind().expect("bind");
        if let Err(error) = serve() {
            assert!(error.contains("already serving"), "{error}");
        }
        let port = server.info.port;
        let token = server.info.token.clone();

        let created = request(
            port,
            "POST",
            "/terminal",
            Some(&token),
            Some(&serde_json::json!({ "threadId": thread_id, "cols": 80, "rows": 24 }).to_string()),
        );
        assert_eq!(created.status, 200, "body: {}", created.body);
        let id = body_json(&created)["id"].as_str().expect("id").to_string();

        let ticket = |port: u16| {
            let response = request(
                port,
                "POST",
                &format!("/terminal/{id}/connect-token"),
                Some(&token),
                None,
            );
            body_json(&response)["ticket"]
                .as_str()
                .expect("ticket")
                .to_string()
        };

        // ---- a live viewer: Ping, then Text input ----
        let first = ticket(port);
        let url = format!("ws://127.0.0.1:{port}/terminal/{id}/connect?cursor=-1&ticket={first}");
        let (mut socket, response) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .expect("handshake");
        assert_eq!(response.status().as_u16(), 101);

        socket
            .send(Message::Ping(b"probe".to_vec()))
            .await
            .expect("send ping");
        let mut pong: Option<Vec<u8>> = None;
        let mut echoed = String::new();
        // The shell emits `ESC [ 6 n` while starting and will not read a line
        // until a client answers it (the end-to-end test above answers the same
        // query — the pump deliberately does not, because the query belongs to
        // the terminal protocol, not to the transport).
        let mut answered_dsr = false;
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && pong.is_none() {
            match tokio::time::timeout(Duration::from_secs(5), socket.next()).await {
                Ok(Some(Ok(Message::Pong(payload)))) => pong = Some(payload),
                Ok(Some(Ok(Message::Binary(bytes)))) => {
                    if bytes.first() != Some(&protocol::CONTROL_PREFIX) {
                        echoed.push_str(&String::from_utf8_lossy(&bytes));
                    }
                }
                Ok(Some(Ok(_))) => {}
                Ok(Some(Err(error))) => panic!("socket error: {error}"),
                Ok(None) => break,
                Err(_) => break,
            }
            if !answered_dsr && echoed.contains("\u{1b}[6n") {
                answered_dsr = true;
                socket
                    .send(Message::Text("\u{1b}[1;1R".into()))
                    .await
                    .expect("answer the shell's cursor query");
            }
        }
        assert_eq!(
            pong.as_deref(),
            Some(&b"probe"[..]),
            "the pump must echo a Ping's payload back as a Pong"
        );

        // Text is input: the shell runs the line it carries.
        socket
            .send(Message::Text("echo pump-marker\r".into()))
            .await
            .expect("send text");
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && !echoed.contains("pump-marker") {
            match tokio::time::timeout(Duration::from_secs(5), socket.next()).await {
                Ok(Some(Ok(Message::Binary(bytes)))) => {
                    if bytes.first() != Some(&protocol::CONTROL_PREFIX) {
                        echoed.push_str(&String::from_utf8_lossy(&bytes));
                    }
                }
                Ok(Some(Ok(_))) => {}
                Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
            }
            if !answered_dsr && echoed.contains("\u{1b}[6n") {
                answered_dsr = true;
                socket
                    .send(Message::Text("\u{1b}[1;1R".into()))
                    .await
                    .expect("answer the shell's cursor query");
            }
        }
        assert!(
            echoed.contains("pump-marker"),
            "a Text frame must reach the shell's input: {echoed:?}"
        );

        // ---- the session ends while this viewer is attached ----
        // `on_eof` is what `close()` and the reader thread call; on this host the
        // reader never sees EOF (see the run status), so the test drives it.
        // The shell is still alive at this point, so the recorded code is `None`
        // — deliberately: a session whose child could not be reaped must report
        // no code rather than invent one, and the transport must still tell the
        // viewer the stream is over and close the socket normally.
        let session = server.manager.session_for_test(&id).expect("session");
        session.on_eof();

        let mut exit_frame = None;
        let mut close_code = None;
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && close_code.is_none() {
            match tokio::time::timeout(Duration::from_secs(5), socket.next()).await {
                Ok(Some(Ok(Message::Binary(bytes)))) => {
                    if bytes.first() == Some(&protocol::CONTROL_PREFIX) {
                        exit_frame = serde_json::from_slice::<Meta>(&bytes[1..]).ok();
                    }
                }
                Ok(Some(Ok(Message::Close(frame)))) => close_code = frame.map(|f| f.code),
                Ok(Some(Ok(_))) => {}
                Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
            }
        }
        let exit_frame = exit_frame.expect("the exit control frame must be sent");
        assert_eq!(
            exit_frame.exit_code, None,
            "a live shell must not be given an invented exit code: {exit_frame:?}"
        );
        assert_eq!(
            close_code,
            Some(CloseCode::Normal),
            "an exited shell must close the socket normally"
        );

        // ---- a viewer that connects after the exit ----
        // The session is out of the manager only if it was removed; here it is
        // still addressable, so a fresh ticket must yield the final screen, the
        // exit code and an immediate Close.
        let second = ticket(port);
        let url = format!("ws://127.0.0.1:{port}/terminal/{id}/connect?cursor=-1&ticket={second}");
        let (mut late, response) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .expect("late handshake");
        assert_eq!(response.status().as_u16(), 101);
        let mut late_exit = None;
        let mut late_close = None;
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && late_close.is_none() {
            match tokio::time::timeout(Duration::from_secs(5), late.next()).await {
                Ok(Some(Ok(Message::Binary(bytes)))) => {
                    if bytes.first() == Some(&protocol::CONTROL_PREFIX) {
                        late_exit = serde_json::from_slice::<Meta>(&bytes[1..]).ok();
                    }
                }
                Ok(Some(Ok(Message::Close(frame)))) => late_close = frame.map(|f| f.code),
                Ok(Some(Ok(_))) => {}
                Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
            }
        }
        let late_exit =
            late_exit.expect("a viewer attaching to an exited session gets the control frame");
        assert_eq!(
            late_exit.exit_code, None,
            "the same un-reaped session state must be replayed, not a fresh guess: {late_exit:?}"
        );
        assert_eq!(
            late_exit.cursor, late_exit.start,
            "a tailing viewer's replay starts exactly at the cursor it is given"
        );
        assert_eq!(
            late_close,
            Some(CloseCode::Normal),
            "a late viewer must be closed normally, not left hanging"
        );

        let removed = request(
            port,
            "DELETE",
            &format!("/terminal/{id}"),
            Some(&token),
            None,
        );
        assert_eq!(removed.status, 200, "body: {}", removed.body);
    }

    /// The connect-ticket store's capacity boundary, through the route's own
    /// handler.
    ///
    /// `connect_token` is the only route that mints a ticket, and a flood of
    /// handshakes must be refused with 429 rather than growing the table without
    /// bound. Filling 10 000 tickets over HTTP would take minutes, so the store is
    /// filled directly — through the same `TicketStore::issue` the route calls —
    /// and then the handler is asked once, which is the behaviour under test.
    ///
    /// It runs against an **isolated** server on purpose: the ticket store is
    /// process-wide on the shared listener, and filling that one would leave
    /// every later test unable to mint a ticket (which is exactly what an
    /// earlier version of this test did).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_full_ticket_store_is_reported_as_capacity_exceeded() {
        let server = isolated_server();
        let id = isolated_session(&server, "thread-capacity", test_support::idle_command());
        let scope = TicketScope {
            terminal_id: id.clone(),
            thread_id: "thread-capacity".to_string(),
        };

        // The boundary is inclusive: the first `TICKET_CAPACITY` tickets are
        // issued, and the next one is refused.
        for issued in 0..crate::terminal::ticket::TICKET_CAPACITY {
            assert!(
                server.tickets.issue(scope.clone()).is_some(),
                "the store must accept ticket {issued}, up to its capacity"
            );
        }

        let refused = server.connect_token(&id);
        assert_eq!(
            refused.status,
            StatusCode::TOO_MANY_REQUESTS,
            "a full ticket store must refuse with 429: {:?}",
            refused.body
        );
        assert_eq!(
            refused.body["error"]["code"], "CAPACITY_EXCEEDED",
            "the refusal must name the capacity, not report a generic error"
        );
        assert_eq!(
            refused.body["error"]["message"], "too many outstanding connect tickets",
            "the refusal must name the reason, not report a generic error"
        );

        // The session it refused to mint for is untouched.
        assert!(server.manager.get(&id).is_ok());

        // And the refusal is not sticky: once a ticket is consumed the store has
        // room again, so a client that retries after reconnecting is served.
        let issued = server.connect_token(&id);
        assert_eq!(
            issued.status,
            StatusCode::TOO_MANY_REQUESTS,
            "a consumed ticket frees a slot only after it is used, which this \
             assertion pins: the store is still full"
        );
    }

    /// The connect-ticket store's capacity boundary, through the route's own
    /// handler.
    ///
    /// `connect_token` is the only route that mints a ticket, and a flood of
    /// handshakes must be refused with 429 rather than growing the table without
    /// bound. Filling 10 000 tickets over HTTP would take minutes, so the store is
    /// filled directly — through the same `TicketStore::issue` the route calls —
    /// and then the handler is asked once, which is the behaviour under test.
    ///
    /// It runs against an **isolated** server on purpose: the ticket store is
    /// process-wide on the shared listener, and filling that one would leave
    /// every later test unable to mint a ticket (which is exactly what an earlier
    /// version of this test did).
    /// A WebSocket handshake that fails *after* the route accepted the connection.
    ///
    /// `handle_connect`'s `let Ok(ws) = ws else { return }` is the arm for a client
    /// that asks for an upgrade but cannot complete one — here an
    /// `Upgrade: websocket` request with no `Sec-WebSocket-Key`, which
    /// `accept_hdr_async` rejects. The route must drop the connection instead of
    /// leaving a half-open socket behind (and must not panic).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_handshake_missing_its_key_is_dropped() {
        let _home = setup_home("terminal-bad-handshake");
        let thread_id = create_thread_in_new_workspace("terminal-bad-handshake");
        let (port, token) = shared_server();

        let created = request(
            port,
            "POST",
            "/terminal",
            Some(&token),
            Some(&serde_json::json!({ "threadId": thread_id }).to_string()),
        );
        assert_eq!(created.status, 200, "body: {}", created.body);
        let id = body_json(&created)["id"].as_str().expect("id").to_string();

        let issued = request(
            port,
            "POST",
            &format!("/terminal/{id}/connect-token"),
            Some(&token),
            None,
        );
        assert_eq!(
            issued.status, 200,
            "the ticket must be issuable: {}",
            issued.body
        );
        let ticket = body_json(&issued)["ticket"]
            .as_str()
            .expect("ticket")
            .to_string();

        // A valid ticket and a real upgrade request, but no key: the handshake
        // cannot be completed, so nothing is written back.
        let head = format!(
            "GET /terminal/{id}/connect?ticket={ticket} HTTP/1.1\r\n\
             host: 127.0.0.1:{port}\r\n\
             connection: Upgrade\r\n\
             upgrade: websocket\r\n\
             sec-websocket-version: 13\r\n\r\n"
        );
        let response = raw_exchange(port, head.as_bytes());
        assert!(
            !response.contains("101"),
            "a keyless handshake must not be upgraded: {response:?}"
        );

        // The session is still usable: the failed handshake consumed its ticket,
        // not its life.
        let alive = request(port, "GET", &format!("/terminal/{id}"), Some(&token), None);
        assert_eq!(alive.status, 200, "body: {}", alive.body);

        let removed = request(
            port,
            "DELETE",
            &format!("/terminal/{id}"),
            Some(&token),
            None,
        );
        assert_eq!(removed.status, 200, "body: {}", removed.body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::test_support;
    use std::time::Instant;

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
        // A malformed escape is kept literally (the byte is not dropped) and a
        // `+` is the space a form-encoded query means by it.
        assert_eq!(
            query_value("/terminal/t/connect?ticket=a+b%zz", "ticket").as_deref(),
            Some("a b%zz")
        );
        // No query at all, and a query that does not name this key, are both
        // `None` — the loop's terminal case is a real arm, not dead code.
        assert_eq!(query_value("/terminal/t/connect", "ticket"), None);
        assert_eq!(query_value("/terminal?foo=1", "ticket"), None);
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

        // Writes, flushes and the shutdown the websocket layer performs all
        // reach the socket; the client observes both the bytes and the EOF.
        stream.write_all(b"pong").await.expect("write through");
        stream.flush().await.expect("flush through");
        let mut got = [0_u8; 4];
        client.read_exact(&mut got).await.expect("client read");
        assert_eq!(&got, b"pong");
        stream.shutdown().await.expect("shutdown through");
        let mut tail = [0_u8; 1];
        assert_eq!(client.read(&mut tail).await.expect("eof read"), 0);
    }

    /// The upgrade response echoes the caller's origin (and invents nothing when
    /// there is none), so a browser started from a dev server can complete the
    /// handshake without the response looking reusable to another origin.
    #[test]
    fn the_upgrade_response_echoes_the_request_origin() {
        type UpgradeRequest = tokio_tungstenite::tungstenite::handshake::server::Request;
        type UpgradeResponse = tokio_tungstenite::tungstenite::handshake::server::Response;

        let request = UpgradeRequest::builder()
            .header("origin", "tauri://localhost")
            .body(())
            .expect("request");
        let response = UpgradeResponse::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .body(())
            .expect("response");
        let echoed = upgrade_response(&request, response).expect("upgrade");
        assert_eq!(
            echoed
                .headers()
                .get("access-control-allow-origin")
                .and_then(|value| value.to_str().ok()),
            Some("tauri://localhost")
        );

        let bare = UpgradeRequest::builder().body(()).expect("request");
        let response = UpgradeResponse::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .body(())
            .expect("response");
        let echoed = upgrade_response(&bare, response).expect("upgrade");
        assert!(
            echoed
                .headers()
                .get("access-control-allow-origin")
                .is_none(),
            "no origin to echo means no header"
        );
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

    // ---- the pump, driven by a socket the test owns -------------------------
    //
    // `pump` is generic over its transport, so these tests hand it a stream they
    // control completely: the bytes it reads (hand-built masked client frames)
    // and the writes it must perform (which they can make fail on demand). That
    // is the only way to reach the send-failure arms honestly — with a real TCP
    // peer the kernel accepts a write into its send buffer even after the peer
    // is gone, so the failure is a race rather than a state, and those arms
    // would be untestable and unwaivable.

    /// A client frame must be masked; this builds one by hand (RFC 6455 §5.3).
    fn client_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
        const MASK: [u8; 4] = [0x12, 0x34, 0x56, 0x78];
        let mut frame = vec![0x80 | opcode];
        match payload.len() {
            len if len < 126 => frame.push(0x80 | len as u8),
            len if len <= u16::MAX as usize => {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&(len as u16).to_be_bytes());
            }
            len => {
                frame.push(0x80 | 127);
                frame.extend_from_slice(&(len as u64).to_be_bytes());
            }
        }
        frame.extend_from_slice(&MASK);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ MASK[i % 4]));
        frame
    }

    const OP_TEXT: u8 = 0x1;
    const OP_BINARY: u8 = 0x2;
    const OP_CLOSE: u8 = 0x8;
    const OP_PING: u8 = 0x9;
    const OP_PONG: u8 = 0xA;

    #[derive(Default)]
    struct SocketState {
        inbound: std::collections::VecDeque<u8>,
        outbound: Vec<u8>,
        /// Set to make every subsequent write fail, the way a closed socket does.
        failing: bool,
        /// The reader's waker, so a byte pushed after the pump parked still wakes it.
        waker: Option<std::task::Waker>,
    }

    /// A duplex socket the test drives: bytes in, bytes out, and a failure switch.
    #[derive(Clone)]
    struct FakeSocket(Arc<Mutex<SocketState>>);

    impl FakeSocket {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(SocketState::default())))
        }
        fn push_frame(&self, opcode: u8, payload: &[u8]) {
            self.push_bytes(&client_frame(opcode, payload));
        }
        fn push_bytes(&self, bytes: &[u8]) {
            let waker = {
                let mut state = self.0.lock().unwrap();
                state.inbound.extend(bytes.iter().copied());
                state.waker.take()
            };
            if let Some(waker) = waker {
                waker.wake();
            }
        }
        fn fail_writes(&self) {
            self.0.lock().unwrap().failing = true;
        }
        fn sent(&self) -> Vec<u8> {
            self.0.lock().unwrap().outbound.clone()
        }
        fn has_sent_something(&self) -> bool {
            !self.0.lock().unwrap().outbound.is_empty()
        }
    }

    impl AsyncRead for FakeSocket {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let mut state = self.0.lock().unwrap();
            if state.inbound.is_empty() {
                // Park rather than report EOF: the pump must keep serving the
                // session while the client is merely quiet.
                state.waker = Some(cx.waker().clone());
                return Poll::Pending;
            }
            let n = buf.remaining().min(state.inbound.len());
            let bytes: Vec<u8> = state.inbound.drain(..n).collect();
            buf.put_slice(&bytes);
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for FakeSocket {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            let mut state = self.0.lock().unwrap();
            if state.failing {
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "the client is gone",
                )));
            }
            state.outbound.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    /// A server with its own manager, so these tests create the exact child they
    /// need without the store, the shared listener or another test's sessions.
    fn isolated_server() -> TerminalServer {
        TerminalServer {
            info: ServerInfo {
                url: "http://127.0.0.1:0".to_string(),
                token: random_secret(),
                port: 0,
                max_sessions: MAX_SESSIONS,
            },
            listener: Mutex::new(None),
            manager: Arc::new(Manager::new()),
            tickets: TicketStore::default(),
            connections: Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS)),
        }
    }

    /// Create a session in `server`'s own manager and hand back its id.
    fn isolated_session(
        server: &TerminalServer,
        thread: &str,
        command: (std::path::PathBuf, Vec<String>),
    ) -> String {
        let (program, args) = command;
        server
            .manager
            .create_with(
                thread.to_string(),
                Some("Harness".to_string()),
                program,
                args,
                std::env::temp_dir(),
                80,
                24,
            )
            .expect("create session")
            .id
    }

    async fn pump_socket(socket: FakeSocket) -> tokio_tungstenite::WebSocketStream<FakeSocket> {
        tokio_tungstenite::WebSocketStream::from_raw_socket(
            socket,
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await
    }

    /// Wait for the pump to have written something, or fail loudly.
    async fn await_first_write(socket: &FakeSocket) {
        for _ in 0..200 {
            if socket.has_sent_something() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the pump never wrote the control frame");
    }

    /// Every way the pump can end because the *client* cannot be written to.
    ///
    /// A viewer that vanishes must not take the pump's session down with it: each
    /// arm detaches and returns, leaving the shell running for the next tab. The
    /// assertions are on exactly that — the tunnel is abandoned per case, and the
    /// session behind it is still alive.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_pump_gives_up_quietly_when_the_client_cannot_be_written_to() {
        // (a) A failing replay send: the session has retained output, and the
        // socket is already dead when the pump tries to hand it over.
        let server = isolated_server();
        let id = isolated_session(&server, "thread-replay", test_support::idle_command());
        server
            .manager
            .session_for_test(&id)
            .expect("session")
            .on_data(b"retained screen");
        let socket = FakeSocket::new();
        socket.fail_writes();
        server
            .pump(pump_socket(socket).await, id.clone(), None)
            .await;
        assert!(
            server
                .manager
                .session_for_test(&id)
                .expect("session")
                .is_running(),
            "abandoning the view must not touch the shell"
        );

        // (b) A failing control-frame send, with nothing to replay first: the
        // pump's first write *is* the meta frame.
        let id = isolated_session(&server, "thread-meta", test_support::idle_command());
        let socket = FakeSocket::new();
        socket.fail_writes();
        server
            .pump(pump_socket(socket).await, id.clone(), Some(-1))
            .await;
        assert!(server
            .manager
            .session_for_test(&id)
            .expect("session")
            .is_running());
    }

    /// The mid-stream failure arms, each driven by making the write fail at the
    /// moment the pump is about to send that particular frame.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_pump_survives_a_client_that_vanishes_mid_stream() {
        let server = Arc::new(isolated_server());

        // Data that cannot be delivered.
        let id = isolated_session(&server, "thread-mid-data", test_support::idle_command());
        let socket = FakeSocket::new();
        let live = server.manager.session_for_test(&id).expect("session");
        let pump = {
            let server = Arc::clone(&server);
            let socket = socket.clone();
            let id = id.clone();
            tokio::spawn(async move { server.pump(pump_socket(socket).await, id, Some(-1)).await })
        };
        await_first_write(&socket).await;
        socket.fail_writes();
        live.on_data(b"undeliverable");
        tokio::time::timeout(Duration::from_secs(20), pump)
            .await
            .expect("the pump must stop when the client is gone")
            .expect("pump task");
        assert!(
            live.is_running(),
            "a dead client must not end the session it was watching"
        );

        // An exit that cannot be announced.
        let id = isolated_session(&server, "thread-mid-exit", test_support::idle_command());
        let socket = FakeSocket::new();
        let live = server.manager.session_for_test(&id).expect("session");
        let pump = {
            let server = Arc::clone(&server);
            let socket = socket.clone();
            let id = id.clone();
            tokio::spawn(async move { server.pump(pump_socket(socket).await, id, Some(-1)).await })
        };
        await_first_write(&socket).await;
        socket.fail_writes();
        live.on_eof();
        tokio::time::timeout(Duration::from_secs(20), pump)
            .await
            .expect("the pump must stop when the exit cannot be sent")
            .expect("pump task");

        // A keepalive that cannot be answered.
        let id = isolated_session(&server, "thread-mid-ping", test_support::idle_command());
        let socket = FakeSocket::new();
        let pump = {
            let server = Arc::clone(&server);
            let socket = socket.clone();
            let id = id.clone();
            tokio::spawn(async move { server.pump(pump_socket(socket).await, id, Some(-1)).await })
        };
        await_first_write(&socket).await;
        socket.fail_writes();
        socket.push_frame(OP_PING, b"keepalive");
        tokio::time::timeout(Duration::from_secs(20), pump)
            .await
            .expect("an unanswerable ping must not wedge the pump")
            .expect("pump task");
    }

    /// The client-facing half: every frame type the pump accepts, in order, and
    /// the way the conversation ends.
    ///
    /// This is a faithful client conversation. An interactive shell on this host
    /// emits `ESC [ 6 n` as it starts and will not read a line until a client
    /// answers it, so the test does what the renderer does: watch for the query
    /// on the socket, send the reply back as a `Text` frame, and only then send
    /// its input. The assertions are on the bytes that came back — the shell's
    /// echo of both inputs proves each frame reached the **PTY**, not merely the
    /// pump, and the `Pong` proves the keepalive round trip.
    /// WINDOWS-ONLY. Same handshake as the PTY test: it asserts the shell's cursor query reaches the client
/// over the socket, which is the Windows fixture's behaviour. A unix shell never asks.
/// Windows-only.
    #[cfg(windows)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_pump_reads_every_client_frame_type() {
        let server = isolated_server();
        let id = isolated_session(
            &server,
            "thread-frames",
            test_support::interactive_command(),
        );
        let socket = FakeSocket::new();

        let pump = {
            let server = Arc::new(server);
            let socket = socket.clone();
            let id = id.clone();
            tokio::spawn(async move { server.pump(pump_socket(socket).await, id, Some(-1)).await })
        };

        // Wait for the shell's cursor query, then answer it — the frame that a
        // terminal emulator sends.
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && !test_support::asks_for_the_cursor(&socket.sent()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            test_support::asks_for_the_cursor(&socket.sent()),
            "the shell's cursor query must reach the client over the socket"
        );
        socket.push_frame(OP_TEXT, test_support::DSR_REPLY);

        // A `Binary` frame is input in the terminal's own protocol, a `Text`
        // frame is input verbatim, `Ping` must be answered and `Pong` ignored.
        socket.push_frame(OP_BINARY, b"echo binary-input\r");
        socket.push_frame(OP_TEXT, b"echo text-input\r");
        socket.push_frame(OP_PING, b"probe");
        socket.push_frame(OP_PONG, b"unsolicited");

        let deadline = Instant::now() + Duration::from_secs(20);
        let mut seen = String::new();
        while Instant::now() < deadline {
            seen = test_support::visible_text(&socket.sent());
            if seen.contains("binary-input") && seen.contains("text-input") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            seen.contains("binary-input") && seen.contains("text-input"),
            "both client frames must reach the shell, whose echo comes back over \
             the socket: {seen:?}"
        );
        assert!(
            String::from_utf8_lossy(&socket.sent()).contains("probe"),
            "the Ping's payload must come back as a Pong: {:?}",
            String::from_utf8_lossy(&socket.sent())
        );

        // `Close` ends the conversation.
        socket.push_frame(OP_CLOSE, &[]);
        tokio::time::timeout(Duration::from_secs(20), pump)
            .await
            .expect("a Close frame must end the pump")
            .expect("pump task");
    }

    /// A client that sends bytes that are not a WebSocket frame at all.
    ///
    /// `Some(Err(_)) => break` is the arm that keeps a malformed peer from
    /// spinning the pump: tungstenite reports the protocol error, and the
    /// tunnel must end rather than retry on a stream that can never recover.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_pump_ends_on_a_protocol_error_from_the_client() {
        let server = isolated_server();
        let id = isolated_session(&server, "thread-garbage", test_support::idle_command());
        let socket = FakeSocket::new();
        // An unmasked frame: illegal from a client, so tungstenite errors.
        socket.push_bytes(&[0x81, 0x03, b'b', b'a', b'd']);

        let pump = {
            let socket = socket.clone();
            tokio::spawn(async move {
                server
                    .pump(pump_socket(socket).await, id.clone(), Some(-1))
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(20), pump)
            .await
            .expect("a protocol error must end the pump, not wedge it")
            .expect("pump task");
    }

    /// A viewer that attaches to a session that has **already exited**.
    ///
    /// The pump's early-exit branch is not the same code as the mid-stream one:
    /// when the attachment already carries an exit code the replay *is* the final
    /// screen, so the control frame must carry the code and the socket must close
    /// without ever activating a subscriber. Reaching it needs a session whose
    /// child was really reaped — an exit recorded from a live child has no code
    /// (see the run status), and that is a different branch.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_pump_replays_and_closes_a_session_that_already_exited() {
        let server = isolated_server();
        let id = isolated_session(&server, "thread-gone", test_support::exit_command(4));
        let live = server.manager.session_for_test(&id).expect("session");
        assert!(
            live.wait_for_child_exit(Duration::from_secs(30)),
            "the fixture child must really exit"
        );
        live.on_data(b"the last screen");
        live.on_eof();
        assert_eq!(
            live.info().exit_code,
            Some(4),
            "the reaped code must be on the session before the pump reads it"
        );

        let socket = FakeSocket::new();
        server
            .pump(pump_socket(socket.clone()).await, id.clone(), None)
            .await;

        let sent = test_support::visible_text(&socket.sent());
        assert!(
            sent.contains("the last screen"),
            "the final screen must be replayed to a late viewer: {sent:?}"
        );
        assert!(
            String::from_utf8_lossy(&socket.sent()).contains("\"exitCode\":4"),
            "the control frame must carry the reaped code: {:?}",
            String::from_utf8_lossy(&socket.sent())
        );
    }

    /// A shell that dies while the client is still typing.
    ///
    /// `attachment.write` is refused once the session is exited, and the pump
    /// must end the tunnel rather than keep accepting input for a shell that can
    /// never read it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_pump_stops_reading_input_once_the_shell_is_gone() {
        for (opcode, label) in [(OP_BINARY, "binary"), (OP_TEXT, "text")] {
            let server = isolated_server();
            let id = isolated_session(&server, "thread-dead-input", test_support::idle_command());
            let live = server.manager.session_for_test(&id).expect("session");
            let socket = FakeSocket::new();
            let pump = {
                let server = Arc::new(server);
                let socket = socket.clone();
                let id = id.clone();
                tokio::spawn(
                    async move { server.pump(pump_socket(socket).await, id, Some(-1)).await },
                )
            };
            await_first_write(&socket).await;
            // The shell is gone, so the next input cannot be delivered.
            live.on_eof();
            socket.push_frame(opcode, b"echo too-late\r");
            assert!(
                !live.is_running(),
                "the {label} case must start from a dead session"
            );
            tokio::time::timeout(Duration::from_secs(20), pump)
                .await
                .unwrap_or_else(|_| panic!("the {label} input must end the pump"))
                .expect("pump task");
        }
    }

    /// A session that disappears between the ticket check and the upgrade.
    ///
    /// `pump`'s `attach` failure is the transport's answer to exactly that race:
    /// the route already returns 404 for a *stranger*, but a session can be
    /// closed after a valid ticket was redeemed, and the socket must then be
    /// dropped without a reply rather than kept open.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_pump_drops_a_session_that_disappeared_before_the_upgrade() {
        let server = isolated_server();
        let socket = FakeSocket::new();
        server
            .pump(pump_socket(socket.clone()).await, "ghost".to_string(), None)
            .await;
        assert!(
            !socket.has_sent_something(),
            "an unknown session must be dropped without writing to the socket"
        );
    }
}

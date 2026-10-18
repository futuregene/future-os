//! Fixture F (TAU-D11): the non-test child that closes `remote/supervisor/start.rs`'s
//! `#[cfg(not(test))]` bodies.
//!
//! Why a spawned child and not a lib test: those bodies are compiled only into the
//! **non-test** copy of `futureos_lib` (the one the bins link). A `#[test]` body links the
//! test copy, where `spawn_start_retry`, `spawn_runtime_reconnect`, `GenerationWatch` and
//! `spawn_runtime_supervisor` are replaced by `return;`/`()` stubs. `docs/testing/
//! module-tauri-remote.md` §10.4 named this fixture; §11 records the landing.
//!
//! Two children, one per test, each with its own temp `HOME`/`FUTURE_HOME`:
//!
//! * [`a_broker_that_refuses_the_credential_is_reported`] — the platform issues a
//!   credential, the controlled broker answers `INFO` then `-ERR 'Authorization
//!   Violation'` on `CONNECT`. `classify_nats_connect_error` maps that to
//!   `RemoteAuthorization`, `start_failure` reports `service_authorization`, and the
//!   headless entry exits non-zero **naming** the failure. The same child seeds a corrupt
//!   pending-revoke file so `spawn_revoke_cleanup`'s error arm (`start.rs:653-655`) runs.
//! * [`a_re_pair_reports_a_pending_revoke_it_cannot_settle_or_retire`] — the same corrupt
//!   queue through the `--re-pair` entry point (a pty, because that flag is refused
//!   without a terminal): `re_pair`'s own settlement pass and `create_pairing`'s retire
//!   step both report the unreadable file, and the start after them is still refused.
//! * [`a_live_broker_drives_the_generation_watch_and_reconnect`] — the broker accepts
//!   the generation (`Ready`), a healthy watch tick runs, then the broker sends
//!   `-ERR 'Authorization Violation'` on the *live* connection. With an already-expired
//!   bridge JWT (so the health's `credential_expires_at` is in the past) that sets
//!   `reconnect_required` instead of the terminal `service_config_error`, so
//!   `GenerationWatch` calls `spawn_runtime_reconnect`, which records the failure,
//!   reports `RT001`, and runs `start_once(true)`; the platform then answers the refresh
//!   with a body that does not deserialize, which is the `Err` arm of that worker.
//! * [`a_healthy_generation_resets_the_budget_and_a_retryable_reconnect_arms_the_retry`]
//!   — a 65-second healthy dwell crosses `RUNTIME_HEALTHY_RESET_SECS`, then a refusal
//!   whose reconnect start is a retryable `server` failure takes the worker's
//!   `Ok(status) if retryable` arm.
//!
//! Both brokers are real WebSocket servers (`ws://`), and that is deliberate: the
//! production `connect_nats` calls `require_tls(true)` in the non-test copy, and
//! async-nats skips the TLS upgrade for a WebSocket address (`connector.rs`,
//! `!server_addr.is_websocket()`). A plaintext `nats://` listener can therefore only
//! produce an IO/TLS error (a *retryable* transport status), not the broker's own
//! refusal; a `ws://` listener can speak the protocol. The platform is a loopback HTTP
//! server (the child's `auth.json` `base_url` is pointed at it), so no real account,
//! relay, or NATS server is involved.
//!
//! Headless only: the `futureos-headless` bin has `required-features = ["headless"]`, so
//! the GUI leg of the two-flavour measurement cannot name it. The GUI leg compiles this
//! file to nothing; the headless leg runs it, and the union is the campaign's metric.

#![cfg(feature = "headless")]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Value};

const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::URL_SAFE_NO_PAD;

// ---------------------------------------------------------------------------
// A loopback platform
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum TokenReply {
    /// A credential for `nats_url` whose JWT expires `exp_offset` seconds from now.
    Credential { nats_url: String, exp_offset: i64 },
    /// HTTP 200 with a body the credential response cannot deserialize.
    Malformed,
    /// HTTP 500 with the platform's `{error, message}` body — categorized `server`,
    /// which is a *retryable* start failure.
    ServerError,
}

#[derive(Default)]
struct PlatformState {
    /// Replies consumed in order; the last one repeats for later calls.
    token: Vec<TokenReply>,
    token_calls: usize,
    /// Where a freshly minted invitation points, for the `--re-pair` child.
    pair_url: Option<String>,
}

struct Platform {
    url: String,
    state: Arc<Mutex<PlatformState>>,
}

impl Platform {
    fn start(token: Vec<TokenReply>, pair_url: Option<String>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind the fixture platform");
        let port = listener.local_addr().expect("platform address").port();
        let state = Arc::new(Mutex::new(PlatformState {
            token,
            token_calls: 0,
            pair_url,
        }));
        let server_state = state.clone();
        std::thread::Builder::new()
            .name("fixture-platform".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let state = server_state.clone();
                    std::thread::spawn(move || serve_platform(stream, state));
                }
            })
            .expect("spawn the fixture platform");
        Self {
            url: format!("http://127.0.0.1:{port}"),
            state,
        }
    }

    fn token_calls(&self) -> usize {
        self.state.lock().unwrap().token_calls
    }
}

fn serve_platform(mut stream: TcpStream, state: Arc<Mutex<PlatformState>>) {
    let mut request = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
        if request.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if request.len() > 64 * 1024 {
            return;
        }
    }
    let text = String::from_utf8_lossy(&request);
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let header_end = text
        .find("\r\n\r\n")
        .map(|index| index + 4)
        .unwrap_or(request.len());
    let mut body = request[header_end..].to_vec();
    while body.len() < content_length {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&buf[..n]),
        }
    }

    let (status, payload): (u16, Value) = if method == "GET"
        && path.ends_with("/client/v1/account/profile")
    {
        (
            200,
            json!({"email": "fixture@example.com", "user_id": "fixture", "email_verified": true}),
        )
    } else if method == "POST" && path.ends_with("/client/v1/remote/auth/token") {
        token_response(&state)
    } else if method == "POST" && path.ends_with("/client/v1/remote/pair/code") {
        pair_code_response(&state)
    } else if method == "POST" && path.ends_with("/client/v1/remote/pair/revoke") {
        (200, json!({}))
    } else {
        (404, json!({"error": "not_found"}))
    };
    let body = payload.to_string();
    let head = format!(
        "HTTP/1.1 {status} Fixture\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

fn token_response(state: &Arc<Mutex<PlatformState>>) -> (u16, Value) {
    let mut state = state.lock().unwrap();
    let index = state.token_calls.min(state.token.len().saturating_sub(1));
    state.token_calls += 1;
    match state.token.get(index) {
        Some(TokenReply::Credential {
            nats_url,
            exp_offset,
        }) => (
            200,
            json!({
                "user_jwt": fixture_jwt(*exp_offset),
                "nats_url": nats_url,
                "nats_ws_url": nats_url,
            }),
        ),
        Some(TokenReply::Malformed) | None => (200, json!({"user_jwt": 5})),
        Some(TokenReply::ServerError) => (500, json!({"error": "internal"})),
    }
}

/// The invitation the platform mints for a `--re-pair` child. `pairing_code` must be
/// base64url JSON carrying `exp`: `create_pairing` derives the secure identity's expiry
/// from it, and a code without one is refused before the identity exists.
fn pair_code_response(state: &Arc<Mutex<PlatformState>>) -> (u16, Value) {
    let url = state.lock().unwrap().pair_url.clone().unwrap_or_default();
    let code = B64.encode(json!({ "exp": unix_now() + 600 }).to_string());
    (
        200,
        json!({
            "pair_id": "pair_fixture",
            "pairing_code": code,
            "user_jwt": fixture_jwt(3600),
            "nats_url": url,
            "nats_ws_url": url,
        }),
    )
}

/// A syntactically valid JWT whose payload carries `exp` — the shape the platform's
/// bridge token has. Only the payload is read (`jwt_expiry`), and the fixture broker
/// never verifies the signature.
fn fixture_jwt(exp_offset: i64) -> String {
    let exp = unix_now() + exp_offset;
    let payload = B64.encode(json!({ "exp": exp }).to_string());
    format!("hdr.{payload}.sig")
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// A controlled WebSocket NATS broker
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum BrokerMode {
    /// Answer `CONNECT` with `-ERR 'Authorization Violation'` and close.
    Refuse,
    /// Accept the connection, then send the same error on the live link after
    /// `error_after` and keep the socket open (async-nats reconnects on its own).
    Live { error_after: Duration },
}

struct Broker {
    url: String,
    connects: Arc<AtomicUsize>,
}

impl Broker {
    fn start(mode: BrokerMode) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("broker runtime");
        let connects = Arc::new(AtomicUsize::new(0));
        let counter = connects.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<String>();
        std::thread::Builder::new()
            .name("fixture-broker".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                        .await
                        .expect("bind the fixture broker");
                    let port = listener.local_addr().expect("broker address").port();
                    let _ = ready_tx.send(format!("ws://127.0.0.1:{port}"));
                    loop {
                        let Ok((stream, _)) = listener.accept().await else {
                            return;
                        };
                        counter.fetch_add(1, Ordering::AcqRel);
                        tokio::spawn(serve_broker(stream, mode));
                    }
                })
            })
            .expect("spawn the fixture broker");
        let url = ready_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("broker address");
        Self { url, connects }
    }
}

async fn serve_broker(stream: tokio::net::TcpStream, mode: BrokerMode) {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    // The minimum NATS handshake over a WebSocket transport. `auth_required` makes the
    // CONNECT carry the JWT the broker is about to reject, which is the production shape
    // of a refused bridge credential.
    let info = concat!(
        "INFO {\"server_id\":\"FIXTURE\",\"server_name\":\"fixture-broker\",",
        "\"version\":\"2.10.2\",\"host\":\"127.0.0.1\",\"port\":0,",
        "\"max_payload\":1048576,\"proto\":1,\"headers\":true,",
        "\"auth_required\":true,\"nonce\":\"Zml4dHVyZQ==\"}\r\n"
    );
    if socket
        .send(Message::Binary(info.as_bytes().to_vec()))
        .await
        .is_err()
    {
        return;
    }
    let mut deadline = match mode {
        BrokerMode::Live { error_after } => Some(tokio::time::Instant::now() + error_after),
        BrokerMode::Refuse => None,
    };
    loop {
        let next = async {
            match deadline {
                Some(at) => {
                    tokio::select! {
                        biased;
                        _ = tokio::time::sleep_until(at) => None,
                        message = socket.next() => Some(message),
                    }
                }
                None => Some(socket.next().await),
            }
        };
        match next.await {
            None => {
                let _ = socket
                    .send(Message::Binary(
                        b"-ERR 'Authorization Violation'\r\n".to_vec(),
                    ))
                    .await;
                // A live generation keeps its socket after the refusal, which is what
                // the health callback observes; the refusal case ends the connect.
                if mode == BrokerMode::Refuse {
                    let _ = socket.close(None).await;
                    return;
                }
                deadline = None;
            }
            Some(Some(Ok(Message::Binary(data)))) => {
                let text = String::from_utf8_lossy(&data);
                if text.contains("CONNECT") && mode == BrokerMode::Refuse {
                    let _ = socket
                        .send(Message::Binary(
                            b"-ERR 'Authorization Violation'\r\n".to_vec(),
                        ))
                        .await;
                    let _ = socket.close(None).await;
                    return;
                }
                if text.lines().any(|line| line.starts_with("PING")) {
                    let _ = socket.send(Message::Binary(b"PONG\r\n".to_vec())).await;
                }
            }
            Some(Some(Ok(Message::Ping(payload)))) => {
                let _ = socket.send(Message::Pong(payload)).await;
            }
            Some(Some(Ok(Message::Close(_)))) | Some(None) | Some(Some(Err(_))) => return,
            Some(Some(Ok(_))) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// The child
// ---------------------------------------------------------------------------

struct ChildRun {
    child: Child,
    transcript: Arc<Mutex<String>>,
    home: PathBuf,
}

impl ChildRun {
    fn spawn(home: PathBuf) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_futureos-headless"));
        command.arg("--no-qr");
        // Hermetic: the fixture controls every variable the child reads. A developer
        // shell (or cargo's own environment) must not decide whether this child attaches
        // to a real Agent or to a real Desktop.
        command.env_clear();
        for key in [
            "PATH",
            "SHELL",
            "TMPDIR",
            "LANG",
            "LLVM_PROFILE_FILE",
            "RUST_BACKTRACE",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        let mut child = command
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("FUTURE_HOME", home.join(".future"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn futureos-headless");
        let transcript = Arc::new(Mutex::new(String::new()));
        for pipe in [
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let transcript = transcript.clone();
            std::thread::spawn(move || {
                let mut pipe = pipe;
                let mut buf = [0u8; 4096];
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => transcript
                            .lock()
                            .unwrap()
                            .push_str(&String::from_utf8_lossy(&buf[..n])),
                    }
                }
            });
        }
        Self {
            child,
            transcript,
            home,
        }
    }

    fn text(&self) -> String {
        self.transcript.lock().unwrap().clone()
    }

    fn wait_for(&self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.text().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.text().contains(needle)
    }

    /// Interrupt the child the way an operator does (Ctrl+C), then wait for its own exit.
    fn interrupt(&mut self) {
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGINT);
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ChildRun {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// The `--re-pair` entry point refuses to run without a terminal, so its child gets a pty
/// on stdin/stdout/stderr. `openpty` is the platform call the launcher record's test
/// achieves with `/usr/bin/script`; owning the child directly keeps signals simple.
struct PtyRun {
    child: Child,
    transcript: Arc<Mutex<String>>,
    home: PathBuf,
}

impl PtyRun {
    fn spawn(home: PathBuf, args: &[&str]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_futureos-headless"));
        command.args(args);
        command.env_clear();
        for key in [
            "PATH",
            "SHELL",
            "TMPDIR",
            "LANG",
            "LLVM_PROFILE_FILE",
            "RUST_BACKTRACE",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("FUTURE_HOME", home.join(".future"));
        let mut master: libc::c_int = -1;
        let mut slave: libc::c_int = -1;
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0, "openpty failed: {}", std::io::Error::last_os_error());
        use std::os::fd::FromRawFd;
        let slave_file = unsafe { std::fs::File::from_raw_fd(slave) };
        let master_file = unsafe { std::fs::File::from_raw_fd(master) };
        command
            .stdin(Stdio::from(
                slave_file.try_clone().expect("clone the pty slave"),
            ))
            .stdout(Stdio::from(
                slave_file.try_clone().expect("clone the pty slave"),
            ))
            .stderr(Stdio::from(slave_file));
        let child = command.spawn().expect("spawn futureos-headless on a pty");
        let transcript = Arc::new(Mutex::new(String::new()));
        let sink = transcript.clone();
        std::thread::spawn(move || {
            let mut master = master_file;
            let mut buf = [0u8; 4096];
            loop {
                match master.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => sink
                        .lock()
                        .unwrap()
                        .push_str(&String::from_utf8_lossy(&buf[..n])),
                }
            }
        });
        Self {
            child,
            transcript,
            home,
        }
    }

    fn text(&self) -> String {
        self.transcript.lock().unwrap().clone()
    }

    fn wait_for(&self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.text().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.text().contains(needle)
    }

    fn interrupt(&mut self) {
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGINT);
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PtyRun {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn isolated_home(tag: &str) -> PathBuf {
    // The path must stay under 104 characters: the child's agent IPC endpoint is
    // `<home>/.future/run/agent.sock`, and macOS `sun_path` rejects longer paths with
    // `EINVAL`, which `select_agent` (correctly) refuses to read as "absent".
    let home = std::env::temp_dir().join(format!("fos-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".future/agent")).expect("isolated data root");
    home
}

fn write_auth(home: &Path, platform_url: &str) {
    let auth = json!({
        "future": {
            "type": "api_key",
            "key": "fixture-account-key",
            "base_url": format!("{platform_url}/api"),
        }
    });
    std::fs::write(
        home.join(".future/agent/auth.json"),
        serde_json::to_vec(&auth).expect("encode auth"),
    )
    .expect("write auth.json");
}

/// A stored v2 pairing for `pair_id`. `confirm` decides whether the refresh worker is
/// allowed to run (`pairing_confirmed`), which is what keeps a fixture generation from
/// rotating its credential while the test drives the watcher.
fn write_pairing(home: &Path, nats_url: &str, confirm: bool) {
    let seed = nkeys::KeyPair::new_user()
        .seed()
        .expect("generate a user seed");
    let (private, public) =
        future_remote_crypto::generate_identity().expect("generate a pairing identity");
    let peer = future_remote_crypto::generate_identity()
        .expect("generate a peer identity")
        .1;
    let creds = json!({
        "handshakeVersion": 2,
        "secure": {
            "privateKey": B64.encode(private.as_slice()),
            "publicKey": B64.encode(public),
            "peerPublicKey": confirm.then(|| B64.encode(peer)),
            "secret": Value::Null,
            "expiresAt": unix_now() + 600,
        },
        "pairId": "pair_fixture",
        "desktopId": "desktop_fixture",
        "nkeySeed": seed,
        "userJwt": fixture_jwt(3600),
        "natsUrl": nats_url,
        "natsWsUrl": nats_url,
        "jwtExpiresAt": unix_now() + 3600,
    });
    std::fs::write(
        home.join(".future/remote_pairing.json"),
        serde_json::to_vec(&creds).expect("encode pairing"),
    )
    .expect("write remote_pairing.json");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A broker that answers CONNECT with `-ERR 'Authorization Violation'` is a
/// **non-retryable** start failure (`service_authorization`), and the headless entry
/// must report it — not merely attempt it. The refused credential is the one the platform
/// just issued, which is the production shape of a revoked bridge credential.
///
/// The same child also proves `spawn_revoke_cleanup`'s error arm: a corrupt
/// `remote_pending_revokes.json` makes `retry_pending_revokes()` fail, and the worker
/// records it instead of dying.
#[test]
fn a_broker_that_refuses_the_credential_is_reported() {
    let broker = Broker::start(BrokerMode::Refuse);
    let platform = Platform::start(
        vec![TokenReply::Credential {
            nats_url: broker.url.clone(),
            exp_offset: 3600,
        }],
        None,
    );
    let home = isolated_home("refused");
    write_auth(&home, &platform.url);
    write_pairing(&home, &broker.url, true);
    // `read_json_object` refuses a non-object root; the compensation loop must log it.
    std::fs::write(
        home.join(".future/remote_pending_revokes.json"),
        b"{ this is not json",
    )
    .expect("write corrupt revoke queue");

    let mut child = ChildRun::spawn(home);
    let reported = child.wait_for("Remote connection stopped", Duration::from_secs(30));
    let transcript = child.text();
    let status = child.child.wait().expect("wait for the child");

    assert!(
        reported,
        "the refused start must be reported by name; transcript:\n{transcript}"
    );
    assert!(
        transcript.contains("ServiceAuthorization"),
        "the report must name the authorization class; transcript:\n{transcript}"
    );
    assert!(
        !status.success(),
        "a terminal remote failure must fail the headless entry; transcript:\n{transcript}"
    );
    assert!(
        platform.token_calls() >= 1,
        "the child must have refreshed/obtained a credential from the fixture platform"
    );
    assert!(
        broker.connects.load(Ordering::Acquire) >= 1,
        "the broker must have seen the refused connection"
    );
    assert!(
        transcript.contains("local failure episode started"),
        "the corrupt rewoke queue must be recorded by the compensation worker; \
         transcript:\n{transcript}"
    );
    child.interrupt(); // already exited; keep the Drop path uniform
}

/// `--re-pair` from an unpaired root whose queued revoke cannot be read: both the
/// settlement pass in `re_pair`'s caller (headless's `unpair`) and `create_pairing`'s
/// `retire_pending_revoke` must **report** the unreadable file rather than panic, and the
/// start that follows is still refused by the controlled broker.
///
/// `remote_host/pairing.rs:261-263` is the retire log; `start.rs:648` is the second
/// `spawn_revoke_cleanup` (the worker is already running from `unpair`) returning early.
#[test]
fn a_re_pair_with_an_unreadable_revoke_queue_reports_and_still_refuses() {
    let broker = Broker::start(BrokerMode::Refuse);
    let platform = Platform::start(vec![], Some(broker.url.clone()));
    let home = isolated_home("repair");
    write_auth(&home, &platform.url);
    // No `remote_pairing.json`: `--re-pair` starts from an unpaired root.
    std::fs::write(
        home.join(".future/remote_pending_revokes.json"),
        b"{ this is not json",
    )
    .expect("write corrupt revoke queue");

    let mut child = PtyRun::spawn(home, &["--re-pair", "--no-qr"]);
    let reported = child.wait_for("Remote connection stopped", Duration::from_secs(30));
    let transcript = child.text();
    assert!(
        reported,
        "the start after the re-pair must still report the refusal; transcript:\n{transcript}"
    );
    assert!(
        transcript.contains("could not retire the queued revoke"),
        "create_pairing must report the unreadable queued revoke; transcript:\n{transcript}"
    );
    assert!(
        transcript.contains("Revoking the saved phone pairing"),
        "the child must have taken the --re-pair entry; transcript:\n{transcript}"
    );
    child.interrupt();
}

/// A live broker drives `GenerationWatch`: a healthy tick runs, then the broker rejects
/// the *live* connection. The bridge JWT is already expired, so `NatsHealth` records
/// `reconnect_required` (not the terminal `service_config_error`), and the watcher calls
/// `spawn_runtime_reconnect`, which reports `RT001` and runs `start_once(true)`. The
/// platform then answers the refresh with a body that cannot deserialize, which is that
/// worker's `Err` arm.
#[test]
fn a_live_broker_drives_the_generation_watch_and_reconnect() {
    let broker = Broker::start(BrokerMode::Live {
        error_after: Duration::from_secs(5),
    });
    let platform = Platform::start(
        vec![
            // `establish()` on the first start: the credential expires immediately, so a
            // live refusal is a transient generation failure rather than terminal.
            TokenReply::Credential {
                nats_url: broker.url.clone(),
                exp_offset: -60,
            },
            // Every later token request is reached but undecodable: the reconnect worker's
            // `Err` arm, and the credential-refresh worker's retry path.
            TokenReply::Malformed,
        ],
        None,
    );
    let home = isolated_home("live");
    write_auth(&home, &platform.url);
    write_pairing(&home, &broker.url, true);

    let mut child = ChildRun::spawn(home);
    assert!(
        child.wait_for("Remote: Ready", Duration::from_secs(30)),
        "the live broker must accept the generation; transcript:\n{}",
        child.text()
    );
    let rebuilt = child.wait_for("critical task stopped", Duration::from_secs(20));
    // The worker's `start_once(true)` contends with the credential-refresh worker for
    // `start_lock` (that worker sleeps up to `RETRY_WHILE_EXPIRED` while holding it), so
    // the `Err` arm can report several seconds after the rebuild line.
    let reported = child.wait_for("automatic bridge reconnect failed", Duration::from_secs(30));
    let transcript = child.text();
    assert!(
        rebuilt,
        "the live refusal must reach GenerationWatch -> spawn_runtime_reconnect; \
         transcript:\n{transcript}"
    );
    assert!(
        transcript.contains("rebuilding generation"),
        "the reconnect worker must report what it is doing; transcript:\n{transcript}"
    );
    assert!(
        reported,
        "the reconnect worker's failed `start_once` must be reported, not swallowed; \
         transcript:\n{transcript}"
    );
    assert!(
        broker.connects.load(Ordering::Acquire) >= 1,
        "the generation must have connected to the controlled broker"
    );
    child.interrupt();
    let after = child.text();
    assert!(
        after.contains("cleanup stage=remote"),
        "SIGINT must take the graceful stop path; transcript:\n{after}"
    );
}

/// A generation that stays healthy through `RUNTIME_HEALTHY_RESET_SECS` (60 s) runs the
/// watchdog's reset arm, and a later refusal whose reconnect *start* is a retryable
/// `server` failure takes `spawn_runtime_reconnect`'s `Ok(status) if retryable` arm,
/// which arms the start-retry worker instead of reporting a hard failure.
///
/// The 60-second dwell is not padding: `healthy_secs` is a counter over one-second watch
/// ticks, and the reset only happens when it reaches the production constant. The broker
/// is held healthy for 65 s so the counter has crossed 60 before the refusal.
#[test]
fn a_healthy_generation_resets_the_budget_and_a_retryable_reconnect_arms_the_retry() {
    let broker = Broker::start(BrokerMode::Live {
        error_after: Duration::from_secs(65),
    });
    let platform = Platform::start(
        vec![
            TokenReply::Credential {
                nats_url: broker.url.clone(),
                exp_offset: -60,
            },
            TokenReply::ServerError,
        ],
        None,
    );
    let home = isolated_home("healthy");
    write_auth(&home, &platform.url);
    write_pairing(&home, &broker.url, true);

    let mut child = ChildRun::spawn(home);
    assert!(
        child.wait_for("Remote: Ready", Duration::from_secs(30)),
        "the live broker must accept the generation; transcript:\n{}",
        child.text()
    );
    let rebuilt = child.wait_for("critical task stopped", Duration::from_secs(90));
    let transcript = child.text();
    assert!(
        rebuilt,
        "the refusal after the healthy dwell must still reach the watcher; \
         transcript:\n{transcript}"
    );
    assert!(
        transcript.contains("rebuilding generation"),
        "the reconnect worker must report what it is doing; transcript:\n{transcript}"
    );
    // The retryable arm hands the failure to the start-retry worker; its own attempt is
    // what records the `server` episode. Wait for it so the worker is not interrupted
    // inside `start_once`.
    let retried = child.wait_for("server failure episode started", Duration::from_secs(25));
    assert!(
        retried,
        "a retryable reconnect must arm the start-retry worker; transcript:\n{}",
        child.text()
    );
    child.interrupt();
}

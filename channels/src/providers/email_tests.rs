// Unit tests for the email provider.
//
// These cover what is platform-specific and easy to get wrong: the SMTP
// submission conversation (dot-stuffing, CRLF framing, AUTH PLAIN and AUTH
// LOGIN, the STARTTLS requirement), the IMAP conversation (tagged commands,
// synchronizing literals, UID FETCH pairing), the MIME subset (multipart,
// quoted-printable, base64, RFC 2047, charset decoding, HTML reduction),
// address and date parsing, the loop-safety gates, and the delivered-message
// store that keeps a restart from answering the same mail twice.
//
// Nothing here touches a live mailbox: every conversation runs against a
// scripted TCP server, and the agent address is a closed port.

use super::*;
use crate::test_support::{temp_dir, wait_until};
use serde_json::json;
use std::sync::Mutex as StdMutex;

// ─── a scripted line server ────────────────────────────────────────────────

/// One step of a scripted conversation.
enum Step {
    /// Send these lines before reading anything (a greeting).
    Greet { lines: Vec<&'static str> },
    /// Assert the next client line starts with `expect`, then reply.
    Reply {
        expect: &'static str,
        lines: Vec<&'static str>,
    },
    /// The same, for an expectation and a reply built at runtime (a base64
    /// challenge, for instance).
    ReplyOwned { expect: String, lines: Vec<String> },
    /// Assert the next client line, then answer with a `FETCH` record whose body
    /// arrives in a second write, after the response line announcing it — the
    /// shape a body larger than one TCP segment has.
    FetchSplit {
        expect: &'static str,
        uid: u32,
        payload: Vec<u8>,
        completion: &'static str,
    },
    /// Assert the next client line, then announce a literal of `size` bytes
    /// without sending them.
    AnnounceLiteral {
        expect: &'static str,
        uid: u32,
        size: usize,
    },
    /// Assert the next client line, then answer with a `FETCH` record whose body
    /// is a synchronizing literal.
    Fetch {
        expect: &'static str,
        uid: u32,
        payload: Vec<u8>,
        completion: &'static str,
    },
    /// Read `DATA` lines until the terminating dot, then reply. Every line the
    /// client sent is recorded, which is how the framing assertions work.
    ReadData { reply: &'static str },
    /// Close the connection without another word.
    Close,
}

/// A scripted server, and everything it saw.
struct Mock {
    port: u16,
    lines: Arc<StdMutex<Vec<String>>>,
    /// Protocol-level complaints: a line that did not match the script, or one
    /// that did not end in CRLF.
    errors: Arc<StdMutex<Vec<String>>>,
}

impl Mock {
    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }

    fn errors(&self) -> Vec<String> {
        self.errors.lock().unwrap().clone()
    }

    /// Fail the test if the script and the client disagreed anywhere.
    fn assert_clean(&self) {
        assert!(self.errors().is_empty(), "{:?}", self.errors());
    }

    fn saw(&self, line: &str) -> bool {
        self.lines().iter().any(|recorded| recorded == line)
    }
}

fn note(errors: &Lines, message: String) {
    errors.lock().unwrap().push(message);
}

/// What a scripted server recorded, and what it complained about.
type Lines = Arc<StdMutex<Vec<String>>>;

async fn spawn(script: Vec<Step>) -> Mock {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

    /// One line from the client, or `None` when it hung up.
    async fn read_line(
        reader: &mut BufReader<OwnedReadHalf>,
        lines: &Lines,
        errors: &Lines,
    ) -> Option<String> {
        let mut raw = String::new();
        match reader.read_line(&mut raw).await {
            Ok(0) => None,
            Ok(_) => {
                if !raw.ends_with("\r\n") {
                    note(errors, format!("line not CRLF terminated: {raw:?}"));
                }
                let line = raw.trim_end_matches(['\r', '\n']).to_string();
                lines.lock().unwrap().push(line.clone());
                Some(line)
            }
            Err(error) => {
                note(errors, format!("read failed: {error}"));
                None
            }
        }
    }

    /// Assert the next client line against the script.
    ///
    /// A mismatch is reported *and* answered before the task returns: a mock
    /// that simply went away would leave the client blocked on a reply that is
    /// never coming, which turns a test bug into a hang.
    async fn expect(
        reader: &mut BufReader<OwnedReadHalf>,
        lines: &Lines,
        errors: &Lines,
        prefix: &str,
    ) -> bool {
        match read_line(reader, lines, errors).await {
            Some(line) if line.starts_with(prefix) => true,
            Some(line) => {
                note(
                    errors,
                    format!("expected `{prefix}`, got `{line}` (the client sent: {lines:?})"),
                );
                false
            }
            None => {
                note(errors, format!("expected `{prefix}`, then EOF"));
                false
            }
        }
    }

    async fn send(writer: &mut OwnedWriteHalf, payload: &[u8], errors: &Lines) {
        if let Err(error) = writer.write_all(payload).await {
            note(errors, format!("write failed: {error}"));
        }
        let _ = writer.flush().await;
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    let lines: Lines = Arc::new(StdMutex::new(Vec::new()));
    let errors: Lines = Arc::new(StdMutex::new(Vec::new()));
    let task_lines = lines.clone();
    let task_errors = errors.clone();

    tokio::spawn(async move {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let (read_half, mut writer) = socket.into_split();
        let mut reader = BufReader::new(read_half);

        for step in script {
            match step {
                Step::Greet { lines: greeting } => {
                    let mut payload = String::new();
                    for line in greeting {
                        payload.push_str(line);
                        payload.push_str("\r\n");
                    }
                    send(&mut writer, payload.as_bytes(), &task_errors).await;
                }
                Step::Reply {
                    expect: prefix,
                    lines: reply,
                } => {
                    if !expect(&mut reader, &task_lines, &task_errors, prefix).await {
                        // Answer with a refusal so the client fails with a
                        // message rather than waiting for a reply forever.
                        send(&mut writer, b"500 unexpected line\r\n", &task_errors).await;
                        return;
                    }
                    let mut payload = String::new();
                    for line in reply {
                        payload.push_str(line);
                        payload.push_str("\r\n");
                    }
                    send(&mut writer, payload.as_bytes(), &task_errors).await;
                }
                Step::ReplyOwned {
                    expect: prefix,
                    lines: reply,
                } => {
                    if !expect(&mut reader, &task_lines, &task_errors, &prefix).await {
                        send(&mut writer, b"500 unexpected line\r\n", &task_errors).await;
                        return;
                    }
                    let mut payload = String::new();
                    for line in reply {
                        payload.push_str(&line);
                        payload.push_str("\r\n");
                    }
                    send(&mut writer, payload.as_bytes(), &task_errors).await;
                }
                Step::Fetch {
                    expect: prefix,
                    uid,
                    payload,
                    completion,
                } => {
                    if !expect(&mut reader, &task_lines, &task_errors, prefix).await {
                        send(&mut writer, b"a1 BAD unexpected line\r\n", &task_errors).await;
                        return;
                    }
                    // The `{n}` marker ends the response line: the octets follow
                    // it directly, and the rest of the item list comes after.
                    let mut bytes =
                        format!("* 1 FETCH (UID {uid} BODY[] {{{}}}\r\n", payload.len())
                            .into_bytes();
                    bytes.extend_from_slice(&payload);
                    bytes.extend_from_slice(b")\r\n");
                    bytes.extend_from_slice(format!("{completion}\r\n").as_bytes());
                    send(&mut writer, &bytes, &task_errors).await;
                }
                Step::FetchSplit {
                    expect: prefix,
                    uid,
                    payload,
                    completion,
                } => {
                    if !expect(&mut reader, &task_lines, &task_errors, prefix).await {
                        return;
                    }
                    let header = format!("* 1 FETCH (UID {uid} BODY[] {{{}}}\r\n", payload.len());
                    send(&mut writer, header.as_bytes(), &task_errors).await;
                    // A separate write, so the client has to come back for the
                    // octets rather than finding them already buffered.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    let mut tail = payload.clone();
                    tail.extend_from_slice(b")\r\n");
                    tail.extend_from_slice(format!("{completion}\r\n").as_bytes());
                    send(&mut writer, &tail, &task_errors).await;
                }
                Step::AnnounceLiteral {
                    expect: prefix,
                    uid,
                    size,
                } => {
                    if !expect(&mut reader, &task_lines, &task_errors, prefix).await {
                        return;
                    }
                    let header = format!("* 1 FETCH (UID {uid} BODY[] {{{size}}}\r\n");
                    send(&mut writer, header.as_bytes(), &task_errors).await;
                }
                Step::ReadData { reply } => {
                    loop {
                        match read_line(&mut reader, &task_lines, &task_errors).await {
                            Some(line) if line == "." => break,
                            Some(_) => {}
                            None => return,
                        }
                    }
                    send(&mut writer, format!("{reply}\r\n").as_bytes(), &task_errors).await;
                }
                Step::Close => return,
            }
        }
    });

    Mock {
        port,
        lines,
        errors,
    }
}

// ─── helpers ───────────────────────────────────────────────────────────────

/// The read deadline every scripted test uses. Short enough that a mock which
/// misses a line is a fast failure, long enough that a loopback round trip
/// never trips it.
const TEST_TIMEOUT_SECONDS: u64 = 1;

/// A configuration pointed at `imap_port`/`smtp_port` on the loopback, with no
/// TLS and no authentication — the two things the scripted servers cannot do —
/// and the short read deadline above.
fn config_for(imap_port: u16, smtp_port: u16) -> EmailConfig {
    EmailConfig {
        enabled: true,
        imap: ImapConfig {
            host: "127.0.0.1".into(),
            port: imap_port,
            username: "bot@example.com".into(),
            password: "secret".into(),
            // No TLS either: a scripted loopback server cannot handshake.
            security: Security::Plain,
            timeout_seconds: TEST_TIMEOUT_SECONDS,
            ..ImapConfig::default()
        },
        smtp: SmtpConfig {
            host: "127.0.0.1".into(),
            port: smtp_port,
            from: "bot@example.com".into(),
            security: Security::Plain,
            timeout_seconds: TEST_TIMEOUT_SECONDS,
            ..SmtpConfig::default()
        },
        poll_seconds: 60,
        sender_allowlist: Vec::new(),
        subject_prefix: String::new(),
    }
}

fn config_value(imap_port: u16, smtp_port: u16) -> serde_json::Value {
    json!({
        "enabled": true,
        "imap": {
            "host": "127.0.0.1",
            "port": imap_port,
            "username": "bot@example.com",
            "password": "secret",
            "mailbox": "INBOX",
            "security": "plain",
            "timeout_seconds": TEST_TIMEOUT_SECONDS,
        },
        "smtp": {
            "host": "127.0.0.1",
            "port": smtp_port,
            "from": "bot@example.com",
            "security": "plain",
            "timeout_seconds": TEST_TIMEOUT_SECONDS,
        },
        "poll_seconds": 60,
        "dm_policy": "open",
    })
}

fn ctx_with(
    value: serde_json::Value,
    shutdown: std::sync::Arc<crate::bridge::Shutdown>,
) -> ProviderCtx {
    // Nothing listens on port 1: the agent is unreachable, which is how the
    // bridge tests stay deterministic.
    ctx_at(
        temp_dir("email-provider"),
        "http://127.0.0.1:1",
        value,
        shutdown,
    )
}

/// A context over a chosen data directory and agent address.
fn ctx_at(
    data_dir: std::path::PathBuf,
    grpc_addr: &str,
    value: serde_json::Value,
    shutdown: std::sync::Arc<crate::bridge::Shutdown>,
) -> ProviderCtx {
    let sessions = Arc::new(crate::session_store::SessionStore::new(
        data_dir.join("sessions.json"),
    ));
    let bridge = crate::bridge::Bridge::new(
        Arc::new(crate::config::AgentConfig {
            grpc_addr: grpc_addr.to_string(),
            cwd: data_dir.to_string_lossy().into_owned(),
            ..crate::config::AgentConfig::default()
        }),
        crate::policy::AccessPolicyConfig {
            dm_policy: "open".into(),
            ..Default::default()
        },
        data_dir.clone(),
        Arc::new(crate::status::StatusBoard::new(
            data_dir.join("status.json"),
        )),
    );
    ProviderCtx::new(
        &DEFINITION,
        value,
        bridge,
        data_dir.clone(),
        sessions,
        shutdown,
    )
}

/// A raw message: headers, a blank line, and the body.
/// A raw message: headers, a blank line, and the body. A header block that
/// already ends with a line break is not given a second one — that would leave a
/// blank line at the start of the body.
fn raw_message(headers: &str, body: &str) -> Vec<u8> {
    format!("{}\r\n\r\n{body}", headers.trim_end_matches(['\r', '\n'])).into_bytes()
}

fn mail_from(headers: &str, body: &str) -> Mail {
    parse_mail(&raw_message(headers, body))
}

/// A connected pair of loopback sockets, for the tests that drive a socket by
/// hand rather than through a scripted server.
async fn tcp_pair() -> (tokio::net::TcpStream, tokio::net::TcpStream) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let accept = tokio::spawn(async move { listener.accept().await.expect("accept").0 });
    let client = tokio::net::TcpStream::connect(addr).await.expect("connect");
    (client, accept.await.expect("server socket"))
}

/// A port with nothing listening on it.
async fn closed_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    port
}

// ─── TLS ───────────────────────────────────────────────────────────────────
//
// The production client verifies against the platform trust store, which cannot
// know a certificate invented for a test. These tests therefore complete a real
// handshake (record layer, key schedule and signatures are all still checked)
// against a server certificate they trust explicitly, which is the only way the
// encrypted half of the wire can be exercised with no network access.

/// A self-signed certificate for these tests, DER-encoded, with `localhost` and
/// `127.0.0.1` as its subject alternative names.
const TEST_CERT_DER_B64: &str = "MIIBrzCCAVWgAwIBAgIUTo6DIUaX8b06BI4JHviCGuptlUwwCgYIKoZIzj0EAwIwHjEcMBoGA1UEAwwTZnV0dXJlLWNoYW5uZWwtdGVzdDAgFw0yNjA5MjIwNjI5NDdaGA8yMTI2MDgyOTA2Mjk0N1owHjEcMBoGA1UEAwwTZnV0dXJlLWNoYW5uZWwtdGVzdDBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABMzcFkuHNdFW0xXd/d8NDu9sewfw+Bv8AUZLTKBTrMsWoHbIkP4T6KszD8lKdNPYLipoR4QgU1VNR3+Lgls3FrWjbzBtMB0GA1UdDgQWBBRONFcKCYpkuNjfPAZTXbNXbRI4FTAfBgNVHSMEGDAWgBRONFcKCYpkuNjfPAZTXbNXbRI4FTAPBgNVHRMBAf8EBTADAQH/MBoGA1UdEQQTMBGCCWxvY2FsaG9zdIcEfwAAATAKBggqhkjOPQQDAgNIADBFAiEA8FeFylkQ9Gp6sgR7j0VMDBTvKu/pi/Y6nCkR8jlLVO0CIFobqdkjT5zXDvUOncTfHH/uADXTx9zbjyeL9je5pi/2";

/// The PKCS#8 private key for [`TEST_CERT_DER_B64`].
const TEST_KEY_DER_B64: &str = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgeEz6v+3qq6a6mV9Uka3i4huh571HFgCedwK7X0EhFt6hRANCAATM3BZLhzXRVtMV3f3fDQ7vbHsH8Pgb/AFGS0ygU6zLFqB2yJD+E+irMw/JSnTT2C4qaEeEIFNVTUd/i4JbNxa1";

fn decode_fixture(encoded: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("the embedded fixture is valid base64")
}

/// Accepts the certificate above and nothing else is asserted about trust.
///
/// Signature verification still runs, so a wrong key or a tampered record is
/// still rejected.
#[derive(Debug)]
struct TrustTestCertificate;

impl rustls::client::danger::ServerCertVerifier for TrustTestCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &test_algorithms())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &test_algorithms())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        test_algorithms().supported_schemes()
    }
}

fn test_algorithms() -> rustls::crypto::WebPkiSupportedAlgorithms {
    rustls::crypto::aws_lc_rs::default_provider().signature_verification_algorithms
}

/// The client TLS configuration these tests use instead of the platform store.
fn trusting_tls() -> rustls::ClientConfig {
    crate::test_support::ensure_crypto_provider();
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TrustTestCertificate))
        .with_no_client_auth()
}

fn server_tls() -> Arc<rustls::ServerConfig> {
    crate::test_support::ensure_crypto_provider();
    Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(decode_fixture(
                    TEST_CERT_DER_B64,
                ))],
                rustls::pki_types::PrivateKeyDer::Pkcs8(
                    rustls::pki_types::PrivatePkcs8KeyDer::from(decode_fixture(TEST_KEY_DER_B64)),
                ),
            )
            .expect("server TLS configuration"),
    )
}

/// The server side of a TLS conversation.
///
/// rustls works on byte buffers rather than sockets, so the two directions are
/// moved explicitly: [`TlsPeer::fill`] pushes socket bytes into the connection,
/// [`TlsPeer::flush`] drains its records back out.
struct TlsPeer {
    conn: rustls::ServerConnection,
    read: tokio::net::tcp::OwnedReadHalf,
    write: tokio::net::tcp::OwnedWriteHalf,
    plain: Vec<u8>,
}

impl TlsPeer {
    /// Complete the handshake on an accepted socket.
    async fn accept(
        read: tokio::net::tcp::OwnedReadHalf,
        write: tokio::net::tcp::OwnedWriteHalf,
    ) -> Option<Self> {
        let mut peer = Self {
            conn: rustls::ServerConnection::new(server_tls()).ok()?,
            read,
            write,
            plain: Vec::new(),
        };
        while peer.conn.is_handshaking() {
            if peer.fill().await.ok()? == 0 {
                return None;
            }
        }
        Some(peer)
    }

    async fn fill(&mut self) -> std::io::Result<usize> {
        use tokio::io::AsyncReadExt;
        let mut raw = [0u8; 8192];
        let read = self.read.read(&mut raw).await?;
        if read == 0 {
            return Ok(0);
        }
        let mut cursor = std::io::Cursor::new(&raw[..read]);
        while (cursor.position() as usize) < read {
            let consumed = self.conn.read_tls(&mut cursor)?;
            if consumed == 0 {
                break;
            }
            self.conn
                .process_new_packets()
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        }
        self.flush().await?;
        Ok(read)
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;
        while self.conn.wants_write() {
            let mut out = Vec::new();
            self.conn.write_tls(&mut out)?;
            if out.is_empty() {
                break;
            }
            self.write.write_all(&out).await?;
        }
        self.write.flush().await
    }

    /// One decrypted CRLF line, or `None` when the client went away.
    async fn read_line(&mut self) -> Option<String> {
        loop {
            let mut buf = [0u8; 4096];
            match std::io::Read::read(&mut self.conn.reader(), &mut buf) {
                Ok(0) => return None,
                Ok(read) => self.plain.extend_from_slice(&buf[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return None,
            }
            if let Some(index) = self.plain.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.plain.drain(..=index).collect();
                let line = &line[..line.len() - 1];
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                return Some(String::from_utf8_lossy(line).into_owned());
            }
            if self.fill().await.ok()? == 0 {
                return None;
            }
        }
    }

    async fn send_line(&mut self, line: &str) -> bool {
        let mut payload = line.as_bytes().to_vec();
        payload.extend_from_slice(b"\r\n");
        std::io::Write::write_all(&mut self.conn.writer(), &payload).is_ok()
            && self.flush().await.is_ok()
    }
}

/// One step of a conversation that starts in the clear and may be upgraded.
enum TlsStep {
    /// Send these lines before reading anything.
    Greet { lines: Vec<&'static str> },
    /// Assert a cleartext line, then answer in the clear.
    Reply {
        expect: &'static str,
        lines: Vec<&'static str>,
    },
    /// Complete a TLS handshake on the same socket, as `STARTTLS` does.
    Upgrade,
    /// Assert a decrypted line, then answer encrypted.
    Secure {
        expect: &'static str,
        lines: Vec<&'static str>,
    },
    /// Read the encrypted body of a message until its terminating dot, then
    /// reply — the shape after `DATA` was accepted.
    SecureData { reply: &'static str },
    /// Go away without another word.
    Close,
}

/// A scripted server that can switch to TLS partway through.
async fn spawn_tls(script: Vec<TlsStep>) -> Mock {
    use tokio::io::AsyncWriteExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    let lines: Lines = Arc::new(StdMutex::new(Vec::new()));
    let errors: Lines = Arc::new(StdMutex::new(Vec::new()));
    let task_lines = lines.clone();
    let task_errors = errors.clone();

    tokio::spawn(async move {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let (read, write) = socket.into_split();
        let mut cleartext = Some((read, write));
        let mut secure: Option<TlsPeer> = None;

        // One byte at a time and no buffering: anything already pulled out of
        // the socket would be lost across an upgrade.
        async fn read_cleartext(
            read: &mut tokio::net::tcp::OwnedReadHalf,
            lines: &Lines,
            errors: &Lines,
        ) -> Option<String> {
            use tokio::io::AsyncReadExt;
            let mut raw = Vec::new();
            loop {
                let mut byte = [0u8; 1];
                match read.read(&mut byte).await {
                    Ok(0) => return None,
                    Ok(_) => {
                        raw.push(byte[0]);
                        if byte[0] == b'\n' {
                            break;
                        }
                    }
                    Err(error) => {
                        note(errors, format!("read failed: {error}"));
                        return None;
                    }
                }
            }
            let line = String::from_utf8_lossy(&raw)
                .trim_end_matches(['\r', '\n'])
                .to_string();
            lines.lock().unwrap().push(line.clone());
            Some(line)
        }

        for step in script {
            match step {
                TlsStep::Greet { lines: greeting } => {
                    let Some((_, write)) = cleartext.as_mut() else {
                        return;
                    };
                    let mut payload = String::new();
                    for line in greeting {
                        payload.push_str(line);
                        payload.push_str("\r\n");
                    }
                    if write.write_all(payload.as_bytes()).await.is_err() {
                        return;
                    }
                    let _ = write.flush().await;
                }
                TlsStep::Reply {
                    expect: prefix,
                    lines: reply,
                } => {
                    let Some((read, write)) = cleartext.as_mut() else {
                        note(&task_errors, "a cleartext step after the upgrade".into());
                        return;
                    };
                    match read_cleartext(read, &task_lines, &task_errors).await {
                        Some(line) if line.starts_with(prefix) => {}
                        Some(line) => {
                            note(&task_errors, format!("expected `{prefix}`, got `{line}`"));
                            return;
                        }
                        None => {
                            note(&task_errors, format!("expected `{prefix}`, then EOF"));
                            return;
                        }
                    }
                    let mut payload = String::new();
                    for line in reply {
                        payload.push_str(line);
                        payload.push_str("\r\n");
                    }
                    if write.write_all(payload.as_bytes()).await.is_err() {
                        return;
                    }
                    let _ = write.flush().await;
                }
                TlsStep::Upgrade => {
                    let Some((read, write)) = cleartext.take() else {
                        note(&task_errors, "a second upgrade".into());
                        return;
                    };
                    match TlsPeer::accept(read, write).await {
                        Some(peer) => secure = Some(peer),
                        None => {
                            note(&task_errors, "the handshake did not complete".into());
                            return;
                        }
                    }
                }
                TlsStep::Secure {
                    expect: prefix,
                    lines: reply,
                } => {
                    let Some(peer) = secure.as_mut() else {
                        note(&task_errors, "an encrypted step before the upgrade".into());
                        return;
                    };
                    let Some(line) = peer.read_line().await else {
                        note(&task_errors, format!("expected `{prefix}`, then EOF"));
                        return;
                    };
                    // Decrypted lines are recorded like cleartext ones, so a
                    // test can assert on what actually crossed the wire.
                    task_lines.lock().unwrap().push(line.clone());
                    if !line.starts_with(prefix) {
                        note(&task_errors, format!("expected `{prefix}`, got `{line}`"));
                        return;
                    }
                    for line in reply {
                        if !peer.send_line(line).await {
                            return;
                        }
                    }
                }
                TlsStep::SecureData { reply } => {
                    let Some(peer) = secure.as_mut() else {
                        note(&task_errors, "an encrypted step before the upgrade".into());
                        return;
                    };
                    loop {
                        match peer.read_line().await {
                            Some(line) => {
                                task_lines.lock().unwrap().push(line.clone());
                                if line == "." {
                                    break;
                                }
                            }
                            None => return,
                        }
                    }
                    if !peer.send_line(reply).await {
                        return;
                    }
                }
                TlsStep::Close => return,
            }
        }
    });

    Mock {
        port,
        lines,
        errors,
    }
}

// ─── SMTP submission ───────────────────────────────────────────────────────

/// The full submission of a reply: `data` is every line the client sent after
/// `DATA`, which is what the framing assertions inspect.
fn data_section(lines: &[String]) -> Vec<String> {
    let start = lines
        .iter()
        .position(|line| line == "DATA")
        .expect("DATA was sent");
    let end = lines
        .iter()
        .position(|line| line == ".")
        .expect("the message was terminated");
    lines[start + 1..end].to_vec()
}

#[tokio::test]
async fn a_reply_is_submitted_with_threading_headers_and_dot_stuffing() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            lines: vec!["250-mock greets you", "250 SIZE 10485760"],
        },
        Step::Reply {
            expect: "MAIL FROM:<bot@example.com>",
            lines: vec!["250 sender ok"],
        },
        Step::Reply {
            expect: "RCPT TO:<alice@example.com>",
            lines: vec!["250 recipient ok"],
        },
        Step::Reply {
            expect: "DATA",
            lines: vec!["354 go ahead"],
        },
        Step::ReadData {
            reply: "250 queued as 1A2B",
        },
        Step::Reply {
            expect: "QUIT",
            lines: vec!["221 bye"],
        },
        Step::Close,
    ])
    .await;

    let sender = EmailSender::new(config_for(mock.port, mock.port));
    let conversation = ConversationRef {
        id: "alice@example.com".into(),
        thread_id: Some("root-1@example.com".into()),
        kind: ChatKind::Direct,
    };
    sender.remember_subject(&conversation.key("email"), "Deploy question");
    let sent = sender
        .send_text(&conversation, "line one\n.hidden\n\nlast")
        .await
        .expect("the submission succeeds");

    // Email hands back no id the bridge could edit later, and `edit` is not
    // advertised.
    assert_eq!(sent, None);
    mock.assert_clean();

    let lines = mock.lines();
    assert_eq!(lines[0], "EHLO example.com");
    assert_eq!(lines[1], "MAIL FROM:<bot@example.com>");
    assert_eq!(lines[2], "RCPT TO:<alice@example.com>");
    assert_eq!(lines[3], "DATA");
    assert_eq!(*lines.last().unwrap(), "QUIT");

    let data = data_section(&lines);
    assert!(
        data.contains(&"From: <bot@example.com>".to_string()),
        "{data:?}"
    );
    assert!(
        data.contains(&"To: <alice@example.com>".to_string()),
        "{data:?}"
    );
    assert!(
        data.contains(&"Subject: Re: Deploy question".to_string()),
        "{data:?}"
    );
    assert!(
        data.contains(&"In-Reply-To: <root-1@example.com>".to_string()),
        "{data:?}"
    );
    assert!(
        data.contains(&"References: <root-1@example.com>".to_string()),
        "{data:?}"
    );
    assert!(
        data.contains(&"Auto-Submitted: auto-replied".to_string()),
        "{data:?}"
    );
    assert!(
        data.contains(&"Content-Transfer-Encoding: 7bit".to_string()),
        "{data:?}"
    );
    // The dot-stuffed line, and the body kept whole.
    assert!(data.contains(&"..hidden".to_string()), "{data:?}");
    assert!(data.contains(&"line one".to_string()), "{data:?}");
    assert!(data.contains(&"last".to_string()), "{data:?}");
}

#[tokio::test]
async fn auth_plain_is_sent_as_one_initial_response() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            lines: vec!["250-mock", "250 AUTH PLAIN LOGIN"],
        },
        Step::Reply {
            expect: "AUTH PLAIN ",
            lines: vec!["235 authenticated"],
        },
        Step::Reply {
            expect: "QUIT",
            lines: vec!["221 bye"],
        },
        Step::Close,
    ])
    .await;

    let mut config = config_for(mock.port, mock.port);
    config.imap.port = 1;
    config.smtp.username = "bot@example.com".into();
    config.smtp.password = "secret".into();
    let session = SmtpSession::connect(&config.smtp).await.expect("connect");
    mock.assert_clean();
    assert!(mock.saw(&format!(
        "AUTH PLAIN {}",
        base64_encode(b"\0bot@example.com\0secret")
    )));
    drop(session);
}

#[tokio::test]
async fn auth_login_answers_both_challenges() {
    // The challenge payloads and the expected answers are built at runtime, so
    // this script uses the owned steps.
    let user = base64_encode(b"bot@example.com");
    let password = base64_encode(b"secret");
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            lines: vec!["250-mock", "250 AUTH LOGIN"],
        },
        Step::ReplyOwned {
            expect: "AUTH LOGIN".into(),
            lines: vec![format!("334 {}", base64_encode(b"Username:"))],
        },
        Step::ReplyOwned {
            expect: user.clone(),
            lines: vec![format!("334 {}", base64_encode(b"Password:"))],
        },
        Step::ReplyOwned {
            expect: password.clone(),
            lines: vec!["235 accepted".into()],
        },
        Step::Reply {
            expect: "QUIT",
            lines: vec!["221 bye"],
        },
        Step::Close,
    ])
    .await;

    let mut config = config_for(mock.port, mock.port);
    config.smtp.username = "bot@example.com".into();
    config.smtp.password = "secret".into();
    let session = SmtpSession::connect(&config.smtp).await.expect("connect");
    mock.assert_clean();
    assert!(mock.saw(&user), "{:?}", mock.lines());
    assert!(mock.saw(&password), "{:?}", mock.lines());
    drop(session);
}

#[tokio::test]
async fn a_rejected_password_is_fatal_and_permanent() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            lines: vec!["250-mock", "250 AUTH PLAIN"],
        },
        Step::Reply {
            expect: "AUTH PLAIN ",
            lines: vec!["535 5.7.8 bad credentials"],
        },
        Step::Close,
    ])
    .await;

    let mut config = config_for(mock.port, mock.port);
    config.smtp.username = "bot@example.com".into();
    config.smtp.password = "wrong".into();
    let error = SmtpSession::connect(&config.smtp)
        .await
        .expect_err("bad credentials must fail");
    let text = error.to_string();
    assert!(text.contains("unauthorized"), "{text}");
    // The delivery queue must not keep retrying credentials that will never work.
    assert!(crate::delivery::is_permanent_error(&text), "{text}");
}

#[tokio::test]
async fn starttls_is_required_when_it_is_configured() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            lines: vec!["250-mock", "250 SIZE 100000"],
        },
        Step::Close,
    ])
    .await;

    let mut config = config_for(mock.port, mock.port);
    config.smtp.security = Security::Starttls;
    let error = SmtpSession::connect(&config.smtp)
        .await
        .expect_err("a server without STARTTLS must not be used");
    assert!(error.is::<FatalError>(), "{error}");
    assert!(error.to_string().contains("STARTTLS"), "{error}");
}

#[tokio::test]
async fn an_offered_starttls_is_taken_before_authenticating() {
    // The mock answers 220 and then stops: the client is expected to start a TLS
    // handshake, and a peer that says nothing closes the connection, which is
    // exactly the "handshake never completed" path.
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            lines: vec!["250-mock", "250-STARTTLS", "250 AUTH PLAIN"],
        },
        Step::Reply {
            expect: "STARTTLS",
            lines: vec!["220 ready to start TLS"],
        },
        Step::Close,
    ])
    .await;

    let mut config = config_for(mock.port, mock.port);
    config.smtp.security = Security::Starttls;
    config.smtp.username = "bot@example.com".into();
    config.smtp.password = "secret".into();
    let error = SmtpSession::connect(&config.smtp)
        .await
        .expect_err("the handshake cannot complete against the mock");
    assert!(
        !error.to_string().contains("does not offer STARTTLS"),
        "{error}"
    );
    assert!(mock.saw("STARTTLS"), "{:?}", mock.lines());
    // Credentials are only sent after the upgrade, never before it.
    assert!(
        !mock.lines().iter().any(|line| line.starts_with("AUTH ")),
        "{:?}",
        mock.lines()
    );
}

#[test]
fn a_recipient_refusal_is_permanent_for_the_delivery_queue() {
    let reply = SmtpReply {
        code: 550,
        lines: vec!["550 5.1.1 no such user".into()],
    };
    let error = smtp_failure("RCPT TO `nobody@example.com`", &reply).to_string();
    assert!(error.contains("recipient not found"), "{error}");
    assert!(crate::delivery::is_permanent_error(&error), "{error}");

    // A 4xx stays retryable: the mailbox may have room in a minute.
    let transient = SmtpReply {
        code: 451,
        lines: vec!["451 try again later".into()],
    };
    let text = smtp_failure("DATA", &transient).to_string();
    assert!(!crate::delivery::is_permanent_error(&text), "{text}");
}

#[test]
fn a_server_without_a_mechanism_we_speak_is_refused() {
    let error = select_mechanism(&["XOAUTH2".to_string()]).expect_err("no usable mechanism");
    assert!(error.is::<FatalError>(), "{error}");
    assert!(error.to_string().contains("XOAUTH2"), "{error}");
    assert!(error.to_string().contains("unauthorized"), "{error}");

    assert_eq!(
        select_mechanism(&["LOGIN".into(), "PLAIN".into()]).unwrap(),
        AuthMechanism::Plain
    );
    assert_eq!(
        select_mechanism(&["LOGIN".into()]).unwrap(),
        AuthMechanism::Login
    );
}

#[test]
fn smtp_replies_are_parsed_including_multiline_and_junk() {
    assert_eq!(parse_reply_code("250 ok"), Some(250));
    assert_eq!(parse_reply_code("220-extra"), Some(220));
    assert_eq!(parse_reply_code("abc"), None);
    assert_eq!(parse_reply_code(""), None);

    let reply = SmtpReply {
        code: 250,
        lines: vec![
            "250-mock.example.com greets you".into(),
            "250-STARTTLS".into(),
            "250 AUTH PLAIN LOGIN".into(),
        ],
    };
    assert!(reply.is_ok());
    // The greeting line is not a capability, and a capability is its name
    // without the reply code that carried it.
    assert_eq!(
        reply.capabilities(),
        vec!["STARTTLS".to_string(), "AUTH PLAIN LOGIN".to_string()]
    );
    assert!(reply.text().starts_with("250-mock.example.com"));
}

#[test]
fn a_body_that_is_not_seven_bit_ascii_is_base64_encoded() {
    assert_eq!(
        encode_body("hello\nworld"),
        ("7bit", "hello\r\nworld".to_string())
    );
    let (encoding, body) = encode_body("héllo 世界");
    assert_eq!(encoding, "base64");
    for line in body.split("\r\n").filter(|line| !line.is_empty()) {
        assert!(line.len() <= 76, "{line}");
    }
    assert_eq!(decode_base64(body.as_bytes()), "héllo 世界".as_bytes());

    // A line past the RFC limit is what an unencoded body gets wrong.
    let long = "a".repeat(MAX_7BIT_LINE + 1);
    assert_eq!(encode_body(&long).0, "base64");
}

#[test]
fn the_data_payload_is_crlf_framed_dot_stuffed_and_terminated() {
    let payload = encode_data("first\n.second\n\nthird");
    assert_eq!(
        String::from_utf8(payload).unwrap(),
        "first\r\n..second\r\n\r\nthird\r\n.\r\n"
    );
    // A message that already ends its lines with CRLF is not double-framed.
    assert_eq!(
        String::from_utf8(encode_data("a\r\nb\r\n")).unwrap(),
        "a\r\nb\r\n\r\n.\r\n"
    );
}

#[test]
fn header_values_cannot_inject_another_header() {
    let config = SmtpConfig {
        from: "bot@example.com".into(),
        ..SmtpConfig::default()
    };
    let message = build_message(
        &config,
        "alice@example.com",
        "hi\r\nBcc: someone@example.com",
        Some("<root@example.com>\r\nX-Evil: 1"),
        "body",
    )
    .unwrap();
    // The injected text stays *inside* the value it was written into, on one
    // line: what must never appear is a header of its own.
    let header_lines: Vec<&str> = message
        .split("\r\n\r\n")
        .next()
        .unwrap()
        .split("\r\n")
        .collect();
    assert!(
        !header_lines
            .iter()
            .any(|line| line.starts_with("Bcc:") || line.starts_with("X-Evil:")),
        "{message}"
    );
    assert!(
        header_lines.contains(&"Subject: hi Bcc: someone@example.com"),
        "{message}"
    );
    assert!(
        header_lines.contains(&"In-Reply-To: <root@example.com> X-Evil: 1"),
        "{message}"
    );
    // Exactly one line per header field, and the body is untouched.
    assert_eq!(message.matches("Bcc:").count(), 1, "{message}");
    assert!(message.ends_with("\r\n\r\nbody"), "{message}");

    // Without an `@` there is no sender to claim, and saying so beats sending.
    let broken = SmtpConfig {
        from: "not-an-address".into(),
        ..SmtpConfig::default()
    };
    let error = build_message(&broken, "alice@example.com", "hi", None, "body")
        .expect_err("a broken sender is refused");
    assert!(error.to_string().contains("not configured"), "{error}");
}

#[test]
fn a_non_ascii_subject_is_rfc2047_encoded_and_round_trips() {
    assert_eq!(encode_rfc2047("plain subject"), "plain subject");
    let encoded = encode_rfc2047("部署问题：完成了吗？");
    assert!(encoded.starts_with("=?UTF-8?B?"), "{encoded}");
    assert!(encoded.ends_with("?="), "{encoded}");
    assert_eq!(
        decode_header(&encoded.replace("\r\n ", " ")),
        "部署问题：完成了吗？"
    );
    // A subject long enough to need folding keeps every word inside a line.
    let long = "很长的主题".repeat(40);
    let folded = encode_rfc2047(&long);
    assert!(folded.contains("\r\n "), "{folded}");
    for line in folded.split("\r\n") {
        assert!(line.len() <= 78, "{} bytes: {line}", line.len());
    }
}

#[test]
fn a_reply_subject_adds_one_re_and_the_configured_prefix() {
    assert_eq!(reply_subject(Some("Deploy"), ""), "Re: Deploy");
    assert_eq!(reply_subject(Some("Re: Deploy"), ""), "Re: Deploy");
    assert_eq!(reply_subject(Some("RE: Deploy"), ""), "RE: Deploy");
    assert_eq!(reply_subject(Some("Deploy"), "[bot]"), "[bot] Re: Deploy");
    assert_eq!(
        reply_subject(Some("[bot] Re: Deploy"), "[bot]"),
        "[bot] Re: Deploy"
    );
    assert_eq!(reply_subject(None, ""), "Future agent reply");
    assert_eq!(reply_subject(Some("   "), ""), "Future agent reply");
}

#[test]
fn a_recipient_that_could_forge_a_header_is_refused() {
    let good = ConversationRef {
        id: "alice@example.com".into(),
        ..ConversationRef::default()
    };
    assert_eq!(recipient_address(&good).unwrap(), "alice@example.com");

    for broken in [
        "",
        "alice",
        "alice@example.com\r\nBcc: someone@example.com",
        "alice@example.com, bob@example.com",
        "<alice@example.com>",
    ] {
        let conversation = ConversationRef {
            id: broken.into(),
            ..ConversationRef::default()
        };
        assert!(
            recipient_address(&conversation).is_err(),
            "`{broken}` must be refused"
        );
    }
}

// ─── IMAP retrieval ────────────────────────────────────────────────────────

#[tokio::test]
async fn an_implicit_tls_connection_never_speaks_plaintext() {
    // The server answers with a plaintext greeting; a client that fell back to
    // the clear would send a command and put the password on the wire.
    let mock = spawn(vec![Step::Greet {
        lines: vec!["* OK mock ready"],
    }])
    .await;
    let mut config = config_for(mock.port, mock.port);
    config.imap.security = Security::Implicit;
    let error = ImapSession::connect(&config.imap)
        .await
        .expect_err("a plaintext greeting is not a TLS handshake");
    assert!(!error.to_string().is_empty());
    assert!(
        mock.lines().is_empty(),
        "nothing may reach the server in the clear: {:?}",
        mock.lines()
    );
}

#[test]
fn the_tls_configuration_comes_from_the_platform_trust_store() {
    // Building this is what proves the verifier wires up on this platform; a
    // mailbox behind a corporate TLS terminator depends on it.
    let config = tls_config().expect("a TLS client configuration");
    assert_eq!(config.alpn_protocols.len(), 0);
}

#[tokio::test]
async fn the_mailbox_is_polled_with_uid_commands_and_literals() {
    let message = raw_message(
        "From: Alice <alice@example.com>\r\nSubject: hello\r\nMessage-ID: <m1@example.com>",
        "a question",
    );
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 LOGIN \"bot@example.com\" \"secret\"",
            lines: vec!["a1 OK logged in"],
        },
        Step::Reply {
            expect: "a2 SELECT \"INBOX\"",
            lines: vec![
                "",
                "* 3 EXISTS",
                "* OK [UIDVALIDITY 42] UIDs valid",
                "a2 OK [READ-WRITE] selected",
            ],
        },
        Step::Reply {
            expect: "a3 UID SEARCH UNSEEN",
            lines: vec!["* SEARCH 7 8", "a3 OK done"],
        },
        Step::Fetch {
            expect: "a4 UID FETCH 7 (BODY.PEEK[])",
            uid: 7,
            payload: message.clone(),
            completion: "a4 OK done",
        },
        Step::Reply {
            expect: "a5 UID STORE 7 +FLAGS (\\Seen)",
            lines: vec!["* 1 FETCH (FLAGS (\\Seen))", "a5 OK done"],
        },
        Step::Reply {
            expect: "a6 LOGOUT",
            lines: vec!["* BYE bye", "a6 OK done"],
        },
        Step::Close,
    ])
    .await;

    let config = config_for(mock.port, mock.port);
    let mut session = ImapSession::connect(&config.imap).await.expect("connect");
    session.login(&config.imap).await.expect("login");
    let mailbox = session.select(&config.imap.mailbox).await.expect("select");
    assert_eq!(
        mailbox,
        MailboxState {
            exists: 3,
            uid_validity: 42
        }
    );
    assert_eq!(session.search_unseen().await.unwrap(), vec![7, 8]);
    assert_eq!(session.fetch_body(7).await.unwrap(), message);
    session.mark_seen(7).await.expect("mark seen");
    session.logout().await.expect("logout");
    mock.assert_clean();
    assert_eq!(
        mock.lines(),
        vec![
            "a1 LOGIN \"bot@example.com\" \"secret\"",
            "a2 SELECT \"INBOX\"",
            "a3 UID SEARCH UNSEEN",
            "a4 UID FETCH 7 (BODY.PEEK[])",
            "a5 UID STORE 7 +FLAGS (\\Seen)",
            "a6 LOGOUT",
        ]
    );
}

#[tokio::test]
async fn search_with_nothing_unseen_is_not_an_error() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 LOGIN",
            lines: vec!["a1 OK"],
        },
        Step::Reply {
            expect: "a2 SELECT",
            lines: vec!["* 0 EXISTS", "a2 OK [READ-ONLY] selected"],
        },
        Step::Reply {
            expect: "a3 UID SEARCH UNSEEN",
            lines: vec!["* SEARCH", "a3 OK done"],
        },
        Step::Close,
    ])
    .await;

    let config = config_for(mock.port, mock.port);
    let mut session = ImapSession::connect(&config.imap).await.expect("connect");
    session.login(&config.imap).await.expect("login");
    let mailbox = session.select("INBOX").await.expect("select");
    // The server reported no UIDVALIDITY at all, which is legal.
    assert_eq!(mailbox, MailboxState::default());
    assert!(session.search_unseen().await.unwrap().is_empty());
    mock.assert_clean();
}

#[tokio::test]
async fn rejected_credentials_stop_the_channel_instead_of_retrying() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 LOGIN",
            lines: vec!["a1 NO [AUTHENTICATIONFAILED] invalid credentials"],
        },
        Step::Close,
    ])
    .await;

    let config = config_for(mock.port, mock.port);
    let mut session = ImapSession::connect(&config.imap).await.expect("connect");
    let error = session.login(&config.imap).await.expect_err("must fail");
    assert!(error.is::<FatalError>(), "{error}");
    let text = error.to_string();
    assert!(text.contains("unauthorized"), "{text}");
    assert!(crate::delivery::is_permanent_error(&text), "{text}");
}

#[tokio::test]
async fn a_missing_mailbox_is_fatal_rather_than_retried() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 SELECT \"Missing\"",
            lines: vec!["a1 NO Mailbox does not exist"],
        },
        Step::Close,
    ])
    .await;

    let mut config = config_for(mock.port, mock.port);
    config.imap.mailbox = "Missing".into();
    let mut session = ImapSession::connect(&config.imap).await.expect("connect");
    let error = session
        .select(&config.imap.mailbox)
        .await
        .expect_err("must fail");
    assert!(error.is::<FatalError>(), "{error}");
    assert!(error.to_string().contains("Missing"), "{error}");
}

#[test]
fn literals_are_paired_with_the_line_they_follow() {
    let response = ImapResponse {
        status: "OK".into(),
        detail: "done".into(),
        lines: vec![
            // An unrelated literal first: pairing by arrival order would shift
            // every record after it.
            "* LIST () \"/\" \"INBOX\"".into(),
            "* 1 FETCH (UID 7 BODY[] {5}".into(),
            ")".into(),
            "* 2 FETCH (UID 8 RFC822.SIZE 12)".into(),
        ],
        literals: vec![
            Literal {
                after_line: 0,
                bytes: b"junk".to_vec(),
            },
            Literal {
                after_line: 1,
                bytes: b"first".to_vec(),
            },
        ],
    };
    let records = fetch_records(&response);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].uid, Some(7));
    assert_eq!(records[0].body.as_deref(), Some(&b"first"[..]));
    assert_eq!(records[1].uid, Some(8));
    assert_eq!(records[1].body, None);
}

#[test]
fn literal_length_reads_synchronizing_markers_only() {
    assert_eq!(literal_length("* 1 FETCH (BODY[] {3456}"), Some(3456));
    assert_eq!(literal_length("* 1 FETCH (BODY[] {0}"), Some(0));
    assert_eq!(literal_length("a1 OK done"), None);
    assert_eq!(literal_length("* 1 FETCH (BODY[] {nope}"), None);
    assert_eq!(literal_length("* 1 FETCH (BODY[] {}"), None);
}

#[test]
fn imap_arguments_are_quoted_so_a_value_cannot_start_a_command() {
    assert_eq!(quote_imap("INBOX"), "\"INBOX\"");
    assert_eq!(quote_imap("a\"b"), "\"a\\\"b\"");
    assert_eq!(quote_imap("a\\b"), "\"a\\\\b\"");
    assert_eq!(quote_imap("a\r\nb2 DELETE x"), "\"ab2 DELETE x\"");
}

// ─── MIME parsing ──────────────────────────────────────────────────────────

#[test]
fn a_multipart_alternative_prefers_the_plain_text_part() {
    let mail = mail_from(
        "From: Alice <alice@example.com>\r\nSubject: hi\r\nContent-Type: multipart/alternative; boundary=\"b1\"",
        "--b1\r\nContent-Type: text/plain; charset=iso-8859-1\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nplain =E9 body\r\n--b1\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>html body</p>\r\n--b1--\r\n",
    );
    assert_eq!(mail.text, "plain é body");
    assert!(!mail.from_html);
    assert!(mail.attachments.is_empty());
}

#[test]
fn html_only_mail_is_reduced_to_readable_text() {
    let mail = mail_from(
        "From: Alice <alice@example.com>\r\nContent-Type: text/html; charset=utf-8",
        "<html><head><style>p{color:red}</style></head><body><p>Hello &amp; welcome</p><div>Second&nbsp;line</div><script>alert(1)</script><br>Third</body></html>",
    );
    assert!(mail.from_html);
    // Both `</div>` and the `br` end a line, so the two runs stay separate.
    assert_eq!(mail.text, "Hello & welcome\n\nSecond line\n\nThird");
    assert!(!mail.text.contains("alert"), "{}", mail.text);
    assert!(!mail.text.contains("color:red"), "{}", mail.text);
    assert!(!mail.text.contains('<'), "{}", mail.text);
}

#[test]
fn a_nested_multipart_mixed_body_keeps_the_text_and_lists_the_attachments() {
    let encoded = base64_encode("notes the user attached".as_bytes());
    let raw = raw_message(
        "From: Alice <alice@example.com>\r\nContent-Type: multipart/mixed; boundary=\"mix\"",
        &format!(
            "--mix\r\nContent-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n--alt\r\nContent-Type: text/plain\r\n\r\nthe answer\r\n--alt\r\nContent-Type: text/html\r\n\r\n<p>the answer</p>\r\n--alt--\r\n--mix\r\nContent-Type: text/plain; name=\"notes.txt\"\r\nContent-Disposition: attachment; filename=\"notes.txt\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{encoded}\r\n--mix\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\n\r\n%PDF-1.4 fake\r\n--mix--\r\n"
        ),
    );
    let mail = parse_mail(&raw);
    // A `text/plain` *attachment* is not the reply the user wrote.
    assert_eq!(mail.text, "the answer");
    assert!(!mail.from_html);
    let names: Vec<String> = mail
        .attachments
        .iter()
        .map(|attachment| attachment.filename.clone().unwrap_or_default())
        .collect();
    assert_eq!(names, vec!["notes.txt", "report.pdf"]);
    assert_eq!(mail.attachments[1].content_type, "application/pdf");
    assert!(mail.attachments[1].bytes > 0);
}

#[test]
fn base64_and_quoted_printable_bodies_are_decoded() {
    let encoded = base64_encode("你好，世界".as_bytes());
    let mail = mail_from(
        "From: a@b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64",
        &encoded,
    );
    assert_eq!(mail.text, "你好，世界");

    let quoted = mail_from(
        "From: a@b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable",
        "one =\r\ntwo =E2=82=AC three",
    );
    assert_eq!(quoted.text, "one two € three");
}

#[test]
fn charsets_that_a_mailbox_actually_contains_are_decoded() {
    assert_eq!(decode_text(&[0xE9], "ISO-8859-1"), "é");
    assert_eq!(decode_text(&[0x92], "windows-1252"), "\u{2019}");
    assert_eq!(decode_text("ok".as_bytes(), "windows-1252"), "ok");
    // An unknown charset must not lose the message.
    assert!(decode_text("ok".as_bytes(), "x-no-such-charset").contains("ok"));
    assert_eq!(decode_text("héllo".as_bytes(), "utf-8"), "héllo");
}

#[test]
fn rfc2047_headers_are_decoded_across_folds_and_encodings() {
    let headers =
        parse_headers("Subject: =?UTF-8?B?6YOo572y?=\r\n =?UTF-8?Q?second_part?=\r\nFrom: a@b\r\n");
    let subject = headers.decoded("subject").expect("subject");
    // Adjacent encoded words are joined without the whitespace between them.
    assert_eq!(subject, "部署second part");
    // Encoded words are case-insensitive in their framing too.
    assert_eq!(decode_header("=?utf-8?q?plain_text?="), "plain text");
    // Something that only looks like an encoded word is left alone.
    assert_eq!(decode_header("=?not really"), "=?not really");
    assert_eq!(
        decode_header("a =?UTF-8?X?nope?= b"),
        "a =?UTF-8?X?nope?= b"
    );
}

#[test]
fn headers_are_folded_case_insensitively_and_missing_ones_are_none() {
    let headers =
        parse_headers("From: Alice <a@b>\r\nSubject: first\r\n second\r\nX-Long: a\r\n\tb\r\n");
    assert_eq!(headers.get("subject"), Some("first second"));
    assert_eq!(headers.get("FROM"), None);
    assert_eq!(headers.get("x-long"), Some("a b"));
    assert_eq!(headers.get("missing"), None);
    assert_eq!(headers.names(), vec!["from", "subject", "x-long"]);
    // A part with no blank line is all headers, and the body is empty.
    let (headers, body) = split_message(b"Content-Type: text/plain");
    assert_eq!(headers.get("content-type"), Some("text/plain"));
    assert!(body.is_empty());
}

#[test]
fn content_type_parameters_survive_quotes_and_semicolons() {
    let (kind, parameters) = parse_content_type("Multipart/Mixed; boundary=\"a;b\"; charset=UTF-8");
    assert_eq!(kind, "multipart/mixed");
    assert_eq!(
        parameters,
        vec![
            ("boundary".to_string(), "a;b".to_string()),
            ("charset".to_string(), "UTF-8".to_string()),
        ]
    );
    let (kind, parameters) = parse_content_type("TEXT/PLAIN");
    assert_eq!(kind, "text/plain");
    assert!(parameters.is_empty());
    let (kind, _) = parse_content_type("");
    assert_eq!(kind, "");
}

#[test]
fn a_parts_are_split_without_the_preamble_or_the_closing_marker() {
    let body = "preamble text\r\n--b\r\none\r\n--b\r\ntwo\r\n--b--\r\nepilogue";
    let parts = split_multipart(body.as_bytes(), "b");
    assert_eq!(parts.len(), 2);
    assert_eq!(String::from_utf8_lossy(&parts[0]), "one");
    assert_eq!(String::from_utf8_lossy(&parts[1]), "two");
    assert!(split_multipart(b"nothing here", "b").is_empty());
}

#[test]
fn an_attachment_is_recognized_but_never_downloaded() {
    let raw = raw_message(
        "From: Alice <alice@example.com>\r\nContent-Type: multipart/mixed; boundary=\"m\"",
        "--m\r\nContent-Type: image/png; name=\"shot.png\"\r\nContent-Disposition: attachment; filename=\"shot.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n--m--\r\n",
    );
    let mail = parse_mail(&raw);
    assert!(mail.text.is_empty(), "{:?}", mail.text);
    assert_eq!(mail.attachments.len(), 1);
    let config = config_for(1, 1);
    let inbound = to_inbound(
        9,
        &MailboxState::default(),
        "bot@example.com",
        &config,
        &mail,
    )
    .expect("an attachment-only message is still a prompt");
    assert_eq!(inbound.media.len(), 1);
    assert_eq!(inbound.media[0].kind, MediaKind::Image);
    assert_eq!(inbound.media[0].filename.as_deref(), Some("shot.png"));
    // Recognized, not fetched: no bytes, no url.
    assert!(inbound.media[0].data.is_none());
    assert!(inbound.media[0].url.is_none());
}

#[test]
fn addresses_are_parsed_in_both_shapes_and_refused_when_ambiguous() {
    let plain = parse_address("Alice <A@Example.COM>").expect("name and address");
    assert_eq!(plain.address, "a@example.com");
    assert_eq!(plain.display.as_deref(), Some("Alice"));

    let commented = parse_address("bob@example.com (Bob)").expect("comment form");
    assert_eq!(commented.address, "bob@example.com");
    assert_eq!(commented.display, None);

    let quoted = parse_address("\"Bob B\" <bob@example.com>").expect("quoted name");
    assert_eq!(quoted.display.as_deref(), Some("Bob B"));

    assert!(parse_address("").is_none());
    assert!(parse_address("no address here").is_none());
    assert!(parse_address("Alice <not-an-address>").is_none());
}

#[test]
fn a_real_date_is_parsed_and_an_invented_one_is_not() {
    let friday = parse_date("Fri, 21 Nov 1997 09:55:06 -0600").expect("rfc 2822");
    let saturday = parse_date("Sat, 22 Nov 1997 09:55:06 -0600").expect("next day");
    assert_eq!(saturday - friday, 86_400_000);
    // A missing day-of-week and a trailing comment are both common.
    assert_eq!(parse_date("21 Nov 1997 09:55:06 -0600"), Some(friday));
    assert_eq!(
        parse_date("Fri, 21 Nov 1997 09:55:06 -0600 (CST)"),
        Some(friday)
    );
    assert_eq!(parse_date(""), None);
    assert_eq!(parse_date("sometime last year"), None);
}

#[test]
fn the_thread_root_is_the_first_reference_then_the_reply_target() {
    let referenced = parse_headers("References: <root@example.com> <second@example.com>");
    assert_eq!(
        thread_root(&referenced, "own@example.com"),
        "root@example.com"
    );

    let replied = parse_headers("In-Reply-To: <parent@example.com>");
    assert_eq!(
        thread_root(&replied, "own@example.com"),
        "parent@example.com"
    );

    let fresh = parse_headers("Subject: hello");
    assert_eq!(thread_root(&fresh, "own@example.com"), "own@example.com");

    let both = parse_headers("In-Reply-To: <parent@example.com>\r\nReferences: <root@example.com>");
    assert_eq!(thread_root(&both, "own@example.com"), "root@example.com");
}

#[test]
fn the_message_identifier_is_read_without_its_brackets() {
    let headers = parse_headers("Message-ID: <abc@example.com>");
    assert_eq!(
        message_identifier(&headers).as_deref(),
        Some("abc@example.com")
    );
    let bare = parse_headers("Message-Id: abc@example.com");
    assert_eq!(
        message_identifier(&bare).as_deref(),
        Some("abc@example.com")
    );
    let empty = parse_headers("Message-ID: <>");
    assert_eq!(message_identifier(&empty), None);
    assert_eq!(message_identifier(&parse_headers("Subject: x")), None);
}

#[test]
fn a_subject_is_readable_however_it_was_folded() {
    let headers = parse_headers("Subject: Re:   spaced\r\n   out");
    assert_eq!(subject_of(&headers), "Re: spaced out");
    assert_eq!(subject_of(&parse_headers("From: a@b")), "");
}

// ─── loop safety and gating ────────────────────────────────────────────────

#[test]
fn automated_mail_is_never_answered() {
    assert!(is_automated(&parse_headers("Auto-Submitted: auto-replied")));
    assert!(is_automated(&parse_headers(
        "Auto-Submitted: auto-generated"
    )));
    assert!(is_automated(&parse_headers("Precedence: bulk")));
    assert!(is_automated(&parse_headers("Precedence: LIST")));
    assert!(is_automated(&parse_headers("Precedence: junk")));
    // `no` is the one value that means a human wrote it.
    assert!(!is_automated(&parse_headers("Auto-Submitted: no")));
    assert!(!is_automated(&parse_headers("Precedence: first-class")));
    assert!(!is_automated(&parse_headers("Subject: hello")));
}

#[test]
fn the_allowlist_and_subject_prefix_gate_what_gets_answered() {
    let allowlist = vec!["Alice@Example.com".to_string()];
    assert!(sender_allowed("alice@example.com", &allowlist));
    assert!(!sender_allowed("bob@example.com", &allowlist));
    // An empty allowlist means everyone.
    assert!(sender_allowed("bob@example.com", &[]));

    assert!(subject_allows("anything", ""));
    assert!(subject_allows("[bot] do this", "[BOT]"));
    assert!(!subject_allows("hello", "[bot]"));
    assert!(subject_allows("  [bot] padded  ", "[bot]"));
}

#[test]
fn a_prompt_becomes_a_threaded_direct_conversation() {
    let mail = mail_from(
        "From: Alice <alice@example.com>\r\nSubject: deploy\r\nMessage-ID: <m2@example.com>\r\nIn-Reply-To: <m1@example.com>\r\nDate: Fri, 21 Nov 1997 09:55:06 -0600\r\nTo: bot@example.com\r\n",
        "please deploy",
    );
    let config = config_for(1, 1);
    let inbound = to_inbound(
        7,
        &MailboxState {
            exists: 3,
            uid_validity: 42,
        },
        "bot@example.com",
        &config,
        &mail,
    )
    .expect("a genuine message is a prompt");

    assert_eq!(inbound.message_id, "m2@example.com");
    assert_eq!(inbound.sender.id, "alice@example.com");
    assert_eq!(inbound.sender.display.as_deref(), Some("Alice"));
    assert_eq!(inbound.conversation.id, "alice@example.com");
    assert_eq!(
        inbound.conversation.thread_id.as_deref(),
        Some("m1@example.com")
    );
    assert_eq!(inbound.conversation.kind, ChatKind::Direct);
    assert_eq!(
        inbound.conversation_key("email"),
        "email:alice@example.com:m1@example.com"
    );
    assert!(inbound.addressed_to_bot);
    // The subject must not become part of the text: an approval answer ("yes")
    // is matched on the whole message, so a prefixed prompt would never match.
    assert_eq!(inbound.text, "please deploy");
    // A mailbox is store-and-forward, so the bridge's freshness window is not
    // applied; the real date is kept for the record instead.
    assert_eq!(inbound.created_at_ms, None);
    let raw = inbound.raw.expect("raw payload");
    assert_eq!(raw["subject"], "deploy");
    assert_eq!(raw["uid"], 7);
    assert_eq!(raw["date"], "Fri, 21 Nov 1997 09:55:06 -0600");
    // The date is parsed for the record, but is deliberately not the freshness
    // timestamp the bridge filters on.
    assert_eq!(
        raw["date_ms"],
        json!(parse_date("Fri, 21 Nov 1997 09:55:06 -0600"))
    );
}

#[test]
fn our_own_address_unknown_senders_and_off_subject_mail_are_refused() {
    let config = config_for(1, 1);
    let mailbox = MailboxState::default();
    let from = |sender: &str, subject: &str| {
        parse_mail(&raw_message(
            &format!("From: {sender}\r\nSubject: {subject}\r\nMessage-ID: <m@example.com>"),
            "hello",
        ))
    };
    // `Inbound` is deliberately not `PartialEq` (it carries raw payloads), so the
    // refusal is read off the error side.
    let skip = |mail: &Mail, config: &EmailConfig| {
        to_inbound(1, &mailbox, "bot@example.com", config, mail).err()
    };

    assert_eq!(
        skip(&from("Bot <bot@example.com>", "hi"), &config),
        Some(Skip::OwnMessage)
    );
    assert_eq!(skip(&from("", "hi"), &config), Some(Skip::NoSender));
    assert_eq!(skip(&from("Alice", "hi"), &config), Some(Skip::NoSender));

    let mut allowlisted = config.clone();
    allowlisted.sender_allowlist = vec!["someone@else.com".into()];
    assert_eq!(
        skip(&from("Alice <alice@example.com>", "hi"), &allowlisted),
        Some(Skip::NotAllowed)
    );

    let mut prefixed = config.clone();
    prefixed.subject_prefix = "[bot]".into();
    assert_eq!(
        skip(&from("Alice <alice@example.com>", "hello"), &prefixed),
        Some(Skip::NoSubjectMatch)
    );
    assert_eq!(
        skip(&from("Alice <alice@example.com>", "[bot] hello"), &prefixed),
        None
    );

    let automated = parse_mail(&raw_message(
        "From: Alice <alice@example.com>\r\nAuto-Submitted: auto-replied\r\nSubject: out of office",
        "I am away",
    ));
    assert_eq!(skip(&automated, &config), Some(Skip::Automated));
    assert!(!Skip::Automated.reason().is_empty());
}

#[test]
fn a_message_with_no_message_id_falls_back_to_its_mailbox_position() {
    let mail = mail_from("From: alice@example.com\r\nSubject: hi", "hello");
    let mailbox = MailboxState {
        exists: 1,
        uid_validity: 42,
    };
    let config = config_for(1, 1);
    let inbound = to_inbound(7, &mailbox, "bot@example.com", &config, &mail).expect("prompt");
    assert_eq!(inbound.message_id, "42/7");
    assert_eq!(inbound.conversation.thread_id.as_deref(), Some("42/7"));
}

// ─── the delivered-message store ───────────────────────────────────────────

#[test]
fn the_seen_store_survives_a_restart_and_is_bounded() {
    let path = temp_dir("email-seen").join("seen.json");
    let store = SeenStore::load(path.clone());
    assert_eq!(store.len(), 0);
    store.record("42:7").expect("record");
    store.record("mid:m1@example.com").expect("record");
    store.record("42:7").expect("recording twice is a no-op");
    assert_eq!(store.len(), 2);

    // A second process reads what the first wrote.
    let reloaded = SeenStore::load(path.clone());
    assert!(reloaded.contains("42:7"));
    assert!(reloaded.contains("mid:m1@example.com"));
    assert!(!reloaded.contains("42:8"));

    // A corrupt file is treated as empty rather than fatal.
    std::fs::write(&path, "{not json").expect("write");
    assert_eq!(SeenStore::load(path).len(), 0);
}

#[test]
fn a_delivered_message_is_remembered_under_both_of_its_identities() {
    let path = temp_dir("email-seen-both").join("seen.json");
    let store = SeenStore::load(path);
    remember_delivery(&store, "42:7", "m1@example.com").expect("record");
    // The mailbox position and the platform id are both recorded, and a
    // redelivery under a new UID is still recognised as the same mail.
    assert!(store.contains("42:7"));
    assert!(store.contains(&delivery_key("m1@example.com")));
    assert!(was_delivered(&store, "42:8", "m1@example.com"));
    assert!(!was_delivered(&store, "42:8", "m2@example.com"));
    assert!(was_delivered(&store, "42:7", "m2@example.com"));
}

#[test]
fn the_seen_store_drops_the_oldest_keys_first() {
    let mut inner = SeenInner::default();
    for index in 0..SEEN_CAPACITY + 5 {
        inner.keys.insert(format!("k{index}"));
        inner.order.push_back(format!("k{index}"));
    }
    trim(&mut inner);
    assert_eq!(inner.keys.len(), SEEN_CAPACITY);
    assert!(!inner.keys.contains("k0"));
    assert!(inner.keys.contains(&format!("k{}", SEEN_CAPACITY + 4)));
}

// ─── configuration, sender and probe ───────────────────────────────────────

#[test]
fn the_definition_declares_what_this_channel_does() {
    assert!(DEFINITION.is_implemented());
    assert_eq!(DEFINITION.maturity, Maturity::Preview);
    assert!(DEFINITION.capabilities.receive && DEFINITION.capabilities.send);
    assert!(DEFINITION.capabilities.threads);
    // Nothing is fetched from the mailbox, so no inbound media reaches the model.
    assert!(!DEFINITION.capabilities.media_in);
    assert!(!DEFINITION.capabilities.edit);
    assert!(!DEFINITION.capabilities.mention_gate);
    let example: serde_json::Value =
        serde_json::from_str(DEFINITION.config_example).expect("valid JSON");
    assert_eq!(example["enabled"], json!(true));
    let parsed: EmailConfig = serde_json::from_value(example).expect("the example is usable");
    assert_eq!(parsed.imap.port, 993);
    assert_eq!(parsed.imap.security, Security::Implicit);
    assert_eq!(parsed.smtp.port, 587);
    assert_eq!(parsed.smtp.security, Security::Starttls);
    assert_eq!(provider().definition().id, "email");
}

#[test]
fn a_configuration_problem_is_reported_with_the_channel_and_the_field() {
    let empty = EmailConfig::default();
    let error = empty.validate("email").expect_err("empty config");
    let text = error.to_string();
    assert!(text.contains("providers.email"), "{text}");
    assert!(text.contains("imap.host"), "{text}");
    assert!(text.contains("smtp.from"), "{text}");

    let good = config_for(993, 587);
    assert!(good.validate("email").is_ok());

    let cases: Vec<(EmailConfig, &str)> = vec![
        (
            EmailConfig {
                poll_seconds: 0,
                ..good.clone()
            },
            "poll_seconds",
        ),
        (
            EmailConfig {
                imap: ImapConfig {
                    port: 0,
                    ..good.imap.clone()
                },
                ..good.clone()
            },
            "port",
        ),
        (
            EmailConfig {
                imap: ImapConfig {
                    mailbox: "  ".into(),
                    ..good.imap.clone()
                },
                ..good.clone()
            },
            "imap.mailbox",
        ),
        (
            EmailConfig {
                smtp: SmtpConfig {
                    from: "not-an-address".into(),
                    ..good.smtp.clone()
                },
                ..good.clone()
            },
            "smtp.from",
        ),
        (
            EmailConfig {
                smtp: SmtpConfig {
                    username: "bot@example.com".into(),
                    password: String::new(),
                    ..good.smtp.clone()
                },
                ..good.clone()
            },
            "smtp.password",
        ),
    ];
    for (config, field) in cases {
        let error = config.validate("email").expect_err(field);
        assert!(error.to_string().contains(field), "{field}: {error}");
    }
}

#[test]
fn the_poll_interval_is_at_least_a_second() {
    let mut config = config_for(1, 1);
    config.poll_seconds = 60;
    assert_eq!(config.poll_interval(), Duration::from_secs(60));
}

#[tokio::test]
async fn building_a_sender_checks_the_configuration_without_connecting() {
    let shutdown = crate::bridge::Shutdown::new();
    // A closed port: constructing a sender must not try to reach it.
    let ctx = ctx_with(json!({"enabled": true}), shutdown.clone());
    let error = match Email.sender(&ctx) {
        Ok(_) => panic!("an incomplete config must be refused"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("providers.email"), "{error}");

    let ctx = ctx_with(config_value(1, 1), shutdown);
    let sender = Email
        .sender(&ctx)
        .expect("a complete config builds a sender");
    assert_eq!(sender.definition().id, "email");
    // Nothing was connected, so a send now fails on the socket, not on a header.
    let conversation = ConversationRef {
        id: "alice@example.com".into(),
        ..ConversationRef::default()
    };
    let error = sender
        .send_text(&conversation, "hello")
        .await
        .expect_err("there is no server on port 1");
    assert!(!error.to_string().is_empty());
}

#[tokio::test]
async fn a_sender_with_an_invalid_recipient_never_opens_a_connection() {
    let config = EmailConfig {
        smtp: SmtpConfig {
            from: "bot@example.com".into(),
            ..SmtpConfig::default()
        },
        ..EmailConfig::default()
    };
    let sender = EmailSender::new(config);
    let conversation = ConversationRef {
        id: "alice@example.com\r\nBcc: someone@example.com".into(),
        ..ConversationRef::default()
    };
    let error = sender
        .send_text(&conversation, "hello")
        .await
        .expect_err("an injecting recipient is refused");
    assert!(error.to_string().contains("recipient not found"), "{error}");
    assert!(crate::delivery::is_permanent_error(&error.to_string()));
}

#[tokio::test]
async fn the_probe_reports_the_mailbox_and_the_submission_server() {
    let message = raw_message("From: alice@example.com\r\nSubject: hi", "hello");
    let imap = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 LOGIN \"bot@example.com\" \"secret\"",
            lines: vec!["a1 OK logged in"],
        },
        Step::Reply {
            expect: "a2 SELECT \"INBOX\"",
            lines: vec!["* 5 EXISTS", "* OK [UIDVALIDITY 42] ok", "a2 OK done"],
        },
        Step::Reply {
            expect: "a3 UID SEARCH UNSEEN",
            lines: vec!["* SEARCH 7 8", "a3 OK done"],
        },
        Step::Reply {
            expect: "a4 LOGOUT",
            lines: vec!["* BYE bye", "a4 OK done"],
        },
        Step::Close,
    ])
    .await;
    let _ = message;
    let smtp = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            lines: vec!["250-mock", "250 SIZE 100000"],
        },
        Step::Reply {
            expect: "QUIT",
            lines: vec!["221 bye"],
        },
        Step::Close,
    ])
    .await;

    let shutdown = crate::bridge::Shutdown::new();
    let ctx = ctx_with(config_value(imap.port, smtp.port), shutdown);
    let summary = Email.probe(&ctx).await.expect("probe");
    assert!(summary.contains("INBOX"), "{summary}");
    assert!(summary.contains("5 message(s)"), "{summary}");
    assert!(summary.contains("2 unseen"), "{summary}");
    assert!(summary.contains("no authentication"), "{summary}");
    imap.assert_clean();
    smtp.assert_clean();
}

#[tokio::test]
async fn the_poller_answers_unseen_mail_and_stops_when_asked() {
    let message = raw_message(
        "From: Alice <alice@example.com>\r\nSubject: hello\r\nMessage-ID: <m1@example.com>",
        "a question",
    );
    let imap = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 LOGIN \"bot@example.com\" \"secret\"",
            lines: vec!["a1 OK logged in"],
        },
        Step::Reply {
            expect: "a2 SELECT \"INBOX\"",
            lines: vec!["* 1 EXISTS", "* OK [UIDVALIDITY 42] ok", "a2 OK done"],
        },
        Step::Reply {
            expect: "a3 UID SEARCH UNSEEN",
            lines: vec!["* SEARCH 7", "a3 OK done"],
        },
        Step::Fetch {
            expect: "a4 UID FETCH 7 (BODY.PEEK[])",
            uid: 7,
            payload: message,
            completion: "a4 OK done",
        },
        Step::Reply {
            expect: "a5 LOGOUT",
            lines: vec!["* BYE bye", "a5 OK done"],
        },
        Step::Close,
    ])
    .await;

    // Nothing listens on port 1: the agent is unreachable, so the bridge refuses
    // the message with backpressure instead of answering it.
    let shutdown = crate::bridge::Shutdown::new();
    let ctx = ctx_with(config_value(imap.port, 1), shutdown.clone());
    let running = tokio::spawn(async move { Email.run(ctx).await });

    let fetched = wait_until(
        || imap.saw("a4 UID FETCH 7 (BODY.PEEK[])"),
        Duration::from_secs(10),
    )
    .await;
    assert!(fetched, "{:?}", imap.lines());
    shutdown.trigger();

    let stopped = tokio::time::timeout(Duration::from_secs(10), running)
        .await
        .expect("the poller must stop when asked")
        .expect("the task must not panic");
    stopped.expect("a clean stop");
    imap.assert_clean();
    assert!(imap.saw("a5 LOGOUT"), "{:?}", imap.lines());
    // Backpressure leaves the message unseen, so a later run can still answer it.
    assert!(
        !imap.saw("a5 UID STORE 7 +FLAGS (\\Seen)"),
        "{:?}",
        imap.lines()
    );
}

#[tokio::test]
async fn an_imap_session_debug_line_reports_its_command_counter() {
    // The counter is what says how far a failing conversation got.
    let session = ImapSession {
        stream: LineStream::with_timeout(
            Wire::Plain(dummy_tcp()),
            Duration::from_secs(TEST_TIMEOUT_SECONDS),
        ),
        tag: 7,
    };
    let rendered = format!("{session:?}");
    assert!(rendered.contains("ImapSession"), "{rendered}");
    assert!(rendered.contains('7'), "{rendered}");
}

/// A loopback socket that goes nowhere: the TLS state these tests build works on
/// buffers, so only the socket half has to exist.
fn dummy_tcp() -> tokio::net::TcpStream {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let socket = std::net::TcpStream::connect(addr).expect("connect");
    socket.set_nonblocking(true).expect("nonblocking");
    tokio::net::TcpStream::from_std(socket).expect("tokio socket")
}

// ─── TLS, end to end on the loopback ───────────────────────────────────────

#[tokio::test]
async fn the_platform_verifier_is_used_when_no_configuration_is_supplied() {
    // The production path: nothing is substituted, so the platform trust store
    // answers and a certificate invented for a loopback server is refused.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let (read, write) = socket.into_split();
        // The handshake fails in verification, so no IMAP is ever spoken.
        let _ = TlsPeer::accept(read, write).await;
    });
    let mut config = config_for(port, port);
    config.imap.security = Security::Implicit;
    let error = ImapSession::connect(&config.imap)
        .await
        .expect_err("an unknown certificate must be refused");
    assert!(!error.to_string().is_empty());
}

#[tokio::test]
async fn a_starttls_submission_is_re_greeted_over_an_encrypted_socket() {
    let mock = spawn_tls(vec![
        TlsStep::Greet {
            lines: vec!["220 mock ready"],
        },
        TlsStep::Reply {
            expect: "EHLO example.com",
            lines: vec!["250-mock", "250-STARTTLS", "250 AUTH PLAIN"],
        },
        TlsStep::Reply {
            expect: "STARTTLS",
            lines: vec!["220 go ahead"],
        },
        // Everything past this point is inside TLS: the credentials never
        // travel in the clear.
        TlsStep::Upgrade,
        TlsStep::Secure {
            expect: "EHLO example.com",
            lines: vec!["250-mock re-greeted", "250 AUTH PLAIN"],
        },
        TlsStep::Secure {
            expect: "AUTH PLAIN ",
            lines: vec!["235 accepted"],
        },
        TlsStep::Secure {
            expect: "MAIL FROM:<bot@example.com>",
            lines: vec!["250 sender ok"],
        },
        TlsStep::Secure {
            expect: "RCPT TO:<alice@example.com>",
            lines: vec!["250 recipient ok"],
        },
        TlsStep::Secure {
            expect: "DATA",
            lines: vec!["354 go ahead"],
        },
        TlsStep::SecureData {
            reply: "250 queued over tls",
        },
        TlsStep::Secure {
            expect: "QUIT",
            lines: vec!["221 bye"],
        },
        TlsStep::Close,
    ])
    .await;

    let mut config = config_for(mock.port, mock.port);
    config.smtp.security = Security::Starttls;
    config.smtp.username = "bot@example.com".into();
    config.smtp.password = "secret".into();
    let tls = trusting_tls();
    let mut session = SmtpSession::connect_with(&config.smtp, Some(&tls))
        .await
        .expect("the encrypted session starts");
    session
        .submit(
            "bot@example.com",
            "alice@example.com",
            &encode_data("Subject: over tls\r\n\r\nbody"),
        )
        .await
        .expect("the encrypted submission succeeds");
    let _ = session.quit().await;
    mock.assert_clean();
    assert!(
        mock.saw(&format!(
            "AUTH PLAIN {}",
            base64_encode(b"\0bot@example.com\0secret")
        )),
        "{:?}",
        mock.lines()
    );
    assert!(mock.saw("DATA"), "{:?}", mock.lines());
}

#[tokio::test]
async fn an_implicit_tls_mailbox_is_reached_from_the_first_byte() {
    let message = raw_message(
        "From: alice@example.com\r\nSubject: secure\r\nMessage-ID: <s1@example.com>",
        "hello over tls",
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    let payload = message.clone();
    let received: Lines = Arc::new(StdMutex::new(Vec::new()));
    let task_received = received.clone();
    let errors: Lines = Arc::new(StdMutex::new(Vec::new()));
    let task_errors = errors.clone();

    tokio::spawn(async move {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let (read, write) = socket.into_split();
        // Implicit TLS: the handshake starts before a single byte of IMAP.
        let Some(mut peer) = TlsPeer::accept(read, write).await else {
            note(&task_errors, "the handshake did not complete".into());
            return;
        };
        // The greeting is what the client reads first, so it goes out before
        // anything is read from it.
        if !peer.send_line("* OK mock ready").await {
            note(&task_errors, "the greeting could not be sent".into());
            return;
        }
        while let Some(line) = peer.read_line().await {
            task_received.lock().unwrap().push(line.clone());
            if line.contains("UID FETCH") {
                // The literal: the response line, then the octets after it.
                let mut bytes =
                    format!("* 1 FETCH (UID 7 BODY[] {{{}}}\r\n", payload.len()).into_bytes();
                bytes.extend_from_slice(&payload);
                bytes.extend_from_slice(b")\r\n");
                bytes.extend_from_slice(b"a4 OK done\r\n");
                if std::io::Write::write_all(&mut peer.conn.writer(), &bytes).is_err()
                    || peer.flush().await.is_err()
                {
                    return;
                }
                continue;
            }
            let answer = if line.contains("LOGIN") {
                "a1 OK logged in"
            } else if line.contains("UID SEARCH") {
                "* SEARCH 7\r\na3 OK done"
            } else if line.contains("SELECT") {
                "* 0 EXISTS\r\na2 OK done"
            } else {
                "a9 OK done"
            };
            for part in answer.split("\r\n") {
                if !peer.send_line(part).await {
                    return;
                }
            }
        }
    });

    let mut config = config_for(port, port);
    config.imap.security = Security::Implicit;
    let tls = trusting_tls();
    let mut session = match ImapSession::connect_with(&config.imap, Some(&tls)).await {
        Ok(session) => session,
        Err(error) => panic!(
            "the handshake and greeting complete: {error}; the server said {:?}",
            errors.lock().unwrap()
        ),
    };
    session.login(&config.imap).await.expect("login");
    let mailbox = session.select("INBOX").await.expect("select");
    assert_eq!(mailbox.exists, 0);
    assert_eq!(session.search_unseen().await.expect("search"), vec![7]);
    let body = session
        .fetch_body(7)
        .await
        .expect("the body arrives over TLS");
    assert_eq!(body, message);
    assert!(
        received
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.starts_with("a1 LOGIN")),
        "{:?}",
        received
    );
    assert!(errors.lock().unwrap().is_empty(), "{:?}", errors);
}

#[tokio::test]
async fn a_connection_upgraded_by_hand_carries_protocol_lines() {
    // The upgrade path on its own: a plain socket becomes TLS, and the same line
    // framing runs over it.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();

    tokio::spawn(async move {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let (read, write) = socket.into_split();
        let Some(mut peer) = TlsPeer::accept(read, write).await else {
            return;
        };
        while let Some(line) = peer.read_line().await {
            let reply = format!("250 echoing {line}");
            if !peer.send_line(&reply).await {
                return;
            }
        }
    });

    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let wire = Wire::Plain(tcp)
        .start_tls("localhost", trusting_tls())
        .await
        .expect("the handshake completes");
    let mut stream = LineStream::with_timeout(wire, Duration::from_secs(TEST_TIMEOUT_SECONDS));
    stream.write_line("EHLO example.com").await.expect("write");
    assert_eq!(
        stream.read_line().await.expect("read").as_deref(),
        Some("250 echoing EHLO example.com")
    );
    stream.write_line("UID FETCH 7").await.expect("write");
    assert!(stream
        .read_line()
        .await
        .expect("read")
        .expect("a line")
        .contains("UID FETCH 7"));
}

#[tokio::test]
async fn an_already_encrypted_connection_is_not_upgraded_twice() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let (read, write) = socket.into_split();
        let _ = TlsPeer::accept(read, write).await;
    });
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let wire = Wire::Plain(tcp)
        .start_tls("localhost", trusting_tls())
        .await
        .expect("the handshake completes");
    let error = match wire.start_tls("localhost", trusting_tls()).await {
        Ok(_) => panic!("a second upgrade must be refused"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("already encrypted"), "{error}");
}

#[tokio::test]
async fn a_peer_that_vanishes_during_the_handshake_does_not_hang() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        // Read the ClientHello, then walk away without answering it.
        let mut hello = [0u8; 4096];
        let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut hello).await;
    });
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let error = match Wire::Plain(tcp)
        .start_tls("localhost", trusting_tls())
        .await
    {
        Ok(_) => panic!("the handshake cannot complete"),
        Err(error) => error,
    };
    assert!(!error.to_string().is_empty());
}

#[tokio::test]
async fn a_name_that_is_not_a_dns_name_is_refused_before_any_handshake() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        let _ = listener.accept().await;
    });
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let error = match handshake_with(tcp, "not a name\n", trusting_tls()).await {
        Ok(_) => panic!("an unusable server name must be refused"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("not a usable TLS name"),
        "{error}"
    );
}

#[tokio::test]
async fn an_imap_starttls_refusal_stops_the_connection() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 STARTTLS",
            lines: vec!["a1 NO TLS not available here"],
        },
        Step::Close,
    ])
    .await;
    let mut config = config_for(mock.port, mock.port);
    config.imap.security = Security::Starttls;
    let error = ImapSession::connect(&config.imap)
        .await
        .expect_err("a server without STARTTLS must not be used");
    assert!(error.is::<FatalError>(), "{error}");
    assert!(error.to_string().contains("STARTTLS"), "{error}");
    assert!(error.to_string().contains("not available"), "{error}");
}

// ─── Read deadlines ────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_never_answers_times_out_instead_of_blocking() {
    // Accept the connection, then say nothing at all: without a deadline the
    // poll loop would wait here forever.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(socket);
    });
    let config = config_for(port, port);

    let started = std::time::Instant::now();
    let error = ImapSession::connect(&config.imap)
        .await
        .expect_err("no greeting means no session");
    assert!(
        error.to_string().contains("did not answer within"),
        "{error}"
    );
    assert!(started.elapsed() < Duration::from_secs(10), "{error}");
    // A silent server is worth retrying, not giving up on.
    assert!(!crate::delivery::is_permanent_error(&error.to_string()));

    let started = std::time::Instant::now();
    let error = SmtpSession::connect(&SmtpConfig {
        host: "127.0.0.1".into(),
        port,
        from: "bot@example.com".into(),
        security: Security::Plain,
        timeout_seconds: TEST_TIMEOUT_SECONDS,
        ..SmtpConfig::default()
    })
    .await
    .expect_err("no greeting means no session");
    assert!(
        error.to_string().contains("did not answer within"),
        "{error}"
    );
    assert!(started.elapsed() < Duration::from_secs(10), "{error}");
}

#[test]
fn the_read_deadline_is_never_zero() {
    assert_eq!(read_deadline(0), Duration::from_secs(1));
    assert_eq!(read_deadline(45), Duration::from_secs(45));
    assert_eq!(SmtpConfig::default().timeout_seconds, 60);
    assert_eq!(ImapConfig::default().timeout_seconds, 60);
    assert_eq!(DEFAULT_READ_TIMEOUT, Duration::from_secs(60));
}

// ─── IMAP failures the poll loop must survive ──────────────────────────────

#[tokio::test]
async fn every_imap_command_reports_its_own_refusal() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 UID SEARCH UNSEEN",
            lines: vec!["a1 NO search is not supported"],
        },
        Step::Reply {
            expect: "a2 UID FETCH 7 (BODY.PEEK[])",
            lines: vec!["a2 NO message is gone"],
        },
        Step::Reply {
            expect: "a3 UID FETCH 8 (BODY.PEEK[])",
            lines: vec!["a3 OK but nothing follows"],
        },
        Step::Reply {
            expect: "a4 UID STORE 7 +FLAGS (\\Seen)",
            lines: vec!["a4 NO mailbox is read-only"],
        },
        Step::Close,
    ])
    .await;
    let config = config_for(mock.port, mock.port);
    let mut session = ImapSession::connect(&config.imap).await.expect("connect");

    let error = session
        .search_unseen()
        .await
        .expect_err("a refused search is an error");
    assert!(error.to_string().contains("UID SEARCH UNSEEN"), "{error}");
    assert!(error.to_string().contains("not supported"), "{error}");

    let error = session
        .fetch_body(7)
        .await
        .expect_err("a refused fetch is an error");
    assert!(error.to_string().contains("UID FETCH 7"), "{error}");

    // An `OK` carrying no body is still a failure: answering nothing would
    // silently lose the message.
    let error = session
        .fetch_body(8)
        .await
        .expect_err("an empty fetch is an error");
    assert!(
        error.to_string().contains("no body for message 8"),
        "{error}"
    );

    let error = session
        .mark_seen(7)
        .await
        .expect_err("a refused store is an error");
    assert!(error.to_string().contains("read-only"), "{error}");
    mock.assert_clean();
}

#[tokio::test]
async fn a_search_answer_without_a_results_line_is_an_error() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 UID SEARCH UNSEEN",
            lines: vec!["* OK nothing to report", "a1 OK done"],
        },
        Step::Close,
    ])
    .await;
    let config = config_for(mock.port, mock.port);
    let mut session = ImapSession::connect(&config.imap).await.expect("connect");
    let error = session
        .search_unseen()
        .await
        .expect_err("a missing SEARCH line is an error");
    assert!(error.to_string().contains("without results"), "{error}");
}

#[tokio::test]
async fn every_unseen_uid_is_reported() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 UID SEARCH UNSEEN",
            lines: vec!["* SEARCH 4 9", "a1 OK done"],
        },
        Step::Close,
    ])
    .await;
    let config = config_for(mock.port, mock.port);
    let mut session = ImapSession::connect(&config.imap).await.expect("connect");
    assert_eq!(session.search_unseen().await.expect("search"), vec![4, 9]);
}

// ─── The poll loop's own arms ──────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn the_poller_answers_mail_and_skips_what_it_must_not() {
    let (grpc, state) = crate::test_support::spawn_mock_grpc(Default::default()).await;
    let message = raw_message(
        "From: Alice <alice@example.com>\r\nSubject: hello\r\nMessage-ID: <m1@example.com>",
        "a question",
    );
    // A message the allowlist refuses, and one whose Message-ID was already
    // answered.
    let blocked = raw_message(
        "From: Bob <bob@example.com>\r\nSubject: hi\r\nMessage-ID: <m2@example.com>",
        "let me in",
    );
    let repeat = raw_message(
        "From: Alice <alice@example.com>\r\nSubject: hello again\r\nMessage-ID: <m1@example.com>",
        "the same message again",
    );
    let imap = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 LOGIN",
            lines: vec!["a1 OK logged in"],
        },
        Step::Reply {
            expect: "a2 SELECT",
            lines: vec!["* 3 EXISTS", "* OK [UIDVALIDITY 42] ok", "a2 OK done"],
        },
        Step::Reply {
            expect: "a3 UID SEARCH UNSEEN",
            lines: vec!["* SEARCH 7 8 9", "a3 OK done"],
        },
        Step::Fetch {
            expect: "a4 UID FETCH 7",
            uid: 7,
            payload: message,
            completion: "a4 OK done",
        },
        Step::Reply {
            expect: "a5 UID STORE 7 +FLAGS (\\Seen)",
            lines: vec!["a5 OK done"],
        },
        Step::Fetch {
            expect: "a6 UID FETCH 8",
            uid: 8,
            payload: blocked,
            completion: "a6 OK done",
        },
        Step::Reply {
            expect: "a7 UID STORE 8 +FLAGS (\\Seen)",
            lines: vec!["a7 OK done"],
        },
        Step::Fetch {
            expect: "a8 UID FETCH 9",
            uid: 9,
            payload: repeat,
            completion: "a8 OK done",
        },
        Step::Reply {
            expect: "a9 UID STORE 9 +FLAGS (\\Seen)",
            lines: vec!["a9 OK done"],
        },
        Step::Reply {
            expect: "a10 LOGOUT",
            lines: vec!["* BYE bye", "a10 OK done"],
        },
        Step::Close,
    ])
    .await;

    let data_dir = temp_dir("email-poller-skips");
    let mut config = config_value(imap.port, 1);
    config["sender_allowlist"] = json!(["alice@example.com"]);
    let shutdown = crate::bridge::Shutdown::new();
    let ctx = ctx_at(data_dir.clone(), &grpc, config, shutdown.clone());
    let running = tokio::spawn(async move { Email.run(ctx).await });

    let marked = wait_until(
        || imap.saw("a9 UID STORE 9 +FLAGS (\\Seen)"),
        Duration::from_secs(20),
    )
    .await;
    assert!(marked, "the poll runs to the end: {:?}", imap.lines());
    shutdown.trigger();
    let stopped = tokio::time::timeout(Duration::from_secs(10), running)
        .await
        .expect("the poller stops")
        .expect("no panic");
    stopped.expect("a clean stop");
    imap.assert_clean();

    // Only the allowlisted sender produced a turn; the other two were read and
    // marked seen without becoming prompts.
    let prompts = crate::test_support::recorded_of(&state, "prompt");
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    // Every message is remembered, refused ones included, so a restart does not
    // read them again.
    let seen: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(data_dir.join("seen.json")).expect("the store exists"),
    )
    .expect("valid JSON");
    let keys: Vec<String> = seen["keys"]
        .as_array()
        .expect("keys")
        .iter()
        .filter_map(|key| key.as_str().map(str::to_string))
        .collect();
    for expected in ["42:7", "42:8", "42:9", "mid:m1@example.com"] {
        assert!(
            keys.contains(&expected.to_string()),
            "{expected} in {keys:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_already_delivered_under_another_uid_is_marked_without_a_turn() {
    let (grpc, state) = crate::test_support::spawn_mock_grpc(Default::default()).await;
    // The first copy is new, the second repeats its `Message-ID` under a new
    // mailbox position.
    let message = raw_message(
        "From: Alice <alice@example.com>\r\nSubject: hello\r\nMessage-ID: <first@example.com>",
        "hello",
    );
    let repeat = raw_message(
        "From: Alice <alice@example.com>\r\nSubject: hello\r\nMessage-ID: <first@example.com>",
        "hello",
    );
    let imap = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 LOGIN",
            lines: vec!["a1 OK logged in"],
        },
        Step::Reply {
            expect: "a2 SELECT",
            lines: vec!["* 2 EXISTS", "* OK [UIDVALIDITY 7] ok", "a2 OK done"],
        },
        Step::Reply {
            expect: "a3 UID SEARCH UNSEEN",
            lines: vec!["* SEARCH 7 8", "a3 OK done"],
        },
        Step::Fetch {
            expect: "a4 UID FETCH 7",
            uid: 7,
            payload: message.clone(),
            completion: "a4 OK done",
        },
        Step::Reply {
            expect: "a5 UID STORE 7 +FLAGS (\\Seen)",
            lines: vec!["a5 OK done"],
        },
        // The same Message-ID at a second mailbox position: the UID differs, so
        // only the identity recorded in the store catches it.
        Step::Fetch {
            expect: "a6 UID FETCH 8",
            uid: 8,
            payload: repeat,
            completion: "a6 OK done",
        },
        Step::Reply {
            expect: "a7 UID STORE 8 +FLAGS (\\Seen)",
            lines: vec!["a7 OK done"],
        },
        Step::Reply {
            expect: "a8 LOGOUT",
            lines: vec!["a8 OK done"],
        },
        Step::Close,
    ])
    .await;

    let data_dir = temp_dir("email-poller-dup");

    let shutdown = crate::bridge::Shutdown::new();
    let ctx = ctx_at(
        data_dir,
        &grpc,
        config_value(imap.port, 1),
        shutdown.clone(),
    );
    let running = tokio::spawn(async move { Email.run(ctx).await });

    let marked = wait_until(
        || imap.saw("a7 UID STORE 8 +FLAGS (\\Seen)"),
        Duration::from_secs(20),
    )
    .await;
    assert!(marked, "{:?}", imap.lines());
    shutdown.trigger();
    let _ = tokio::time::timeout(Duration::from_secs(10), running).await;
    imap.assert_clean();
    // One turn for the first copy, none for the second.
    assert_eq!(crate::test_support::recorded_of(&state, "prompt").len(), 1);
}

// ─── MIME depth and address escapes ────────────────────────────────────────

/// A message nested `depth` multiparts deep, innermost a `text/plain` part.
fn nested_multipart(depth: usize) -> Vec<u8> {
    if depth == 0 {
        return b"Content-Type: text/plain\r\n\r\nbottom".to_vec();
    }
    let boundary = format!("b{depth}");
    let inner = nested_multipart(depth - 1);
    let mut message =
        format!("Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n").into_bytes();
    message.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    message.extend_from_slice(&inner);
    message.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    message
}

#[test]
fn nesting_within_the_limit_is_followed_to_the_bottom() {
    // A positive control: the nesting has to be readable before the limit below
    // is a statement about the limit rather than about the fixture.
    let mail = parse_mail(&nested_multipart(3));
    assert_eq!(mail.text, "bottom");
    assert!(mail.attachments.is_empty());

    let mail = parse_mail(&nested_multipart(MAX_MIME_DEPTH));
    assert_eq!(mail.text, "bottom");
}

#[test]
fn nesting_past_the_limit_is_not_followed() {
    let mail = parse_mail(&nested_multipart(MAX_MIME_DEPTH + 2));
    // Past the limit the part is opaque: nothing is claimed about its text.
    assert!(mail.text.is_empty(), "{:?}", mail.text);
}

#[test]
fn the_depth_guard_is_a_plain_refusal() {
    let headers = parse_headers("Content-Type: multipart/mixed; boundary=\"x\"");
    let mut leaves = Vec::new();
    collect_leaves(
        &headers,
        b"--x\r\n\r\n--x--\r\n",
        MAX_MIME_DEPTH + 1,
        &mut leaves,
    );
    assert!(leaves.is_empty());
}

#[test]
fn a_backslash_outside_a_comment_escapes_the_next_character() {
    let parsed = parse_address("Bob \\X <bob@example.com>").expect("address");
    assert_eq!(parsed.address, "bob@example.com");
    // The escape is consumed, and the display name keeps the letter it escaped.
    assert_eq!(parsed.display.as_deref(), Some("Bob X"));
}

// ─── SMTP refusals and fallbacks ───────────────────────────────────────────

#[tokio::test]
async fn a_greeting_that_is_not_a_greeting_fails_the_connection() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["554 no service for you"],
        },
        Step::Close,
    ])
    .await;
    let mut config = config_for(mock.port, mock.port);
    config.smtp.security = Security::Plain;
    let error = SmtpSession::connect(&config.smtp)
        .await
        .expect_err("a 554 greeting is not a greeting");
    assert!(error.to_string().contains("greeting"), "{error}");
    assert!(error.to_string().contains("554"), "{error}");
}

#[tokio::test]
async fn ehlo_falls_back_to_helo_and_reports_no_capabilities() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            lines: vec!["500 command not recognized"],
        },
        Step::Reply {
            expect: "HELO example.com",
            lines: vec!["250 hello"],
        },
        Step::Reply {
            expect: "QUIT",
            lines: vec!["221 bye"],
        },
        Step::Close,
    ])
    .await;
    let mut config = config_for(mock.port, mock.port);
    config.smtp.security = Security::Plain;
    let mut session = SmtpSession::connect(&config.smtp)
        .await
        .expect("HELO is enough to continue");
    // A server that needed HELO offered nothing, so no extension is available.
    assert!(session.capabilities.is_empty());
    assert!(!session.supports("STARTTLS"));
    assert!(session.auth_mechanisms().is_empty());
    let _ = session.quit().await;
    mock.assert_clean();
}

#[tokio::test]
async fn a_helo_that_is_also_refused_fails_the_connection() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            lines: vec!["502 not implemented"],
        },
        Step::Reply {
            expect: "HELO example.com",
            lines: vec!["550 go away"],
        },
        Step::Close,
    ])
    .await;
    let mut config = config_for(mock.port, mock.port);
    config.smtp.security = Security::Plain;
    let error = SmtpSession::connect(&config.smtp)
        .await
        .expect_err("neither greeting is accepted");
    assert!(error.to_string().contains("HELO"), "{error}");
}

#[tokio::test]
async fn an_ehlo_refused_before_the_fallback_threshold_is_reported_as_ehlo() {
    // 421 is below the "too old for EHLO" threshold, so falling back to HELO
    // would be wrong: the server is telling us to come back later.
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            lines: vec!["421 try again later"],
        },
        Step::Close,
    ])
    .await;
    let mut config = config_for(mock.port, mock.port);
    config.smtp.security = Security::Plain;
    let error = SmtpSession::connect(&config.smtp)
        .await
        .expect_err("a 421 EHLO is refused");
    assert!(error.to_string().contains("EHLO"), "{error}");
    assert!(!mock.saw("HELO example.com"), "{:?}", mock.lines());
}

#[tokio::test]
async fn auth_plain_answers_a_challenge_when_the_server_asks_for_one() {
    let payload = base64_encode(b"\0bot@example.com\0secret");
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            lines: vec!["250-mock", "250 AUTH PLAIN"],
        },
        Step::ReplyOwned {
            expect: format!("AUTH PLAIN {payload}"),
            lines: vec!["334 go ahead".into()],
        },
        Step::ReplyOwned {
            expect: payload.clone(),
            lines: vec!["235 accepted".into()],
        },
        Step::Reply {
            expect: "QUIT",
            lines: vec!["221 bye"],
        },
        Step::Close,
    ])
    .await;
    let mut config = config_for(mock.port, mock.port);
    config.smtp.security = Security::Plain;
    config.smtp.username = "bot@example.com".into();
    config.smtp.password = "secret".into();
    let mut session = SmtpSession::connect(&config.smtp).await.expect("connect");
    assert!(session.supports("AUTH"));
    let _ = session.quit().await;
    mock.assert_clean();
    assert!(mock.saw(&payload), "{:?}", mock.lines());
}

#[tokio::test]
async fn auth_login_stops_at_a_refusal_instead_of_guessing_at_the_next_challenge() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            lines: vec!["250-mock", "250 AUTH LOGIN"],
        },
        Step::Reply {
            expect: "AUTH LOGIN",
            lines: vec!["535 5.7.8 bad credentials"],
        },
        Step::Close,
    ])
    .await;
    let mut config = config_for(mock.port, mock.port);
    config.smtp.security = Security::Plain;
    config.smtp.username = "bot@example.com".into();
    config.smtp.password = "secret".into();
    let error = SmtpSession::connect(&config.smtp)
        .await
        .expect_err("a refused AUTH LOGIN is fatal");
    let text = error.to_string();
    assert!(text.contains("AUTH LOGIN"), "{text}");
    assert!(text.contains("unauthorized"), "{text}");
    // Nothing was encoded and sent after the refusal.
    assert!(
        !mock
            .lines()
            .iter()
            .any(|line| line == &base64_encode(b"secret")),
        "{:?}",
        mock.lines()
    );
}

#[tokio::test]
async fn a_recipient_the_server_refuses_names_the_address() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            // A single-line reply: `250-` would promise a continuation.
            lines: vec!["250 accepted"],
        },
        Step::Reply {
            expect: "MAIL FROM:<bot@example.com>",
            lines: vec!["250 sender ok"],
        },
        Step::Reply {
            expect: "RCPT TO:<gone@example.com>",
            lines: vec!["550 5.1.1 no such user"],
        },
        Step::Close,
    ])
    .await;
    let mut config = config_for(mock.port, mock.port);
    config.smtp.security = Security::Plain;
    let sender = EmailSender::new(config);
    let conversation = ConversationRef {
        id: "gone@example.com".into(),
        ..ConversationRef::default()
    };
    let error = sender
        .send_text(&conversation, "hello")
        .await
        .err()
        .unwrap_or_else(|| panic!("the server refused the recipient: {:?}", mock.lines()));
    let text = error.to_string();
    assert!(text.contains("gone@example.com"), "{text}");
    assert!(text.contains("recipient not found"), "{text}");
    assert!(crate::delivery::is_permanent_error(&text), "{text}");
    // The message itself was never sent.
    assert!(!mock.saw("DATA"), "{:?}", mock.lines());
}

#[tokio::test]
async fn a_refused_mail_from_names_the_stage_that_failed() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 mock ready"],
        },
        Step::Reply {
            expect: "EHLO example.com",
            // A single-line reply: `250-` would promise a continuation.
            lines: vec!["250 accepted"],
        },
        Step::Reply {
            expect: "MAIL FROM:<bot@example.com>",
            lines: vec!["550 relaying denied"],
        },
        Step::Close,
    ])
    .await;
    let mut config = config_for(mock.port, mock.port);
    config.smtp.security = Security::Plain;
    let sender = EmailSender::new(config);
    let conversation = ConversationRef {
        id: "alice@example.com".into(),
        ..ConversationRef::default()
    };
    let error = sender
        .send_text(&conversation, "hello")
        .await
        .err()
        .unwrap_or_else(|| panic!("the sender was refused: {:?}", mock.lines()));
    assert!(error.to_string().contains("MAIL FROM"), "{error}");
    assert!(
        !mock.saw("RCPT TO:<alice@example.com>"),
        "{:?}",
        mock.lines()
    );
}

#[test]
fn every_refusal_the_server_can_make_is_classified() {
    let of = |code: u16, text: &str| SmtpReply {
        code,
        lines: vec![text.to_string()],
    };
    // Credentials: permanent.
    for code in [530, 534, 535, 538] {
        let text = smtp_failure("AUTH PLAIN", &of(code, "nope")).to_string();
        assert!(text.contains("unauthorized"), "{code}: {text}");
        assert!(crate::delivery::is_permanent_error(&text), "{code}: {text}");
    }
    // Addresses: permanent.
    for code in [550, 551, 553] {
        let text = smtp_failure("RCPT TO", &of(code, "no")).to_string();
        assert!(text.contains("recipient not found"), "{code}: {text}");
        assert!(crate::delivery::is_permanent_error(&text), "{code}: {text}");
    }
    // Too large: permanent, and the size vocabulary is the shared one.
    let text = smtp_failure("DATA", &of(552, "message exceeds size limit")).to_string();
    assert!(text.contains("message too long"), "{text}");
    assert!(crate::delivery::is_permanent_error(&text), "{text}");
    // A transient 4xx stays retryable.
    let text = smtp_failure("DATA", &of(451, "try later")).to_string();
    assert!(!crate::delivery::is_permanent_error(&text), "{text}");
    // The stage is always named, so a log line says what was being attempted.
    assert!(text.contains("DATA"), "{text}");
}

#[test]
fn a_server_that_offers_nothing_names_that_in_the_refusal() {
    let error = select_mechanism(&[]).expect_err("nothing is offered");
    assert!(error.to_string().contains("none"), "{error}");
    assert!(error.to_string().contains("unauthorized"), "{error}");
    assert!(error.is::<FatalError>(), "{error}");

    assert_eq!(AuthMechanism::Login.name(), "LOGIN");
    assert_eq!(AuthMechanism::Plain.name(), "PLAIN");
}

#[test]
fn a_session_debug_line_reports_what_the_server_offered() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let rendered = runtime.block_on(async {
        let mock = spawn(vec![
            Step::Greet {
                lines: vec!["220 mock ready"],
            },
            Step::Reply {
                expect: "EHLO",
                lines: vec!["250-mock", "250 AUTH PLAIN"],
            },
            Step::Close,
        ])
        .await;
        let mut config = config_for(mock.port, mock.port);
        config.smtp.security = Security::Plain;
        let session = SmtpSession::connect(&config.smtp).await.expect("connect");
        format!("{session:?}")
    });
    // A log line from a failed connection should say what the server offered.
    assert!(rendered.contains("SmtpSession"), "{rendered}");
    assert!(rendered.contains("AUTH PLAIN"), "{rendered}");
}

// ─── IMAP framing limits ───────────────────────────────────────────────────

#[tokio::test]
async fn a_server_that_says_nothing_is_not_an_imap_server() {
    let mock = spawn(vec![Step::Close]).await;
    let config = config_for(mock.port, mock.port);
    let error = ImapSession::connect(&config.imap)
        .await
        .expect_err("a connection with no greeting is not usable");
    assert!(error.to_string().contains("without a greeting"), "{error}");
}

#[tokio::test]
async fn a_greeting_from_another_protocol_is_refused() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["220 smtp on the wrong port"],
        },
        Step::Close,
    ])
    .await;
    let config = config_for(mock.port, mock.port);
    let error = ImapSession::connect(&config.imap)
        .await
        .expect_err("an SMTP banner is not an IMAP greeting");
    assert!(
        error.to_string().contains("not an IMAP greeting"),
        "{error}"
    );
}

#[tokio::test]
async fn a_connection_that_ends_mid_command_is_reported() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 LOGIN",
            lines: vec![],
        },
        Step::Close,
    ])
    .await;
    let config = config_for(mock.port, mock.port);
    let mut session = ImapSession::connect(&config.imap).await.expect("connect");
    let error = session
        .login(&config.imap)
        .await
        .expect_err("the server left without answering");
    assert!(error.to_string().contains("LOGIN"), "{error}");
}

#[tokio::test]
async fn an_over_long_line_is_refused_rather_than_buffered() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 LOGIN",
            // No line break at all, and far past any sane protocol line.
            lines: vec![],
        },
        Step::Close,
    ])
    .await;
    let config = config_for(mock.port, mock.port);
    let mut session = ImapSession::connect(&config.imap).await.expect("connect");
    session.stream.buffer = vec![b'x'; MAX_LINE_BYTES + 1];
    let error = session.login(&config.imap).await.expect_err("must refuse");
    assert!(
        error.to_string().contains("without a line break"),
        "{error}"
    );
}

#[tokio::test]
async fn a_literal_larger_than_the_cap_is_refused() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::AnnounceLiteral {
            expect: "a1 UID FETCH 7",
            uid: 7,
            size: MAX_LITERAL_BYTES + 1,
        },
        Step::Close,
    ])
    .await;
    let config = config_for(mock.port, mock.port);
    let mut session = ImapSession::connect(&config.imap).await.expect("connect");
    let error = session.fetch_body(7).await.expect_err("must refuse");
    assert!(error.to_string().contains("refusing to buffer"), "{error}");
    assert!(crate::delivery::is_permanent_error(&error.to_string()));
}

#[tokio::test]
async fn a_body_arriving_in_several_writes_is_assembled() {
    let message = raw_message(
        "From: alice@example.com\r\nSubject: big\r\nMessage-ID: <big@example.com>",
        &"a line that is long enough to arrive on its own\r\n".repeat(40),
    );
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::FetchSplit {
            expect: "a1 UID FETCH 7",
            uid: 7,
            payload: message.clone(),
            completion: "a1 OK done",
        },
        Step::Close,
    ])
    .await;
    let config = config_for(mock.port, mock.port);
    let mut session = ImapSession::connect(&config.imap).await.expect("connect");
    let body = session.fetch_body(7).await.expect("the body arrives");
    assert_eq!(body, message);
    mock.assert_clean();
}

#[tokio::test]
async fn a_peer_that_goes_away_mid_literal_is_reported() {
    let mock = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::AnnounceLiteral {
            expect: "a1 UID FETCH 7",
            uid: 7,
            size: 64,
        },
        Step::Close,
    ])
    .await;
    let config = config_for(mock.port, mock.port);
    let mut session = ImapSession::connect(&config.imap).await.expect("connect");
    let error = session
        .fetch_body(7)
        .await
        .expect_err("must report the loss");
    assert!(error.to_string().contains("mid-payload"), "{error}");
}

#[test]
fn fetch_records_without_a_uid_are_still_paired_with_their_body() {
    let response = ImapResponse {
        status: "OK".into(),
        detail: "done".into(),
        lines: vec!["* 1 FETCH (BODY[] {3}".into()],
        literals: vec![Literal {
            after_line: 0,
            bytes: b"abc".to_vec(),
        }],
    };
    let records = fetch_records(&response);
    assert_eq!(records.len(), 1);
    // A server that did not echo the UID item leaves it unknown, which is what
    // makes a single-message FETCH still usable.
    assert_eq!(records[0].uid, None);
    assert_eq!(records[0].body.as_deref(), Some(&b"abc"[..]));
}

#[test]
fn protocol_lines_strip_their_reply_code_and_keep_short_lines_whole() {
    assert_eq!(strip_reply_code("250 AUTH PLAIN"), "AUTH PLAIN");
    assert_eq!(strip_reply_code("250-STARTTLS"), "STARTTLS");
    // Too short to carry a code, and a line whose fourth byte is neither a
    // space nor a hyphen.
    assert_eq!(strip_reply_code("250"), "250");
    assert_eq!(strip_reply_code("2500 not a reply"), "2500 not a reply");
    assert_eq!(strip_reply_code(""), "");
    assert_eq!(strip_reply_code("abc def"), "abc def");
}

#[test]
fn the_round_trip_date_is_parsed_from_the_envelope_state() {
    let mail = mail_from(
        "From: alice@example.com\r\nDate: Fri, 21 Nov 1997 09:55:06 -0600\r\nMessage-ID: <m@example.com>",
        "hi",
    );
    let config = config_for(1, 1);
    let inbound = to_inbound(
        9,
        &MailboxState::default(),
        "bot@example.com",
        &config,
        &mail,
    )
    .expect("prompt");
    let raw = inbound.raw.expect("raw");
    // The parsed date is recorded in milliseconds next to the header text.
    assert!(raw["date_ms"].is_i64(), "{raw}");

    let undated = mail_from("From: alice@example.com\r\nSubject: x", "hi");
    let inbound = to_inbound(
        9,
        &MailboxState::default(),
        "bot@example.com",
        &config,
        &undated,
    )
    .expect("prompt");
    assert!(inbound.raw.expect("raw")["date_ms"].is_null());
}

// ─── MIME: headers, parameters and parts ───────────────────────────────────

#[test]
fn a_line_that_is_not_a_header_is_ignored() {
    let headers = parse_headers("From: a@b\r\ngarbage without a colon\r\nSubject: x\r\n");
    assert_eq!(headers.get("from"), Some("a@b"));
    assert_eq!(headers.get("subject"), Some("x"));
    assert_eq!(headers.names().len(), 2);
}

#[test]
fn ignored_content_type_parameters_are_skipped_rather_than_stored() {
    // A bare token and an empty name are both malformed; neither may become a
    // parameter, and neither may swallow the one that follows.
    let (kind, parameters) = parse_content_type("text/plain; bare; charset=utf-8");
    assert_eq!(kind, "text/plain");
    assert_eq!(
        parameters,
        vec![("charset".to_string(), "utf-8".to_string())]
    );

    let (kind, parameters) = parse_content_type("text/plain; =orphan; charset=utf-8");
    assert_eq!(kind, "text/plain");
    assert_eq!(
        parameters,
        vec![("charset".to_string(), "utf-8".to_string())]
    );
}

#[test]
fn a_quoted_parameter_keeps_its_escapes_and_its_semicolon() {
    let (kind, parameters) = parse_content_type("application/pdf; name=\"a\\\"b;c\\d\"");
    assert_eq!(kind, "application/pdf");
    assert_eq!(
        parameters,
        vec![("name".to_string(), "a\"b;c\\d".to_string())]
    );
}

#[test]
fn a_multipart_without_a_boundary_is_not_walked() {
    let mail = mail_from(
        "From: a@b\r\nContent-Type: multipart/mixed; charset=utf-8",
        "--not-the-boundary\r\ntext\r\n--not-the-boundary--",
    );
    // No boundary means no parts to read, so there is no body and no attachment
    // invented from the raw text.
    assert!(mail.text.is_empty(), "{:?}", mail.text);
    assert!(mail.attachments.is_empty());
}

#[test]
fn an_inline_part_that_is_not_text_is_still_listed() {
    let raw = raw_message(
        "From: a@b\r\nContent-Type: multipart/mixed; boundary=\"m\"",
        "--m\r\nContent-Type: text/plain\r\n\r\nthe reply\r\n--m\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n--m--\r\n",
    );
    let mail = parse_mail(&raw);
    assert_eq!(mail.text, "the reply");
    // Inline, unnamed, and still reported: the reader is told what arrived.
    assert_eq!(mail.attachments.len(), 1);
    assert_eq!(mail.attachments[0].filename, None);
    assert_eq!(mail.attachments[0].content_type, "image/png");
}

#[test]
fn a_part_that_looks_like_a_multipart_but_is_an_attachment_is_left_alone() {
    // The disposition wins over the content type: a forwarded message must not
    // be unrolled into the reply's own parts.
    let raw = raw_message(
        "From: a@b\r\nContent-Type: multipart/mixed; boundary=\"outer\"",
        "--outer\r\nContent-Type: multipart/mixed; boundary=\"inner\"\r\nContent-Disposition: attachment; filename=\"forwarded.eml\"\r\n\r\n--inner\r\nContent-Type: text/plain\r\n\r\nnot my text\r\n--inner--\r\n--outer--\r\n",
    );
    let mail = parse_mail(&raw);
    assert!(mail.text.is_empty(), "{:?}", mail.text);
    assert_eq!(mail.attachments.len(), 1);
    assert_eq!(
        mail.attachments[0].filename.as_deref(),
        Some("forwarded.eml")
    );
}

// ─── Decoding edge cases ───────────────────────────────────────────────────

#[test]
fn base64_bodies_are_accepted_unpadded_and_refused_when_they_are_not_base64() {
    // Unpadded input is what some clients emit; the padding is restored.
    assert_eq!(decode_base64(b"aGVsbG8"), b"hello");
    assert_eq!(decode_base64(b"aGVsbG8="), b"hello");
    // Whitespace inside a wrapped body is not part of the payload.
    assert_eq!(decode_base64(b"aGVs\r\nbG8="), b"hello");
    // Something that cannot be base64 keeps the bytes it arrived with, so a
    // malformed part still reaches the reader.
    assert_eq!(decode_base64(b"!!!!"), b"!!!!");
}

#[test]
fn quoted_printable_handles_soft_breaks_and_a_stray_equals() {
    // A bare LF ends a line just as CRLF does.
    assert_eq!(decode_quoted_printable(b"one=\ntwo"), b"onetwo");
    assert_eq!(decode_quoted_printable(b"one=\r\ntwo"), b"onetwo");
    // An equals sign that is not an escape stays put: the pair is not hex, and
    // there is nothing after it at all.
    assert_eq!(decode_quoted_printable(b"a=zzb"), b"a=zzb");
    assert_eq!(decode_quoted_printable(b"trailing="), b"trailing=");
    // A valid hex escape beside an invalid one: only the valid pair decodes.
    assert_eq!(decode_quoted_printable(b"=5E"), b"^");
    assert_eq!(decode_quoted_printable(b"=zz"), b"=zz");
    assert_eq!(decode_quoted_printable(b"=5E=zz"), b"^=zz");
}

#[test]
fn text_entities_are_decoded_named_numeric_or_left_alone() {
    assert_eq!(
        decode_entities(
            "a &ndash; b &mdash; c &hellip; \u{2018}q\u{2019} \u{201c}w\u{201d} &copy;"
        ),
        "a – b — c … ‘q’ “w” ©"
    );
    assert_eq!(decode_entities("&#8212;&#x2014;&#65;"), "——A");
    // An entity nobody knows, a code point that is not a character, and an
    // ampersand that never terminates are all kept verbatim.
    assert_eq!(decode_entities("&nosuch;"), "&nosuch;");
    assert_eq!(decode_entities("&#xD800;"), "&#xD800;");
    assert_eq!(decode_entities("&#xZZ;"), "&#xZZ;");
    assert_eq!(decode_entities("a & b"), "a & b");
    assert_eq!(decode_entities("a & &amp; b"), "a & & b");
}

#[test]
fn html_that_never_closes_a_tag_still_yields_the_text_before_it() {
    // A truncated document and a tag with no `>` at all.
    assert_eq!(strip_html("<p>hello</p><p>wor"), "hello\n\nwor");
    assert_eq!(strip_html("before <b"), "before");
    // Script content is not text, however the document ends.
    assert_eq!(strip_html("<p>a</p><script>var x = 1; no closing tag"), "a");
}

#[test]
fn a_body_that_is_only_markup_collapses_to_nothing() {
    // The trailing blank line a block tag leaves must not become content, and
    // an empty document must stay empty rather than becoming a newline.
    assert_eq!(strip_html("<div></div>"), "");
    assert_eq!(strip_html("<p>one</p>"), "one");
    assert_eq!(strip_html(""), "");
}

#[test]
fn a_comment_may_hold_an_escaped_paren_and_nest() {
    // The escapes and the nesting are both legal in an address comment, and
    // neither may truncate the address that follows.
    let parsed = parse_address("alice@example.com (a \\( b (nested) c)").expect("address");
    assert_eq!(parsed.address, "alice@example.com");
    // A lone backslash outside a comment is part of the value.
    let parsed = parse_address("alice@example.com").expect("address");
    assert_eq!(parsed.display, None);
}

#[test]
fn media_kinds_come_from_the_content_type() {
    assert_eq!(media_kind("image/png"), MediaKind::Image);
    assert_eq!(media_kind("audio/ogg"), MediaKind::Audio);
    assert_eq!(media_kind("video/mp4"), MediaKind::Video);
    assert_eq!(media_kind("application/pdf"), MediaKind::Document);
    // Something that is not a type at all is still reported as a document
    // rather than dropped.
    assert_eq!(media_kind(""), MediaKind::Document);
}

#[test]
fn every_refusal_has_a_reason_a_user_could_read() {
    for skip in [
        Skip::OwnMessage,
        Skip::Automated,
        Skip::NoSender,
        Skip::NotAllowed,
        Skip::NoSubjectMatch,
    ] {
        let reason = skip.reason();
        assert!(reason.len() > 10, "{skip:?}: {reason}");
        assert!(!reason.contains("Skip"), "{reason}");
    }
    assert!(Skip::OwnMessage.reason().contains("own address"));
    assert!(Skip::NoSender.reason().contains("From"));
    assert!(Skip::NotAllowed.reason().contains("allowlist"));
    assert!(Skip::NoSubjectMatch.reason().contains("prefix"));
}

#[test]
fn a_subject_that_is_only_whitespace_is_not_remembered() {
    let mut cache = SubjectCache::default();
    cache.insert("email:a@b", "");
    cache.insert("email:a@b", "   ");
    assert_eq!(cache.get("email:a@b"), None);
    cache.insert("email:a@b", "a subject");
    assert_eq!(cache.get("email:a@b"), Some("a subject"));
    // Re-inserting an existing conversation updates it instead of duplicating
    // it, so the cache cannot grow without bound on one busy thread.
    cache.insert("email:a@b", "a newer subject");
    assert_eq!(cache.get("email:a@b"), Some("a newer subject"));
}

#[test]
fn the_subject_cache_evicts_the_oldest_conversation() {
    let mut cache = SubjectCache::default();
    for index in 0..SUBJECT_CACHE + 3 {
        cache.insert(
            &format!("email:user{index}@example.com"),
            &format!("s{index}"),
        );
    }
    assert_eq!(cache.map.len(), SUBJECT_CACHE);
    assert_eq!(cache.order.len(), SUBJECT_CACHE);
    assert_eq!(cache.get("email:user0@example.com"), None);
    assert_eq!(
        cache.get(&format!("email:user{}@example.com", SUBJECT_CACHE + 2)),
        Some("s202")
    );
}

// ─── STARTTLS for IMAP, a reset socket, and the poll loop's own failures ────

#[tokio::test]
async fn an_imap_session_upgrades_to_tls_before_logging_in() {
    let mock = spawn_tls(vec![
        TlsStep::Greet {
            lines: vec!["* OK mock ready"],
        },
        TlsStep::Reply {
            expect: "a1 STARTTLS",
            lines: vec!["a1 OK begin TLS"],
        },
        // From here on the conversation is encrypted, so the password is never
        // on the wire in the clear.
        TlsStep::Upgrade,
        TlsStep::Secure {
            expect: "a2 LOGIN",
            lines: vec!["a2 OK logged in"],
        },
        TlsStep::Secure {
            expect: "a3 SELECT",
            lines: vec!["* 1 EXISTS", "a3 OK done"],
        },
        TlsStep::Secure {
            expect: "a4 LOGOUT",
            lines: vec!["a4 OK done"],
        },
        TlsStep::Close,
    ])
    .await;

    let mut config = config_for(mock.port, mock.port);
    config.imap.security = Security::Starttls;
    let tls = trusting_tls();
    let mut session = ImapSession::connect_with(&config.imap, Some(&tls))
        .await
        .expect("the upgrade succeeds");
    session.login(&config.imap).await.expect("login over TLS");
    let mailbox = session.select("INBOX").await.expect("select");
    assert_eq!(mailbox.exists, 1);
    session.logout().await.expect("logout");
    mock.assert_clean();
    assert!(
        mock.saw("a2 LOGIN \"bot@example.com\" \"secret\""),
        "{:?}",
        mock.lines()
    );
}

#[tokio::test]
async fn a_server_that_stops_speaking_tls_is_an_error_not_a_hang() {
    // The socket is up and the handshake is done, but the bytes after it are not
    // TLS at all: reading them fails the connection instead of waiting for a
    // greeting that can never arrive.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let (read, write) = socket.into_split();
        let Some(mut peer) = TlsPeer::accept(read, write).await else {
            return;
        };
        // Raw bytes, written straight to the socket past the TLS connection.
        let _ = peer.write.write_all(b"* OK not a tls record\r\n").await;
        let _ = peer.write.flush().await;
        tokio::time::sleep(Duration::from_secs(6)).await;
    });
    let mut config = config_for(port, port);
    config.imap.security = Security::Implicit;
    let tls = trusting_tls();
    let error = ImapSession::connect_with(&config.imap, Some(&tls))
        .await
        .expect_err("garbage is not a greeting");
    assert!(!error.to_string().is_empty());
}
#[tokio::test(flavor = "multi_thread")]
async fn a_store_that_cannot_be_written_is_reported_but_the_mail_is_answered() {
    let (grpc, state) = crate::test_support::spawn_mock_grpc(Default::default()).await;
    let message = raw_message(
        "From: Alice <alice@example.com>\r\nSubject: hello\r\nMessage-ID: <m1@example.com>",
        "a question",
    );
    let refused = raw_message(
        "From: Bob <bob@example.com>\r\nSubject: hello\r\nMessage-ID: <m2@example.com>",
        "let me in",
    );
    let imap = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 LOGIN",
            lines: vec!["a1 OK logged in"],
        },
        Step::Reply {
            expect: "a2 SELECT",
            lines: vec!["* 2 EXISTS", "* OK [UIDVALIDITY 42] ok", "a2 OK done"],
        },
        Step::Reply {
            expect: "a3 UID SEARCH UNSEEN",
            lines: vec!["* SEARCH 7 8", "a3 OK done"],
        },
        Step::Fetch {
            expect: "a4 UID FETCH 7",
            uid: 7,
            payload: message,
            completion: "a4 OK done",
        },
        // The mailbox refuses to record the flag: the message is still answered,
        // and stays unseen for a later run.
        Step::Reply {
            expect: "a5 UID STORE 7 +FLAGS (\\Seen)",
            lines: vec!["a5 NO the mailbox is read-only"],
        },
        Step::Fetch {
            expect: "a6 UID FETCH 8",
            uid: 8,
            payload: refused,
            completion: "a6 OK done",
        },
        Step::Reply {
            expect: "a7 UID STORE 8 +FLAGS (\\Seen)",
            lines: vec!["a7 OK done"],
        },
        Step::Reply {
            expect: "a8 LOGOUT",
            lines: vec!["a8 OK done"],
        },
        Step::Close,
    ])
    .await;

    let data_dir = temp_dir("email-poller-unwritable");
    // The state file is a directory, so writing it fails: a channel that cannot
    // remember what it did must still answer the mail.
    std::fs::create_dir_all(data_dir.join("seen.json")).expect("a directory in the file's place");
    let shutdown = crate::bridge::Shutdown::new();
    let mut config = config_value(imap.port, 1);
    // A second message the allowlist refuses: the refused-and-not-recorded path
    // is taken as well.
    config["sender_allowlist"] = json!(["alice@example.com"]);
    let ctx = ctx_at(data_dir, &grpc, config, shutdown.clone());
    let running = tokio::spawn(async move { Email.run(ctx).await });

    let asked = wait_until(
        || !crate::test_support::recorded_of(&state, "prompt").is_empty(),
        Duration::from_secs(20),
    )
    .await;
    assert!(asked, "the mail is answered: {:?}", imap.lines());
    let store_refused = wait_until(
        || imap.saw("a7 UID STORE 8 +FLAGS (\\Seen)"),
        Duration::from_secs(20),
    )
    .await;
    assert!(store_refused, "{:?}", imap.lines());
    shutdown.trigger();
    let stopped = tokio::time::timeout(Duration::from_secs(10), running)
        .await
        .expect("the poller stops")
        .expect("no panic");
    stopped.expect("a clean stop");
    imap.assert_clean();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_uid_that_was_already_recorded_is_marked_without_being_fetched() {
    let (grpc, state) = crate::test_support::spawn_mock_grpc(Default::default()).await;
    let imap = spawn(vec![
        Step::Greet {
            lines: vec!["* OK mock ready"],
        },
        Step::Reply {
            expect: "a1 LOGIN",
            lines: vec!["a1 OK logged in"],
        },
        Step::Reply {
            expect: "a2 SELECT",
            lines: vec!["* 1 EXISTS", "* OK [UIDVALIDITY 42] ok", "a2 OK done"],
        },
        Step::Reply {
            expect: "a3 UID SEARCH UNSEEN",
            lines: vec!["* SEARCH 7", "a3 OK done"],
        },
        // No FETCH step: a message an earlier run recorded is not read again, it
        // is only finished off.
        Step::Reply {
            expect: "a4 UID STORE 7 +FLAGS (\\Seen)",
            lines: vec!["a4 OK done"],
        },
        Step::Reply {
            expect: "a5 LOGOUT",
            lines: vec!["a5 OK done"],
        },
        Step::Close,
    ])
    .await;

    let data_dir = temp_dir("email-poller-seen-uid");
    let seen = SeenStore::load(data_dir.join("seen.json"));
    seen.record("42:7").expect("seed the mailbox position");

    let shutdown = crate::bridge::Shutdown::new();
    let ctx = ctx_at(
        data_dir,
        &grpc,
        config_value(imap.port, 1),
        shutdown.clone(),
    );
    let running = tokio::spawn(async move { Email.run(ctx).await });

    let finished = wait_until(
        || imap.saw("a5 LOGOUT") || imap.saw("a4 UID STORE 7 +FLAGS (\\Seen)"),
        Duration::from_secs(20),
    )
    .await;
    assert!(finished, "{:?}", imap.lines());
    shutdown.trigger();
    let _ = tokio::time::timeout(Duration::from_secs(10), running).await;
    imap.assert_clean();
    // Nothing was asked of the agent for a message that was already handled.
    assert!(crate::test_support::recorded_of(&state, "prompt").is_empty());
}

#[tokio::test]
async fn a_truncated_tls_connection_is_an_error_and_a_closed_one_is_not() {
    // Two ways for a peer to go away, and the difference matters: a
    // `close_notify` is a clean end, while a bare socket drop is a truncation
    // the channel must treat as a failure it can retry.
    for (clean, expected) in [(true, "greeting"), (false, "eof")] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        tokio::spawn(async move {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            let (read, write) = socket.into_split();
            let Some(mut peer) = TlsPeer::accept(read, write).await else {
                return;
            };
            if clean {
                peer.conn.send_close_notify();
                let _ = peer.flush().await;
            }
            // A truncated connection is the socket going away with no
            // `close_notify`.
            drop(peer);
        });

        let mut config = config_for(port, port);
        config.imap.security = Security::Implicit;
        let tls = trusting_tls();
        let error = ImapSession::connect_with(&config.imap, Some(&tls))
            .await
            .expect_err("a connection that ends is not a greeting");
        assert!(
            error.to_string().to_lowercase().contains(expected),
            "clean={clean}: {error}"
        );
    }
}

#[tokio::test]
async fn a_record_with_an_impossible_length_ends_the_read() {
    // A record header claiming more than a TLS record can hold. The connection
    // accepts no bytes for it, so the read loop has to stop rather than keep
    // feeding a chunk the connection will never take.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local_addr").port();
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let (read, write) = socket.into_split();
        let Some(mut peer) = TlsPeer::accept(read, write).await else {
            return;
        };
        // Type 23, TLS 1.2, length 65535 — past anything a record may carry.
        let _ = peer.write.write_all(&[0x17, 0x03, 0x03, 0xff, 0xff]).await;
        let _ = peer.write.flush().await;
        tokio::time::sleep(Duration::from_secs(6)).await;
    });
    let mut config = config_for(port, port);
    config.imap.security = Security::Implicit;
    let tls = trusting_tls();
    let error = ImapSession::connect_with(&config.imap, Some(&tls))
        .await
        .expect_err("an impossible record is not a greeting");
    assert!(!error.to_string().is_empty());
}

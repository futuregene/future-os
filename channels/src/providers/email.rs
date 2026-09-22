//! Email: SMTP submission and IMAP retrieval.
//!
//! Implemented directly against the two protocols rather than through a mail
//! crate, because the subset a chat bridge needs is small — submit one message,
//! read the unseen ones — and both protocols are stable, text-based, and
//! testable against a scripted session.
//!
//! * **Send** — one submission per answer: connect, `EHLO`, `STARTTLS` (or
//!   implicit TLS), `AUTH PLAIN`/`AUTH LOGIN`, `MAIL FROM`/`RCPT TO`/`DATA`,
//!   `QUIT`. A body that is not plain 7-bit ASCII is base64-encoded, which keeps
//!   non-ASCII text and long lines intact on servers without `8BITMIME`.
//! * **Receive** — `LOGIN`, `SELECT`, `UID SEARCH UNSEEN`, `UID FETCH
//!   BODY.PEEK[]` per message, then `UID STORE +FLAGS (\Seen)`. Polling rather
//!   than `IDLE` keeps one code path for every server.
//! * **Parsing** — `multipart/alternative` and `mixed` (nested), quoted-printable
//!   and base64 bodies, RFC 2047 headers, `text/plain` preferred with
//!   `text/html` reduced to text as the fallback. Attachments are *recognized*
//!   and listed, never downloaded: a mailbox is not a file transfer, and the
//!   sender can resend.
//! * **Loop safety** — our own address, automated mail (`Auto-Submitted`,
//!   `Precedence: bulk|list|junk`) and an optional subject-prefix gate are all
//!   refused, so a vacation autoresponder cannot start a mail loop with the
//!   agent.
//! * **Dedup** — the mailbox's `\Seen` flag covers the ordinary case; a small
//!   file in the channel's data directory covers the window between fetching a
//!   message and marking it, which a restart would otherwise replay.

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::bridge::{
    ChatKind, ConversationRef, HandleOutcome, Inbound, MediaKind, MediaRef, ProviderCtx, SenderRef,
};
use crate::providers::traits::{
    Capabilities, ChannelDefinition, ChannelSender, Maturity, Provider,
};
use crate::transport::ws::Backoff;
use crate::transport::LengthUnit;

#[cfg(test)]
#[path = "email_tests.rs"]
mod tests;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "email",
    display_name: "Email (IMAP + SMTP)",
    description: "Mailbox channel: IMAP polling for mail, SMTP submission for threaded replies, attachments listed but not downloaded.",
    docs: "docs/guide/channels-providers.md#email",
    maturity: Maturity::Preview,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: false,
        // Replies carry In-Reply-To/References, so a mail client threads them.
        threads: true,
        typing: false,
        reactions: false,
        // Attachment *names* are listed, but nothing is fetched from the
        // mailbox, so there is no inbound media for the model.
        media_in: false,
        media_out: false,
        mention_gate: false,
    },
    max_text_len: 100_000,
    length_unit: LengthUnit::Bytes,
    config_example: r#"{
  "enabled": true,
  "imap": { "host": "imap.example.com", "port": 993, "username": "", "password": "", "mailbox": "INBOX", "security": "implicit", "timeout_seconds": 60 },
  "smtp": { "host": "smtp.example.com", "port": 587, "username": "", "password": "", "from": "", "security": "starttls", "timeout_seconds": 60 },
  "poll_seconds": 30,
  "sender_allowlist": [],
  "subject_prefix": ""
}"#,
    requires: &["a mailbox that allows IMAP and SMTP with a password or app password"],
};

/// How long a TCP connect (and the TLS handshake on top) may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Reconnect ceiling; the supervisor's own backoff starts after this loop gives
/// up, so it stays modest.
const RECONNECT_MAX: Duration = Duration::from_secs(60);
/// A connection that stayed up this long counts as healthy, so the next retry is
/// not punished for it.
const HEALTHY_CONNECTION: Duration = Duration::from_secs(60);
/// Longest protocol line accepted before the peer is treated as not speaking the
/// protocol at all (a hostile or confused server must not grow our buffer).
const MAX_LINE_BYTES: usize = 64 * 1024;
/// How long a protocol read may block before the connection is treated as dead.
///
/// A server that accepts the connection and then stops talking would otherwise
/// block the mailbox poll loop forever: TCP cannot tell that the peer is gone,
/// so without a deadline a half-open connection is indistinguishable from a
/// slow one.
pub(crate) const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(60);
/// Largest octet payload (an IMAP literal) read into memory.
const MAX_LITERAL_BYTES: usize = 16 * 1024 * 1024;
/// Messages answered per poll. A large unseen backlog drains over several polls
/// instead of arriving as one burst of agent turns.
const MAX_FETCH_PER_POLL: usize = 20;
/// How many delivered message identifiers are remembered across restarts.
const SEEN_CAPACITY: usize = 1000;
/// How many conversation subjects are remembered for threading replies.
const SUBJECT_CACHE: usize = 200;
/// Deepest MIME nesting followed before a part is treated as opaque.
const MAX_MIME_DEPTH: usize = 8;
/// Longest line an unencoded (`7bit`) body may contain. RFC 5322 caps a line at
/// 998 octets; the margin covers servers that count the terminator too.
const MAX_7BIT_LINE: usize = 900;

// ─── configuration ─────────────────────────────────────────────────────────

/// How the socket is protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    /// TLS from the first byte (port 993 for IMAP, 465 for SMTP).
    Implicit,
    /// A plain connection that is upgraded before any credential is sent.
    Starttls,
    /// No TLS at all, for a loopback or in-network relay.
    Plain,
}

/// The `providers.email.imap` block.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// The mailbox polled for unseen mail.
    pub mailbox: String,
    pub security: Security,
    /// Seconds a protocol read may block before the connection is treated as
    /// half-open. Raise it for a server that is simply slow to answer.
    pub timeout_seconds: u64,
}

impl Default for ImapConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 993,
            username: String::new(),
            password: String::new(),
            mailbox: "INBOX".to_string(),
            security: Security::Implicit,
            timeout_seconds: DEFAULT_READ_TIMEOUT.as_secs(),
        }
    }
}

/// The `providers.email.smtp` block.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    /// Credentials; empty for a relay that authenticates by network.
    pub username: String,
    pub password: String,
    /// The envelope sender, and the address a probe reports on.
    pub from: String,
    pub security: Security,
    /// Seconds a protocol read may block before the connection is treated as
    /// half-open.
    pub timeout_seconds: u64,
}

impl Default for SmtpConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 587,
            username: String::new(),
            password: String::new(),
            from: String::new(),
            security: Security::Starttls,
            timeout_seconds: DEFAULT_READ_TIMEOUT.as_secs(),
        }
    }
}

/// The `providers.email` block.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct EmailConfig {
    pub enabled: bool,
    pub imap: ImapConfig,
    pub smtp: SmtpConfig,
    /// Seconds between mailbox polls.
    pub poll_seconds: u64,
    /// When non-empty, only mail from these addresses is answered.
    pub sender_allowlist: Vec<String>,
    /// When non-empty, only mail whose subject starts with this is answered (and
    /// replies carry it).
    pub subject_prefix: String,
}

impl EmailConfig {
    fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_seconds.max(1))
    }

    fn validate(&self, channel: &str) -> Result<()> {
        let mut missing: Vec<&str> = Vec::new();
        if self.imap.host.trim().is_empty() {
            missing.push("`imap.host`");
        }
        if self.imap.username.trim().is_empty() {
            missing.push("`imap.username`");
        }
        if self.imap.password.is_empty() {
            missing.push("`imap.password`");
        }
        if self.smtp.host.trim().is_empty() {
            missing.push("`smtp.host`");
        }
        if self.smtp.from.trim().is_empty() {
            missing.push("`smtp.from`");
        }
        if !missing.is_empty() {
            bail!(
                "invalid `providers.{channel}` configuration: {} required",
                missing.join(", ")
            );
        }
        if self.imap.port == 0 || self.smtp.port == 0 {
            bail!("invalid `providers.{channel}` configuration: `port` must not be 0");
        }
        if self.poll_seconds == 0 {
            bail!("invalid `providers.{channel}` configuration: `poll_seconds` must be at least 1");
        }
        if self.imap.mailbox.trim().is_empty() {
            bail!("invalid `providers.{channel}` configuration: `imap.mailbox` must not be empty");
        }
        if !self.smtp.from.contains('@') {
            bail!(
                "invalid `providers.{channel}` configuration: `smtp.from` must be an email address"
            );
        }
        let has_user = !self.smtp.username.trim().is_empty();
        // A username without a password (or the reverse) is a half-written
        // credential block, which would fail at the first send.
        if has_user == self.smtp.password.is_empty() {
            bail!(
                "invalid `providers.{channel}` configuration: SMTP authentication needs both `smtp.username` and `smtp.password`"
            );
        }
        Ok(())
    }
}

// ─── the wire ──────────────────────────────────────────────────────────────

/// A failure a reconnect cannot repair: bad configuration, rejected
/// credentials, or a mailbox that does not exist.
#[derive(Debug)]
pub(crate) struct FatalError(pub(crate) String);

impl std::fmt::Display for FatalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for FatalError {}

/// A connected socket, plain or wrapped in TLS.
enum Wire {
    Plain(TcpStream),
    Tls(Box<TlsStream>),
}

/// TLS over a TCP socket.
///
/// rustls works on buffers rather than on a socket, so the two directions are
/// explicit: [`TlsStream::fill`] moves bytes from the socket into the connection,
/// [`TlsStream::flush`] drains the connection's pending records back out.
struct TlsStream {
    tcp: TcpStream,
    conn: rustls::ClientConnection,
}

impl TlsStream {
    async fn fill(&mut self) -> std::io::Result<usize> {
        let mut raw = [0u8; 8192];
        let read = self.tcp.read(&mut raw).await?;
        if read == 0 {
            // The socket ended. Telling the connection about it is what lets it
            // report a truncation rather than a clean end of stream.
            self.conn.read_tls(&mut std::io::empty())?;
            return Ok(0);
        }
        let mut cursor = std::io::Cursor::new(&raw[..read]);
        // Bounded by the bytes that just arrived: rustls is never handed an empty
        // reader here, which it would read as the end of the stream. The `> 0` is
        // the same no-progress guard: a `read_tls` that takes nothing cannot make
        // progress with the remainder, so the feed ends rather than spinning.
        while (cursor.position() as usize) < read && self.conn.read_tls(&mut cursor)? > 0 {
            self.conn
                .process_new_packets()
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        }
        // A handshake needs a reply (ClientHello, Finished, a key update): the
        // records rustls produced are only on their way out after this.
        self.flush().await?;
        Ok(read)
    }

    /// Write every pending TLS record to the socket.
    ///
    /// One pass is enough: `write_tls` drains its whole pending buffer into the
    /// writer, and the writer here is an unbounded `Vec`, so nothing is left for
    /// a second call to pick up.
    async fn flush(&mut self) -> std::io::Result<()> {
        let mut out = Vec::new();
        self.conn.write_tls(&mut out)?;
        if !out.is_empty() {
            self.tcp.write_all(&out).await?;
        }
        self.tcp.flush().await
    }

    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            // The reader knows the difference between a clean close and a
            // truncated one, so it is asked before more bytes are fed in.
            match std::io::Read::read(&mut self.conn.reader(), buf) {
                Ok(read) => return Ok(read),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error),
            }
            self.fill().await?;
        }
    }
}

/// The platform trust store, so a mailbox behind a corporate TLS terminator
/// works the same way it does for HTTP.
fn tls_config() -> Result<rustls::ClientConfig> {
    use rustls_platform_verifier::BuilderVerifierExt;
    // Idempotent: production installs this in `run()`, tests in their setup.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    Ok(rustls::ClientConfig::builder()
        .with_platform_verifier()
        .with_no_client_auth())
}

impl Wire {
    /// Open the socket and, for [`Security::Implicit`], finish the handshake
    /// before returning.
    async fn connect(
        host: &str,
        port: u16,
        security: Security,
        tls: Option<&rustls::ClientConfig>,
    ) -> Result<Self> {
        let address = format!("{host}:{port}");
        let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&address))
            .await
            .map_err(|_| anyhow!("timed out connecting to {address}"))?
            .map_err(|error| anyhow!("cannot connect to {address}: {error}"))?;
        match security {
            Security::Plain => Ok(Wire::Plain(tcp)),
            Security::Implicit => Ok(Wire::Tls(Box::new(
                handshake_with(tcp, host, resolve_tls(tls)?).await?,
            ))),
            // A plain socket is what STARTTLS upgrades from; the caller must ask
            // for the upgrade before sending anything that matters.
            Security::Starttls => Ok(Wire::Plain(tcp)),
        }
    }

    /// Upgrade a plain connection with `config`; production passes
    /// [`tls_config`].
    ///
    /// Believing a socket is encrypted when it is not is the failure this whole
    /// path exists to prevent.
    async fn start_tls(self, host: &str, config: rustls::ClientConfig) -> Result<Self> {
        match self {
            Wire::Tls(_) => bail!("this connection is already encrypted"),
            Wire::Plain(tcp) => Ok(Wire::Tls(Box::new(
                handshake_with(tcp, host, config).await?,
            ))),
        }
    }

    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Wire::Plain(tcp) => tcp.read(buf).await,
            Wire::Tls(stream) => stream.read(buf).await,
        }
    }

    async fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        match self {
            Wire::Plain(tcp) => tcp.write_all(bytes).await,
            Wire::Tls(stream) => {
                std::io::Write::write_all(&mut stream.conn.writer(), bytes)?;
                stream.flush().await
            }
        }
    }
}

/// The client TLS configuration a session uses when none is supplied.
///
/// The platform verifier, which is what production wants; a caller that already
/// has a configuration of its own (the tests, against a certificate no trust
/// store can know) passes it to the `*_with` constructors below.
fn resolve_tls(tls: Option<&rustls::ClientConfig>) -> Result<rustls::ClientConfig> {
    match tls {
        Some(config) => Ok(config.clone()),
        None => tls_config(),
    }
}

/// Complete a TLS handshake on `tcp` using `config`.
///
/// The configuration is a parameter so the handshake can be exercised against a
/// peer this machine's trust store cannot know about.
async fn handshake_with(
    tcp: TcpStream,
    host: &str,
    config: rustls::ClientConfig,
) -> Result<TlsStream> {
    let name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|error| anyhow!("`{host}` is not a usable TLS name: {error}"))?;
    let conn = rustls::ClientConnection::new(Arc::new(config), name)?;
    let mut stream = TlsStream { tcp, conn };
    stream.flush().await?;
    while stream.conn.is_handshaking() {
        if stream.fill().await? == 0 {
            bail!("{host} closed the connection during the TLS handshake");
        }
    }
    Ok(stream)
}

// ─── line-oriented framing ─────────────────────────────────────────────────

/// Both protocols are CRLF-terminated text with an octet payload in the middle
/// (an SMTP body, an IMAP literal), so one buffered reader serves both.
struct LineStream {
    wire: Wire,
    buffer: Vec<u8>,
    /// A read that blocks longer than this fails instead of waiting forever.
    read_timeout: Duration,
}

impl LineStream {
    /// A stream with an explicit deadline, for a configured server.
    fn with_timeout(wire: Wire, read_timeout: Duration) -> Self {
        Self {
            wire,
            buffer: Vec::new(),
            // A zero would fail every read immediately.
            read_timeout: read_timeout.max(Duration::from_millis(1)),
        }
    }

    fn into_wire(self) -> Wire {
        self.wire
    }

    /// One socket read, under the connection's deadline.
    async fn read_some(&mut self, chunk: &mut [u8]) -> Result<usize> {
        match tokio::time::timeout(self.read_timeout, self.wire.read(chunk)).await {
            Ok(Ok(read)) => Ok(read),
            Ok(Err(error)) => Err(error.into()),
            Err(_) => bail!(
                "the server did not answer within {}s; the connection looks half-open (transient)",
                self.read_timeout.as_secs()
            ),
        }
    }

    /// The next line, CRLF stripped. `None` when the peer closed the connection,
    /// which is a normal end for a server that just answered `QUIT`.
    async fn read_line(&mut self) -> Result<Option<String>> {
        loop {
            if let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=index).collect();
                let line = &line[..line.len() - 1];
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                return Ok(Some(String::from_utf8_lossy(line).into_owned()));
            }
            if self.buffer.len() > MAX_LINE_BYTES {
                bail!(
                    "the server sent {} bytes without a line break; not speaking this protocol",
                    self.buffer.len()
                );
            }
            let mut chunk = [0u8; 4096];
            let read = self.read_some(&mut chunk).await?;
            if read == 0 {
                return Ok(None);
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }

    /// Exactly `count` bytes, for the octet payload that follows a `{count}`
    /// marker.
    async fn read_bytes(&mut self, count: usize) -> Result<Vec<u8>> {
        if count > MAX_LITERAL_BYTES {
            bail!(
                "the server announced a {count}-byte payload; refusing to buffer it (message too long)"
            );
        }
        let mut out = Vec::with_capacity(count.min(64 * 1024));
        while !self.buffer.is_empty() && out.len() < count {
            let take = (count - out.len()).min(self.buffer.len());
            out.extend(self.buffer.drain(..take));
        }
        while out.len() < count {
            let mut chunk = vec![0u8; (count - out.len()).min(64 * 1024)];
            let read = self.read_some(&mut chunk).await?;
            if read == 0 {
                bail!("the server closed the connection mid-payload (transient)");
            }
            out.extend_from_slice(&chunk[..read]);
        }
        Ok(out)
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.wire.write_all(bytes).await?;
        Ok(())
    }

    /// Write one protocol line with the CRLF both protocols require.
    async fn write_line(&mut self, line: &str) -> Result<()> {
        debug_assert!(!line.contains('\n') && !line.contains('\r'));
        let mut bytes = line.as_bytes().to_vec();
        bytes.extend_from_slice(b"\r\n");
        self.write(&bytes).await
    }
}

// ─── SMTP ──────────────────────────────────────────────────────────────────

/// One server reply, which may span several lines (`250-first…250 last`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SmtpReply {
    pub code: u16,
    pub lines: Vec<String>,
}

impl SmtpReply {
    /// The reply text with the multi-line structure reduced to one string.
    fn text(&self) -> String {
        self.lines.join(" ").trim().to_string()
    }

    /// Whether the reply is a 2xx/3xx, the only codes that continue a session.
    fn is_ok(&self) -> bool {
        (200..400).contains(&self.code)
    }

    /// The capability lines of an `EHLO` reply, which follow the greeting.
    ///
    /// Each line still carries its own reply code (`250-STARTTLS`), which is not
    /// part of the capability name and has to come off before one is matched.
    fn capabilities(&self) -> Vec<String> {
        self.lines
            .iter()
            .skip(1)
            .map(|line| strip_reply_code(line).to_string())
            .collect()
    }
}

/// `250-AUTH PLAIN` (or `250 AUTH PLAIN`) without its reply code.
fn strip_reply_code(line: &str) -> &str {
    let bytes = line.as_bytes();
    if bytes.len() >= 4
        && bytes[..3].iter().all(u8::is_ascii_digit)
        && matches!(bytes[3], b' ' | b'-')
    {
        return &line[4..];
    }
    line
}

/// The read deadline from a configured value, never zero.
fn read_deadline(seconds: u64) -> Duration {
    Duration::from_secs(seconds.max(1))
}

/// The mechanisms worth speaking here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthMechanism {
    Plain,
    Login,
}

impl AuthMechanism {
    fn name(self) -> &'static str {
        match self {
            AuthMechanism::Plain => "PLAIN",
            AuthMechanism::Login => "LOGIN",
        }
    }
}

/// One SMTP submission connection.
struct SmtpSession {
    stream: LineStream,
    capabilities: Vec<String>,
}

impl std::fmt::Debug for SmtpSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately does not walk the socket: what a failed connection needs
        // in a log line is what the server offered, not the TLS state.
        formatter
            .debug_struct("SmtpSession")
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}

impl SmtpSession {
    /// Connect, greet, upgrade to TLS when configured, and authenticate.
    async fn connect(config: &SmtpConfig) -> Result<Self> {
        Self::connect_with(config, None).await
    }

    /// The same, with a client TLS configuration the caller supplies instead of
    /// the platform verifier.
    async fn connect_with(config: &SmtpConfig, tls: Option<&rustls::ClientConfig>) -> Result<Self> {
        let client_tls = || -> Result<rustls::ClientConfig> { resolve_tls(tls) };
        let host = config.host.trim();
        let wire = Wire::connect(host, config.port, config.security, tls).await?;
        let mut session = Self {
            stream: LineStream::with_timeout(wire, read_deadline(config.timeout_seconds)),
            capabilities: Vec::new(),
        };
        let greeting = session.read_reply().await?;
        if greeting.code != 220 {
            return Err(smtp_failure("greeting", &greeting));
        }
        session.hello(&ehlo_name(config)).await?;
        if config.security == Security::Starttls {
            if !session.supports("STARTTLS") {
                // Continuing would put the password and the mail on the wire in
                // the clear; a configuration that cannot be honoured is an
                // error, never a silent downgrade.
                return Err(FatalError(format!(
                    "the SMTP server {host}:{} does not offer STARTTLS; refusing to send credentials unencrypted (set `smtp.security` to \"plain\" only on a network you trust)",
                    config.port
                ))
                .into());
            }
            let reply = session.command("STARTTLS").await?;
            check("STARTTLS")(reply)?;
            let wire = session
                .stream
                .into_wire()
                .start_tls(host, client_tls()?)
                .await?;
            session.stream = LineStream::with_timeout(wire, read_deadline(config.timeout_seconds));
            // The server forgets everything negotiated before the handshake.
            session.hello(&ehlo_name(config)).await?;
        }
        if !config.username.trim().is_empty() {
            session.authenticate(config).await?;
        }
        Ok(session)
    }

    async fn read_reply(&mut self) -> Result<SmtpReply> {
        let first = self
            .stream
            .read_line()
            .await?
            .ok_or_else(|| anyhow!("the SMTP server closed the connection"))?;
        let code = parse_reply_code(&first)
            .ok_or_else(|| anyhow!("the SMTP server sent `{first}`, which is not a reply"))?;
        let mut lines = vec![first];
        // A hyphen after the code means another line follows; a space ends it.
        while lines
            .last()
            .and_then(|line| line.as_bytes().get(3))
            .is_some_and(|byte| *byte == b'-')
        {
            let line = self
                .stream
                .read_line()
                .await?
                .ok_or_else(|| anyhow!("the SMTP server stopped mid-reply"))?;
            lines.push(line);
        }
        Ok(SmtpReply { code, lines })
    }

    /// Send one command and read its reply.
    async fn command(&mut self, line: &str) -> Result<SmtpReply> {
        self.stream.write_line(line).await?;
        self.read_reply().await
    }

    /// `EHLO`, falling back to `HELO` for a server too old to answer it. The
    /// capability list is what says whether STARTTLS and AUTH exist, so the
    /// fallback is recorded as a server with no capabilities.
    async fn hello(&mut self, name: &str) -> Result<()> {
        let reply = self.command(&format!("EHLO {name}")).await?;
        if reply.is_ok() {
            self.capabilities = reply.capabilities();
            return Ok(());
        }
        if reply.code >= 500 {
            let reply = self.command(&format!("HELO {name}")).await?;
            if !reply.is_ok() {
                return Err(smtp_failure("HELO", &reply));
            }
            self.capabilities = Vec::new();
            return Ok(());
        }
        Err(smtp_failure("EHLO", &reply))
    }

    fn supports(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|line| {
            line.split_whitespace()
                .next()
                .is_some_and(|first| first.eq_ignore_ascii_case(capability))
        })
    }

    /// The mechanisms the server advertised, uppercased.
    fn auth_mechanisms(&self) -> Vec<String> {
        self.capabilities
            .iter()
            .filter(|line| {
                line.split_whitespace()
                    .next()
                    .is_some_and(|first| first.eq_ignore_ascii_case("AUTH"))
            })
            .flat_map(|line| line.split_whitespace().skip(1).map(str::to_uppercase))
            .collect()
    }

    async fn authenticate(&mut self, config: &SmtpConfig) -> Result<()> {
        let mechanism = select_mechanism(&self.auth_mechanisms())?;
        let user = config.username.trim();
        match mechanism {
            AuthMechanism::Plain => {
                // `AUTH PLAIN <initial response>` is the short form; a server
                // that prefers to ask answers 334 instead.
                let payload = base64_encode(format!("\0{user}\0{}", config.password).as_bytes());
                let reply = self.command(&format!("AUTH PLAIN {payload}")).await?;
                let reply = if reply.code == 334 {
                    self.command(&payload).await?
                } else {
                    reply
                };
                if reply.code != 235 {
                    return Err(auth_failure(mechanism, &reply));
                }
            }
            AuthMechanism::Login => {
                // Two 334 challenges: the user name, then the password.
                let mut reply = self.command("AUTH LOGIN").await?;
                for value in [user, config.password.as_str()] {
                    if reply.code != 334 {
                        break;
                    }
                    reply = self.command(&base64_encode(value.as_bytes())).await?;
                }
                if reply.code != 235 {
                    return Err(auth_failure(mechanism, &reply));
                }
            }
        }
        tracing::debug!(mechanism = mechanism.name(), "SMTP authentication accepted");
        Ok(())
    }

    /// `MAIL FROM`/`RCPT TO`/`DATA` for one message, which must already be
    /// CRLF-framed with the body's dot-stuffing applied.
    async fn submit(&mut self, from: &str, to: &str, data: &[u8]) -> Result<()> {
        let reply = self.command(&format!("MAIL FROM:<{from}>")).await?;
        check("MAIL FROM")(reply)?;
        let reply = self.command(&format!("RCPT TO:<{to}>")).await?;
        match reply.code {
            250 | 251 => {}
            _ => return Err(smtp_failure(&format!("RCPT TO `{to}`"), &reply)),
        }
        let reply = self.command("DATA").await?;
        check("DATA")(reply)?;
        self.stream.write(data).await?;
        let reply = self.read_reply().await?;
        check("DATA")(reply)?;
        Ok(())
    }

    async fn quit(&mut self) -> Result<()> {
        // Best-effort: the message is already accepted, and a server that closes
        // without answering is not a delivery failure.
        let _ = self.command("QUIT").await;
        Ok(())
    }
}

/// Accept 2xx/3xx, report anything else with the stage that failed.
fn check(stage: &str) -> impl FnOnce(SmtpReply) -> Result<SmtpReply> {
    let stage = stage.to_string();
    move |reply: SmtpReply| {
        if reply.is_ok() {
            Ok(reply)
        } else {
            Err(smtp_failure(&stage, &reply))
        }
    }
}

fn parse_reply_code(line: &str) -> Option<u16> {
    let code = line.get(..3)?;
    if !code.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    code.parse().ok()
}

/// Whether the server's `EHLO` list offers a mechanism we can speak, preferring
/// `PLAIN` (one round trip) over `LOGIN`.
pub(crate) fn select_mechanism(offered: &[String]) -> Result<AuthMechanism> {
    if offered.iter().any(|name| name == "PLAIN") {
        return Ok(AuthMechanism::Plain);
    }
    if offered.iter().any(|name| name == "LOGIN") {
        return Ok(AuthMechanism::Login);
    }
    Err(FatalError(format!(
        "the SMTP server offers no usable AUTH mechanism (offered: {}); the configured credentials cannot be used (unauthorized)",
        if offered.is_empty() {
            "none".to_string()
        } else {
            offered.join(", ")
        }
    ))
    .into())
}

/// Translate a server refusal into the vocabulary the delivery queue already
/// knows: the 5xx codes that will never succeed are named as such, so a queued
/// message stops being retried; everything else stays retryable.
fn smtp_failure(stage: &str, reply: &SmtpReply) -> anyhow::Error {
    let detail = reply.text();
    let described = match reply.code {
        // Authentication: retrying with the same credentials cannot help.
        530 | 534 | 535 | 538 => format!(
            "the SMTP server rejected the credentials (unauthorized): {stage} said {} {detail}",
            reply.code
        ),
        // The address, not the server, is wrong.
        550 | 551 | 553 => format!(
            "recipient not found: {stage} was refused with {} {detail}",
            reply.code
        ),
        // Over quota, or past the server's message limit.
        552 => format!(
            "the message was rejected as too large or over quota (message too long): {stage} said {} {detail}",
            reply.code
        ),
        _ => format!("the SMTP server refused {stage}: {} {detail}", reply.code),
    };
    anyhow!("{described}")
}

fn auth_failure(mechanism: AuthMechanism, reply: &SmtpReply) -> anyhow::Error {
    smtp_failure(&format!("AUTH {}", mechanism.name()), reply)
}

/// The name given to the server at `EHLO`: the domain our sender address lives
/// in, which is what a relay expects to see.
fn ehlo_name(config: &SmtpConfig) -> String {
    config
        .from
        .trim()
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim().to_string())
        .filter(|domain| !domain.is_empty())
        .unwrap_or_else(|| "localhost".to_string())
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

// ─── outbound message assembly ─────────────────────────────────────────────

/// Everything one reply needs, assembled into an RFC 5322 message with CRLF
/// line endings. [`encode_data`] turns it into the `DATA` payload.
pub(crate) fn build_message(
    config: &SmtpConfig,
    to: &str,
    subject: &str,
    thread: Option<&str>,
    text: &str,
) -> Result<String> {
    let from = sanitize_header(config.from.trim());
    if !from.contains('@') {
        bail!("invalid `smtp.from`: `{from}` is not an email address (not configured)");
    }
    let to = sanitize_header(to.trim());
    let (encoding, body) = encode_body(text);
    let mut message = String::new();
    message.push_str(&format!("From: <{from}>\r\n"));
    message.push_str(&format!("To: <{to}>\r\n"));
    message.push_str(&format!("Subject: {}\r\n", encode_rfc2047(subject)));
    message.push_str(&format!("Date: {}\r\n", chrono::Utc::now().to_rfc2822()));
    message.push_str(&format!("Message-ID: {}\r\n", message_id(&from)));
    if let Some(thread) = thread.map(sanitize_header).filter(|id| !id.is_empty()) {
        // One ancestor is enough for a client to file the reply in the thread;
        // the full chain a `References` would carry is not something a
        // conversation keeps.
        let id = if thread.starts_with('<') {
            thread
        } else {
            format!("<{thread}>")
        };
        message.push_str(&format!("In-Reply-To: {id}\r\n"));
        message.push_str(&format!("References: {id}\r\n"));
    }
    message.push_str("MIME-Version: 1.0\r\n");
    message.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
    message.push_str(&format!("Content-Transfer-Encoding: {encoding}\r\n"));
    // Machines should not answer this, which is what keeps two agents from
    // talking to each other forever.
    message.push_str("Auto-Submitted: auto-replied\r\n");
    message.push_str("\r\n");
    message.push_str(&body);
    Ok(message)
}

/// The `DATA` payload: CRLF line endings, dot-stuffed, terminated.
///
/// A line starting with `.` would otherwise end the message early, which is the
/// injection this exists to stop.
pub(crate) fn encode_data(message: &str) -> Vec<u8> {
    let normalized = normalize_crlf(message);
    let mut out = String::with_capacity(normalized.len() + 8);
    for line in normalized.split("\r\n") {
        if line.starts_with('.') {
            out.push('.');
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    // The terminating dot lives on its own line and is not part of the data.
    out.push_str(".\r\n");
    out.into_bytes()
}

/// `\n` (and a bare `\r`) become CRLF, as both protocols require.
pub(crate) fn normalize_crlf(text: &str) -> String {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// Choose a content-transfer-encoding for `text`.
///
/// Base64 whenever the text is not 7-bit ASCII or has a line past RFC 5322's
/// limit: those are exactly what an unencoded body gets wrong on a server that
/// does not advertise `8BITMIME`.
pub(crate) fn encode_body(text: &str) -> (&'static str, String) {
    let normalized = normalize_crlf(text);
    let lines_ok = normalized
        .split("\r\n")
        .all(|line| line.len() <= MAX_7BIT_LINE);
    if normalized.is_ascii() && lines_ok {
        return ("7bit", normalized);
    }
    let encoded = base64_encode(text.as_bytes());
    let mut wrapped = String::with_capacity(encoded.len() + encoded.len() / 76 + 2);
    let mut rest = encoded.as_str();
    while !rest.is_empty() {
        let take = rest.len().min(76);
        let (head, tail) = rest.split_at(take);
        wrapped.push_str(head);
        wrapped.push_str("\r\n");
        rest = tail;
    }
    ("base64", wrapped)
}

/// A header value may not contain a line break: a subject with one would let the
/// text of a reply add a header of its own.
pub(crate) fn sanitize_header(value: &str) -> String {
    let flattened = value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    // The control characters became spaces; collapsing the runs keeps the
    // result readable instead of leaving a `Bcc:` fragment after two spaces.
    flattened.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// RFC 2047 encode a header value, leaving pure ASCII alone.
pub(crate) fn encode_rfc2047(value: &str) -> String {
    let clean = sanitize_header(value);
    if clean.is_ascii() {
        return clean;
    }
    // A word is `=?UTF-8?B?` + payload + `?=`, and a header line stays under 78
    // characters, so 45 payload bytes per word leave room for the framing.
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_bytes = 0usize;
    for ch in clean.chars() {
        let mut raw = [0u8; 4];
        let bytes = ch.encode_utf8(&mut raw).len();
        if current_bytes + bytes > 45 && !current.is_empty() {
            words.push(std::mem::take(&mut current));
            current_bytes = 0;
        }
        current.push(ch);
        current_bytes += bytes;
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
        .iter()
        .map(|word| format!("=?UTF-8?B?{}?=", base64_encode(word.as_bytes())))
        .collect::<Vec<_>>()
        .join("\r\n ")
}

fn message_id(from: &str) -> String {
    let domain = from
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .unwrap_or("localhost");
    format!("<{}@{}>", uuid::Uuid::new_v4(), domain)
}

/// The subject of a reply: the original with one `Re:` (never `Re: Re:`) and the
/// configured prefix, so a mailbox rule that recognises the gate keeps working
/// in both directions.
pub(crate) fn reply_subject(original: Option<&str>, prefix: &str) -> String {
    let prefix = prefix.trim();
    let original = original.map(str::trim).filter(|value| !value.is_empty());
    let base = match original {
        Some(subject) => {
            // Our own prefix comes off first: otherwise every round trip of a
            // gated thread would add another `[bot] Re:` in front.
            let without_prefix = strip_prefix_ci(subject, prefix)
                .unwrap_or(subject)
                .trim_start();
            if without_prefix.to_ascii_lowercase().starts_with("re:") {
                without_prefix.to_string()
            } else {
                format!("Re: {without_prefix}")
            }
        }
        None => "Future agent reply".to_string(),
    };
    if prefix.is_empty() {
        base
    } else {
        format!("{prefix} {base}")
    }
}

/// `value` without a leading `prefix`, compared case-insensitively.
fn strip_prefix_ci<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() || value.len() < prefix.len() {
        return None;
    }
    let head = value.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &value[prefix.len()..])
}

/// The conversation's address, checked before it reaches a header or an
/// envelope.
pub(crate) fn recipient_address(conversation: &ConversationRef) -> Result<String> {
    let raw = conversation.id.trim();
    if raw.is_empty() {
        bail!("the email conversation has no address to send to");
    }
    if raw.chars().any(char::is_control) || raw.contains(['<', '>', ',']) || !raw.contains('@') {
        bail!("the recipient address is invalid: `{raw}` (recipient not found)");
    }
    Ok(raw.to_string())
}

/// Bounded memory of what each conversation was about, so a reply can carry the
/// original subject.
#[derive(Default)]
struct SubjectCache {
    map: HashMap<String, String>,
    order: VecDeque<String>,
}

impl SubjectCache {
    fn insert(&mut self, key: &str, subject: &str) {
        if subject.trim().is_empty() {
            return;
        }
        if self
            .map
            .insert(key.to_string(), subject.to_string())
            .is_none()
        {
            self.order.push_back(key.to_string());
        }
        while self.order.len() > SUBJECT_CACHE {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }
}

// ─── IMAP ──────────────────────────────────────────────────────────────────

/// One octet payload in a response, and the response line it followed. Pairing
/// them this way is what lets a multi-message `FETCH` be read without guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Literal {
    pub after_line: usize,
    pub bytes: Vec<u8>,
}

/// One untagged response, up to the tagged completion of its command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImapResponse {
    /// The status word (`OK`, `NO`, `BAD`).
    pub status: String,
    pub detail: String,
    /// Untagged lines, in arrival order.
    pub lines: Vec<String>,
    pub literals: Vec<Literal>,
}

impl ImapResponse {
    fn is_ok(&self) -> bool {
        self.status == "OK"
    }

    /// The first untagged line starting with `prefix`, case-insensitively.
    fn find(&self, prefix: &str) -> Option<&str> {
        self.lines
            .iter()
            .find(|line| {
                line.get(..prefix.len())
                    .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
            })
            .map(String::as_str)
    }
}

/// One `* n FETCH (…)` record and the body that followed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FetchRecord {
    pub uid: Option<u32>,
    pub body: Option<Vec<u8>>,
}

/// Pair every `* n FETCH` line with the literal that carried its payload.
///
/// The line index is checked rather than the arrival order, because an unrelated
/// literal earlier in the same response would otherwise shift every pairing.
pub(crate) fn fetch_records(response: &ImapResponse) -> Vec<FetchRecord> {
    let mut records = Vec::new();
    for (index, line) in response.lines.iter().enumerate() {
        if !is_fetch_line(line) {
            continue;
        }
        let uid = fetch_uid(line);
        let body = response
            .literals
            .iter()
            .find(|literal| literal.after_line == index)
            .map(|literal| literal.bytes.clone());
        records.push(FetchRecord { uid, body });
    }
    records
}

fn is_fetch_line(line: &str) -> bool {
    let mut parts = line.split_whitespace();
    parts.next() == Some("*")
        && parts.next().is_some()
        && parts
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("FETCH"))
}

/// The `UID <n>` item of a `FETCH` line.
///
/// The item list opens with a bracket (`FETCH (UID 7 BODY[] {5}`), which is not
/// part of the item name, so it is stripped before matching.
fn fetch_uid(line: &str) -> Option<u32> {
    let mut parts = line.split_whitespace();
    while let Some(part) = parts.next() {
        if part.trim_matches(['(', ')']).eq_ignore_ascii_case("UID") {
            return parts
                .next()
                .and_then(|value| value.trim_matches(['(', ')']).parse().ok());
        }
    }
    None
}

/// The octet count of a line ending in a synchronizing literal marker
/// (`… {3456}`).
pub(crate) fn literal_length(line: &str) -> Option<usize> {
    let end = line.strip_suffix('}')?;
    let start = end.rfind('{')?;
    end[start + 1..].parse().ok()
}

/// The state `SELECT` reports for a mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct MailboxState {
    pub exists: u32,
    /// Changes when the server renumbers the mailbox; UID keys recorded against
    /// another value mean nothing here.
    pub uid_validity: u32,
}

/// One IMAP connection.
struct ImapSession {
    stream: LineStream,
    tag: u32,
}

impl std::fmt::Debug for ImapSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The socket is not worth walking; the command counter is what a
        // failing conversation needs.
        formatter
            .debug_struct("ImapSession")
            .field("tag", &self.tag)
            .finish_non_exhaustive()
    }
}

impl ImapSession {
    /// Connect, finish any TLS handshake, and read the greeting.
    async fn connect(config: &ImapConfig) -> Result<Self> {
        Self::connect_with(config, None).await
    }

    /// The same, with a client TLS configuration the caller supplies instead of
    /// the platform verifier.
    async fn connect_with(config: &ImapConfig, tls: Option<&rustls::ClientConfig>) -> Result<Self> {
        let client_tls = || -> Result<rustls::ClientConfig> { resolve_tls(tls) };
        let host = config.host.trim();
        let wire = Wire::connect(host, config.port, config.security, tls).await?;
        let mut session = Self {
            stream: LineStream::with_timeout(wire, read_deadline(config.timeout_seconds)),
            tag: 0,
        };
        let greeting =
            session.stream.read_line().await?.ok_or_else(|| {
                anyhow!("the IMAP server closed the connection without a greeting")
            })?;
        let upper = greeting.to_uppercase();
        // `* PREAUTH` is acceptable; anything else that is not `* OK` is not a
        // greeting at all.
        if !upper.starts_with("* OK") && !upper.starts_with("* PREAUTH") {
            bail!("the IMAP server greeted with `{greeting}`, which is not an IMAP greeting");
        }
        if config.security == Security::Starttls {
            let reply = session.command("STARTTLS").await?;
            if !reply.is_ok() {
                return Err(FatalError(format!(
                    "the IMAP server {host}:{} refused STARTTLS ({} {}); refusing to send credentials unencrypted",
                    config.port, reply.status, reply.detail
                ))
                .into());
            }
            let wire = session
                .stream
                .into_wire()
                .start_tls(host, client_tls()?)
                .await?;
            session.stream = LineStream::with_timeout(wire, read_deadline(config.timeout_seconds));
        }
        Ok(session)
    }

    /// Send one tagged command and collect everything up to its completion.
    async fn command(&mut self, args: &str) -> Result<ImapResponse> {
        self.tag += 1;
        let tag = format!("a{}", self.tag);
        self.stream.write_line(&format!("{tag} {args}")).await?;
        let mut lines: Vec<String> = Vec::new();
        let mut literals: Vec<Literal> = Vec::new();
        loop {
            let line =
                self.stream.read_line().await?.ok_or_else(|| {
                    anyhow!("the IMAP server closed the connection during `{args}`")
                })?;
            if let Some(rest) = line.strip_prefix(&format!("{tag} ")) {
                let (status, detail) = rest.split_once(' ').unwrap_or((rest, ""));
                return Ok(ImapResponse {
                    status: status.to_uppercase(),
                    detail: detail.trim().to_string(),
                    lines,
                    literals,
                });
            }
            let after_line = lines.len();
            lines.push(line.clone());
            if let Some(count) = literal_length(&line) {
                let bytes = self.stream.read_bytes(count).await?;
                literals.push(Literal { after_line, bytes });
            }
        }
    }

    async fn login(&mut self, config: &ImapConfig) -> Result<()> {
        let user = quote_imap(config.username.trim());
        let password = quote_imap(&config.password);
        let reply = self.command(&format!("LOGIN {user} {password}")).await?;
        if !reply.is_ok() {
            // Retrying rejected credentials only gets the account locked.
            return Err(FatalError(format!(
                "IMAP login for `{}` was rejected: {} {} (unauthorized)",
                config.username, reply.status, reply.detail
            ))
            .into());
        }
        Ok(())
    }

    async fn select(&mut self, mailbox: &str) -> Result<MailboxState> {
        let reply = self
            .command(&format!("SELECT {}", quote_imap(mailbox)))
            .await?;
        if !reply.is_ok() {
            // A mailbox that does not exist is a configuration error; retrying
            // cannot create it.
            return Err(FatalError(format!(
                "cannot open the IMAP mailbox `{mailbox}`: {} {}",
                reply.status, reply.detail
            ))
            .into());
        }
        let mut state = MailboxState::default();
        for line in &reply.lines {
            let upper = line.to_uppercase();
            if let Some(rest) = upper.strip_prefix("* ") {
                if let Some(exists) = rest.strip_suffix(" EXISTS") {
                    state.exists = exists.trim().parse().unwrap_or(0);
                }
            }
            if let Some(index) = upper.find("[UIDVALIDITY ") {
                let rest = &upper[index + "[UIDVALIDITY ".len()..];
                let value = rest.split([']', ' ']).next().unwrap_or("");
                state.uid_validity = value.parse().unwrap_or(0);
            }
        }
        Ok(state)
    }

    /// The UIDs of unseen messages, in the server's own (ascending) order.
    async fn search_unseen(&mut self) -> Result<Vec<u32>> {
        let reply = self.command("UID SEARCH UNSEEN").await?;
        if !reply.is_ok() {
            bail!(
                "IMAP `UID SEARCH UNSEEN` failed: {} {}",
                reply.status,
                reply.detail
            );
        }
        let line = reply
            .find("* SEARCH")
            .ok_or_else(|| anyhow!("the IMAP server answered `UID SEARCH` without results"))?;
        Ok(line
            .split_whitespace()
            .skip(2)
            .filter_map(|value| value.parse().ok())
            .collect())
    }

    /// The raw message for one UID.
    ///
    /// `BODY.PEEK[]` rather than `BODY[]`: fetching the body must not be what
    /// marks the message read, because a crash in between would then lose it.
    async fn fetch_body(&mut self, uid: u32) -> Result<Vec<u8>> {
        let reply = self
            .command(&format!("UID FETCH {uid} (BODY.PEEK[])"))
            .await?;
        if !reply.is_ok() {
            bail!(
                "IMAP `UID FETCH {uid}` failed: {} {}",
                reply.status,
                reply.detail
            );
        }
        fetch_records(&reply)
            .iter()
            .find(|record| record.uid.is_none() || record.uid == Some(uid))
            .and_then(|record| record.body.clone())
            .ok_or_else(|| anyhow!("the IMAP server returned no body for message {uid}"))
    }

    async fn mark_seen(&mut self, uid: u32) -> Result<()> {
        let reply = self
            .command(&format!("UID STORE {uid} +FLAGS (\\Seen)"))
            .await?;
        if !reply.is_ok() {
            bail!(
                "cannot mark message {uid} as seen: {} {} (the mailbox may be read-only)",
                reply.status,
                reply.detail
            );
        }
        Ok(())
    }

    async fn logout(&mut self) -> Result<()> {
        let _ = self.command("LOGOUT").await;
        Ok(())
    }
}

/// Quote a string for an IMAP command argument.
fn quote_imap(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        // A bare CR/LF would end the command and start another one.
        if ch.is_control() {
            continue;
        }
        out.push(ch);
    }
    out.push('"');
    out
}

// ─── MIME ──────────────────────────────────────────────────────────────────

/// The header block of a message, or of one MIME part.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Headers {
    /// Lowercased field names in arrival order, with folded values unwrapped.
    entries: Vec<(String, String)>,
}

impl Headers {
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }

    /// A header with its RFC 2047 encoded words decoded.
    pub(crate) fn decoded(&self, name: &str) -> Option<String> {
        self.get(name).map(decode_header)
    }

    pub(crate) fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|(name, _)| name.as_str()).collect()
    }
}

/// Split a message into its header block and its body.
pub(crate) fn split_message(raw: &[u8]) -> (Headers, Vec<u8>) {
    let (head, body) = match find_body_start(raw) {
        Some(body_start) => (&raw[..body_start.0], &raw[body_start.1..]),
        // A part with no blank line (a few clients omit it) is all headers;
        // whatever did not parse as a header is simply ignored.
        None => (raw, &raw[raw.len()..]),
    };
    (parse_headers(&String::from_utf8_lossy(head)), body.to_vec())
}

/// The end of the header block and the start of the body, for CRLF and bare LF
/// messages alike.
fn find_body_start(raw: &[u8]) -> Option<(usize, usize)> {
    let mut line_start = 0usize;
    for (index, byte) in raw.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let end = if index > 0 && raw[index - 1] == b'\r' {
            index - 1
        } else {
            index
        };
        if end == line_start {
            return Some((line_start, index + 1));
        }
        line_start = index + 1;
    }
    None
}

/// Parse a header block: one field per line, and a continuation line (starting
/// with a space or tab) belongs to the field above it.
pub(crate) fn parse_headers(text: &str) -> Headers {
    let mut entries: Vec<(String, String)> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = entries.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        entries.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    Headers { entries }
}

/// A parsed message: its headers, the body a reader would see, and what was
/// attached.
#[derive(Debug, Clone, Default)]
pub(crate) struct Mail {
    pub headers: Headers,
    /// `text/plain` when the message has one, otherwise `text/html` reduced to
    /// text.
    pub text: String,
    /// Whether `text` came from HTML.
    pub from_html: bool,
    pub attachments: Vec<Attachment>,
}

/// An attachment we can name but deliberately do not fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attachment {
    pub filename: Option<String>,
    pub content_type: String,
    /// The encoded size, which is what arrived.
    pub bytes: usize,
}

/// One leaf of the MIME tree, before a body is chosen.
struct Leaf {
    content_type: String,
    text: String,
    /// Set for anything that must not be mistaken for the body the user wrote —
    /// a `text/plain` *attachment* is not a reply.
    attachment: Option<Attachment>,
}

/// Parse a whole message.
pub(crate) fn parse_mail(raw: &[u8]) -> Mail {
    let (headers, body) = split_message(raw);
    let mut leaves = Vec::new();
    collect_leaves(&headers, &body, 0, &mut leaves);
    let mut mail = Mail {
        headers,
        ..Mail::default()
    };
    let is_body = |leaf: &Leaf| leaf.attachment.is_none() && !leaf.text.trim().is_empty();
    let chosen = leaves
        .iter()
        .find(|leaf| leaf.content_type == "text/plain" && is_body(leaf))
        .or_else(|| {
            leaves
                .iter()
                .find(|leaf| leaf.content_type == "text/html" && is_body(leaf))
        });
    if let Some(leaf) = chosen {
        mail.from_html = leaf.content_type == "text/html";
        mail.text = if mail.from_html {
            strip_html(&leaf.text)
        } else {
            // Trimmed on both ends: a body routinely opens with the blank line
            // some clients emit, and leading whitespace would also stop a bare
            // `yes` from matching a pending approval.
            leaf.text.trim().to_string()
        };
    }
    mail.attachments = leaves
        .into_iter()
        .filter_map(|leaf| leaf.attachment)
        .collect();
    mail
}

/// Walk a MIME part, appending the leaves it contains.
fn collect_leaves(headers: &Headers, body: &[u8], depth: usize, out: &mut Vec<Leaf>) {
    if depth > MAX_MIME_DEPTH {
        return;
    }
    let (content_type, parameters) =
        parse_content_type(headers.get("content-type").unwrap_or("text/plain"));
    let (disposition, disposition_parameters) =
        parse_content_type(headers.get("content-disposition").unwrap_or("inline"));
    let filename = disposition_parameters
        .iter()
        .find(|(name, _)| name == "filename")
        .map(|(_, value)| value.clone())
        .or_else(|| {
            parameters
                .iter()
                .find(|(name, _)| name == "name")
                .map(|(_, value)| value.clone())
        });

    if content_type.starts_with("multipart/") && disposition != "attachment" {
        let boundary = parameters
            .iter()
            .find(|(name, _)| name == "boundary")
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        if boundary.is_empty() {
            return;
        }
        for part in split_multipart(body, &boundary) {
            let (part_headers, part_body) = split_message(&part);
            collect_leaves(&part_headers, &part_body, depth + 1, out);
        }
        return;
    }

    let encoding = headers
        .get("content-transfer-encoding")
        .unwrap_or("7bit")
        .trim()
        .to_ascii_lowercase();
    let decoded = decode_transfer(body, &encoding);
    let is_text = content_type == "text/plain" || content_type == "text/html";
    let attachment = if disposition == "attachment" || filename.is_some() {
        Some(Attachment {
            filename,
            content_type: content_type.clone(),
            bytes: body.len(),
        })
    } else if !is_text {
        Some(Attachment {
            filename: None,
            content_type: content_type.clone(),
            bytes: body.len(),
        })
    } else {
        None
    };
    let charset = parameters
        .iter()
        .find(|(name, _)| name == "charset")
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| "utf-8".to_string());
    let text = if is_text {
        decode_text(&decoded, &charset)
    } else {
        String::new()
    };
    out.push(Leaf {
        content_type,
        text,
        attachment,
    });
}

/// `type/subtype` (lowercased) plus its parameters, as they were written.
pub(crate) fn parse_content_type(value: &str) -> (String, Vec<(String, String)>) {
    let mut chunks = split_parameters(value).into_iter();
    let content_type = chunks
        .next()
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let mut parameters = Vec::new();
    for chunk in chunks {
        let Some((name, raw)) = chunk.split_once('=') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        parameters.push((name, unquote(raw.trim())));
    }
    (content_type, parameters)
}

/// Split on `;` while respecting quoted strings, so a boundary or a filename
/// that contains a semicolon survives.
fn split_parameters(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => {
                current.push(ch);
                escaped = true;
            }
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            ';' if !quoted => out.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    out.push(current);
    out
}

fn unquote(value: &str) -> String {
    let inner = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value);
    inner.replace("\\\"", "\"").replace("\\\\", "\\")
}

/// The parts of a multipart body: the preamble and the closing marker are not
/// parts and are dropped.
pub(crate) fn split_multipart(body: &[u8], boundary: &str) -> Vec<Vec<u8>> {
    let marker = format!("--{boundary}");
    let closing = format!("{marker}--");
    let text = String::from_utf8_lossy(body);
    let mut parts = Vec::new();
    let mut current: Option<String> = None;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let trimmed = line.trim_end();
        if trimmed == marker {
            if let Some(part) = current.take() {
                parts.push(part);
            }
            current = Some(String::new());
            continue;
        }
        if trimmed == closing {
            if let Some(part) = current.take() {
                parts.push(part);
            }
            break;
        }
        if let Some(part) = current.as_mut() {
            part.push_str(line);
            part.push('\n');
        }
    }
    parts
        .into_iter()
        .map(|part| part.trim_end_matches('\n').as_bytes().to_vec())
        .filter(|part| !part.is_empty())
        .collect()
}

/// Undo the content-transfer-encoding of one part.
pub(crate) fn decode_transfer(bytes: &[u8], encoding: &str) -> Vec<u8> {
    match encoding {
        "base64" => decode_base64(bytes),
        "quoted-printable" => decode_quoted_printable(bytes),
        // 7bit, 8bit, binary, and anything unrecognized, are already literal.
        _ => bytes.to_vec(),
    }
}

fn decode_base64(bytes: &[u8]) -> Vec<u8> {
    use base64::Engine;
    let cleaned: String = bytes
        .iter()
        .map(|byte| *byte as char)
        .filter(|ch| !ch.is_whitespace())
        .collect();
    let mut padded = cleaned;
    while !padded.len().is_multiple_of(4) {
        padded.push('=');
    }
    match base64::engine::general_purpose::STANDARD.decode(padded.as_bytes()) {
        Ok(decoded) => decoded,
        // A malformed body must not lose the message: keep what arrived.
        Err(_) => bytes.to_vec(),
    }
}

pub(crate) fn decode_quoted_printable(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'=' {
            out.push(bytes[index]);
            index += 1;
            continue;
        }
        // A soft line break: `=` at the end of a line continues it.
        if index + 1 < bytes.len() && bytes[index + 1] == b'\n' {
            index += 2;
            continue;
        }
        if index + 2 < bytes.len() && bytes[index + 1] == b'\r' && bytes[index + 2] == b'\n' {
            index += 3;
            continue;
        }
        if index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(value) = hex.and_then(|text| u8::from_str_radix(text, 16).ok()) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        // A stray `=` is literal.
        out.push(b'=');
        index += 1;
    }
    out
}

/// The `windows-1252` escapes for 0x80–0x9F, which `latin1` gets wrong and
/// which older clients emit constantly.
const WINDOWS_1252_HIGH: [char; 32] = [
    '\u{20ac}', '\u{fffd}', '\u{201a}', '\u{0192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02c6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{fffd}', '\u{017d}', '\u{fffd}',
    '\u{fffd}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02dc}', '\u{2122}', '\u{0161}', '\u{203a}', '\u{0153}', '\u{fffd}', '\u{017e}', '\u{0178}',
];

/// Text from bytes, honouring the charsets a mailbox actually contains.
pub(crate) fn decode_text(bytes: &[u8], charset: &str) -> String {
    let name = charset.trim().to_ascii_lowercase();
    let name = name.split(';').next().unwrap_or("").trim().to_string();
    match name.as_str() {
        "iso-8859-1" | "iso8859-1" | "latin1" | "latin-1" | "cp819" => {
            bytes.iter().map(|byte| char::from(*byte)).collect()
        }
        "windows-1252" | "cp1252" | "x-cp1252" => bytes
            .iter()
            .map(|byte| match *byte {
                b @ 0x80..=0x9f => WINDOWS_1252_HIGH[(b - 0x80) as usize],
                b => char::from(b),
            })
            .collect(),
        // UTF-8 (the default), US-ASCII, and anything we do not know: a lossy
        // decode beats dropping the message.
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Decode RFC 2047 encoded words in a header value.
///
/// Whitespace *between* two encoded words is not part of the value, which is
/// what stops a folded subject from arriving with gaps in it.
pub(crate) fn decode_header(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    let mut previous_encoded = false;
    loop {
        let Some(start) = rest.find("=?") else {
            out.push_str(rest);
            return out;
        };
        let prefix = &rest[..start];
        if !(previous_encoded && prefix.trim().is_empty()) {
            out.push_str(prefix);
        }
        match decode_word(&rest[start..]) {
            Some((consumed, decoded)) => {
                out.push_str(&decoded);
                previous_encoded = true;
                rest = &rest[start + consumed..];
            }
            None => {
                // Not an encoded word after all: keep the text verbatim.
                out.push_str("=?");
                previous_encoded = false;
                rest = &rest[start + 2..];
            }
        }
    }
}

/// Decode one `=?charset?B|Q?text?=` at the start of `value`, returning how many
/// bytes it consumed.
fn decode_word(value: &str) -> Option<(usize, String)> {
    // `=?charset?B|Q?payload?=`: the `=?` is the marker, not a field, so it comes
    // off before the fields are split.
    let rest = value.strip_prefix("=?")?;
    let mut parts = rest.splitn(3, '?');
    let charset = parts.next()?;
    let encoding = parts.next()?.to_ascii_uppercase();
    let payload = parts.next()?;
    let end = payload.find("?=")?;
    let text = &payload[..end];
    let bytes = match encoding.as_str() {
        "B" => decode_base64(text.as_bytes()),
        "Q" => decode_quoted_printable(text.replace('_', " ").as_bytes()),
        _ => return None,
    };
    let consumed = 2 + (rest.len() - payload.len()) + end + 2;
    Some((consumed, decode_text(&bytes, charset)))
}

/// Reduce HTML to the text a reader would see.
pub(crate) fn strip_html(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut index = 0;
    while index < html.len() {
        if lower.as_bytes()[index] != b'<' {
            let ch = html[index..].chars().next().unwrap_or(' ');
            out.push(ch);
            index += ch.len_utf8();
            continue;
        }
        let Some(end) = lower[index..].find('>').map(|offset| index + offset) else {
            break;
        };
        // A closing tag ends the same block an opening tag starts, so `</p>`
        // has to count as a boundary too — otherwise two paragraphs run
        // together into one line.
        let tag = lower[index + 1..end].trim_end_matches('/').trim();
        let name = tag
            .trim_start_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or("");
        // Script and style *content* is not text; every other tag is only markup
        // we drop while keeping the words around it.
        if name == "script" || name == "style" {
            let closing = format!("</{name}");
            index = match lower[end..].find(&closing) {
                Some(offset) => lower[end + offset..]
                    .find('>')
                    .map(|close| end + offset + close + 1)
                    .unwrap_or(lower.len()),
                None => lower.len(),
            };
            continue;
        }
        if matches!(
            name,
            "br" | "p"
                | "div"
                | "tr"
                | "li"
                | "ul"
                | "ol"
                | "table"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "blockquote"
                | "pre"
        ) {
            out.push('\n');
        }
        index = end + 1;
    }
    collapse(&decode_entities(&out))
}

/// Collapse the runs of blank lines and spaces HTML leaves behind.
fn collapse(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let trimmed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.is_empty() {
            if lines.last().is_some_and(|last| !last.is_empty()) {
                lines.push(String::new());
            }
        } else {
            lines.push(trimmed);
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

/// The entity references a plain-text rendering needs, named and numeric.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        match tail.find(';').filter(|offset| *offset <= 12) {
            Some(end) => {
                let entity = &tail[1..end];
                match entity_name(entity) {
                    Some(ch) => {
                        out.push(ch);
                        rest = &tail[end + 1..];
                    }
                    None => {
                        // Not an entity we know: keep the ampersand and carry on
                        // from the next character, so a stray `&` earlier in the
                        // text cannot stop a real `&amp;` later from decoding.
                        out.push('&');
                        rest = &tail[1..];
                    }
                }
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity_name(entity: &str) -> Option<char> {
    Some(match entity.to_ascii_lowercase().as_str() {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "ndash" => '\u{2013}',
        "mdash" => '\u{2014}',
        "hellip" => '\u{2026}',
        "rsquo" => '\u{2019}',
        "lsquo" => '\u{2018}',
        "ldquo" => '\u{201c}',
        "rdquo" => '\u{201d}',
        "copy" => '\u{a9}',
        other => {
            let number = other.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse::<u32>().ok()?,
            };
            char::from_u32(code)?
        }
    })
}

/// `Name <address>`, `address (Name)` and a bare address, all understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Address {
    pub address: String,
    pub display: Option<String>,
}

pub(crate) fn parse_address(value: &str) -> Option<Address> {
    let without_comments = strip_comments(value);
    let (address, display) = match (without_comments.find('<'), without_comments.rfind('>')) {
        (Some(open), Some(close)) if close > open => (
            without_comments[open + 1..close].trim().to_string(),
            without_comments[..open]
                .trim()
                .trim_matches('"')
                .trim()
                .to_string(),
        ),
        _ => (without_comments.trim().to_string(), String::new()),
    };
    // An address is one token with an `@`; anything else is not something we can
    // answer, and putting it in a header would be worse than dropping the mail.
    if address.is_empty()
        || !address.contains('@')
        || address.split_whitespace().count() != 1
        || address.chars().any(char::is_control)
    {
        return None;
    }
    let display = decode_header(&display);
    Some(Address {
        address: address.to_ascii_lowercase(),
        display: (!display.is_empty()).then_some(display),
    })
}

/// Remove `(comment)` groups, which RFC 5322 allows around an address.
fn strip_comments(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut depth = 0usize;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            if depth == 0 {
                out.push(ch);
            }
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            other if depth == 0 => out.push(other),
            _ => {}
        }
    }
    out.trim().to_string()
}

// ─── gating ────────────────────────────────────────────────────────────────

/// Why a message the mailbox offered is not a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Skip {
    /// Our own sent copy, or a bounce addressed to us.
    OwnMessage,
    /// A machine sent it (`Auto-Submitted`, bulk precedence): answering is how
    /// mail loops start.
    Automated,
    /// No usable `From`.
    NoSender,
    /// Not on `sender_allowlist`.
    NotAllowed,
    /// The subject does not carry the configured prefix.
    NoSubjectMatch,
}

impl Skip {
    fn reason(self) -> &'static str {
        match self {
            Skip::OwnMessage => "the message is from this mailbox's own address",
            Skip::Automated => "the message is automated mail (Auto-Submitted or bulk precedence)",
            Skip::NoSender => "the message has no usable From address",
            Skip::NotAllowed => "the sender is not on the allowlist",
            Skip::NoSubjectMatch => "the subject does not start with the configured prefix",
        }
    }
}

/// Whether automated mail must not be answered. `Auto-Submitted: no` is the one
/// value that means a human wrote it.
pub(crate) fn is_automated(headers: &Headers) -> bool {
    if let Some(value) = headers.get("auto-submitted") {
        let value = value.trim().to_ascii_lowercase();
        if !value.is_empty() && value != "no" {
            return true;
        }
    }
    headers
        .get("precedence")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "bulk" | "list" | "junk"
            )
        })
        .unwrap_or(false)
}

pub(crate) fn sender_allowed(address: &str, allowlist: &[String]) -> bool {
    if allowlist.is_empty() {
        return true;
    }
    allowlist
        .iter()
        .any(|entry| entry.trim().eq_ignore_ascii_case(address))
}

pub(crate) fn subject_allows(subject: &str, prefix: &str) -> bool {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return true;
    }
    subject
        .trim()
        .to_ascii_lowercase()
        .starts_with(&prefix.to_ascii_lowercase())
}

/// Parse an RFC 2822 `Date`. `None` when it is missing or unparsable, because an
/// invented timestamp would be worse than none.
pub(crate) fn parse_date(value: &str) -> Option<i64> {
    let clean = strip_comments(value);
    let clean = clean.trim();
    if clean.is_empty() {
        return None;
    }
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc2822(clean) {
        return Some(parsed.timestamp_millis());
    }
    // A missing day-of-week is common enough to be worth a second attempt.
    chrono::DateTime::parse_from_str(clean, "%d %b %Y %H:%M:%S %z")
        .ok()
        .map(|parsed| parsed.timestamp_millis())
}

/// The `Message-ID` header, without its angle brackets.
pub(crate) fn message_identifier(headers: &Headers) -> Option<String> {
    let value = headers.get("message-id")?.trim();
    let value = value.strip_prefix('<').unwrap_or(value);
    let value = value.strip_suffix('>').unwrap_or(value);
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// The thread a message belongs to: the root of its `References`, else what it
/// answers, else itself.
pub(crate) fn thread_root(headers: &Headers, own_id: &str) -> String {
    fn first_id(value: &str) -> Option<String> {
        let start = value.find('<')?;
        let end = value[start..].find('>')? + start;
        let id = value[start + 1..end].trim().to_string();
        (!id.is_empty()).then_some(id)
    }
    headers
        .get("references")
        .and_then(first_id)
        .or_else(|| headers.get("in-reply-to").and_then(first_id))
        .unwrap_or_else(|| own_id.to_string())
}

/// The subject a reader would see, or an empty string.
pub(crate) fn subject_of(headers: &Headers) -> String {
    headers
        .get("subject")
        .map(decode_header)
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

/// Turn a fetched message into the bridge's envelope, or say why it is not a
/// prompt.
pub(crate) fn to_inbound(
    uid: u32,
    mailbox: &MailboxState,
    own_address: &str,
    config: &EmailConfig,
    mail: &Mail,
) -> std::result::Result<Inbound, Skip> {
    let sender = parse_address(mail.headers.get("from").unwrap_or("")).ok_or(Skip::NoSender)?;
    if sender.address.eq_ignore_ascii_case(own_address.trim()) {
        return Err(Skip::OwnMessage);
    }
    if is_automated(&mail.headers) {
        return Err(Skip::Automated);
    }
    if !sender_allowed(&sender.address, &config.sender_allowlist) {
        return Err(Skip::NotAllowed);
    }
    let subject = subject_of(&mail.headers);
    if !subject_allows(&subject, &config.subject_prefix) {
        return Err(Skip::NoSubjectMatch);
    }
    let own_id = message_identifier(&mail.headers)
        .unwrap_or_else(|| format!("{}/{}", mailbox.uid_validity, uid));
    let thread = thread_root(&mail.headers, &own_id);
    let raw = serde_json::json!({
        "uid": uid,
        "uid_validity": mailbox.uid_validity,
        "subject": subject,
        "from": sender.address,
        "to": mail.headers.get("to").unwrap_or(""),
        "date": mail.headers.get("date").unwrap_or(""),
        "date_ms": parse_date(mail.headers.get("date").unwrap_or("")),
        "message_id": own_id,
        "in_reply_to": mail.headers.get("in-reply-to").unwrap_or(""),
        "references": mail.headers.get("references").unwrap_or(""),
        "attachments": mail
            .attachments
            .iter()
            .map(|attachment| serde_json::json!({
                "filename": attachment.filename,
                "content_type": attachment.content_type,
                "bytes": attachment.bytes,
            }))
            .collect::<Vec<_>>(),
    });
    Ok(Inbound {
        message_id: own_id,
        sender: SenderRef {
            id: sender.address.clone(),
            display: sender.display,
        },
        conversation: ConversationRef {
            // One conversation per correspondent, one thread per mail thread:
            // the bridge gives each of those its own agent session.
            id: sender.address,
            thread_id: Some(thread),
            kind: ChatKind::Direct,
        },
        text: mail.text.clone(),
        media: mail
            .attachments
            .iter()
            .map(|attachment| MediaRef {
                kind: media_kind(&attachment.content_type),
                filename: attachment.filename.clone(),
                content_type: Some(attachment.content_type.clone()),
                // Nothing is downloaded from the mailbox: the reference says
                // what arrived, not how to fetch it.
                url: None,
                data: None,
            })
            .collect(),
        addressed_to_bot: true,
        // A mailbox is store-and-forward, and a poll answers what is *unseen*,
        // not what is recent. The bridge drops anything older than its
        // freshness window, which on a 30-second poll would silently discard
        // most real mail, so no timestamp is claimed here: the `\Seen` flag and
        // this channel's own record are the replay filter. The real `Date`
        // header is kept in `raw`, as sent and parsed.
        created_at_ms: None,
        raw: Some(raw),
    })
}

fn media_kind(content_type: &str) -> MediaKind {
    match content_type.split('/').next().unwrap_or("") {
        "image" => MediaKind::Image,
        "audio" => MediaKind::Audio,
        "video" => MediaKind::Video,
        _ => MediaKind::Document,
    }
}

// ─── delivered-message memory ──────────────────────────────────────────────

/// Identifiers of messages already handed to the bridge, kept across restarts.
///
/// `\Seen` covers the ordinary case; this covers the window between fetching a
/// message and marking it, and a mailbox that cannot be written to at all.
pub(crate) struct SeenStore {
    path: PathBuf,
    inner: Mutex<SeenInner>,
}

#[derive(Default, Serialize, Deserialize)]
struct SeenFile {
    #[serde(default)]
    keys: Vec<String>,
}

#[derive(Default)]
struct SeenInner {
    keys: HashSet<String>,
    order: VecDeque<String>,
}

impl SeenStore {
    pub(crate) fn load(path: PathBuf) -> Self {
        let keys = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<SeenFile>(&text).ok())
            .map(|file| file.keys)
            .unwrap_or_default();
        let mut inner = SeenInner::default();
        for key in keys {
            if inner.keys.insert(key.clone()) {
                inner.order.push_back(key);
            }
        }
        trim(&mut inner);
        Self {
            path,
            inner: Mutex::new(inner),
        }
    }

    pub(crate) fn contains(&self, key: &str) -> bool {
        self.inner.lock().keys.contains(key)
    }

    pub(crate) fn len(&self) -> usize {
        self.inner.lock().keys.len()
    }

    /// Remember a message, and persist. A failure to write is reported but not
    /// fatal: the message has to be answered either way.
    pub(crate) fn record(&self, key: &str) -> Result<()> {
        {
            let mut inner = self.inner.lock();
            if !inner.keys.insert(key.to_string()) {
                return Ok(());
            }
            inner.order.push_back(key.to_string());
            trim(&mut inner);
        }
        self.save()
    }

    fn save(&self) -> Result<()> {
        let file = SeenFile {
            keys: self.inner.lock().order.iter().cloned().collect(),
        };
        let text = serde_json::to_string(&file)?;
        std::fs::write(&self.path, text)
            .map_err(|error| anyhow!("cannot write {}: {error}", self.path.display()))
    }
}

/// Keep the store bounded, oldest key first.
fn trim(inner: &mut SeenInner) {
    while inner.order.len() > SEEN_CAPACITY {
        if let Some(oldest) = inner.order.pop_front() {
            inner.keys.remove(&oldest);
        }
    }
}

/// The `Message-ID` identity of a message, as a store key.
fn delivery_key(message_id: &str) -> String {
    format!("mid:{message_id}")
}

/// Whether this message has already been handed to the bridge, either at the
/// same mailbox position or under the same `Message-ID`.
fn was_delivered(seen: &SeenStore, uid_key: &str, message_id: &str) -> bool {
    seen.contains(uid_key) || seen.contains(&delivery_key(message_id))
}

/// Remember a delivered message under both of its identities.
///
/// The mailbox position is what a restart needs; the `Message-ID` is what
/// catches the same mail arriving under a second UID, which a move between
/// mailboxes or a server redelivery produces.
fn remember_delivery(seen: &SeenStore, uid_key: &str, message_id: &str) -> Result<()> {
    seen.record(uid_key)?;
    seen.record(&delivery_key(message_id))
}

// ─── the provider ──────────────────────────────────────────────────────────

/// The outbound half: one SMTP submission per answer.
pub struct EmailSender {
    config: EmailConfig,
    subjects: Mutex<SubjectCache>,
}

impl EmailSender {
    fn new(config: EmailConfig) -> Self {
        Self {
            config,
            subjects: Mutex::new(SubjectCache::default()),
        }
    }

    /// Remember what a conversation was about, so the reply can say `Re: …`.
    fn remember_subject(&self, key: &str, subject: &str) {
        self.subjects.lock().insert(key, subject);
    }

    fn subject_for(&self, conversation: &ConversationRef) -> String {
        let key = conversation.key(DEFINITION.id);
        let cache = self.subjects.lock();
        reply_subject(cache.get(&key), &self.config.subject_prefix)
    }
}

#[async_trait]
impl ChannelSender for EmailSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    /// Send one mail.
    ///
    /// No message id is returned: a mail server gives none that could be edited
    /// later, so progressive output is not something this channel offers.
    async fn send_text(
        &self,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        let to = recipient_address(conversation)?;
        let message = build_message(
            &self.config.smtp,
            &to,
            &self.subject_for(conversation),
            conversation.thread_id.as_deref(),
            text,
        )?;
        // A fresh submission per message: the bridge may call this from a
        // process that runs no poller, and an SMTP connection is cheap.
        let mut session = SmtpSession::connect(&self.config.smtp).await?;
        let submitted = session
            .submit(self.config.smtp.from.trim(), &to, &encode_data(&message))
            .await;
        let _ = session.quit().await;
        submitted?;
        Ok(None)
    }
}

/// The receiving half.
#[derive(Debug)]
struct Email;

impl Email {
    /// One mailbox connection: drain what is unseen, then wait and do it again.
    async fn poll(
        &self,
        ctx: &ProviderCtx,
        config: &EmailConfig,
        seen: &SeenStore,
        sender: &Arc<EmailSender>,
        shutdown: &mut std::pin::Pin<&mut impl std::future::Future<Output = ()>>,
    ) -> Result<()> {
        let sender_trait: Arc<dyn ChannelSender> = sender.clone();
        let mut session = ImapSession::connect(&config.imap).await?;
        session.login(&config.imap).await?;
        let mailbox = session.select(&config.imap.mailbox).await?;
        ctx.mark_running();
        tracing::info!(
            user = %config.imap.username,
            mailbox = %config.imap.mailbox,
            messages = mailbox.exists,
            "email mailbox connected"
        );

        loop {
            for uid in session
                .search_unseen()
                .await?
                .into_iter()
                .take(MAX_FETCH_PER_POLL)
            {
                let uid_key = format!("{}:{}", mailbox.uid_validity, uid);
                if seen.contains(&uid_key) {
                    // Answered in an earlier run and never marked: finish the job.
                    let _ = session.mark_seen(uid).await;
                    continue;
                }
                let raw = session.fetch_body(uid).await?;
                let mail = parse_mail(&raw);
                let inbound = match to_inbound(uid, &mailbox, &config.smtp.from, config, &mail) {
                    Ok(inbound) => inbound,
                    Err(skip) => {
                        tracing::debug!(uid, reason = skip.reason(), "ignoring a mailbox message");
                        if let Err(error) = seen.record(&uid_key) {
                            tracing::warn!(%error, "cannot record a skipped message");
                        }
                        let _ = session.mark_seen(uid).await;
                        continue;
                    }
                };
                sender.remember_subject(
                    &inbound.conversation_key(DEFINITION.id),
                    &subject_of(&mail.headers),
                );
                // Kept before `inbound` is handed to the bridge, which consumes
                // it.
                let message_id = inbound.message_id.clone();
                if was_delivered(seen, &uid_key, &message_id) {
                    // The same message under a second UID (moved between
                    // mailboxes, or redelivered by the server).
                    tracing::debug!(uid, "ignoring a message that was already answered");
                    let _ = seen.record(&uid_key);
                    let _ = session.mark_seen(uid).await;
                    continue;
                }
                let outcome = ctx.handle(inbound, sender_trait.clone()).await;
                tracing::debug!(uid, ?outcome, "mailbox message handled");
                if outcome == HandleOutcome::Backpressure {
                    // The bridge could not take it: leave it unseen so a restart
                    // answers it, rather than marking it read and forgetting it.
                    continue;
                }
                if let Err(error) = remember_delivery(seen, &uid_key, &message_id) {
                    tracing::warn!(%error, "cannot record a delivered message");
                }
                if let Err(error) = session.mark_seen(uid).await {
                    tracing::warn!(uid, %error, "the message was answered but not marked seen");
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(config.poll_interval()) => {}
                _ = shutdown.as_mut() => {
                    let _ = session.logout().await;
                    return Ok(());
                }
            }
        }
    }
}

#[async_trait]
impl Provider for Email {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        // Building a sender never connects: `future channel send` uses one with
        // no poller running.
        let config: EmailConfig = ctx.config()?;
        config.validate(DEFINITION.id)?;
        Ok(Arc::new(EmailSender::new(config)))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        let config: EmailConfig = ctx.config()?;
        config.validate(DEFINITION.id)?;
        ctx.ensure_data_dir()?;
        let seen = SeenStore::load(ctx.data_dir().join("seen.json"));
        let sender = Arc::new(EmailSender::new(config.clone()));
        let mut backoff = Backoff::new(Duration::from_secs(2), RECONNECT_MAX);
        // One shutdown waiter for the whole loop: `notify_waiters` only wakes
        // waiters that already exist, so a fresh `notified()` per attempt would
        // miss the signal and the channel would never stop.
        let shutdown = ctx.shutdown().notified();
        tokio::pin!(shutdown);

        loop {
            let started = std::time::Instant::now();
            let result = self
                .poll(&ctx, &config, &seen, &sender, &mut shutdown)
                .await;
            match result {
                // `poll` only returns `Ok` when it was asked to stop; a mailbox
                // that goes away is an error that wants a reconnect.
                Ok(()) => return Ok(()),
                Err(error) if error.is::<FatalError>() => return Err(error),
                Err(error) => {
                    ctx.mark_failed(&error.to_string());
                    tracing::warn!(%error, "email polling failed; reconnecting");
                }
            }
            if started.elapsed() >= HEALTHY_CONNECTION {
                backoff.reset();
            }
            let delay = backoff.next_delay();
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = shutdown.as_mut() => return Ok(()),
            }
        }
    }

    /// `future channel test`: log in to the mailbox and authenticate to the
    /// submission server. Nothing here is simulated — a probe that reported
    /// success without connecting would be worse than no probe.
    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        let config: EmailConfig = ctx.config()?;
        config.validate(DEFINITION.id)?;
        let mut session = ImapSession::connect(&config.imap).await?;
        session.login(&config.imap).await?;
        let mailbox = session.select(&config.imap.mailbox).await?;
        let unseen = session.search_unseen().await?.len();
        let _ = session.logout().await;

        let mut smtp = SmtpSession::connect(&config.smtp).await?;
        let mechanism = if config.smtp.username.trim().is_empty() {
            "no authentication"
        } else {
            "authenticated"
        };
        let _ = smtp.quit().await;
        Ok(format!(
            "IMAP {}@{}: {} has {} message(s), {unseen} unseen; SMTP {}:{} {mechanism}",
            config.imap.username,
            config.imap.host,
            config.imap.mailbox,
            mailbox.exists,
            config.smtp.host,
            config.smtp.port,
        ))
    }
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(Email)
}

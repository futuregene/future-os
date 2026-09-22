//! IRC: the line protocol itself, over TCP or TLS.
//!
//! IRC is the cheapest channel to support well because it is plain text, but it
//! is also where the details bite: a message cap measured in *bytes including
//! the protocol overhead*, a nickname collision on connect, and servers that
//! silently drop an idle client.
//!
//! * **Connection** — `CAP` negotiation with `SASL PLAIN` when credentials are
//!   configured, `NICK`/`USER` registration, a rename when the server reports
//!   the nickname is taken, `PING`/`PONG` keepalive, a periodic client `PING`
//!   (the only way a silently dropped TCP connection is ever noticed), and an
//!   internal reconnect loop with backoff.
//! * **Inbound** — `PRIVMSG` becomes [`crate::bridge::Inbound`], resolving
//!   `nick!user@host` to the nickname. Only the channels named in the
//!   configuration are conversations at all: being invited somewhere else does
//!   not put that channel's chatter in front of the agent. A channel message
//!   counts as addressed when it starts with the bot's nickname (`nick:`,
//!   `nick,` or `@nick`).
//! * **Outbound** — every line is split so the *whole* line, command and CRLF
//!   included, stays inside the 512-byte protocol limit — the definition's
//!   limit is only what the bridge knows about, and the overhead depends on the
//!   target's length, so the split happens here. Control characters are
//!   replaced, because a line break in a message would inject a command.
//! * **Noise control** — CTCP (`ACTION`, `VERSION`) and our own echoes are
//!   ignored, and the numeric replies that mean "this target will never work"
//!   are remembered so a later send fails immediately instead of being retried
//!   forever by the delivery queue.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::bridge::{ChatKind, ConversationRef, Inbound, ProviderCtx, SenderRef};
use crate::providers::traits::{
    Capabilities, ChannelDefinition, ChannelSender, Maturity, Provider,
};
use crate::transport::ws::Backoff;
use crate::transport::{chunk, LengthUnit};

#[cfg(test)]
#[path = "irc_tests.rs"]
mod tests;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "irc",
    display_name: "IRC",
    description:
        "IRC over TCP or TLS: SASL authentication, channel allowlist, byte-capped messages.",
    docs: "docs/guide/channels-irc.md",
    maturity: Maturity::Preview,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: false,
        threads: false,
        typing: false,
        reactions: false,
        media_in: false,
        media_out: false,
        mention_gate: true,
    },
    // The 512-byte protocol cap minus the PRIVMSG envelope; the provider
    // subtracts its own overhead again when addressing a channel.
    max_text_len: 400,
    length_unit: LengthUnit::Bytes,
    config_example: r##"{
  "enabled": true,
  "server": "irc.libera.chat",
  "port": 6697,
  "tls": true,
  "nick": "",
  "password": "",
  "sasl": { "account": "", "password": "" },
  "channels": ["#future"],
  "idle_ping_seconds": 120,
  "require_mention": true
}"##,
    requires: &[],
};

/// A line, CRLF included, may not exceed this — the protocol's hard cap.
const MAX_LINE_BYTES: usize = 512;
/// The command and its separator: `PRIVMSG `.
const PRIVMSG_OVERHEAD: usize = "PRIVMSG ".len();
/// Longest inbound line accepted before the peer is treated as not speaking
/// IRC at all (a hostile or confused server must not grow our buffer).
const MAX_INBOUND_LINE_BYTES: usize = 16 * 1024;
/// One `AUTHENTICATE` line carries at most this many bytes of payload.
const SASL_CHUNK_BYTES: usize = 400;
/// How many times a colliding nickname is renamed before giving up.
const MAX_RENAMES: u32 = 4;
/// Outbound lines buffered while the connection is being (re)established.
const OUTBOUND_QUEUE: usize = 256;
/// How long the initial TCP connect may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Reconnect ceiling; the supervisor's own backoff starts after this loop
/// finally gives up, so this stays modest.
const RECONNECT_MAX: Duration = Duration::from_secs(60);

/// The `providers.irc` block.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct IrcConfig {
    pub server: String,
    pub port: u16,
    /// `true` for the TLS port (the default; port 6697 on most networks).
    pub tls: bool,
    pub nick: String,
    /// Server password (`PASS`), distinct from SASL credentials.
    pub password: String,
    pub sasl: SaslConfig,
    /// The only channels this channel answers in.
    pub channels: Vec<String>,
    /// Seconds between client `PING`s; `0` disables them.
    pub idle_ping_seconds: u64,
}

impl Default for IrcConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            port: 6697,
            tls: true,
            nick: String::new(),
            password: String::new(),
            sasl: SaslConfig::default(),
            channels: Vec::new(),
            idle_ping_seconds: 120,
        }
    }
}

impl IrcConfig {
    fn validate(&self, channel: &str) -> Result<()> {
        let mut missing = Vec::new();
        if self.server.trim().is_empty() {
            missing.push("server");
        }
        if self.nick.trim().is_empty() {
            missing.push("nick");
        }
        if !missing.is_empty() {
            anyhow::bail!(
                "invalid `providers.{channel}` configuration: {} required",
                missing
                    .iter()
                    .map(|field| format!("`{field}` is"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            );
        }
        if self.port == 0 {
            anyhow::bail!("invalid `providers.{channel}` configuration: `port` is required");
        }
        let account = !self.sasl.account.trim().is_empty();
        let password = !self.sasl.password.is_empty();
        if account != password {
            anyhow::bail!(
                "invalid `providers.{channel}` configuration: SASL needs both `sasl.account` and `sasl.password`"
            );
        }
        Ok(())
    }

    /// Whether `target` is a channel this bridge answers in.
    fn is_configured_channel(&self, target: &str) -> bool {
        self.channels
            .iter()
            .any(|channel| channel.eq_ignore_ascii_case(target))
    }
}

/// SASL PLAIN credentials.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SaslConfig {
    pub account: String,
    pub password: String,
}

impl SaslConfig {
    fn is_configured(&self) -> bool {
        !self.account.trim().is_empty() && !self.password.is_empty()
    }

    /// The `AUTHENTICATE PLAIN` payload: base64 of `\0account\0password`.
    fn plain_payload(&self) -> String {
        use base64::Engine;
        let raw = format!("\0{}\0{}", self.account, self.password);
        base64::engine::general_purpose::STANDARD.encode(raw.as_bytes())
    }
}

/// A failure a reconnect cannot repair: bad configuration, rejected
/// credentials, or a ban. Everything else is retried inside the run loop.
#[derive(Debug)]
pub(crate) struct FatalError(pub(crate) String);

impl std::fmt::Display for FatalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for FatalError {}

// ─── the wire ──────────────────────────────────────────────────────────────

/// A connected socket, plain or wrapped in TLS.
///
/// The TLS variant is boxed: rustls keeps a large per-connection buffer, and
/// a `Wire` outlives every message that passes through it, so it stays small.
enum Wire {
    Plain(TcpStream),
    Tls(Box<TlsStream>),
}

/// TLS over a TCP socket.
///
/// rustls works on buffers rather than on a socket, so the two directions are
/// explicit: [`Wire::fill`] moves bytes from the socket into the connection,
/// [`TlsStream::flush`] drains the connection's pending records back out.
struct TlsStream {
    tcp: TcpStream,
    conn: rustls::ClientConnection,
}

impl TlsStream {
    /// Move bytes from the socket into the TLS connection.
    async fn fill(&mut self) -> std::io::Result<usize> {
        let mut raw = [0u8; 8192];
        let read = tokio::io::AsyncReadExt::read(&mut self.tcp, &mut raw).await?;
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
        // A handshake needs a reply (ClientHello, Finished, a key update): the
        // records rustls produced are only on their way out after this.
        self.flush().await?;
        Ok(read)
    }

    /// Write every pending TLS record to the socket.
    async fn flush(&mut self) -> std::io::Result<()> {
        while self.conn.wants_write() {
            let mut out = Vec::new();
            self.conn.write_tls(&mut out)?;
            if out.is_empty() {
                break;
            }
            self.tcp.write_all(&out).await?;
        }
        self.tcp.flush().await
    }

    /// Read decrypted bytes, filling the connection as often as it takes.
    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            match std::io::Read::read(&mut self.conn.reader(), buf) {
                Ok(read) => return Ok(read),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error),
            }
            if self.fill().await? == 0 {
                return Ok(0);
            }
        }
    }
}

/// The platform trust store, so a network behind a corporate TLS terminator
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
    /// Open the socket and, when configured, complete the TLS handshake.
    async fn connect(config: &IrcConfig) -> Result<Self> {
        let address = format!("{}:{}", config.server, config.port);
        let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&address))
            .await
            .map_err(|_| anyhow!("timed out connecting to {address}"))?
            .map_err(|error| anyhow!("cannot connect to {address}: {error}"))?;
        if !config.tls {
            return Ok(Wire::Plain(tcp));
        }
        let name = rustls::pki_types::ServerName::try_from(config.server.clone())
            .map_err(|error| anyhow!("`{}` is not a usable TLS name: {error}", config.server))?;
        let conn = rustls::ClientConnection::new(Arc::new(tls_config()?), name)?;
        let mut stream = TlsStream { tcp, conn };
        stream.flush().await?;
        while stream.conn.is_handshaking() {
            if stream.fill().await? == 0 {
                anyhow::bail!("{address} closed the connection during the TLS handshake");
            }
        }
        Ok(Wire::Tls(Box::new(stream)))
    }

    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Wire::Plain(tcp) => tokio::io::AsyncReadExt::read(tcp, buf).await,
            Wire::Tls(stream) => stream.read(buf).await,
        }
    }

    async fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        match self {
            Wire::Plain(tcp) => tokio::io::AsyncWriteExt::write_all(tcp, bytes).await,
            Wire::Tls(stream) => {
                std::io::Write::write_all(&mut stream.conn.writer(), bytes)?;
                stream.flush().await
            }
        }
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Wire::Plain(tcp) => tokio::io::AsyncWriteExt::flush(tcp).await,
            Wire::Tls(stream) => stream.flush().await,
        }
    }
}

// ─── the protocol ──────────────────────────────────────────────────────────

/// One parsed IRC line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// IRCv3 tags (`@name=value`), which is where a server timestamp lives.
    pub tags: Vec<(String, String)>,
    /// `nick!user@host` (or a server name) when the server sent one.
    pub prefix: Option<String>,
    /// The command or numeric, as the server wrote it.
    pub command: String,
    pub params: Vec<String>,
}

impl Message {
    fn param(&self, index: usize) -> &str {
        self.params.get(index).map(String::as_str).unwrap_or("")
    }

    /// The last parameter — the trailing one, when the line has one.
    fn trailing(&self) -> &str {
        self.params.last().map(String::as_str).unwrap_or("")
    }

    fn tag(&self, name: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The nickname a prefix belongs to, without `!user@host`.
    fn nickname(&self) -> Option<&str> {
        let prefix = self.prefix.as_deref()?;
        let nick = prefix.split(['!', '@']).next().unwrap_or("").trim();
        (!nick.is_empty()).then_some(nick)
    }
}

/// Parse one line: `[@tags] [:prefix] COMMAND [params] [:trailing]`.
///
/// `None` for a line with no command at all (a bare tag block, or a blank
/// line), which is not an error worth acting on.
pub fn parse_line(line: &str) -> Option<Message> {
    let mut rest = line.trim_end_matches(['\r', '\n']);
    if rest.is_empty() {
        return None;
    }

    let mut tags = Vec::new();
    if let Some(after) = rest.strip_prefix('@') {
        let (block, remainder) = match after.find(' ') {
            Some(index) => (&after[..index], &after[index + 1..]),
            None => (after, ""),
        };
        for pair in block.split(';') {
            // A tag without a value is a flag; keep it with an empty value so
            // the shape is uniform.
            let (key, value) = match pair.split_once('=') {
                Some((key, value)) => (key, unescape_tag(value)),
                None => (pair, String::new()),
            };
            tags.push((key.to_string(), value));
        }
        rest = remainder;
    }

    let mut prefix = None;
    if let Some(after) = rest.strip_prefix(':') {
        let (value, remainder) = match after.find(' ') {
            Some(index) => (&after[..index], &after[index + 1..]),
            None => (after, ""),
        };
        prefix = Some(value.to_string());
        rest = remainder;
    }

    let command = match rest.find(' ') {
        Some(index) => {
            let command = rest[..index].to_string();
            rest = &rest[index + 1..];
            command
        }
        None => {
            let command = rest.to_string();
            rest = "";
            command
        }
    };
    if command.is_empty() {
        return None;
    }
    let mut params = Vec::new();
    while !rest.is_empty() {
        // A trailing parameter is the rest of the line, spaces included.
        if let Some(trailing) = rest.strip_prefix(':') {
            params.push(trailing.to_string());
            break;
        }
        match rest.find(' ') {
            Some(index) => {
                let param = &rest[..index];
                if !param.is_empty() {
                    params.push(param.to_string());
                }
                rest = &rest[index + 1..];
            }
            None => {
                params.push(rest.to_string());
                break;
            }
        }
    }

    Some(Message {
        tags,
        prefix,
        command,
        params,
    })
}

/// The IRCv3 tag escaping (`\:` is `;`, `\s` is a space, and so on).
fn unescape_tag(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some(':') => out.push(';'),
            Some('s') => out.push(' '),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The server timestamp of a line, when the server sent one.
///
/// IRCv3's `server-time` is RFC3339; a message without it has no timestamp at
/// all, and inventing one would defeat the bridge's replay filter.
pub(crate) fn server_time_ms(tags: &[(String, String)]) -> Option<i64> {
    let value = tags
        .iter()
        .find(|(key, _)| key == "time")
        .map(|(_, value)| value.as_str())?;
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp_millis())
}

/// Whether a target names a channel rather than a user.
pub(crate) fn is_channel(target: &str) -> bool {
    matches!(
        target.chars().next(),
        Some('#') | Some('&') | Some('+') | Some('!')
    )
}

/// IRC compares nicknames and channels case-insensitively (ASCII casemapping).
pub(crate) fn normalize(target: &str) -> String {
    target.to_ascii_lowercase()
}

/// Whether a channel message addresses the bot, and the text without the
/// address.
///
/// The convention is a leading `nick:` (or `nick,`, or `@nick`), which every
/// client inserts when the user tabs a nickname. Matching is on the whole first
/// token, so `bother:` never addresses `bot`.
pub(crate) fn strip_mention(text: &str, nick: &str) -> (bool, String) {
    let trimmed = text.trim_start();
    let without_at = trimmed.strip_prefix('@').unwrap_or(trimmed);
    let (token, rest) = match without_at.find(char::is_whitespace) {
        Some(index) => (&without_at[..index], &without_at[index..]),
        None => (without_at, ""),
    };
    let bare = token.trim_end_matches([':', ',']);
    if !bare.is_empty() && bare.eq_ignore_ascii_case(nick) {
        (true, rest.trim_start().to_string())
    } else {
        (false, text.to_string())
    }
}

/// The bytes of message text a `PRIVMSG` to `target` may carry.
///
/// `PRIVMSG <target> :<text>\r\n` is what has to fit in 512 bytes, so the
/// budget shrinks as the target grows.
pub(crate) fn privmsg_budget(target: &str) -> usize {
    MAX_LINE_BYTES.saturating_sub(2 + PRIVMSG_OVERHEAD + target.len() + 2)
}

/// A message cannot contain a line break: one would let a reply inject an
/// arbitrary command. Every control character becomes a space.
pub(crate) fn sanitize(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

/// Split one reply into complete `PRIVMSG` lines, each inside the cap.
pub(crate) fn split_privmsg(target: &str, text: &str) -> Result<Vec<String>> {
    let budget = privmsg_budget(target);
    // Room for at least a few characters, or the configuration is unusable.
    if budget < 16 {
        anyhow::bail!("the IRC target `{target}` leaves no room for a message");
    }
    let clean = sanitize(text);
    Ok(chunk(&clean, budget, LengthUnit::Bytes)
        .into_iter()
        .map(|piece| format!("PRIVMSG {target} :{piece}"))
        .collect())
}

/// The nickname to try after the server reported a collision.
pub(crate) fn next_nick(preferred: &str, attempt: u32) -> String {
    format!("{preferred}{}", "_".repeat(attempt as usize))
}

/// The `AUTHENTICATE` payload, split into protocol-sized lines.
///
/// The server cannot tell a full chunk from a short one, so a payload whose
/// length is an exact multiple of the chunk size ends with the explicit empty
/// chunk (`+`) — without it the server waits forever for the rest.
pub(crate) fn authenticate_lines(payload: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = payload;
    while rest.len() >= SASL_CHUNK_BYTES {
        let (head, tail) = rest.split_at(SASL_CHUNK_BYTES);
        lines.push(format!("AUTHENTICATE {head}"));
        rest = tail;
    }
    if rest.is_empty() {
        lines.push("AUTHENTICATE +".to_string());
    } else {
        lines.push(format!("AUTHENTICATE {rest}"));
    }
    lines
}

/// What a server numeric means for a target, when it means "never works".
///
/// The wording matters: the delivery queue decides whether to keep retrying
/// from this text, so each reason says what actually happened in words the
/// shared vocabulary already knows.
pub(crate) fn numeric_refusal(code: u16, target: &str, reason: &str) -> Option<String> {
    let described = match code {
        401 => {
            format!("the recipient is not on the network (user not found): `{target}` — {reason}")
        }
        403 => format!("channel not found: `{target}` ({reason})"),
        404 => format!("`{target}` does not accept messages from this bot (forbidden): {reason}"),
        442 => format!("the bot is not a member of `{target}`: {reason}"),
        473..=475 => format!("cannot join `{target}` (forbidden): {reason}"),
        _ => return None,
    };
    Some(described)
}

// ─── outbound ──────────────────────────────────────────────────────────────

/// The outbound half: it queues complete lines for the connection task, which
/// owns the socket.
///
/// IRC has no message ids, so a reply cannot be edited later and the bridge
/// never streams into place.
pub struct IrcSender {
    out: mpsc::Sender<String>,
    /// Targets the server rejected, and why. Kept across reconnects: a channel
    /// the bot was kicked from is not suddenly writable again.
    failures: Arc<Mutex<HashMap<String, String>>>,
}

impl IrcSender {
    /// The queued lines are written by the session task, which owns the socket;
    /// a full queue means the connection is not keeping up, and the caller is
    /// told rather than having the message dropped.
    fn queue(&self, line: String) -> Result<()> {
        use tokio::sync::mpsc::error::TrySendError;
        match self.out.try_send(line) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(anyhow!(
                "the IRC connection is not keeping up with outbound messages (transient)"
            )),
            // No live session owns a receiver: this is the `future channel
            // send` path with nothing connected, which is worth saying plainly
            // instead of hiding behind a queue message.
            Err(TrySendError::Closed(_)) => Err(anyhow!(
                "the IRC channel is not connected; start the bridge before sending to it (transient)"
            )),
        }
    }

    fn refusal(&self, target: &str) -> Option<String> {
        self.failures.lock().get(&normalize(target)).cloned()
    }
}

#[async_trait]
impl ChannelSender for IrcSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        let target = conversation.id.trim();
        if target.is_empty() {
            anyhow::bail!("the IRC conversation has no target to send to");
        }
        // A target the server already refused will be refused again; failing
        // now is what stops the delivery queue from retrying it forever.
        if let Some(reason) = self.refusal(target) {
            anyhow::bail!("{reason}");
        }
        for line in split_privmsg(target, text)? {
            self.queue(line)?;
        }
        Ok(None)
    }
}

// ─── the session ───────────────────────────────────────────────────────────

/// One connection's state.
struct Session {
    wire: Wire,
    /// Bytes read from the socket that do not yet form a whole line.
    buffer: Vec<u8>,
    /// Our current nickname — it changes when the server reports a collision.
    nick: String,
    /// How many times we have renamed on this connection.
    renames: u32,
    /// Whether `CAP` negotiation has already been answered.
    /// Targets the server refused, shared with the sender.
    failures: Arc<Mutex<HashMap<String, String>>>,
}

/// What the loop should do with one line.
enum Step {
    /// Nothing worth acting on.
    Ignore,
    /// A message for the bridge.
    Inbound(Box<Inbound>),
    /// The server said something a reconnect cannot fix.
    Fatal(String),
}

impl Session {
    /// Read the next line, or `None` when the server closed the connection.
    async fn read_line(&mut self) -> Result<Option<String>> {
        loop {
            if let Some(index) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=index).collect();
                let line = &line[..line.len() - 1];
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                return Ok(Some(String::from_utf8_lossy(line).into_owned()));
            }
            if self.buffer.len() > MAX_INBOUND_LINE_BYTES {
                anyhow::bail!(
                    "the IRC server sent {} bytes without a line break; not speaking IRC",
                    self.buffer.len()
                );
            }
            let mut chunk = [0u8; 4096];
            let read = self.wire.read(&mut chunk).await?;
            if read == 0 {
                return Ok(None);
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }

    async fn write_line(&mut self, line: &str) -> Result<()> {
        // A line that got here was sanitized and split by `split_privmsg`; the
        // assertion is that no caller ever assembles one from raw input.
        debug_assert!(!line.contains('\n') && !line.contains('\r'));
        let mut bytes = line.as_bytes().to_vec();
        bytes.extend_from_slice(b"\r\n");
        self.wire.write_all(&bytes).await?;
        self.wire.flush().await?;
        Ok(())
    }

    /// The registration burst: password, capability negotiation, identity.
    async fn register(&mut self, config: &IrcConfig) -> Result<()> {
        if !config.password.is_empty() {
            // `PASS` has to precede registration.
            self.write_line(&format!("PASS {}", sanitize(&config.password)))
                .await?;
        }
        if config.sasl.is_configured() {
            self.write_line("CAP LS 302").await?;
        }
        self.write_line(&format!("NICK {}", self.nick)).await?;
        self.write_line(&format!("USER {} 0 * :{}", self.nick, self.nick))
            .await?;
        Ok(())
    }

    /// Act on one line from the server.
    async fn step(&mut self, msg: &Message, config: &IrcConfig, sequence: u64) -> Result<Step> {
        let command = msg.command.to_ascii_uppercase();
        match command.as_str() {
            // The server's keepalive: answering is what keeps the connection.
            "PING" => {
                self.write_line(&format!("PONG :{}", msg.trailing()))
                    .await?;
            }
            "PONG" => {}
            "AUTHENTICATE" => {
                // `+` is the server asking for the payload.
                if msg.trailing() == "+" {
                    for line in authenticate_lines(&config.sasl.plain_payload()) {
                        self.write_line(&line).await?;
                    }
                }
            }
            "CAP" => {
                // `CAP <nick|*> <LS|ACK|NAK> :<list>`
                let subcommand = msg.param(1).to_ascii_uppercase();
                match subcommand.as_str() {
                    "LS" if config.sasl.is_configured() => {
                        self.write_line("CAP REQ :sasl").await?;
                    }
                    "ACK" => {
                        if msg.trailing().split_whitespace().any(|cap| cap == "sasl") {
                            self.write_line("AUTHENTICATE PLAIN").await?;
                        } else {
                            self.write_line("CAP END").await?;
                        }
                    }
                    "NAK" => {
                        tracing::warn!(
                            offered = msg.trailing(),
                            "the IRC server refused SASL; continuing without it"
                        );
                        self.write_line("CAP END").await?;
                    }
                    _ => {}
                }
            }
            "JOIN" => {
                // Our own join echo: the channel is writable again, so forget
                // any refusal recorded for it (a kick, an invite-only join).
                if let Some(nick) = msg.nickname() {
                    if nick.eq_ignore_ascii_case(&self.nick) {
                        self.failures.lock().remove(&normalize(msg.param(0)));
                    }
                }
            }
            "PRIVMSG" => {
                if let Some(inbound) = to_inbound(msg, &self.nick, config, sequence) {
                    return Ok(Step::Inbound(Box::new(inbound)));
                }
            }
            "NOTICE" | "MODE" | "TOPIC" | "PART" | "QUIT" | "NICK" => {}
            "ERROR" => {
                // The server is closing on purpose; reconnecting is right.
                tracing::debug!(
                    reason = msg.trailing(),
                    "the IRC server closed the connection"
                );
                anyhow::bail!("the IRC server closed the connection: {}", msg.trailing());
            }
            other => {
                let code: Option<u16> = other.parse().ok();
                match code {
                    Some(1) => {
                        // Registration finished: ask for the configured channels.
                        tracing::info!(server = %config.server, nick = %self.nick, "registered");
                        for channel in &config.channels {
                            self.write_line(&format!("JOIN {channel}")).await?;
                        }
                    }
                    Some(433) | Some(437) => {
                        self.renames += 1;
                        if self.renames > MAX_RENAMES {
                            return Ok(Step::Fatal(format!(
                                "the nickname `{}` is taken and the variants are too",
                                config.nick
                            )));
                        }
                        self.nick = next_nick(&config.nick, self.renames);
                        tracing::info!(nick = %self.nick, "nickname taken; renaming");
                        self.write_line(&format!("NICK {}", self.nick)).await?;
                    }
                    Some(432) => {
                        return Ok(Step::Fatal(format!(
                            "the IRC server rejects `{}` as a nickname",
                            config.nick
                        )))
                    }
                    Some(464) => {
                        return Ok(Step::Fatal(
                            "the IRC server rejected the server password".to_string(),
                        ))
                    }
                    Some(465) => {
                        return Ok(Step::Fatal(
                            "the IRC server has banned this client".to_string(),
                        ))
                    }
                    // SASL: failed, aborted, or "already authenticated as
                    // somebody else". All of them mean the credentials are not
                    // usable, and retrying would hammer the network.
                    Some(904) | Some(905) | Some(906) | Some(907) => {
                        return Ok(Step::Fatal(format!(
                            "SASL authentication failed: {}",
                            msg.trailing()
                        )))
                    }
                    Some(903) => {
                        tracing::info!(account = %config.sasl.account, "SASL authentication accepted");
                        self.write_line("CAP END").await?;
                    }
                    Some(900) => {
                        tracing::debug!(detail = msg.trailing(), "login recorded by the server");
                    }
                    // A refusal the sender must not keep retrying, or a numeric
                    // (or a non-numeric command) worth no more than a trace.
                    other => {
                        let refusal = other
                            .and_then(|code| numeric_refusal(code, msg.param(1), msg.trailing()));
                        match refusal {
                            Some(reason) if !msg.param(1).is_empty() => {
                                let target = msg.param(1).to_string();
                                tracing::warn!(target = %target, %reason, "the IRC server refused a target");
                                self.failures.lock().insert(normalize(&target), reason);
                            }
                            _ => tracing::trace!(command = %msg.command, "ignoring an IRC line"),
                        }
                    }
                }
            }
        }
        Ok(Step::Ignore)
    }
}

/// Turn a `PRIVMSG` into the bridge's envelope.
///
/// `None` for everything that is not a prompt: CTCP requests, messages in
/// channels this bridge does not answer in, and lines with no sender.
pub(crate) fn to_inbound(
    msg: &Message,
    nick: &str,
    config: &IrcConfig,
    sequence: u64,
) -> Option<Inbound> {
    if !msg.command.eq_ignore_ascii_case("PRIVMSG") || msg.params.len() < 2 {
        return None;
    }
    // CTCP travels inside PRIVMSG with a \x01 marker (`ACTION`, `VERSION`);
    // none of it is something to answer as a prompt.
    if msg.param(1).starts_with('\u{1}') {
        return None;
    }
    let sender = msg.nickname()?.to_string();
    if sender.eq_ignore_ascii_case(nick) {
        // Our own echo — answering it would be a loop.
        return None;
    }
    let target = msg.param(0);
    let (conversation, text, addressed_to_bot) = if is_channel(target) {
        // The channel gate: only configured channels are conversations.
        if !config.is_configured_channel(target) {
            return None;
        }
        let (addressed, text) = strip_mention(msg.param(1), nick);
        (
            ConversationRef {
                id: target.to_string(),
                thread_id: None,
                kind: ChatKind::Channel,
            },
            text,
            addressed,
        )
    } else {
        // Anything not addressed to a channel was addressed to us.
        (
            ConversationRef {
                id: sender.clone(),
                thread_id: None,
                kind: ChatKind::Direct,
            },
            msg.param(1).to_string(),
            true,
        )
    };

    Some(Inbound {
        // IRC has no message ids at all, so the sequence is the identity: it is
        // unique within this process, and a reconnect cannot replay it.
        message_id: format!("irc-{sequence}"),
        sender: SenderRef {
            id: sender,
            display: msg.prefix.clone(),
        },
        conversation,
        text,
        media: Vec::new(),
        addressed_to_bot,
        created_at_ms: server_time_ms(&msg.tags),
        raw: Some(serde_json::json!({
            "prefix": msg.prefix,
            "target": target,
        })),
    })
}

/// The receiving half.
struct Irc;

/// What the run loop does with one event.
enum Event {
    Line(String),
    Outbound(String),
    Heartbeat,
    Shutdown,
}

#[async_trait]
impl Provider for Irc {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        // Building a sender never connects: the connection belongs to `run`,
        // and `future channel send` must not depend on this process holding it.
        let config: IrcConfig = ctx.config()?;
        config.validate(DEFINITION.id)?;
        let (out, _receiver) = mpsc::channel(OUTBOUND_QUEUE);
        Ok(Arc::new(IrcSender {
            out,
            failures: Arc::new(Mutex::new(HashMap::new())),
        }))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        let config: IrcConfig = ctx.config()?;
        config.validate(DEFINITION.id)?;
        let (out_tx, mut out_rx) = mpsc::channel::<String>(OUTBOUND_QUEUE);
        let failures: Arc<Mutex<HashMap<String, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let sender: Arc<dyn ChannelSender> = Arc::new(IrcSender {
            out: out_tx,
            failures: failures.clone(),
        });
        let mut sequence = 0u64;
        let mut backoff = Backoff::new(Duration::from_secs(2), RECONNECT_MAX);
        // One shutdown waiter for the whole loop, created once and shared with
        // each connection: `notify_waiters` only wakes waiters that already
        // exist, so a fresh `notified()` per attempt would miss it and the
        // channel would never stop.
        let shutdown = ctx.shutdown().notified();
        tokio::pin!(shutdown);

        loop {
            let started = std::time::Instant::now();
            let result = self
                .session(
                    &ctx,
                    &config,
                    &sender,
                    &failures,
                    &mut out_rx,
                    &mut sequence,
                    &mut shutdown,
                )
                .await;
            match result {
                Ok(()) => {
                    tracing::info!(server = %config.server, "IRC connection closed; reconnecting")
                }
                Err(error) if error.is::<FatalError>() => return Err(error),
                Err(error) => {
                    ctx.mark_failed(&error.to_string());
                    tracing::warn!(%error, "IRC connection failed; reconnecting");
                }
            }
            // A connection that stayed up counts as healthy, so the next retry
            // is not punished for it.
            if started.elapsed() >= Duration::from_secs(60) {
                backoff.reset();
            }
            let delay = backoff.next_delay();
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = shutdown.as_mut() => return Ok(()),
            }
        }
    }
}

impl Irc {
    /// One connection, from registration to close.
    #[allow(clippy::too_many_arguments)]
    async fn session(
        &self,
        ctx: &ProviderCtx,
        config: &IrcConfig,
        sender: &Arc<dyn ChannelSender>,
        failures: &Arc<Mutex<HashMap<String, String>>>,
        out_rx: &mut mpsc::Receiver<String>,
        sequence: &mut u64,
        shutdown: &mut std::pin::Pin<&mut tokio::sync::futures::Notified<'_>>,
    ) -> Result<()> {
        let mut session = Session {
            wire: Wire::connect(config).await?,
            buffer: Vec::new(),
            nick: config.nick.clone(),
            renames: 0,
            failures: failures.clone(),
        };
        session.register(config).await?;
        tracing::debug!(server = %config.server, nick = %session.nick, "IRC registration sent");

        let ping_enabled = config.idle_ping_seconds > 0;
        // A zero period would panic; the branch below is disabled when the
        // heartbeat is off, so the value only has to be valid.
        let mut heartbeat =
            tokio::time::interval(Duration::from_secs(config.idle_ping_seconds.max(1)));
        // The first tick fires immediately; skip it so registration is not
        // followed by a pointless PING.
        heartbeat.tick().await;

        loop {
            let event = tokio::select! {
                line = session.read_line() => match line? {
                    Some(line) => Event::Line(line),
                    // The server closed the connection: reconnecting is right.
                    None => return Ok(()),
                },
                Some(line) = out_rx.recv() => Event::Outbound(line),
                _ = heartbeat.tick(), if ping_enabled => Event::Heartbeat,
                _ = shutdown.as_mut() => Event::Shutdown,
            };
            match event {
                Event::Shutdown => return Ok(()),
                Event::Heartbeat => {
                    // The server owns liveness only while the socket is alive;
                    // a dropped NAT mapping is invisible until we write, so a
                    // periodic PING is how a dead connection is found.
                    session
                        .write_line(&format!("PING :{}", session.nick))
                        .await?;
                }
                Event::Outbound(line) => session.write_line(&line).await?,
                Event::Line(line) => {
                    let Some(msg) = parse_line(&line) else {
                        continue;
                    };
                    if msg.command == "001" {
                        // Registration finished: the transport is up and the
                        // nickname is ours, so the channel is running.
                        ctx.mark_running();
                    }
                    *sequence += 1;
                    match session.step(&msg, config, *sequence).await? {
                        Step::Ignore => {}
                        Step::Fatal(reason) => {
                            return Err(FatalError(reason).into());
                        }
                        Step::Inbound(inbound) => {
                            // Handling a message can consult the agent, and a
                            // missed PONG gets us disconnected, so the read loop
                            // stays free (the bridge owns ordering anyway).
                            let ctx = ctx.clone();
                            let sender = sender.clone();
                            tokio::spawn(async move {
                                let outcome = ctx.handle(*inbound, sender).await;
                                tracing::debug!(?outcome, "irc message handled");
                            });
                        }
                    }
                }
            }
        }
    }

    /// `future channel test`: prove the socket, the TLS handshake and the
    /// nickname. Nothing here is simulated — a probe that reports success
    /// without connecting would be worse than no probe.
    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        let config: IrcConfig = ctx.config()?;
        config.validate(DEFINITION.id)?;
        let mut session = Session {
            wire: Wire::connect(&config).await?,
            buffer: Vec::new(),
            nick: config.nick.clone(),
            renames: 0,
            failures: Arc::new(Mutex::new(HashMap::new())),
        };
        session.register(&config).await?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while tokio::time::Instant::now() < deadline {
            let line = match tokio::time::timeout_at(deadline, session.read_line()).await {
                Ok(line) => line?,
                Err(_) => break,
            };
            let Some(line) = line else { break };
            let Some(msg) = parse_line(&line) else {
                continue;
            };
            if let Step::Fatal(reason) = session.step(&msg, &config, 0).await? {
                return Err(FatalError(reason).into());
            }
            if msg.command == "001" {
                let transport = if config.tls { "TLS" } else { "TCP" };
                return Ok(format!(
                    "connected to {} over {transport} as {}",
                    config.server, session.nick
                ));
            }
        }
        anyhow::bail!(
            "{} never completed IRC registration within 30 seconds",
            config.server
        )
    }
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(Irc)
}

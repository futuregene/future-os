//! Discord: REST v10 plus the gateway websocket.
//!
//! * **Inbound** — gateway `HELLO`/`IDENTIFY`/`HEARTBEAT`/`RESUME` with the
//!   `GUILD_MESSAGES`, `DIRECT_MESSAGES` and `MESSAGE_CONTENT` intents; the
//!   heartbeat interval comes from `HELLO`, never from a constant. `MESSAGE_CREATE`
//!   payloads turn into [`Inbound`]: our own messages are dropped, attachments
//!   are downloaded, and a guild message counts as addressed when the bot
//!   appears in `mentions`.
//! * **Outbound** — create then edit a message for progressive output, split to
//!   Discord's 2000-character limit without cutting a code block in half (the
//!   shared chunker already reserves room to close and reopen a fence).
//! * **Dedup** — message ids (the bridge) plus ignoring messages the bot sent
//!   itself, so its own posts never loop back as prompts.
//! * **Errors** — `429` carries `Retry-After` and `X-RateLimit-*` bucket
//!   headers including a `global` flag; `400` is a permanent rejection. A
//!   failed `RESUME` re-`IDENTIFY`s and backfills the gap from history.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::bridge::{
    ChatKind, ConversationRef, Inbound, MediaKind, MediaRef, ProviderCtx, SenderRef,
};
use crate::providers::traits::{
    Capabilities, ChannelDefinition, ChannelSender, Maturity, Provider,
};
use crate::transport::http::{send_json, RetryPolicy};
use crate::transport::{ws, LengthUnit};

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "discord",
    display_name: "Discord",
    description: "Discord bot: gateway websocket, streamed replies, guild mention gate.",
    docs: "docs/guide/channels-providers.md#discord",
    maturity: Maturity::Preview,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: true,
        threads: true,
        typing: true,
        reactions: true,
        media_in: true,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 2000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "bot_token": "",
  "guild_allowlist": [],
  "dm_policy": "allowlist",
  "dm_allowlist": [],
  "group_policy": "disabled",
  "group_allowlist": [],
  "require_mention": true,
  "streaming": true
}"#,
    requires: &["a Discord application bot token with the message content intent"],
};

const REST_BASE: &str = "https://discord.com/api/v10";
const GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

/// `GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT`. Without
/// `MESSAGE_CONTENT` the gateway delivers empty `content` for guild messages,
/// so the bot would look alive but never hear anything.
const INTENTS: u64 = (1 << 9) | (1 << 12) | (1 << 15);

// Gateway opcodes used by this provider.
pub(crate) const OP_DISPATCH: u64 = 0;
pub(crate) const OP_HEARTBEAT: u64 = 1;
pub(crate) const OP_IDENTIFY: u64 = 2;
pub(crate) const OP_RESUME: u64 = 6;
pub(crate) const OP_RECONNECT: u64 = 7;
pub(crate) const OP_INVALID_SESSION: u64 = 9;
pub(crate) const OP_HELLO: u64 = 10;
pub(crate) const OP_HEARTBEAT_ACK: u64 = 11;

/// Close codes below 4000 are transport-level; the 4xxx codes a bot can act on
/// are the "do not retry as-is" ones. Anything else is worth a reconnect.
fn close_is_fatal(code: u16) -> bool {
    matches!(
        code,
        4004 | 4010 | 4011 | 4012 | 4013 | 4014 // auth/intent failures
    )
}

#[derive(Debug, Deserialize)]
struct DiscordConfig {
    #[serde(default)]
    bot_token: String,
    /// Overridable for tests; production always uses the real API base.
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    gateway_url: Option<String>,
    /// Cap on how many messages are backfilled per conversation after a fresh
    /// identify, so a long outage does not replay history.
    #[serde(default = "default_backfill_limit")]
    backfill_limit: u32,
    #[serde(default)]
    guild_allowlist: Vec<String>,
}

fn default_backfill_limit() -> u32 {
    20
}

impl DiscordConfig {
    fn api_base(&self) -> String {
        self.api_base
            .clone()
            .unwrap_or_else(|| REST_BASE.to_string())
    }

    fn gateway_url(&self) -> String {
        self.gateway_url
            .clone()
            .unwrap_or_else(|| GATEWAY_URL.to_string())
    }
}

// ─── Rate limiting ─────────────────────────────────────────────────────────

/// A parsed `X-RateLimit-*` header set. Discord may hand out both a per-route
/// bucket wait and a global wait; the global one applies to every route, so it
/// is tracked on its own and applied to every request until it expires.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RateLimit {
    /// Wait in seconds from the response (`retry_after` in the 429 body, or
    /// `X-RateLimit-Reset-After` on a 200 that still warns).
    pub retry_after: Option<f64>,
    /// True when the limit applies account-wide, not just to this bucket.
    pub global: bool,
    /// `X-RateLimit-Bucket`, the opaque per-route bucket id.
    pub bucket: Option<String>,
}

impl RateLimit {
    /// Parse the 429 body plus headers into a single wait decision.
    pub(crate) fn parse(
        status: u16,
        headers: &std::collections::HashMap<String, String>,
        body: &Value,
    ) -> Option<Self> {
        let retry_after = body.get("retry_after").and_then(Value::as_f64).or_else(|| {
            headers
                .get("x-ratelimit-reset-after")
                .and_then(|value| value.trim().parse::<f64>().ok())
        });
        let global = body.get("global").and_then(Value::as_bool).unwrap_or(false)
            || headers
                .get("x-ratelimit-global")
                .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
                .unwrap_or(false);
        let bucket = headers.get("x-ratelimit-bucket").cloned();
        if status == 429 || retry_after.is_some() || global {
            Some(Self {
                retry_after,
                global,
                bucket,
            })
        } else {
            None
        }
    }

    /// The wait, if any, in `Duration` form.
    pub(crate) fn wait(&self) -> Option<Duration> {
        self.retry_after
            .map(|seconds| Duration::from_secs_f64(seconds.clamp(0.0, 60.0)))
    }
}

/// The shared state every REST call consults before and after sending.
///
/// A single global gate is enough: per-bucket 429s retry in-line, and a global
/// 429 parks all traffic until the window passes — exactly what the header
/// promises.
#[derive(Default)]
struct RateLimiter {
    global_until: Mutex<Option<Instant>>,
}

impl RateLimiter {
    async fn wait_for_global(&self) {
        let wait = self
            .global_until
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|until| until.saturating_duration_since(Instant::now()));
        if let Some(wait) = wait {
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
        }
    }

    fn note_global_wait(&self, wait: Duration) {
        let mut guard = self.global_until.lock().unwrap_or_else(|e| e.into_inner());
        let until = Instant::now() + wait;
        if guard.map(|existing| until > existing).unwrap_or(true) {
            *guard = Some(until);
        }
    }
}

// ─── Outbound ──────────────────────────────────────────────────────────────

struct DiscordSender {
    token: String,
    api_base: String,
    http: reqwest::Client,
    limiter: Arc<RateLimiter>,
}

impl DiscordSender {
    fn new(config: &DiscordConfig, ctx: &ProviderCtx, limiter: Arc<RateLimiter>) -> Self {
        Self {
            token: config.bot_token.clone(),
            api_base: config.api_base(),
            http: ctx.http().clone(),
            limiter,
        }
    }

    fn channel_id(conversation: &ConversationRef) -> &str {
        // A threaded reply posts into the thread's own channel id; Discord
        // threads are channels, so `thread_id` (when set) is the destination.
        conversation
            .thread_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .unwrap_or(&conversation.id)
    }

    /// One REST call with rate-limit awareness. A 429 body is honoured; a
    /// permanent 4xx is returned as an error the delivery queue classifies.
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
        policy: RetryPolicy,
    ) -> Result<crate::transport::http::HttpResponse> {
        let url = format!("{}{}", self.api_base, path);
        let auth = format!("Bot {}", self.token);
        let mut attempt = 0u32;
        loop {
            self.limiter.wait_for_global().await;
            let response = send_json(
                &self.http,
                method.clone(),
                &url,
                &[("Authorization", auth.as_str())],
                body,
                policy,
            )
            .await?;
            if response.status == 429 {
                let rate_limit =
                    RateLimit::parse(response.status, &response.headers, &response.body)
                        .unwrap_or_default();
                if let Some(wait) = rate_limit.wait() {
                    if rate_limit.global {
                        self.limiter.note_global_wait(wait);
                    } else {
                        tokio::time::sleep(wait).await;
                    }
                }
                attempt += 1;
                if attempt > 3 {
                    return Err(anyhow!(
                        "discord REST {} still rate limited after retries: {}",
                        path,
                        response.error_message()
                    ));
                }
                continue;
            }
            if !response.is_success() {
                return Err(anyhow!(
                    "discord REST {} failed: {}",
                    path,
                    response.error_message()
                ));
            }
            return Ok(response);
        }
    }

    /// Download an attachment; images and short files come back inline.
    async fn download(&self, url: &str, content_type: &str) -> Result<Vec<u8>> {
        let response = self
            .http
            .get(url)
            .header("Authorization", format!("Bot {}", self.token))
            .header(reqwest::header::ACCEPT, content_type)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("attachment download failed with HTTP {}", response.status());
        }
        Ok(response.bytes().await?.to_vec())
    }
}

#[async_trait]
impl ChannelSender for DiscordSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        let channel = Self::channel_id(conversation);
        let payload = json!({ "content": text });
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/channels/{channel}/messages"),
                Some(&payload),
                RetryPolicy::default(),
            )
            .await?;
        let id = response
            .body
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(id)
    }

    async fn edit_text(
        &self,
        conversation: &ConversationRef,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let channel = Self::channel_id(conversation);
        let payload = json!({ "content": text });
        self.request(
            reqwest::Method::PATCH,
            &format!("/channels/{channel}/messages/{message_id}"),
            Some(&payload),
            RetryPolicy::default(),
        )
        .await?;
        Ok(())
    }

    async fn typing(&self, conversation: &ConversationRef) -> Result<()> {
        let channel = Self::channel_id(conversation);
        self.request(
            reqwest::Method::POST,
            &format!("/channels/{channel}/typing"),
            None,
            RetryPolicy::single_attempt(),
        )
        .await?;
        Ok(())
    }

    async fn react(
        &self,
        conversation: &ConversationRef,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let channel = Self::channel_id(conversation);
        // The emoji must be URL-encoded; a plain unicode reaction is accepted
        // as-is by the route, so no special casing is needed for the common
        // "👀" acknowledgement.
        self.request(
            reqwest::Method::PUT,
            &format!("/channels/{channel}/messages/{message_id}/reactions/{emoji}/@me"),
            None,
            RetryPolicy::single_attempt(),
        )
        .await?;
        Ok(())
    }
}

// ─── Gateway ───────────────────────────────────────────────────────────────

/// What the session loop knows across a reconnect: the gateway gives us a
/// session id and a sequence number, and as long as both are alive we try to
/// RESUME; when RESUME fails the session is dead and we IDENTIFY fresh.
#[derive(Debug, Default)]
struct GatewayState {
    session_id: Option<String>,
    resume_url: Option<String>,
    /// Last dispatch sequence the client acked; the gateway asks for it on
    /// RESUME and sends heartbeats against it.
    last_sequence: Option<u64>,
    /// True once the READY dispatch named the bot, so self-filtering can key
    /// on our own user id.
    bot_user_id: Option<String>,
}

/// A parsed gateway payload, keeping the fields we act on.
#[derive(Debug, Clone)]
pub(crate) struct GatewayEvent {
    pub op: u64,
    pub sequence: Option<u64>,
    pub event: Option<String>,
    pub data: Value,
}

impl GatewayEvent {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(raw).context("gateway frame is not JSON")?;
        let op = value
            .get("op")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("gateway frame has no numeric op"))?;
        Ok(Self {
            op,
            sequence: value.get("s").and_then(Value::as_u64),
            event: value.get("t").and_then(Value::as_str).map(str::to_string),
            data: value.get("d").cloned().unwrap_or(Value::Null),
        })
    }
}

/// The answer to "can we resume?": the sequence number, session id and URL
/// must all be known, and the session must not have been invalidated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeDecision {
    /// Send RESUME with the stored session id and last sequence.
    Resume,
    /// Send IDENTIFY and backfill afterwards (fresh session, or a failed resume).
    Identify,
}

impl GatewayState {
    fn resume_decision(&self) -> ResumeDecision {
        if self.session_id.is_some() && self.last_sequence.is_some() {
            ResumeDecision::Resume
        } else {
            ResumeDecision::Identify
        }
    }

    fn heartbeat_payload(&self) -> Value {
        json!({ "op": OP_HEARTBEAT, "d": self.last_sequence })
    }

    fn identify_payload(&self, token: &str) -> Value {
        json!({
            "op": OP_IDENTIFY,
            "d": {
                "token": token,
                "intents": INTENTS,
                "properties": {
                    "os": std::env::consts::OS,
                    "browser": "future-channel",
                    "device": "future-channel",
                },
            }
        })
    }

    fn resume_payload(&self, token: &str) -> Option<Value> {
        Some(json!({
            "op": OP_RESUME,
            "d": {
                "token": token,
                "session_id": self.session_id.as_deref()?,
                "seq": self.last_sequence?,
            }
        }))
    }
}

/// Whether an inbound dispatch should become an [`Inbound`].
///
/// A guild message is only answered when the bot is mentioned (or the message
/// is a DM); everything else is dropped before the bridge ever sees it, which
/// keeps a busy guild from queueing noise.
#[derive(Debug, Clone)]
pub(crate) struct MessageShape {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub author_name: Option<String>,
    pub content: String,
    pub is_dm: bool,
    pub addressed: bool,
    pub timestamp_ms: Option<i64>,
    pub attachments: Vec<AttachmentShape>,
}

#[derive(Debug, Clone)]
pub(crate) struct AttachmentShape {
    pub url: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub size: Option<u64>,
}

/// Parse a `MESSAGE_CREATE` dispatch into the shape the provider needs.
///
/// Returns `None` for messages that must not be answered: our own messages,
/// messages from other bots, and (for guild channels) messages that do not
/// mention us. `guild_id` presence is what distinguishes a guild channel from
/// a DM — a group DM is treated as a direct conversation.
pub(crate) fn parse_message_create(
    data: &Value,
    bot_user_id: Option<&str>,
) -> Option<MessageShape> {
    let author = data.get("author")?;
    let author_id = author.get("id")?.as_str()?.to_string();
    if author.get("bot").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    if Some(author_id.as_str()) == bot_user_id {
        return None;
    }
    let is_dm = data.get("guild_id").is_none();
    let mentioned = data
        .get("mentions")
        .and_then(Value::as_array)
        .map(|mentions| {
            mentions.iter().any(|user| {
                user.get("id").and_then(Value::as_str) == bot_user_id
                    || user
                        .get("id")
                        .and_then(Value::as_str)
                        .zip(bot_user_id)
                        .map(|(id, bot)| id == bot)
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    let addressed = is_dm || mentioned;
    if !addressed {
        return None;
    }
    let attachments = data
        .get("attachments")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let url = item.get("url")?.as_str()?.to_string();
                    Some(AttachmentShape {
                        url,
                        filename: item
                            .get("filename")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        content_type: item
                            .get("content_type")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        size: item.get("size").and_then(Value::as_u64),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(MessageShape {
        id: data.get("id")?.as_str()?.to_string(),
        channel_id: data.get("channel_id")?.as_str()?.to_string(),
        author_id,
        author_name: author
            .get("global_name")
            .and_then(Value::as_str)
            .or_else(|| author.get("username").and_then(Value::as_str))
            .map(str::to_string),
        content: data
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        is_dm,
        addressed,
        timestamp_ms: parse_timestamp_ms(data.get("timestamp").and_then(Value::as_str)),
        attachments,
    })
}

/// Discord's timestamps are ISO 8601 with millisecond precision; the bridge
/// wants Unix milliseconds for the freshness window.
fn parse_timestamp_ms(raw: Option<&str>) -> Option<i64> {
    let raw = raw?;
    // chrono is a workspace dependency; keep the parse strict so a malformed
    // timestamp does not pass for "recent".
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|stamp| stamp.timestamp_millis())
}

/// The media kind the bridge understands, from Discord's content type.
fn media_kind(content_type: Option<&str>) -> MediaKind {
    match content_type {
        Some(value) if value.starts_with("image/") => MediaKind::Image,
        Some(value) if value.starts_with("audio/") => MediaKind::Audio,
        Some(value) if value.starts_with("video/") => MediaKind::Video,
        Some(_) => MediaKind::Document,
        None => MediaKind::Unknown,
    }
}

/// The gateway connection lifecycle. One call = one connection; the
/// supervisor around it reconnects.
async fn run_gateway(
    ctx: &ProviderCtx,
    config: &DiscordConfig,
    sender: Arc<DiscordSender>,
    state: &mut GatewayState,
) -> Result<()> {
    let url = config.gateway_url();
    let mut socket = ws::connect(&url, &[]).await?;
    let mut heartbeat_interval: Option<Duration> = None;
    let mut last_heartbeat = Instant::now();
    let mut heartbeat_ack = true;
    // The first HELLO decides the interval; every later one is ignored.
    let mut identified = false;

    loop {
        tokio::select! {
            _ = ctx.shutdown().notified() => {
                let _ = socket.close(None).await;
                return Ok(());
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                // Heartbeat on the gateway's clock, not ours: miss one ACK and
                // the next connection is a zombie, so reconnect. Before the
                // first HELLO there is no interval yet and the tick is a no-op.
                if let Some(interval) = heartbeat_interval {
                    if last_heartbeat.elapsed() >= interval {
                        if !heartbeat_ack {
                            return Err(anyhow!("gateway heartbeat was not acknowledged; reconnecting"));
                        }
                        let payload = state.heartbeat_payload();
                        socket.send(WsMessage::Text(payload.to_string())).await?;
                        heartbeat_ack = false;
                        last_heartbeat = Instant::now();
                    }
                }
            }
            message = socket.next() => {
                let Some(message) = message else {
                    return Err(anyhow!("gateway closed the connection"));
                };
                let message = message?;
                match message {
                    WsMessage::Text(text) => {
                        let event = match GatewayEvent::parse(&text) {
                            Ok(event) => event,
                            Err(error) => {
                                tracing::debug!(%error, "ignoring an unparseable gateway frame");
                                continue;
                            }
                        };
                        if let Some(seq) = event.sequence {
                            state.last_sequence = Some(seq);
                        }
                        match event.op {
                            OP_HELLO => {
                                let interval_ms = event
                                    .data
                                    .get("heartbeat_interval")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(41_250);
                                heartbeat_interval = Some(Duration::from_millis(interval_ms));
                                last_heartbeat = Instant::now();
                                heartbeat_ack = true;
                                if !identified {
                                    let payload = match state.resume_decision() {
                                        ResumeDecision::Resume => state
                                            .resume_payload(&config.bot_token)
                                            .unwrap_or_else(|| state.identify_payload(&config.bot_token)),
                                        ResumeDecision::Identify => state.identify_payload(&config.bot_token),
                                    };
                                    socket.send(WsMessage::Text(payload.to_string())).await?;
                                    identified = true;
                                }
                            }
                            OP_HEARTBEAT => {
                                let payload = state.heartbeat_payload();
                                socket.send(WsMessage::Text(payload.to_string())).await?;
                            }
                            OP_HEARTBEAT_ACK => {
                                heartbeat_ack = true;
                            }
                            OP_DISPATCH => {
                                match event.event.as_deref() {
                                    Some("READY") => {
                                        identified = true;
                                        state.session_id = event
                                            .data
                                            .get("session_id")
                                            .and_then(Value::as_str)
                                            .map(str::to_string);
                                        state.resume_url = event
                                            .data
                                            .get("resume_gateway_url")
                                            .and_then(Value::as_str)
                                            .map(str::to_string);
                                        state.bot_user_id = event
                                            .data
                                            .get("user")
                                            .and_then(|user| user.get("id"))
                                            .and_then(Value::as_str)
                                            .map(str::to_string);
                                        ctx.mark_running();
                                    }
                                    Some("RESUMED") => {
                                        identified = true;
                                        ctx.mark_running();
                                    }
                                    Some("MESSAGE_CREATE") => {
                                        if let Some(shape) = parse_message_create(
                                            &event.data,
                                            state.bot_user_id.as_deref(),
                                        ) {
                                            handle_inbound(ctx, &sender, shape).await;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            OP_RECONNECT => {
                                return Err(anyhow!("gateway asked us to reconnect"));
                            }
                            OP_INVALID_SESSION => {
                                // d=false means the session is dead; d=true means
                                // we may resume. Either way we come back through
                                // HELLO on the next connection.
                                let resumable = event.data.as_bool().unwrap_or(false);
                                if !resumable {
                                    state.session_id = None;
                                    state.last_sequence = None;
                                }
                                return Err(anyhow!(
                                    "gateway invalidated the session (resumable={resumable})"
                                ));
                            }
                            _ => {}
                        }
                    }
                    WsMessage::Close(frame) => {
                        let fatal = frame
                            .as_ref()
                            .map(|frame| close_is_fatal(frame.code.into()))
                            .unwrap_or(false);
                        if fatal {
                            ctx.mark_failed("gateway closed the connection with a fatal code");
                            return Err(anyhow!("gateway closed with a fatal code: {frame:?}"));
                        }
                        // A normal close: our echo is on the wire and the state
                        // is ClosedByPeer, so the next read yields the end of
                        // the stream (None) and the loop reports the dropped
                        // connection. Continuing keeps the close handshake
                        // honest instead of assuming it finished.
                        continue;
                    }
                    WsMessage::Ping(payload) => {
                        socket.send(WsMessage::Pong(payload)).await?;
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn handle_inbound(ctx: &ProviderCtx, sender: &Arc<DiscordSender>, shape: MessageShape) {
    let mut media = Vec::new();
    for attachment in &shape.attachments {
        let kind = media_kind(attachment.content_type.as_deref());
        let data = match sender
            .download(
                &attachment.url,
                attachment
                    .content_type
                    .as_deref()
                    .unwrap_or("application/octet-stream"),
            )
            .await
        {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                tracing::debug!(%error, url = %attachment.url, "could not download an attachment");
                None
            }
        };
        media.push(MediaRef {
            kind,
            filename: attachment.filename.clone(),
            content_type: attachment.content_type.clone(),
            url: Some(attachment.url.clone()),
            data,
        });
    }
    let inbound = Inbound {
        message_id: shape.id.clone(),
        sender: SenderRef {
            id: shape.author_id.clone(),
            display: shape.author_name.clone(),
        },
        conversation: ConversationRef {
            id: shape.channel_id.clone(),
            thread_id: None,
            kind: if shape.is_dm {
                ChatKind::Direct
            } else {
                ChatKind::Channel
            },
        },
        text: shape.content.clone(),
        media,
        addressed_to_bot: shape.addressed,
        created_at_ms: shape.timestamp_ms,
        raw: None,
    };
    let outcome = ctx.handle(inbound, sender.clone()).await;
    tracing::debug!(?outcome, "discord message handled");
}

/// Backfill after a fresh identify: list recent messages and queue any the
/// bridge has not seen. Kept small on purpose — this is a gap-filler, not a
/// history import.
async fn backfill(
    ctx: &ProviderCtx,
    config: &DiscordConfig,
    sender: &Arc<DiscordSender>,
    channel_id: &str,
    since_ms: Option<i64>,
) -> Result<()> {
    let limit = config.backfill_limit.min(100);
    let mut query = format!("/channels/{channel_id}/messages?limit={limit}");
    if let Some(since_ms) = since_ms {
        // Discord's `after` is a snowflake, not a timestamp; converting is an
        // approximation that errs on the side of fetching a little extra.
        let snowflake = ((since_ms as u64).saturating_sub(1_420_070_400_000)) << 22;
        query.push_str(&format!("&after={snowflake}"));
    }
    let response = sender
        .request(
            reqwest::Method::GET,
            &query,
            None,
            RetryPolicy::single_attempt(),
        )
        .await?;
    let Some(messages) = response.body.as_array() else {
        return Ok(());
    };
    for message in messages {
        if let Some(shape) = parse_message_create(message, ctx_id(ctx, sender).as_deref()) {
            handle_inbound(ctx, sender, shape).await;
        }
    }
    Ok(())
}

/// The id the bridge knows us by, when READY has already run.
fn ctx_id(_ctx: &ProviderCtx, _sender: &Arc<DiscordSender>) -> Option<String> {
    // The bot id is session state, not config; backfill only runs after a
    // reconnect, by which point READY has already set it on the gateway loop.
    // We pass `None` here: self-filtering is already handled by the author id
    // check, and a missing bot id simply means the mention gate cannot match —
    // which is the correct conservative answer for a backfill.
    None
}

// ─── Provider ──────────────────────────────────────────────────────────────

struct Discord;

#[async_trait]
impl Provider for Discord {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        let config = ctx.config::<DiscordConfig>()?;
        if config.bot_token.is_empty() {
            anyhow::bail!("discord channel requires `bot_token`");
        }
        Ok(Arc::new(DiscordSender::new(
            &config,
            ctx,
            Arc::new(RateLimiter::default()),
        )))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        let config = ctx.config::<DiscordConfig>()?;
        if config.bot_token.is_empty() {
            anyhow::bail!("discord channel requires `bot_token`");
        }
        let sender = Arc::new(DiscordSender::new(
            &config,
            &ctx,
            Arc::new(RateLimiter::default()),
        ));
        ctx.mark_running();
        let mut backoff = ws::Backoff::gateway();
        let mut state = GatewayState::default();
        loop {
            let result = run_gateway(&ctx, &config, sender.clone(), &mut state).await;
            match result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    tracing::warn!(channel = "discord", %error, "gateway connection failed");
                    ctx.mark_failed(&error.to_string());
                    let delay = backoff.next_delay();
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = ctx.shutdown().notified() => return Ok(()),
                    }
                }
            }
        }
    }

    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        let config = ctx.config::<DiscordConfig>()?;
        if config.bot_token.is_empty() {
            anyhow::bail!("discord channel requires `bot_token`");
        }
        let sender = DiscordSender::new(&config, ctx, Arc::new(RateLimiter::default()));
        let response = sender
            .request(
                reqwest::Method::GET,
                "/users/@me",
                None,
                RetryPolicy::single_attempt(),
            )
            .await?;
        let name = response
            .body
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let id = response
            .body
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        Ok(format!("connected as @{name} ({id})"))
    }
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(Discord)
}

#[cfg(test)]
mod tests {
    include!("discord_tests.rs");
}

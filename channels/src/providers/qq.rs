//! QQ: the official bot platform, REST plus a websocket gateway.
//!
//! * **Auth** — an app access token from `POST /app/getAppAccessToken`
//!   (`appId` + `clientSecret`). The response carries a lifetime in seconds,
//!   and tokens are single-use-the-latest (requesting a new one can
//!   invalidate the old one), so the token is shared behind one lock and
//!   refreshed with a margin before it expires.
//! * **Inbound** — the gateway handshake: `op 10` hello carries the
//!   heartbeat interval (never a constant), `op 2` identify carries the
//!   intents and token, `op 1` heartbeat echoes the last sequence number,
//!   `op 11` acks it, and `op 0` dispatch carries `C2C_MESSAGE_CREATE`
//!   (direct messages) and `GROUP_AT_MESSAGE_CREATE` (group messages that
//!   mention the bot). `op 7` and `op 9` mean re-identify, not
//!   backoff-and-hope.
//! * **Outbound** — `POST /v2/users/{openid}/messages` and
//!   `POST /v2/groups/{group_openid}/messages`, always replying with the
//!   originating `msg_id` and a per-message `msg_seq`, so a multi-chunk
//!   answer stays one reply instead of flooding the chat. The originating
//!   message id rides in `conversation.thread_id`, and the sender keeps one
//!   `msg_seq` counter per conversation.
//! * **Errors** — `401` is permanent (bad credentials), and the platform's
//!   own error codes about unknown users/groups or message-length limits are
//!   folded into the permanent vocabulary so the delivery queue stops
//!   retrying a target that will never work.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::bridge::{ChatKind, ConversationRef, Inbound, ProviderCtx, SenderRef};
use crate::providers::traits::{
    Capabilities, ChannelDefinition, ChannelSender, Maturity, Provider,
};
use crate::transport::http::{send_json, RetryPolicy};
use crate::transport::{ws, LengthUnit};

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "qq",
    display_name: "QQ",
    description: "QQ bot (open platform v2): gateway events, C2C and group replies.",
    docs: "docs/guide/channels-providers.md#qq",
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
    max_text_len: 4000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "app_id": "",
  "app_secret": "",
  "sandbox": false,
  "group_allowlist": [],
  "require_mention": true
}"#,
    requires: &["a QQ open-platform bot"],
};

const API_BASE: &str = "https://api.sgroup.qq.com";
const SANDBOX_API_BASE: &str = "https://sandbox.api.sgroup.qq.com";

/// The platform does not publish a fixed gateway URL: `GET /gateway` answers
/// the websocket endpoint for the current token.
const GATEWAY_DISCOVERY_PATH: &str = "/gateway";
const ACCESS_TOKEN_PATH: &str = "/app/getAppAccessToken";

/// The interval a HELLO falls back to when the platform omits one. The
/// handshake usually sets this, but a defensive default keeps a malformed
/// frame from disabling heartbeats entirely.
const DEFAULT_HEARTBEAT_INTERVAL_MS: u64 = 30_000;

// Gateway opcodes used by this provider.
pub(crate) const OP_DISPATCH: u64 = 0;
pub(crate) const OP_HEARTBEAT: u64 = 1;
pub(crate) const OP_IDENTIFY: u64 = 2;
pub(crate) const OP_RECONNECT: u64 = 7;
pub(crate) const OP_INVALID_SESSION: u64 = 9;
pub(crate) const OP_HELLO: u64 = 10;
pub(crate) const OP_HEARTBEAT_ACK: u64 = 11;

/// Dispatch event names the provider answers.
const EVENT_C2C_MESSAGE_CREATE: &str = "C2C_MESSAGE_CREATE";
const EVENT_GROUP_AT_MESSAGE_CREATE: &str = "GROUP_AT_MESSAGE_CREATE";
const EVENT_READY: &str = "READY";

/// Conversation id prefixes: `c2c:` for direct messages, `group:` for a
/// group chat. The sender routes on the prefix, and the bridge keys sessions
/// on the whole string.
const CONVERSATION_C2C_PREFIX: &str = "c2c:";
const CONVERSATION_GROUP_PREFIX: &str = "group:";

/// Gateway intents this provider subscribes to, as named bit flags. The
/// platform's intent table assigns one bit per event family; subscribing to
/// both lets one connection serve direct and group messages.
pub(crate) const INTENT_C2C_MESSAGES: u64 = 1 << 0;
pub(crate) const INTENT_GROUP_AT_MESSAGES: u64 = 1 << 1;
pub(crate) const INTENTS: u64 = INTENT_C2C_MESSAGES | INTENT_GROUP_AT_MESSAGES;

/// Whether a gateway close code means "re-identify" or "this token/intent
/// set is wrong". The platform's 4xxx codes are policy decisions a fresh
/// identify cannot fix, so those fail the provider instead of looping.
pub(crate) fn close_requires_reidentify(code: u16) -> bool {
    // 4000-4999 with a handful of "do not retry as-is" exceptions.
    (4000..5000).contains(&code) && !matches!(code, 4004 | 4910 | 4913)
}

/// One channel's config block in `~/.future/channels/config.json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct QqConfig {
    pub enabled: bool,
    /// The bot's app id, from the open-platform console.
    pub app_id: String,
    /// The bot's client secret, paired with `app_id` on the token endpoint.
    pub app_secret: String,
    /// True to point at the sandbox environment (staging tokens and events).
    pub sandbox: bool,
    /// Groups the bot answers in; empty means every group it can see.
    pub group_allowlist: Vec<String>,
    /// Group messages arriving on this gateway are already `@`-filtered by
    /// the platform; the flag stays so the bridge policy reads the same key
    /// it reads for every channel.
    pub require_mention: bool,
    /// Explicit API base (tests); derived from `sandbox` when empty.
    pub api_base: String,
    /// Explicit websocket URL (tests); discovered via `GET /gateway` when
    /// empty.
    pub gateway_url: String,
}

impl Default for QqConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            app_id: String::new(),
            app_secret: String::new(),
            sandbox: false,
            group_allowlist: Vec::new(),
            require_mention: true,
            api_base: String::new(),
            gateway_url: String::new(),
        }
    }
}

impl QqConfig {
    fn api_base(&self) -> String {
        if !self.api_base.is_empty() {
            return self.api_base.trim_end_matches('/').to_string();
        }
        if self.sandbox {
            SANDBOX_API_BASE.to_string()
        } else {
            API_BASE.to_string()
        }
    }
}

/// The seconds a token response claims, whether it arrived as a JSON number
/// or (as the platform sometimes sends) a string.
pub(crate) fn parse_expires_in(body: &Value) -> Option<u64> {
    body.get("expires_in").and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str()?.trim().parse().ok())
    })
}

/// The refresh moment for a token with `expires_in` seconds of life: a
/// minute before expiry, but never in the past.
pub(crate) fn refresh_margin(expires_in: u64) -> Duration {
    Duration::from_secs(expires_in.saturating_sub(60).max(1))
}

/// The shared access token: one per bot, refreshed in one place so
/// concurrent sends do not stampede the token endpoint.
#[derive(Default)]
struct TokenState {
    cache: Mutex<Option<TokenCache>>,
    refresh_lock: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone)]
struct TokenCache {
    value: String,
    /// Refresh strictly before this instant.
    refresh_after: Instant,
}

impl TokenState {
    fn current(&self) -> Option<TokenCache> {
        let guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        guard.clone()
    }

    /// The current token, fetching a fresh one when expired or missing.
    async fn get(&self, client: &reqwest::Client, config: &QqConfig) -> Result<String> {
        if let Some(cache) = self.current() {
            if Instant::now() < cache.refresh_after {
                return Ok(cache.value);
            }
        }
        self.refresh(client, config).await
    }

    /// Fetch a new token, serialized on a `tokio` mutex so two expiring
    /// reads do not race the single-use-the-latest endpoint. (The cache
    /// itself sits behind a `std` mutex that is never held across an await.)
    async fn refresh(&self, client: &reqwest::Client, config: &QqConfig) -> Result<String> {
        let _permit = self.refresh_lock.lock().await;
        if let Some(cache) = self.current() {
            if Instant::now() < cache.refresh_after {
                return Ok(cache.value);
            }
        }
        let url = format!("{}{}", config.api_base(), ACCESS_TOKEN_PATH);
        let body = json!({
            "appId": config.app_id,
            "clientSecret": config.app_secret,
        });
        let response = send_json(
            client,
            reqwest::Method::POST,
            &url,
            &[],
            Some(&body),
            RetryPolicy::single_attempt(),
        )
        .await?;
        if !response.is_success() {
            anyhow::bail!(
                "QQ access-token request failed: {}",
                response.error_message()
            );
        }
        let token = response
            .body
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| anyhow!("QQ access-token response has no access_token"))?
            .to_string();
        let expires_in = parse_expires_in(&response.body)
            .ok_or_else(|| anyhow!("QQ access-token response has no expires_in"))?;
        let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(TokenCache {
            value: token.clone(),
            refresh_after: Instant::now() + refresh_margin(expires_in),
        });
        Ok(token)
    }
}

/// The REST verbs against the open-platform API.
pub(crate) struct QqApi {
    base: String,
    http: reqwest::Client,
    token: Arc<TokenState>,
}

impl QqApi {
    fn new(config: &QqConfig, ctx: &ProviderCtx) -> Self {
        Self {
            base: config.api_base(),
            http: ctx.http().clone(),
            token: Arc::new(TokenState::default()),
        }
    }

    async fn auth_header(&self, config: &QqConfig) -> Result<String> {
        Ok(format!(
            "QQBot {}",
            self.token.get(&self.http, config).await?
        ))
    }

    /// Discover the gateway URL for the current token.
    async fn gateway_url(&self, config: &QqConfig) -> Result<String> {
        let url = format!("{}{}", self.base, GATEWAY_DISCOVERY_PATH);
        let auth = self.auth_header(config).await?;
        let response = send_json(
            &self.http,
            reqwest::Method::GET,
            &url,
            &[("Authorization", auth.as_str())],
            None,
            RetryPolicy::single_attempt(),
        )
        .await?;
        if !response.is_success() {
            anyhow::bail!("QQ gateway discovery failed: {}", response.error_message());
        }
        response
            .body
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("QQ gateway discovery response has no url"))
    }

    /// Send one message into a conversation.
    ///
    /// `msg_id` is the message being replied to (the platform threads a reply
    /// on it); `msg_seq` orders the chunks of a multi-part answer. A send
    /// without `msg_id` posts a plain message, which is what proactive
    /// delivery does.
    async fn send_message(
        &self,
        config: &QqConfig,
        conversation: &ConversationRef,
        text: &str,
        msg_id: Option<&str>,
        msg_seq: u64,
    ) -> Result<crate::transport::http::HttpResponse> {
        let path = match conversation.id.strip_prefix(CONVERSATION_GROUP_PREFIX) {
            Some(openid) => format!("/v2/groups/{openid}/messages"),
            None => {
                let openid = conversation
                    .id
                    .strip_prefix(CONVERSATION_C2C_PREFIX)
                    .unwrap_or(&conversation.id);
                format!("/v2/users/{openid}/messages")
            }
        };
        let url = format!("{}{}", self.base, path);
        let auth = self.auth_header(config).await?;
        let mut body = json!({
            "content": text,
            "msg_type": 0,
        });
        if let Some(id) = msg_id {
            body["msg_id"] = json!(id);
            body["msg_seq"] = json!(msg_seq);
        }
        send_json(
            &self.http,
            reqwest::Method::POST,
            &url,
            &[("Authorization", auth.as_str())],
            Some(&body),
            RetryPolicy::default(),
        )
        .await
    }
}

/// Whether an HTTP status from this platform is worth retrying.
///
/// The shared classifier already handles the generic codes; the platform
/// adds one of its own: a 404 on a send path means the user or group openid
/// no longer resolves, which is a permanent "no such user" — so the delivery
/// queue must not retry it forever.
fn send_error_is_permanent(status: u16, message: &str) -> bool {
    if crate::transport::http::classify_status(status)
        == crate::transport::http::ErrorClass::Permanent
    {
        return true;
    }
    let lowered = message.to_ascii_lowercase();
    [
        "openid",
        "msg_id",
        "msg_seq",
        "message length",
        "message too long",
    ]
    .iter()
    .any(|pattern| lowered.contains(pattern))
}

/// The outbound half of the channel.
pub(crate) struct QqSender {
    config: QqConfig,
    api: QqApi,
    /// One `msg_seq` counter per conversation: the platform orders a
    /// multi-chunk reply on it, and a shared counter would interleave two
    /// conversations' chunks.
    sequences: Mutex<HashMap<String, u64>>,
}

impl QqSender {
    fn new(config: &QqConfig, ctx: &ProviderCtx) -> Self {
        Self {
            config: config.clone(),
            api: QqApi::new(config, ctx),
            sequences: Mutex::new(HashMap::new()),
        }
    }

    /// The next `msg_seq` for a conversation.
    fn next_seq(&self, conversation: &ConversationRef) -> u64 {
        let mut guard = self.sequences.lock().unwrap_or_else(|e| e.into_inner());
        let seq = guard.entry(conversation.id.clone()).or_insert(0);
        *seq += 1;
        *seq
    }
}

#[async_trait]
impl ChannelSender for QqSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        // The originating message id rides in thread_id: the bridge passes
        // the conversation through untouched, and this platform is the one
        // that needs the replied-to message named on every chunk.
        let msg_id = conversation
            .thread_id
            .as_deref()
            .filter(|id| !id.is_empty());
        let msg_seq = self.next_seq(conversation);
        let response = self
            .api
            .send_message(&self.config, conversation, text, msg_id, msg_seq)
            .await?;
        if !response.is_success() {
            let message = response.error_message();
            if send_error_is_permanent(response.status, &message) {
                anyhow::bail!("QQ send failed permanently: {message}");
            }
            anyhow::bail!("QQ send failed: {message}");
        }
        Ok(response
            .body
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string))
    }
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

/// Whether the session state calls for a fresh identify: `op 7` (reconnect)
/// always does, `op 9` (invalid session) does only when the platform says
/// the session cannot be resumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReconnectAction {
    /// Open a new connection and identify fresh.
    Reidentify,
    /// The platform allows resuming; this build identifies fresh anyway, but
    /// the distinction matters for logging.
    ResumePossible,
}

pub(crate) fn invalid_session_action(resumable: bool) -> ReconnectAction {
    if resumable {
        ReconnectAction::ResumePossible
    } else {
        ReconnectAction::Reidentify
    }
}

/// An inbound dispatch reduced to the shape the provider needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MessageShape {
    pub id: String,
    pub conversation_id: String,
    pub author_id: String,
    pub content: String,
    pub is_group: bool,
    pub timestamp_ms: Option<i64>,
}

/// Parse a `C2C_MESSAGE_CREATE` or `GROUP_AT_MESSAGE_CREATE` dispatch.
///
/// Returns `None` for shapes that must not be answered (missing ids, empty
/// content). Group dispatches are already `@`-filtered by the platform, so
/// `addressed_to_bot` is always true here — the policy layer still decides.
pub(crate) fn parse_message_create(data: &Value, is_group: bool) -> Option<MessageShape> {
    let id = data.get("id")?.as_str()?.to_string();
    let author = data.get("author")?;
    let (author_id, conversation_id) = if is_group {
        let author_id = author.get("member_openid")?.as_str()?.to_string();
        let openid = data.get("group_openid")?.as_str()?;
        (author_id, format!("{CONVERSATION_GROUP_PREFIX}{openid}"))
    } else {
        let author_id = author.get("user_openid")?.as_str()?.to_string();
        (
            author_id.clone(),
            format!("{CONVERSATION_C2C_PREFIX}{author_id}"),
        )
    };
    let content = data
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if content.is_empty() {
        return None;
    }
    let timestamp_ms = data
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp_ms);
    Some(MessageShape {
        id,
        conversation_id,
        author_id,
        content,
        is_group,
        timestamp_ms,
    })
}

/// The platform timestamps dispatches as RFC 3339; the bridge wants Unix
/// milliseconds for the freshness window.
fn parse_timestamp_ms(raw: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|stamp| stamp.timestamp_millis())
}

impl MessageShape {
    fn into_inbound(self, raw: Value) -> Inbound {
        let kind = if self.is_group {
            ChatKind::Group
        } else {
            ChatKind::Direct
        };
        Inbound {
            message_id: self.id.clone(),
            sender: SenderRef {
                id: self.author_id,
                display: None,
            },
            conversation: ConversationRef {
                id: self.conversation_id,
                // The replied-to message id, for the sender's msg_id.
                thread_id: Some(self.id),
                kind,
            },
            text: self.content,
            media: Vec::new(),
            addressed_to_bot: true,
            created_at_ms: self.timestamp_ms,
            raw: Some(raw),
        }
    }
}

/// The gateway connection lifecycle. One call = one connection; the
/// supervisor around it reconnects.
async fn run_gateway(
    ctx: &ProviderCtx,
    config: &QqConfig,
    api: &QqApi,
    sender: Arc<dyn ChannelSender>,
) -> Result<()> {
    let url = if !config.gateway_url.is_empty() {
        config.gateway_url.clone()
    } else {
        api.gateway_url(config).await?
    };
    let mut socket = ws::connect(&url, &[]).await?;
    let mut heartbeat_interval: Option<Duration> = None;
    let mut last_heartbeat = Instant::now();
    let mut heartbeat_ack = true;
    let mut identified = false;
    let mut last_sequence: Option<u64> = None;

    loop {
        tokio::select! {
            _ = ctx.shutdown().notified() => {
                let _ = socket.close(None).await;
                return Ok(());
            }
            _ = tokio::time::sleep(Duration::from_millis(50)), if heartbeat_interval.is_some() => {
                // Heartbeat on the gateway's clock, not ours: miss one ACK and
                // the connection is a zombie, so reconnect.
                if let Some(interval) = heartbeat_interval {
                    let due = last_heartbeat.elapsed() >= interval;
                    if due && !heartbeat_ack {
                        return Err(anyhow!("gateway heartbeat was not acknowledged; reconnecting"));
                    }
                    if due {
                        let payload = json!({ "op": OP_HEARTBEAT, "d": last_sequence });
                        socket.send(WsMessage::Text(payload.to_string())).await?;
                        heartbeat_ack = false;
                        last_heartbeat = Instant::now();
                    }
                }
            }
            message = socket.next() => {
                let message = message
                    .ok_or_else(|| anyhow!("gateway closed the connection"))?;
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
                            last_sequence = Some(seq);
                        }
                        match event.op {
                            OP_HELLO => {
                                let interval_ms = event
                                    .data
                                    .get("heartbeat_interval")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(DEFAULT_HEARTBEAT_INTERVAL_MS);
                                heartbeat_interval = Some(Duration::from_millis(interval_ms));
                                last_heartbeat = Instant::now();
                                heartbeat_ack = true;
                                if !identified {
                                    let token = api.token.get(&api.http, config).await?;
                                    let payload = json!({
                                        "op": OP_IDENTIFY,
                                        "d": {
                                            "token": token,
                                            "intents": INTENTS,
                                            "shard": [0, 1],
                                            "properties": {
                                                "$os": std::env::consts::OS,
                                                "$browser": "future-channel",
                                                "$device": "future-channel",
                                            },
                                        }
                                    });
                                    let text = payload.to_string();
                                    socket.send(WsMessage::Text(text)).await?;
                                    identified = true;
                                }
                            }
                            OP_HEARTBEAT => {
                                let payload = json!({ "op": OP_HEARTBEAT, "d": last_sequence });
                                let text = payload.to_string();
                                socket.send(WsMessage::Text(text)).await?;
                            }
                            OP_HEARTBEAT_ACK => {
                                heartbeat_ack = true;
                            }
                            OP_DISPATCH => match event.event.as_deref() {
                                Some(EVENT_C2C_MESSAGE_CREATE) => {
                                    let shape = parse_message_create(&event.data, false);
                                    if let Some(shape) = shape {
                                        let inbound = shape.into_inbound(event.data.clone());
                                        let outcome =
                                            ctx.handle(inbound, sender.clone()).await;
                                        tracing::debug!(?outcome, "qq direct message handled");
                                    }
                                }
                                Some(EVENT_GROUP_AT_MESSAGE_CREATE) => {
                                    let shape = parse_message_create(&event.data, true);
                                    if let Some(shape) = shape {
                                        let inbound = shape.into_inbound(event.data.clone());
                                        let outcome =
                                            ctx.handle(inbound, sender.clone()).await;
                                        tracing::debug!(?outcome, "qq group message handled");
                                    }
                                }
                                Some(EVENT_READY) => {
                                    ctx.mark_running();
                                }
                                _ => {}
                            },
                            OP_RECONNECT => {
                                return Err(anyhow!("gateway asked for a reconnect; re-identify"));
                            }
                            OP_INVALID_SESSION => {
                                let resumable = event.data.as_bool().unwrap_or(false);
                                match invalid_session_action(resumable) {
                                    ReconnectAction::ResumePossible => {
                                        tracing::info!("gateway invalidated a resumable session; re-identifying");
                                    }
                                    ReconnectAction::Reidentify => {
                                        tracing::warn!("gateway invalidated the session; re-identifying");
                                    }
                                }
                                return Err(anyhow!("gateway invalidated the session; re-identify"));
                            }
                            _ => {}
                        }
                    }
                    WsMessage::Close(frame) => {
                        let code = frame.map(|f| f.code.into()).unwrap_or(1005);
                        if close_requires_reidentify(code) {
                            return Err(anyhow!("gateway closed with code {code}; re-identify"));
                        }
                        return Err(anyhow!("gateway closed with code {code}"));
                    }
                    _ => {}
                }
            }
        }
    }
}

/// The provider itself: config validation, sender construction, and the
/// gateway supervisor loop.
pub struct Qq;

impl Qq {
    fn validated_config(ctx: &ProviderCtx) -> Result<QqConfig> {
        let config: QqConfig = ctx.config()?;
        if config.app_id.is_empty() || config.app_secret.is_empty() {
            anyhow::bail!(
                "the qq channel needs app_id and app_secret in ~/.future/channels/config.json"
            );
        }
        Ok(config)
    }
}

#[async_trait]
impl Provider for Qq {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        let config = Self::validated_config(ctx)?;
        Ok(Arc::new(QqSender::new(&config, ctx)))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        let config = Self::validated_config(&ctx)?;
        let sender = self.sender(&ctx)?;
        let api = QqApi::new(&config, &ctx);
        ctx.mark_running();
        ws::supervise("qq", ctx.shutdown(), ws::Backoff::gateway(), || {
            run_gateway(&ctx, &config, &api, sender.clone())
        })
        .await
    }

    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        let config = Self::validated_config(ctx)?;
        let api = QqApi::new(&config, ctx);
        let url = api.gateway_url(&config).await?;
        Ok(format!("gateway discovered: {url}"))
    }
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(Qq)
}

#[cfg(test)]
#[path = "qq_tests.rs"]
mod tests;

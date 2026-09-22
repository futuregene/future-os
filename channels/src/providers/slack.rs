//! Slack: Socket Mode inbound, Web API outbound, Events API as the fallback.
//!
//! Socket Mode is the primary transport because it needs no public endpoint:
//! `apps.connections.open` (called with the app-level token) returns a WSS
//! URL, the platform pushes event envelopes down the socket, and every
//! envelope must be acked or it is redelivered. Events API delivery (a signed
//! webhook behind a reverse proxy) is used when no app-level token is
//! configured; its challenge answer, signature check and timestamp freshness
//! gate are handled here.
//!
//! Outbound, `chat.postMessage` creates messages and `chat.update` rewrites
//! them, which is what the bridge's progressive streaming rides on. Slack
//! counts message length in UTF-16 code units and rejects anything over 4000,
//! so the definition advertises [`LengthUnit::Utf16`] and the shared splitter
//! does the rest (including not cutting a fenced code block in half).
//!
//! Error taxonomy: the HTTP helper already retries 429 honoring `Retry-After`;
//! the interesting Slack-specific part is the `ok:false` envelope, where
//! `invalid_auth` (and friends) are permanent while `ratelimited`/`timeout`
//! are transient.

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::bridge::{
    ChatKind, ConversationRef, Inbound, MediaKind, MediaRef, ProviderCtx, SenderRef,
};
use crate::providers::traits::{
    Capabilities, ChannelDefinition, ChannelSender, Maturity, Provider,
};
use crate::transport::http::{send_json, ErrorClass, HttpResponse, RetryPolicy};
use crate::transport::text::LengthUnit;
use crate::transport::ws::{self, Backoff};
use crate::transport::{signature, webhook};

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "slack",
    display_name: "Slack",
    description: "Slack app: Socket Mode or Events API, threaded replies, progress reactions.",
    docs: "docs/guide/channels-slack.md",
    maturity: Maturity::Preview,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: true,
        threads: true,
        typing: false,
        reactions: true,
        media_in: true,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 4000,
    length_unit: LengthUnit::Utf16,
    config_example: r#"{
  "enabled": true,
  "bot_token": "",
  "app_token": "",
  "signing_secret": "",
  "webhook_port": 3100,
  "webhook_path": "/webhooks/slack",
  "dm_policy": "allowlist",
  "dm_allowlist": [],
  "group_policy": "disabled",
  "group_allowlist": [],
  "require_mention": true
}"#,
    requires: &["a Slack app with Socket Mode enabled or an Events API endpoint"],
};

const API_BASE: &str = "https://slack.com/api";

/// Default bind for the Events API fallback; a deployment puts TLS on a
/// reverse proxy and forwards here.
const DEFAULT_WEBHOOK_PORT: u16 = 3100;
const DEFAULT_WEBHOOK_PATH: &str = "/webhooks/slack";

/// Envelope carrying an inbound platform event.
const ENVELOPE_EVENTS_API: &str = "events_api";
/// Server-side disconnect notice; arrives ~10s before the socket is dropped.
const ENVELOPE_DISCONNECT: &str = "disconnect";

/// Web API error codes that mean "retrying this exact call will never work".
/// Kept as a table so a new code is a one-word change.
const PERMANENT_API_ERRORS: &[&str] = &[
    "account_inactive",
    "channel_not_found",
    "fatal_error",
    "invalid_auth",
    "is_archived",
    "message_not_found",
    "not_allowed_token_type",
    "not_authed",
    "not_in_channel",
    "token_revoked",
];

/// Web API error codes that may succeed on a later attempt.
const TRANSIENT_API_ERRORS: &[&str] = &[
    "fatal_timeout",
    "internal_error",
    "ratelimited",
    "request_timeout",
    "service_unavailable",
    "timeout",
];

/// Reject an Events API request whose timestamp is this far from the clock —
/// a signed replay from long ago is an attack, not a redelivery.
const WEBHOOK_TIMESTAMP_TOLERANCE_SECS: i64 = 5 * 60;

/// This app's config block in `~/.future/channels/config.json`.
///
/// Every field defaults so the access-policy keys the framework mixes into
/// the same block (and keys of older/newer configs) deserialize cleanly.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SlackConfig {
    pub enabled: bool,
    /// Bot token (`xoxb-…`): Web API calls, file downloads, auth.test.
    pub bot_token: String,
    /// App-level token (`xapp-…`): only used for `apps.connections.open`.
    pub app_token: String,
    /// Events API request signing secret; required for the webhook fallback.
    pub signing_secret: String,
    /// Bind port for the Events API fallback.
    pub webhook_port: u16,
    /// Path the platform posts events to.
    pub webhook_path: String,
    /// Explicit API base (tests); production uses `https://slack.com/api`.
    pub api_base: String,
}

impl Default for SlackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: String::new(),
            app_token: String::new(),
            signing_secret: String::new(),
            webhook_port: DEFAULT_WEBHOOK_PORT,
            webhook_path: DEFAULT_WEBHOOK_PATH.to_string(),
            api_base: String::new(),
        }
    }
}

impl SlackConfig {
    fn api_base(&self) -> &str {
        if self.api_base.is_empty() {
            API_BASE
        } else {
            &self.api_base
        }
    }
}

/// The HTTP verbs against the Web API.
#[derive(Clone)]
pub struct SlackApi {
    base: String,
    bot_token: String,
    app_token: String,
    http: reqwest::Client,
}

impl SlackApi {
    fn from_config(config: &SlackConfig, ctx: &ProviderCtx) -> Self {
        Self {
            base: config.api_base().to_string(),
            bot_token: config.bot_token.clone(),
            app_token: config.app_token.clone(),
            http: ctx.http().clone(),
        }
    }

    fn bearer(&self, token: &str) -> String {
        format!("Bearer {token}")
    }

    fn bot_header(&self) -> String {
        self.bearer(&self.bot_token)
    }

    /// POST a Web API method with the bot token, retrying transient failures
    /// (the HTTP helper honors `Retry-After`). Outbound sends use the default
    /// policy; the run path (probes, handshakes) goes through
    /// [`SlackApi::call_once`] so a shutdown during the reconnect backoff is
    /// not delayed behind retry sleeps — the supervise loop owns retries.
    async fn call(&self, method: &str, body: &Value) -> Result<HttpResponse> {
        self.call_with(method, body, RetryPolicy::default()).await
    }

    /// One attempt only: the caller retries on its own schedule.
    async fn call_once(&self, method: &str, body: &Value) -> Result<HttpResponse> {
        self.call_with(method, body, RetryPolicy::single_attempt())
            .await
    }

    async fn call_with(
        &self,
        method: &str,
        body: &Value,
        policy: RetryPolicy,
    ) -> Result<HttpResponse> {
        let url = format!("{}/{method}", self.base);
        send_json(
            &self.http,
            reqwest::Method::POST,
            &url,
            &[("Authorization", &self.bot_header())],
            Some(body),
            policy,
        )
        .await
        .with_context(|| format!("slack method {method} failed"))
    }

    /// `ok:false` is a 200, so a Slack call needs a second check past HTTP.
    async fn call_checked(&self, method: &str, body: &Value) -> Result<HttpResponse> {
        let response = self.call(method, body).await?;
        if !response.is_success() {
            bail!("{method}: {}", response.error_message());
        }
        check_ok(method, &response)?;
        Ok(response)
    }

    /// [`SlackApi::call_checked`] on the single-attempt policy.
    async fn call_checked_once(&self, method: &str, body: &Value) -> Result<HttpResponse> {
        let response = self.call_once(method, body).await?;
        if !response.is_success() {
            bail!("{method}: {}", response.error_message());
        }
        check_ok(method, &response)?;
        Ok(response)
    }

    /// Socket Mode handshake: returns the WSS URL to connect to. Single
    /// attempt: the outer reconnect loop owns retries.
    async fn open_connection(&self) -> Result<String> {
        let url = format!("{}/apps.connections.open", self.base);
        let response = send_json(
            &self.http,
            reqwest::Method::POST,
            &url,
            &[("Authorization", &self.bearer(&self.app_token))],
            Some(&json!({})),
            RetryPolicy::single_attempt(),
        )
        .await?;
        if !response.is_success() {
            bail!("apps.connections.open: {}", response.error_message());
        }
        check_ok("apps.connections.open", &response)?;
        response
            .body
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| url.starts_with("wss://") || url.starts_with("ws://"))
            .map(str::to_string)
            .ok_or_else(|| anyhow!("apps.connections.open returned no websocket URL"))
    }

    /// Download a `url_private` file; Slack requires the bot Authorization
    /// header even though the URL looks public.
    async fn download(&self, url: &str) -> Result<(Vec<u8>, Option<String>)> {
        let response = self
            .http
            .get(url)
            .header("Authorization", self.bot_header())
            .send()
            .await
            .with_context(|| format!("download of {url} failed"))?;
        if !response.status().is_success() {
            bail!("download of {url}: HTTP {}", response.status());
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = response.bytes().await?.to_vec();
        Ok((bytes, content_type))
    }
}

/// Enforce the Slack envelope contract: HTTP 200 does not imply success.
fn check_ok(method: &str, response: &HttpResponse) -> Result<()> {
    if response.body.get("ok").and_then(Value::as_bool) == Some(false) {
        let error = response
            .body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown_error");
        let class = match classify_api_error(error, response.status) {
            ErrorClass::Permanent => "permanent",
            ErrorClass::Transient => "transient",
        };
        bail!("{method}: {error} ({class} error)");
    }
    Ok(())
}

/// Whether an API error code (or, lacking one, the HTTP status) is permanent.
pub(crate) fn classify_api_error(error: &str, status: u16) -> ErrorClass {
    if TRANSIENT_API_ERRORS.contains(&error) {
        return ErrorClass::Transient;
    }
    if PERMANENT_API_ERRORS.contains(&error) {
        return ErrorClass::Permanent;
    }
    crate::transport::http::classify_status(status)
}

/// Normalize one Slack event into an inbound message.
///
/// Returns `None` for events that must be dropped: bot-authored messages
/// (including this bridge's own posts, which would otherwise loop back as
/// prompts), edits and other subtypes, anything without a sender or text.
/// `bot_user_id` lets us drop our own posts even when Slack did not attach a
/// subtype to them.
pub(crate) fn parse_event(event: &Value, bot_user_id: &str) -> Option<Inbound> {
    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
    if !matches!(event_type, "message" | "app_mention") {
        return None;
    }

    // Subtypes are system transformations (edits, joins, bot relays, …); only
    // file_share still carries a fresh user prompt, the rest are not prompts.
    let subtype = event.get("subtype").and_then(Value::as_str).unwrap_or("");
    if !subtype.is_empty() && subtype != "file_share" {
        return None;
    }
    if event.get("bot_id").is_some() {
        return None;
    }

    let sender_id = event
        .get("user")
        .and_then(Value::as_str)
        .filter(|user| !user.is_empty());
    let sender_id = sender_id?;
    if sender_id == bot_user_id {
        return None;
    }

    let raw_text = event
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let files: Vec<&Value> = event
        .get("files")
        .and_then(Value::as_array)
        .map(|files| files.iter().collect())
        .unwrap_or_default();
    if raw_text.trim().is_empty() && files.is_empty() {
        return None;
    }

    let channel_id = event
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let channel_type = event
        .get("channel_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = match channel_type {
        "im" => ChatKind::Direct,
        "channel" => ChatKind::Channel,
        // A group DM still needs an explicit address, like a group.
        _ => ChatKind::Group,
    };

    let ts = event
        .get("ts")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let thread_ts = event
        .get("thread_ts")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|thread| *thread != ts);

    let mentioned = event
        .get("text")
        .and_then(Value::as_str)
        .map(|text| mentions_user(text, bot_user_id))
        .unwrap_or(false);
    let addressed_to_bot = kind == ChatKind::Direct || event_type == "app_mention" || mentioned;

    // `client_msg_id` is the sender-assigned id and survives event retries;
    // fall back to the channel+ts pair, which identifies a message uniquely.
    let message_id = event
        .get("client_msg_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| format!("{channel_id}:{ts}"));

    let created_at_ms = ts
        .split('.')
        .next()
        .and_then(|seconds| seconds.parse::<i64>().ok())
        .map(|seconds| seconds * 1000);

    let media = files
        .iter()
        .map(|file| MediaRef {
            kind: file_kind(
                file.get("mimetype").and_then(Value::as_str),
                file.get("filetype").and_then(Value::as_str),
            ),
            filename: file.get("name").and_then(Value::as_str).map(str::to_string),
            content_type: file
                .get("mimetype")
                .and_then(Value::as_str)
                .map(str::to_string),
            url: file
                .get("url_private")
                .and_then(Value::as_str)
                .map(str::to_string),
            data: None,
        })
        .collect::<Vec<_>>();

    Some(Inbound {
        message_id,
        sender: SenderRef {
            id: sender_id.to_string(),
            display: None,
        },
        conversation: ConversationRef {
            id: channel_id,
            thread_id: thread_ts,
            kind,
        },
        text: reduce_mrkdwn(&raw_text, bot_user_id),
        media,
        addressed_to_bot,
        created_at_ms,
        raw: None,
    })
}

/// Where one file lands in the provider-neutral media vocabulary.
fn file_kind(mimetype: Option<&str>, filetype: Option<&str>) -> MediaKind {
    if let Some(mime) = mimetype {
        if mime.starts_with("image/") {
            return MediaKind::Image;
        }
        if mime.starts_with("audio/") {
            return MediaKind::Audio;
        }
        if mime.starts_with("video/") {
            return MediaKind::Video;
        }
        if mime.starts_with("text/") || mime.starts_with("application/") {
            return MediaKind::Document;
        }
    }
    match filetype.unwrap_or("") {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" => MediaKind::Image,
        "mp3" | "m4a" | "wav" | "ogg" => MediaKind::Audio,
        "mp4" | "mov" | "webm" => MediaKind::Video,
        "" => MediaKind::Unknown,
        _ => MediaKind::Document,
    }
}

/// Whether `text` addresses `<@USERID>`.
fn mentions_user(text: &str, user_id: &str) -> bool {
    !user_id.is_empty() && text.contains(&format!("<@{user_id}>"))
}

/// Reduce Slack mrkdwn to plain text: unwrap links, drop angle-bracket
/// formatting, and strip the bot's own mention from the front.
fn reduce_mrkdwn(text: &str, bot_user_id: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        match after.find('>') {
            // `<https://a.test|label>` → `label`, `<https://a.test>` → the URL,
            // `<@U123>` → `@U123` (kept readable, no longer a mention syntax).
            Some(end) => {
                let inner = &after[1..end];
                let replacement = if inner.starts_with("#C") {
                    inner
                        .split('|')
                        .nth(1)
                        .map(|label| format!("#{label}"))
                        .unwrap_or_else(|| format!("#{}", &inner[1..]))
                } else if inner.starts_with('@') || inner.starts_with('!') {
                    format!("@{}", &inner[1..])
                } else {
                    inner.split('|').nth(1).unwrap_or(inner).to_string()
                };
                out.push_str(&replacement);
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(after);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    let reduced = out.trim().to_string();
    // A message that was only the mention has no prompt left.
    if !bot_user_id.is_empty() && reduced == format!("@{bot_user_id}") {
        return String::new();
    }
    reduced
}

/// Build the acknowledgement for one Socket Mode envelope.
///
/// An envelope that is never acked is redelivered, so the ack is built before
/// the event is even looked at; `retry_reason` is only ever observability.
pub(crate) fn envelope_ack(envelope: &Value) -> Option<Value> {
    let envelope_id = envelope.get("envelope_id").and_then(Value::as_str)?;
    Some(json!({ "envelope_id": envelope_id }))
}

/// Verify the Events API signature: `v0:timestamp:body` HMAC-SHA256 under the
/// signing secret, constant-time compared, with a freshness window so a
/// captured request cannot be replayed later.
pub(crate) fn verify_request_signature(
    secret: &str,
    timestamp: &str,
    signature_header: &str,
    body: &[u8],
    now_unix: i64,
) -> bool {
    if secret.is_empty() || timestamp.is_empty() || signature_header.is_empty() {
        return false;
    }
    let Ok(sent_at) = timestamp.parse::<i64>() else {
        return false;
    };
    if (now_unix - sent_at).abs() > WEBHOOK_TIMESTAMP_TOLERANCE_SECS {
        return false;
    }
    let mut base = format!("v0:{timestamp}:").into_bytes();
    base.extend_from_slice(body);
    let digest = signature::hmac_sha256_hex(secret.as_bytes(), &base);
    // `verify_hex` knows the `sha256=` prefix, not Slack's `v0=` one, so the
    // comparison is done here after stripping.
    let expected = signature_header
        .strip_prefix("v0=")
        .unwrap_or(signature_header);
    signature::constant_time_eq(digest.as_bytes(), expected.as_bytes())
}

/// The outbound half: `chat.postMessage`/`chat.update`/`reactions.add`.
pub struct SlackSender {
    api: SlackApi,
}

#[async_trait]
impl ChannelSender for SlackSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        let mut body = json!({
            "channel": conversation.id,
            "text": text,
        });
        // A reply in a thread must carry thread_ts; without it the platform
        // posts at the top level and the conversation splits in two.
        if let Some(thread) = conversation.thread_id.as_deref() {
            body["thread_ts"] = json!(thread);
        }
        let response = self.api.call_checked("chat.postMessage", &body).await?;
        Ok(response
            .body
            .get("ts")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    async fn edit_text(
        &self,
        conversation: &ConversationRef,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let body = json!({
            "channel": conversation.id,
            "ts": message_id,
            "text": text,
        });
        self.api.call_checked("chat.update", &body).await?;
        Ok(())
    }

    async fn react(
        &self,
        conversation: &ConversationRef,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let body = json!({
            "channel": conversation.id,
            "timestamp": message_id,
            "name": emoji,
        });
        self.api.call_checked("reactions.add", &body).await?;
        Ok(())
    }
}

/// The Slack app.
struct Slack;

#[async_trait]
impl Provider for Slack {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        let config = ctx.config::<SlackConfig>()?;
        if config.bot_token.is_empty() {
            bail!("providers.slack.bot_token is required");
        }
        Ok(Arc::new(SlackSender {
            api: SlackApi::from_config(&config, ctx),
        }))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        let config = ctx.config::<SlackConfig>()?;
        if config.bot_token.is_empty() {
            bail!("providers.slack.bot_token is required");
        }
        let api = SlackApi::from_config(&config, &ctx);
        let bot_user_id = auth_identity(&api).await.unwrap_or_else(|error| {
            tracing::warn!(channel = "slack", %error, "auth.test failed; own posts may loop until it recovers");
            String::new()
        });
        let sender: Arc<dyn ChannelSender> = Arc::new(SlackSender { api: api.clone() });

        if !config.app_token.is_empty() {
            return run_socket_mode(&ctx, &api, sender, &bot_user_id).await;
        }
        if !config.signing_secret.is_empty() {
            return run_events_webhook(&ctx, sender, &config, bot_user_id).await;
        }
        bail!(
            "providers.slack needs either app_token (Socket Mode) or signing_secret (Events API) to receive messages"
        )
    }

    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        let config = ctx.config::<SlackConfig>()?;
        if config.bot_token.is_empty() {
            bail!("providers.slack.bot_token is required");
        }
        let api = SlackApi::from_config(&config, ctx);
        let user_id = auth_identity(&api).await?;
        if user_id.is_empty() {
            return Ok("connected (identity unknown)".to_string());
        }
        Ok(format!("connected as {user_id}"))
    }
}

/// `auth.test` is the smallest real request that proves the bot token works;
/// it also yields the bot's own user id, used to ignore our own messages.
/// Single attempt: run() treats a failure as a warning and the probe reports
/// the error as-is, so no retry is wanted here.
async fn auth_identity(api: &SlackApi) -> Result<String> {
    let response = api.call_checked_once("auth.test", &json!({})).await?;
    Ok(response
        .body
        .get("user_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string())
}

/// Socket Mode loop: handshake, then read envelopes until shutdown.
async fn run_socket_mode(
    ctx: &ProviderCtx,
    api: &SlackApi,
    sender: Arc<dyn ChannelSender>,
    bot_user_id: &str,
) -> Result<()> {
    ws::supervise("slack", ctx.shutdown(), Backoff::gateway(), || async {
        let url = api.open_connection().await?;
        let socket = ws::connect(&url, &[]).await?;
        ctx.mark_running();
        socket_mode_session(ctx, sender.clone(), bot_user_id, socket).await
    })
    .await
}

/// One connected Socket Mode session: ack every envelope, handle events,
/// bail out on a server-requested disconnect so the outer loop reconnects.
async fn socket_mode_session(
    ctx: &ProviderCtx,
    sender: Arc<dyn ChannelSender>,
    bot_user_id: &str,
    socket: ws::Socket,
) -> Result<()> {
    let (mut sink, mut stream) = socket.split();
    let acked = AtomicU64::new(0);
    loop {
        let message = tokio::select! {
            message = stream.next() => message,
            _ = ctx.shutdown().notified() => {
                tracing::info!(channel = "slack", acked = acked.load(Ordering::Relaxed), "socket mode session stopped");
                return Ok(());
            }
        };
        let Some(message) = message else {
            bail!("slack socket closed by the platform");
        };
        match message {
            Ok(WsMessage::Text(text)) => {
                let envelope: Value = match serde_json::from_str(&text) {
                    Ok(envelope) => envelope,
                    Err(error) => {
                        tracing::debug!(channel = "slack", %error, "ignoring a non-JSON frame");
                        continue;
                    }
                };
                if let Some(ack) = envelope_ack(&envelope) {
                    if sink.send(WsMessage::Text(ack.to_string())).await.is_err() {
                        bail!("slack socket is not writable");
                    }
                    acked.fetch_add(1, Ordering::Relaxed);
                }
                match envelope.get("type").and_then(Value::as_str) {
                    Some(ENVELOPE_DISCONNECT) => {
                        // The platform asks for a fresh connection; treat this
                        // as a clean end so the reconnect backoff resets.
                        tracing::info!(channel = "slack", "server requested a reconnect");
                        return Ok(());
                    }
                    Some(ENVELOPE_EVENTS_API) => {
                        if let Some(event) = envelope
                            .get("payload")
                            .and_then(|payload| payload.get("event"))
                        {
                            dispatch_event(ctx, &sender, event, bot_user_id).await;
                        }
                    }
                    // `hello`, interactive payloads and slash commands are not
                    // prompts; acknowledging them above is the whole contract.
                    _ => {}
                }
            }
            Ok(WsMessage::Ping(data)) => {
                if sink.send(WsMessage::Pong(data)).await.is_err() {
                    bail!("slack socket is not writable");
                }
            }
            Ok(WsMessage::Close(_)) => bail!("slack socket closed by the platform"),
            Ok(_) => {}
            Err(error) => bail!("slack socket read failed: {error}"),
        }
    }
}

/// Parse one event and hand it to the bridge; download file attachments that
/// can be fetched without a second user-visible round trip.
async fn dispatch_event(
    ctx: &ProviderCtx,
    sender: &Arc<dyn ChannelSender>,
    event: &Value,
    bot_user_id: &str,
) {
    let Some(mut inbound) = parse_event(event, bot_user_id) else {
        return;
    };
    if let Err(error) = fetch_attachments(ctx, &mut inbound).await {
        tracing::debug!(channel = "slack", %error, "an attachment could not be fetched");
    }
    let message_id = inbound.message_id.clone();
    let conversation = inbound.conversation.clone();
    let outcome = ctx.handle(inbound, sender.clone()).await;
    tracing::debug!(channel = "slack", ?outcome, "event handled");
    // A lightweight acknowledgement: the user sees the message was picked up
    // before the agent has produced anything.
    if outcome.is_accepted() {
        sender.react(&conversation, &message_id, "eyes").await.ok();
    }
}

/// Fill in attachment bytes where the platform exposes a `url_private`.
async fn fetch_attachments(ctx: &ProviderCtx, inbound: &mut Inbound) -> Result<()> {
    let config = ctx.config::<SlackConfig>()?;
    if config.bot_token.is_empty() {
        return Ok(());
    }
    let api = SlackApi::from_config(&config, ctx);
    for media in &mut inbound.media {
        if media.data.is_some() {
            continue;
        }
        let Some(url) = media.url.clone() else {
            continue;
        };
        if media.kind != MediaKind::Image {
            // Non-image media stays a URL reference in the prompt.
            continue;
        }
        match api.download(&url).await {
            Ok((bytes, content_type)) => {
                if let Some(content_type) = content_type {
                    media.content_type = Some(content_type);
                }
                media.data = Some(bytes);
            }
            Err(error) => {
                tracing::debug!(channel = "slack", %error, url, "file download failed");
            }
        }
    }
    Ok(())
}

/// Events API fallback: verify signatures, answer the one-time URL
/// verification challenge, and accept only fresh retries.
async fn run_events_webhook(
    ctx: &ProviderCtx,
    sender: Arc<dyn ChannelSender>,
    config: &SlackConfig,
    bot_user_id: String,
) -> Result<()> {
    let secret = config.signing_secret.clone();
    let path = if config.webhook_path.starts_with('/') {
        config.webhook_path.clone()
    } else {
        format!("/{}", config.webhook_path)
    };
    let ctx_clone = ctx.clone();
    let sender_for_hook = sender.clone();
    // Tests bind an ephemeral port and learn what the server registered
    // through this slot.
    let test_slot = webhook_test_slot();
    let bind_port = webhook_bind_port(config.webhook_port, test_slot.as_ref());
    let server = webhook::WebhookServer::bind(&format!("0.0.0.0:{bind_port}")).await?;
    #[cfg(test)]
    if let Some(slot) = test_slot {
        let bound_port = server.local_addr()?.port();
        let mut slot = slot.lock().unwrap_or_else(|error| error.into_inner());
        slot.1 = path.clone();
        slot.2 = bound_port;
    }
    let server = server.on("POST", &path, move |request| {
        let ctx = ctx_clone.clone();
        let sender = sender_for_hook.clone();
        let secret = secret.clone();
        let bot_user_id = bot_user_id.clone();
        async move {
            let timestamp = request.header("x-slack-request-timestamp").unwrap_or("");
            let signature = request.header("x-slack-signature").unwrap_or("");
            let now = crate::bridge::dedup::now_ms() / 1000;
            if !verify_request_signature(&secret, timestamp, signature, &request.body, now) {
                return webhook::WebhookResponse::unauthorized();
            }
            let body = request.json();
            match body.get("type").and_then(Value::as_str) {
                Some("url_verification") => {
                    let challenge = body
                        .get("challenge")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    webhook::WebhookResponse::text(200, challenge)
                }
                Some("event_callback") => {
                    // Retries of an event we already saw are acked without
                    // work; the bridge dedups on the message id anyway.
                    // Respond fast (the platform gives ~3s) and process the
                    // event off the request path.
                    if let Some(event) = body.get("event") {
                        let ctx = ctx.clone();
                        let sender = sender.clone();
                        let bot_user_id = bot_user_id.clone();
                        let event = event.clone();
                        tokio::spawn(async move {
                            dispatch_event(&ctx, &sender, &event, &bot_user_id).await;
                        });
                    }
                    webhook::WebhookResponse::ok()
                }
                _ => webhook::WebhookResponse::ok(),
            }
        }
    });
    ctx.mark_running();
    tracing::info!(
        channel = "slack",
        port = config.webhook_port,
        path,
        "events API webhook listening"
    );
    server.serve(ctx.shutdown().clone()).await
}

/// The configured port, or the ephemeral port a test armed through the hook.
/// Tests always take the `Some` arm; the `None` arm is the production path
/// (and the no-hook test), which the configured-port test exercises.
fn webhook_bind_port(configured: u16, slot: Option<&WebhookBindingSlot>) -> u16 {
    slot.map(|slot| slot.lock().unwrap_or_else(|error| error.into_inner()).0)
        .unwrap_or(configured)
}

#[cfg(test)]
fn webhook_test_slot() -> Option<std::sync::Arc<std::sync::Mutex<(u16, String, u16)>>> {
    webhook_test_hook::take()
}

#[cfg(not(test))]
fn webhook_test_slot() -> Option<std::sync::Arc<std::sync::Mutex<(u16, String, u16)>>> {
    None
}

/// The bind port and path normalization are only observable from inside the
/// webhook task; tests hand in an ephemeral port and read the registered
/// path back through this slot.
#[cfg(test)]
pub(crate) mod webhook_test_hook {
    use std::sync::{Arc, Mutex, MutexGuard};

    static LOCK: Mutex<()> = Mutex::new(());
    /// (bind port, registered path, bound port).
    type Slot = Arc<Mutex<(u16, String, u16)>>;
    static NEXT: Mutex<Option<Slot>> = Mutex::new(None);

    /// Serialize tests that arm the hook (a single static slot).
    pub fn lock() -> MutexGuard<'static, ()> {
        LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The next `run_events_webhook` call binds the port in `slot.0` and
    /// records the normalized path and the bound port back into the slot.
    pub fn arm(slot: Slot) {
        *NEXT.lock().unwrap_or_else(|error| error.into_inner()) = Some(slot);
    }

    pub fn take() -> Option<Slot> {
        NEXT.lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
}

/// A failure-arm test kills the client socket's write half so the session's
/// next ack/pong flush fails deterministically (rather than racing the
/// reconnect read).
#[cfg(all(test, unix))]
pub(crate) fn kill_write_half(socket: &ws::Socket) {
    use std::os::fd::{AsRawFd, FromRawFd};
    let plain = match socket.get_ref() {
        tokio_tungstenite::MaybeTlsStream::Plain(stream) => stream,
        _ => unreachable!("tests dial plain ws only"),
    };
    let borrowed =
        std::mem::ManuallyDrop::new(unsafe { std::net::TcpStream::from_raw_fd(plain.as_raw_fd()) });
    borrowed
        .shutdown(std::net::Shutdown::Write)
        .expect("shutdown the client socket's write half");
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(Slack)
}

#[cfg(test)]
#[path = "slack_tests.rs"]
mod tests;
/// The test-visible slot that records where the webhook server bound
/// `(port, path, connections)`.
type WebhookBindingSlot = std::sync::Arc<std::sync::Mutex<(u16, String, u16)>>;

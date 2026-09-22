//! Linq: a hosted iMessage / RCS / SMS messaging API.
//!
//! * **Inbound** — a [`WebhookServer`] taking signed deliveries. Every event
//!   shares one envelope (`event_type`, `event_id`, `created_at`, `data`), and
//!   only `message.received` becomes a prompt. The subscription's signing secret
//!   is required: the endpoint is public, and an unverified delivery would let
//!   anyone who can reach the port drive the agent.
//! * **Outbound** — REST with a bearer key. A reply goes to
//!   `POST /v3/chats/{chat_id}/messages`; a send to a bare phone number opens
//!   the conversation first (`POST /v3/chats` when a line is configured,
//!   `POST /v3/messages` to let the platform pick one). The bridge splits text
//!   to [`ChannelDefinition::max_text_len`].
//! * **Addressing** — a 1:1 chat is always addressed; a group chat counts as
//!   addressed only when a text part mentions this line (`mentions[].is_me`).
//!   The 2025-01-01 payload version reports only the first mention on a part
//!   without saying whose it is, so a subscription still on that version needs
//!   `require_mention: false` for group chats to be answered at all.
//! * **Media** — parts carry a signed CDN URL; images are fetched so the model
//!   can see them, other kinds are left as references.
//! * **Errors** — the API publishes numbered error codes in bands: request and
//!   resource errors never succeed on a retry, server errors always may, and
//!   the few retryable members of the non-retryable bands (a rate limit, a chat
//!   still being created) are named individually.
//!
//! Maturity stays [`Maturity::Preview`]: this follows the platform's public API
//! reference, but no live account has exercised it.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use base64::engine::general_purpose::{STANDARD, URL_SAFE};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::bridge::{
    ChatKind, ConversationRef, Inbound, MediaKind, MediaRef, ProviderCtx, SenderRef,
};
use crate::providers::traits::{
    Capabilities, ChannelDefinition, ChannelSender, Maturity, Provider,
};
use crate::transport::http::{send_json, ErrorClass, HttpResponse, RetryPolicy};
use crate::transport::signature::{constant_time_eq, hmac_sha256, hmac_sha256_hex, verify_hex};
use crate::transport::webhook::{WebhookRequest, WebhookResponse, WebhookServer};
use crate::transport::LengthUnit;

#[cfg(test)]
#[path = "linq_tests.rs"]
mod tests;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "linq",
    display_name: "Linq (iMessage / RCS)",
    description: "Hosted iMessage and RCS messaging: signed webhook inbound, REST outbound.",
    docs: "docs/guide/channels-linq.md",
    maturity: Maturity::Preview,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: false,
        threads: false,
        typing: false,
        reactions: false,
        media_in: true,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 4000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "api_key": "",
  "from": "",
  "webhook": {
    "addr": "127.0.0.1:8789",
    "path": "/webhooks/linq",
    "signing_secret": ""
  },
  "dm_policy": "allowlist",
  "dm_allowlist": [],
  "group_policy": "disabled",
  "group_allowlist": [],
  "require_mention": true
}"#,
    requires: &["a Linq account with a provisioned sender"],
};

/// The API origin; `api_base` in the config is the test seam.
const DEFAULT_API_BASE: &str = "https://api.linqapp.com/api/partner";
const DEFAULT_API_VERSION: &str = "v3";
const DEFAULT_WEBHOOK_ADDR: &str = "127.0.0.1:8789";
const DEFAULT_WEBHOOK_PATH: &str = "/webhooks/linq";
/// Largest attachment this provider will pull into model input.
const MAX_DOWNLOAD_BYTES: usize = 16 * 1024 * 1024;
/// How stale a signed delivery may be before it is treated as a replay.
const REPLAY_WINDOW_SECONDS: i64 = 300;
/// The only event type that is a prompt.
const EVENT_MESSAGE_RECEIVED: &str = "message.received";

/// The `providers.linq` block.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LinqConfig {
    /// Bearer key for the API.
    pub api_key: String,
    /// The line to send from, in E.164. Empty lets the platform choose one.
    pub from: String,
    pub webhook: WebhookConfig,
    /// Test seam: replaces the API origin.
    pub api_base: String,
    /// Test seam: replaces the API version segment.
    pub api_version: String,
}

impl LinqConfig {
    fn api_base(&self) -> &str {
        let base = if self.api_base.is_empty() {
            DEFAULT_API_BASE
        } else {
            self.api_base.as_str()
        };
        base.trim_end_matches('/')
    }

    fn api_version(&self) -> &str {
        if self.api_version.is_empty() {
            DEFAULT_API_VERSION
        } else {
            self.api_version.as_str()
        }
    }

    fn webhook_addr(&self) -> &str {
        if self.webhook.addr.is_empty() {
            DEFAULT_WEBHOOK_ADDR
        } else {
            self.webhook.addr.as_str()
        }
    }

    fn webhook_path(&self) -> &str {
        if self.webhook.path.is_empty() {
            DEFAULT_WEBHOOK_PATH
        } else {
            self.webhook.path.as_str()
        }
    }

    fn require_api_key(&self, channel: &str) -> Result<()> {
        if self.api_key.trim().is_empty() {
            anyhow::bail!("invalid `providers.{channel}` configuration: `api_key` is required");
        }
        Ok(())
    }

    /// Receiving without a signing secret is refused rather than trusted: the
    /// only thing standing between a public URL and the agent is this secret.
    fn require_signing_secret(&self, channel: &str) -> Result<()> {
        if self.webhook.signing_secret.trim().is_empty() {
            anyhow::bail!(
                "invalid `providers.{channel}` configuration: `webhook.signing_secret` is required to receive"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WebhookConfig {
    pub addr: String,
    pub path: String,
    /// The subscription's signing secret, `whsec_` followed by base64 key bytes.
    pub signing_secret: String,
}

// ─── Signature verification ─────────────────────────────────────────────────

/// The signing key behind a subscription secret.
///
/// The secret carries a `whsec_` prefix in front of base64 key bytes; a secret
/// configured without the prefix is decoded if it still looks like base64, and
/// otherwise used as raw bytes.
fn signing_key(secret: &str) -> Vec<u8> {
    let encoded = secret.strip_prefix("whsec_").unwrap_or(secret);
    STANDARD
        .decode(encoded)
        .or_else(|_| URL_SAFE.decode(encoded))
        .unwrap_or_else(|_| secret.as_bytes().to_vec())
}

/// The bytes a delivery's signature covers: id, timestamp, then the raw body.
fn signed_content(id: &str, timestamp: &str, body: &[u8]) -> Vec<u8> {
    let mut content = Vec::with_capacity(id.len() + timestamp.len() + body.len() + 2);
    content.extend_from_slice(id.as_bytes());
    content.push(b'.');
    content.extend_from_slice(timestamp.as_bytes());
    content.push(b'.');
    content.extend_from_slice(body);
    content
}

/// Whether any candidate in the signature header matches the expected digest.
///
/// The header may list several space-separated candidates, each `v1,<base64>`,
/// so that a subscription can be rotated without a gap.
fn signature_matches(expected: &[u8], candidates: &str) -> bool {
    candidates
        .split(' ')
        .filter_map(|candidate| candidate.strip_prefix("v1,"))
        .any(|candidate| {
            let decoded = STANDARD
                .decode(candidate)
                .or_else(|_| URL_SAFE.decode(candidate));
            match decoded {
                Ok(bytes) => constant_time_eq(&bytes, expected),
                Err(_) => false,
            }
        })
}

/// Verify a delivery under the current signed-request scheme.
fn verify_signed_request(request: &WebhookRequest, secret: &str, now_seconds: i64) -> bool {
    let Some(id) = request.header("webhook-id") else {
        return false;
    };
    let Some(timestamp) = request.header("webhook-timestamp") else {
        return false;
    };
    let Some(signature) = request.header("webhook-signature") else {
        return false;
    };
    // A signature is only as good as the moment it was made: an intercepted
    // delivery replayed later must not be accepted. The window is symmetric so
    // a clock a little ahead of ours does not lock the channel out.
    let Ok(sent_at) = timestamp.parse::<i64>() else {
        return false;
    };
    if (now_seconds - sent_at).abs() > REPLAY_WINDOW_SECONDS {
        return false;
    }
    let digest = hmac_sha256(
        &signing_key(secret),
        &signed_content(id, timestamp, &request.body),
    );
    signature_matches(&digest, signature)
}

/// Verify a delivery under the deprecated header set, which integrations built
/// against the previous API version still receive: a hex HMAC-SHA256 of the raw
/// body under the same secret.
fn verify_legacy_request(request: &WebhookRequest, secret: &str, now_seconds: i64) -> bool {
    let Some(presented) = request.header("x-webhook-signature") else {
        return false;
    };
    if let Some(timestamp) = request.header("x-webhook-timestamp") {
        let Ok(sent_at) = timestamp.parse::<i64>() else {
            return false;
        };
        if (now_seconds - sent_at).abs() > REPLAY_WINDOW_SECONDS {
            return false;
        }
    }
    verify_hex(
        &hmac_sha256_hex(secret.as_bytes(), &request.body),
        presented,
    )
}

/// Whether one delivery is genuine.
///
/// The current scheme wins: when the modern header is present it must verify,
/// and there is no fallback to the older one — accepting whichever of two
/// schemes happens to pass is how a downgrade sneaks in.
fn verify_delivery(request: &WebhookRequest, secret: &str, now_seconds: i64) -> bool {
    if secret.is_empty() {
        return false;
    }
    if request.header("webhook-signature").is_some() {
        return verify_signed_request(request, secret, now_seconds);
    }
    verify_legacy_request(request, secret, now_seconds)
}

// ─── Event parsing ──────────────────────────────────────────────────────────

/// Every inbound message in one delivery.
///
/// A delivery is one event envelope; an array is tolerated in case a deployment
/// batches, and anything that is not a received message is ignored.
fn parse_deliveries(body: &Value) -> Vec<Inbound> {
    match body {
        Value::Array(events) => events.iter().filter_map(parse_event).collect(),
        event => parse_event(event).into_iter().collect(),
    }
}

fn parse_event(event: &Value) -> Option<Inbound> {
    if event.get("event_type").and_then(Value::as_str) != Some(EVENT_MESSAGE_RECEIVED) {
        return None;
    }
    let data = event.get("data")?;
    // Outbound messages come back as events too; answering one would make the
    // bot talk to itself.
    if is_from_me(data) {
        return None;
    }
    let chat_id = chat_id(data)?;
    let message_id = message_id(data)?;
    let handle = sender_handle(data)?;
    let (text, media, mentioned) = parts_of(data);
    if text.trim().is_empty() && media.is_empty() {
        return None;
    }
    let is_group = chat_is_group(data);
    if is_group && !mentioned {
        // Not addressed: the group policy decides, and this is why.
        tracing::debug!(chat = %chat_id, "linq group message without a mention of this line");
    }
    Some(Inbound {
        message_id,
        sender: SenderRef {
            id: handle,
            display: None,
        },
        conversation: ConversationRef {
            id: chat_id,
            thread_id: None,
            kind: if is_group {
                ChatKind::Group
            } else {
                ChatKind::Direct
            },
        },
        text,
        media,
        addressed_to_bot: !is_group || mentioned,
        created_at_ms: created_at_ms(data, event),
        raw: Some(data.clone()),
    })
}

/// The chat a message belongs to, across both payload versions.
fn chat_id(data: &Value) -> Option<String> {
    let value = data
        .pointer("/chat/id")
        .or_else(|| data.get("chat_id"))?
        .as_str()?;
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

fn chat_is_group(data: &Value) -> bool {
    data.pointer("/chat/is_group")
        .or_else(|| data.get("is_group"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn message_id(data: &Value) -> Option<String> {
    let value = data
        .get("id")
        .or_else(|| data.pointer("/message/id"))?
        .as_str()?;
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

/// The participant's own handle (E.164), which is what an allowlist can name.
/// The participant's opaque id is only a fallback.
fn sender_handle(data: &Value) -> Option<String> {
    let value = data
        .pointer("/sender_handle/handle")
        .or_else(|| data.get("from"))
        .or_else(|| data.pointer("/from_handle/handle"))
        .or_else(|| data.pointer("/sender_handle/id"))?
        .as_str()?;
    if value.is_empty() {
        return None;
    }
    Some(value.to_string())
}

fn is_from_me(data: &Value) -> bool {
    if data.get("is_from_me").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    data.get("direction").and_then(Value::as_str) == Some("outbound")
}

/// The message's own timestamp, falling back to the envelope's.
fn created_at_ms(data: &Value, event: &Value) -> Option<i64> {
    let candidates = [
        data.get("sent_at"),
        data.get("received_at"),
        data.pointer("/message/sent_at"),
        data.pointer("/message/created_at"),
        event.get("created_at"),
    ];
    candidates
        .iter()
        .flatten()
        .find_map(|value| value.as_str())
        .and_then(parse_instant_ms)
}

/// An RFC 3339 instant as Unix milliseconds.
fn parse_instant_ms(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|instant| instant.timestamp_millis())
}

/// The text, attachments and mention status of one message's parts.
fn parts_of(data: &Value) -> (String, Vec<MediaRef>, bool) {
    let Some(parts) = data
        .get("parts")
        .or_else(|| data.pointer("/message/parts"))
        .and_then(Value::as_array)
    else {
        return (String::new(), Vec::new(), false);
    };
    let mut text: Vec<&str> = Vec::new();
    let mut media = Vec::new();
    let mut mentioned = false;
    for part in parts {
        match part.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(value) = part.get("value").and_then(Value::as_str) {
                    if !value.is_empty() {
                        text.push(value);
                    }
                }
                mentioned |= mentions_this_line(part);
            }
            // A link preview is a message whose content *is* the URL.
            Some("link") => {
                if let Some(value) = part.get("value").and_then(Value::as_str) {
                    if !value.is_empty() {
                        text.push(value);
                    }
                }
            }
            Some("media") => media.push(media_of(part)),
            _ => {}
        }
    }
    (text.join("\n"), media, mentioned)
}

/// Whether a text part mentions the line this account sends from.
fn mentions_this_line(part: &Value) -> bool {
    part.get("mentions")
        .and_then(Value::as_array)
        .map(|mentions| {
            mentions
                .iter()
                .any(|mention| mention.get("is_me").and_then(Value::as_bool) == Some(true))
        })
        .unwrap_or(false)
}

fn media_of(part: &Value) -> MediaRef {
    let content_type = part
        .get("mime_type")
        .and_then(Value::as_str)
        .map(str::to_string);
    MediaRef {
        kind: media_kind(content_type.as_deref()),
        filename: part
            .get("filename")
            .and_then(Value::as_str)
            .map(str::to_string),
        content_type,
        url: part.get("url").and_then(Value::as_str).map(str::to_string),
        data: None,
    }
}

/// Map a part's declared type onto the bridge's media vocabulary.
fn media_kind(content_type: Option<&str>) -> MediaKind {
    match content_type.unwrap_or_default() {
        mime if mime.starts_with("image/") => MediaKind::Image,
        mime if mime.starts_with("video/") => MediaKind::Video,
        mime if mime.starts_with("audio/") => MediaKind::Audio,
        "" => MediaKind::Unknown,
        _ => MediaKind::Document,
    }
}

// ─── Errors ─────────────────────────────────────────────────────────────────

/// Whether one numbered error is permanent, transient, or needs the HTTP status
/// to decide.
///
/// The API groups codes by band with an explicit retry answer, so the bands do
/// most of the work and only the named exceptions are listed.
fn classify_error_code(code: i64) -> Option<ErrorClass> {
    match code {
        // The retryable members of the "fix your request" bands: a rate limit,
        // an attachment still being processed, a chat still being created.
        1007 | 2007 | 2009 => Some(ErrorClass::Transient),
        // Everything else the API says not to retry.
        1000..=2999 => Some(ErrorClass::Permanent),
        // Server-side failures.
        3000..=3999 => Some(ErrorClass::Transient),
        // Delivery and file errors depend on the cause (a timed-out send may
        // work later, an unsupported message type never will).
        4006 | 4004 | 4010 => Some(ErrorClass::Transient),
        4002 | 4005 | 4008 | 4009 => Some(ErrorClass::Permanent),
        5001..=5003 | 5007 => Some(ErrorClass::Transient),
        5004..=5006 => Some(ErrorClass::Permanent),
        _ => None,
    }
}

/// The numbered code out of an error body.
fn error_code(response: &HttpResponse) -> Option<i64> {
    response
        .body
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_i64)
}

/// A one-line trace reference, which is what the platform's support asks for.
fn trace_hint(response: &HttpResponse) -> String {
    match response.body.get("trace_id").and_then(Value::as_str) {
        Some(trace) => format!(" (trace {trace})"),
        None => String::new(),
    }
}

/// The `retry_after` an error body carries on a rate limit, when present.
fn retry_after_hint(response: &HttpResponse) -> Option<i64> {
    response
        .body
        .get("error")
        .and_then(|error| error.get("retry_after"))
        .and_then(Value::as_i64)
}

fn send_error(action: &str, response: &HttpResponse) -> anyhow::Error {
    let code = error_code(response);
    let class = code
        .and_then(classify_error_code)
        .unwrap_or_else(|| response.class());
    let label = match class {
        ErrorClass::Permanent => "permanent",
        ErrorClass::Transient => "transient",
    };
    let retry = match retry_after_hint(response) {
        Some(seconds) => format!(", retry after {seconds}s"),
        None => String::new(),
    };
    anyhow!(
        "linq `{action}` rejected ({label}{retry}): {}{}",
        response.error_message(),
        trace_hint(response)
    )
}

// ─── Outbound ───────────────────────────────────────────────────────────────

/// The outbound half.
pub struct LinqSender {
    client: reqwest::Client,
    base: String,
    version: String,
    api_key: String,
    from: String,
}

impl LinqSender {
    fn new(client: reqwest::Client, config: &LinqConfig) -> Self {
        Self {
            client,
            base: config.api_base().to_string(),
            version: config.api_version().to_string(),
            api_key: config.api_key.clone(),
            from: config.from.clone(),
        }
    }

    fn authorization(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    fn url(&self, suffix: &str) -> String {
        format!("{}/{}{}", self.base, self.version, suffix)
    }

    /// One text message, as the API's `parts` array.
    fn message_body(text: &str) -> Value {
        json!({ "message": { "parts": [{ "type": "text", "value": text }] } })
    }

    async fn post(&self, url: &str, body: &Value, action: &str) -> Result<HttpResponse> {
        let authorization = self.authorization();
        let response = send_json(
            &self.client,
            reqwest::Method::POST,
            url,
            &[
                ("Authorization", authorization.as_str()),
                ("Content-Type", "application/json"),
            ],
            Some(body),
            RetryPolicy::default(),
        )
        .await
        .map_err(|error| anyhow!("linq `{action}` failed: {error}"))?;
        if !response.is_success() {
            return Err(send_error(action, &response));
        }
        Ok(response)
    }

    /// Send into an existing chat.
    async fn send_to_chat(&self, chat_id: &str, text: &str) -> Result<Option<String>> {
        let url = self.url(&format!("/chats/{chat_id}/messages"));
        let response = self
            .post(&url, &Self::message_body(text), "send message")
            .await?;
        Ok(response
            .body
            .pointer("/message/id")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    /// Open a conversation with a bare handle and send the first message.
    ///
    /// With a configured line the chat is created on it explicitly; without
    /// one, the platform picks a line and reuses an existing chat where it can.
    async fn send_to_handle(&self, handle: &str, text: &str) -> Result<Option<String>> {
        let mut body = Self::message_body(text);
        body["to"] = json!([handle]);
        let url = if self.from.is_empty() {
            // No line configured: the platform picks one and reuses the chat
            // these recipients are already in.
            self.url("/messages")
        } else {
            // A line is configured, so the chat is created on that line.
            body["from"] = json!(self.from);
            self.url("/chats")
        };
        let response = self.post(&url, &body, "send message").await?;
        Ok(response
            .body
            .pointer("/message/id")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    /// Pull an image part's bytes so the model can see it.
    ///
    /// Attachment URLs are already signed, so no credentials are needed beyond
    /// the URL itself; anything that is not an image stays a reference.
    async fn hydrate(&self, media: &mut [MediaRef]) {
        for attachment in media.iter_mut() {
            if attachment.kind != MediaKind::Image {
                continue;
            }
            let Some(url) = attachment.url.clone() else {
                continue;
            };
            match self.download(&url).await {
                Ok(bytes) => attachment.data = Some(bytes),
                Err(error) => {
                    // The words of the message still matter; a failed download
                    // must not cost the user an answer.
                    tracing::warn!(%error, %url, "linq attachment could not be downloaded");
                }
            }
        }
    }

    async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| anyhow!("download failed: {error}"))?;
        if !response.status().is_success() {
            anyhow::bail!("download failed: HTTP {}", response.status().as_u16());
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| anyhow!("download was truncated: {error}"))?
            .to_vec();
        if bytes.len() > MAX_DOWNLOAD_BYTES {
            anyhow::bail!("attachment is {} bytes, above the limit", bytes.len());
        }
        Ok(bytes)
    }
}

#[async_trait]
impl ChannelSender for LinqSender {
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
            anyhow::bail!("linq conversation has no chat id or handle");
        }
        // A chat the bridge has seen carries the platform's chat id; a bare
        // phone number is a conversation that does not exist yet.
        if target.starts_with('+') {
            self.send_to_handle(target, text).await
        } else {
            self.send_to_chat(target, text).await
        }
    }
}

// ─── Provider ───────────────────────────────────────────────────────────────

struct Linq;

#[async_trait]
impl Provider for Linq {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        let config: LinqConfig = ctx.config()?;
        config.require_api_key(ctx.id())?;
        Ok(Arc::new(LinqSender::new(ctx.http().clone(), &config)))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        let config: LinqConfig = ctx.config()?;
        config.require_api_key(ctx.id())?;
        config.require_signing_secret(ctx.id())?;
        let sender = Arc::new(LinqSender::new(ctx.http().clone(), &config));
        let secret = config.webhook.signing_secret.clone();
        let (addr, path) = (
            config.webhook_addr().to_string(),
            config.webhook_path().to_string(),
        );

        let handler_ctx = ctx.clone();
        let server = WebhookServer::bind(&addr)
            .await?
            .on("POST", &path, move |request| {
                let ctx = handler_ctx.clone();
                let sender = sender.clone();
                let secret = secret.clone();
                async move {
                    // The signature covers the moment it was made, so it is checked
                    // against the clock at arrival, not at startup.
                    let now = crate::bridge::dedup::now_ms() / 1000;
                    if !verify_delivery(&request, &secret, now) {
                        tracing::warn!(
                            channel = "linq",
                            "rejected a delivery whose signature did not verify"
                        );
                        return WebhookResponse::unauthorized();
                    }
                    let body = request.json();
                    // Answer first, work after: a slow download or a turn must not
                    // hold the platform's request open past its timeout.
                    tokio::spawn(async move {
                        deliver(&ctx, &sender, body).await;
                    });
                    WebhookResponse::ok()
                }
            });
        ctx.mark_running();
        tracing::info!(addr = %addr, path = %path, "linq webhook listening");
        server.serve(ctx.shutdown().clone()).await
    }

    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        let config: LinqConfig = ctx.config()?;
        config.require_api_key(ctx.id())?;
        // Listing the account's lines proves the key and says whether the
        // configured sender can actually send.
        let sender = LinqSender::new(ctx.http().clone(), &config);
        let authorization = sender.authorization();
        let response = send_json(
            ctx.http(),
            reqwest::Method::GET,
            &sender.url("/phone_numbers"),
            &[("Authorization", authorization.as_str())],
            None,
            RetryPolicy::single_attempt(),
        )
        .await?;
        if !response.is_success() {
            return Err(send_error("phone numbers", &response));
        }
        let numbers = response
            .body
            .get("phone_numbers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let Some(first) = numbers.first() else {
            anyhow::bail!("linq account has no phone numbers assigned");
        };
        let number = first
            .get("phone_number")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("linq phone number entry has no number"))?;
        let reputation = first
            .pointer("/reputation/status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if config.from.is_empty() {
            return Ok(format!("connected as {number} ({reputation})"));
        }
        let configured = numbers.iter().any(|entry| {
            entry.get("phone_number").and_then(Value::as_str) == Some(config.from.as_str())
        });
        if !configured {
            anyhow::bail!(
                "the configured `from` {} is not assigned to this account ({number} is)",
                config.from
            );
        }
        Ok(format!(
            "connected as {number} ({reputation}); `from` {} is assigned",
            config.from
        ))
    }
}

/// Turn one verified delivery into the bridge pipeline.
async fn deliver(ctx: &ProviderCtx, sender: &Arc<LinqSender>, body: Value) {
    let outbound: Arc<dyn ChannelSender> = sender.clone();
    for mut inbound in parse_deliveries(&body) {
        sender.hydrate(&mut inbound.media).await;
        let outcome = ctx.handle(inbound, outbound.clone()).await;
        tracing::debug!(?outcome, "linq message handled");
    }
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(Linq)
}

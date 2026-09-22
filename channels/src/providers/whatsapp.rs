//! WhatsApp: the official Cloud API.
//!
//! The Cloud API is webhook-driven: the platform verifies the endpoint once
//! with a `hub.challenge` echo, then POSTs signed message events. There is no
//! inbound socket to hold open, so `run` is just "serve the webhook until
//! shutdown".
//!
//! * **Inbound** — a [`WebhookServer`] answering the verification GET and the
//!   delivery POST. Every delivery is signed with the app secret over the raw
//!   body (`X-Hub-Signature-256`); an unsigned or mis-signed body is rejected
//!   before it is parsed, because a webhook that becomes an agent prompt is
//!   otherwise an open door.
//! * **Outbound** — `POST /{phone_number_id}/messages` on the Graph API. The
//!   bridge splits text to the platform's 4096-character limit; `react` sends a
//!   reaction and nothing here chunks or throttles.
//! * **Addressing** — the Cloud API talks to one customer at a time, so every
//!   message is a direct conversation and the policy gate is the DM one.
//! * **Media** — an inbound attachment arrives as a *media id*; the bytes are
//!   behind two authenticated Graph calls, so they are resolved by the provider
//!   before the bridge sees the message.
//! * **Errors** — a rejected send is classified from Meta's numeric error code
//!   where the meaning is unambiguous. Code 131047 means the 24-hour customer
//!   service window has closed: the customer has to write again first, so that
//!   send can never succeed and is reported as permanent. A rate limit
//!   (130429/131048/80007) is transient and the shared HTTP helper paces the
//!   retry.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
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
use crate::transport::signature::{constant_time_eq, hmac_sha256_hex, verify_hex};
use crate::transport::webhook::{WebhookRequest, WebhookResponse, WebhookServer};
use crate::transport::LengthUnit;

#[cfg(test)]
#[path = "whatsapp_tests.rs"]
mod tests;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "whatsapp",
    display_name: "WhatsApp (Cloud API)",
    description: "WhatsApp Business Cloud API: verified webhook inbound, Graph API outbound.",
    docs: "docs/guide/channels-whatsapp.md",
    maturity: Maturity::Preview,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: false,
        threads: false,
        typing: false,
        reactions: true,
        media_in: true,
        media_out: false,
        mention_gate: false,
    },
    max_text_len: 4096,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "phone_number_id": "",
  "access_token": "",
  "verify_token": "",
  "app_secret": "",
  "webhook": { "addr": "127.0.0.1:8788", "path": "/webhooks/whatsapp" },
  "dm_policy": "allowlist",
  "dm_allowlist": []
}"#,
    requires: &["a Meta WhatsApp Business app and a publicly reachable webhook URL"],
};

/// The Graph API origin; `api_base` in the config is the test seam.
const DEFAULT_API_BASE: &str = "https://graph.facebook.com";
/// Graph API version the request paths are built against.
const DEFAULT_API_VERSION: &str = "v21.0";
const DEFAULT_WEBHOOK_ADDR: &str = "127.0.0.1:8788";
const DEFAULT_WEBHOOK_PATH: &str = "/webhooks/whatsapp";
/// Largest attachment this provider will pull into model input.
const MAX_DOWNLOAD_BYTES: usize = 16 * 1024 * 1024;
/// Marks a media reference whose bytes still have to be fetched, so the
/// placeholder id survives in `raw` without being mistaken for a real URL.
const MEDIA_URL_PREFIX: &str = "wa-media:";

/// The `providers.whatsapp` block.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WhatsappConfig {
    /// The business phone number the messages are sent from.
    pub phone_number_id: String,
    /// Permanent token for the Graph API.
    pub access_token: String,
    /// Echoed back during the one-time endpoint verification.
    pub verify_token: String,
    /// Meta app secret; every delivery is signed with it.
    pub app_secret: String,
    pub webhook: WebhookConfig,
    /// Test seam: replaces the Graph API origin.
    pub api_base: String,
    /// Test seam: replaces the Graph API version segment.
    pub api_version: String,
}

impl WhatsappConfig {
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

    /// Sending needs the number the messages belong to and a token to send them.
    fn require_credentials(&self, channel: &str) -> Result<()> {
        if self.phone_number_id.trim().is_empty() || self.access_token.trim().is_empty() {
            anyhow::bail!(
                "invalid `providers.{channel}` configuration: `phone_number_id` and `access_token` are required"
            );
        }
        Ok(())
    }

    /// Receiving needs both halves of the handshake. Refusing to start without
    /// them is deliberate: an unverified, unsigned webhook would let anyone who
    /// can reach the port drive the agent.
    fn require_webhook_secrets(&self, channel: &str) -> Result<()> {
        if self.verify_token.trim().is_empty() || self.app_secret.trim().is_empty() {
            anyhow::bail!(
                "invalid `providers.{channel}` configuration: `verify_token` and `app_secret` are required to receive"
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
}

/// Whether one delivery's signature is genuine.
///
/// The digest covers the *raw* body: re-serializing the parsed JSON would
/// change key order and whitespace, and nothing would match. An empty secret is
/// not a wildcard — a provider without one refuses to start.
fn verify_signature(request: &WebhookRequest, app_secret: &str) -> bool {
    if app_secret.is_empty() {
        return false;
    }
    let Some(presented) = request.header("x-hub-signature-256") else {
        return false;
    };
    let expected = hmac_sha256_hex(app_secret.as_bytes(), &request.body);
    verify_hex(&expected, presented)
}

/// The one-time endpoint verification.
///
/// The platform sends `hub.mode=subscribe`, the token the subscription was
/// created with, and a challenge; echoing the challenge back as plain text is
/// what marks the endpoint verified.
fn verify_endpoint(request: &WebhookRequest, verify_token: &str) -> WebhookResponse {
    if request.method != "GET" {
        return WebhookResponse::not_found();
    }
    let mode = request.query_param("hub.mode").unwrap_or_default();
    let presented = request.query_param("hub.verify_token").unwrap_or_default();
    let challenge = request.query_param("hub.challenge").unwrap_or_default();
    if mode != "subscribe" {
        return WebhookResponse::text(403, "unexpected mode");
    }
    // Constant-time: the token is a shared secret, and a byte-by-byte
    // comparison leaks how much of a guess was right.
    if verify_token.is_empty() || !constant_time_eq(presented.as_bytes(), verify_token.as_bytes()) {
        return WebhookResponse::text(403, "verify token mismatch");
    }
    if challenge.is_empty() {
        return WebhookResponse::bad_request("no challenge");
    }
    WebhookResponse::text(200, challenge)
}

/// Every inbound message in one delivery.
///
/// A delivery is an envelope of entries, each with changes, each with a value
/// that may carry `messages`, `statuses` (delivery receipts), errors, or
/// nothing at all. Only messages become prompts.
fn parse_deliveries(body: &Value) -> Vec<Inbound> {
    let Some(entries) = body.get("entry").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries {
        let Some(changes) = entry.get("changes").and_then(Value::as_array) else {
            continue;
        };
        for change in changes {
            let Some(value) = change.get("value") else {
                continue;
            };
            out.extend(parse_value(value));
        }
    }
    out
}

/// The messages in one change value, with the contact names that came with
/// them (the display name lives beside the message, not inside it).
fn parse_value(value: &Value) -> Vec<Inbound> {
    let names: Vec<(String, String)> = value
        .get("contacts")
        .and_then(Value::as_array)
        .map(|contacts| {
            contacts
                .iter()
                .filter_map(|contact| {
                    Some((
                        contact.get("wa_id")?.as_str()?.to_string(),
                        contact.get("profile")?.get("name")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let Some(messages) = value.get("messages").and_then(Value::as_array) else {
        return Vec::new();
    };
    messages
        .iter()
        .filter_map(|message| parse_message(message, &names))
        .collect()
}

/// One message of any type.
///
/// Types that carry no prompt for the agent — reactions, location, contacts,
/// order updates, and the platform's own bookkeeping — are dropped here rather
/// than turned into an empty turn.
fn parse_message(message: &Value, names: &[(String, String)]) -> Option<Inbound> {
    let message_type = message.get("type")?.as_str()?;
    let message_id = message.get("id")?.as_str()?.to_string();
    let from = message
        .get("from")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if from.is_empty() {
        return None;
    }
    let (text, media) = content_of(message, message_type);
    if text.trim().is_empty() && media.is_empty() {
        return None;
    }
    let display = names
        .iter()
        .find(|(wa_id, _)| wa_id == &from)
        .map(|(_, name)| name.clone());
    Some(Inbound {
        message_id,
        sender: SenderRef {
            id: from.clone(),
            display,
        },
        // One customer per number: the sender *is* the conversation.
        conversation: ConversationRef {
            id: from,
            thread_id: None,
            kind: ChatKind::Direct,
        },
        text,
        media,
        addressed_to_bot: true,
        // The timestamp is a string of seconds; a bad one is left absent
        // rather than invented, so the bridge does not drop a live message as
        // a replay.
        created_at_ms: message
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|seconds| seconds.parse::<i64>().ok())
            .map(|seconds| seconds.saturating_mul(1000)),
        raw: Some(message.clone()),
    })
}

/// The text and attachments one message type carries.
fn content_of(message: &Value, message_type: &str) -> (String, Vec<MediaRef>) {
    match message_type {
        "text" => (
            message
                .get("text")
                .and_then(|text| text.get("body"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            Vec::new(),
        ),
        // A tapped button or list row arrives as its own message type; the
        // label is what the customer chose, so that is the prompt. The id is
        // the platform's routing token and stays in `raw`.
        "interactive" => {
            let reply = message
                .get("interactive")
                .and_then(|interactive| {
                    interactive
                        .get("button_reply")
                        .or_else(|| interactive.get("list_reply"))
                })
                .and_then(|reply| {
                    reply
                        .get("title")
                        .and_then(Value::as_str)
                        .or_else(|| reply.get("id").and_then(Value::as_str))
                })
                .unwrap_or_default()
                .to_string();
            (reply, Vec::new())
        }
        // A quick-reply button on a template message.
        "button" => (
            message
                .get("button")
                .and_then(|button| button.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            Vec::new(),
        ),
        // Attachments carry their words in a caption, which the platform
        // attaches to the media object (and, on some versions, to the message
        // itself).
        "image" | "video" | "audio" | "document" | "sticker" => (
            message
                .get(message_type)
                .and_then(|media| media.get("caption"))
                .and_then(Value::as_str)
                .or_else(|| message.get("caption").and_then(Value::as_str))
                .unwrap_or_default()
                .to_string(),
            media_of(message, message_type).into_iter().collect(),
        ),
        _ => (String::new(), Vec::new()),
    }
}

/// The attachment on a media message, referenced by id.
///
/// The bytes live behind two authenticated Graph calls; the id is carried as a
/// placeholder and resolved before the message reaches the bridge.
fn media_of(message: &Value, message_type: &str) -> Option<MediaRef> {
    let media = message.get(message_type)?;
    let media_id = media.get("id").and_then(Value::as_str)?;
    let content_type = media
        .get("mime_type")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(MediaRef {
        kind: media_kind(message_type, content_type.as_deref()),
        filename: media
            .get("filename")
            .and_then(Value::as_str)
            .map(str::to_string),
        content_type,
        url: Some(format!("{MEDIA_URL_PREFIX}{media_id}")),
        data: None,
    })
}

/// Map the platform's message type onto the bridge's media vocabulary.
fn media_kind(message_type: &str, content_type: Option<&str>) -> MediaKind {
    match message_type {
        // A sticker renders as an image; the model can look at it.
        "image" | "sticker" => MediaKind::Image,
        "video" => MediaKind::Video,
        "audio" | "voice" => MediaKind::Audio,
        "document" => MediaKind::Document,
        _ => match content_type.unwrap_or_default() {
            mime if mime.starts_with("image/") => MediaKind::Image,
            mime if mime.starts_with("video/") => MediaKind::Video,
            mime if mime.starts_with("audio/") => MediaKind::Audio,
            _ => MediaKind::Unknown,
        },
    }
}

/// Error codes whose meaning is unambiguous enough to classify here.
///
/// Everything else is left to the HTTP status, which the shared helper already
/// understands.
fn classify_error_code(code: i64) -> Option<ErrorClass> {
    match code {
        // The access token is wrong, expired, or revoked.
        190 | 102 => Some(ErrorClass::Permanent),
        // The customer service window closed: the customer must write again
        // before this recipient can be messaged outside a template.
        131047 => Some(ErrorClass::Permanent),
        // The recipient cannot receive the message at all.
        131026 | 131052 => Some(ErrorClass::Permanent),
        // The number is not a WhatsApp account.
        133010 | 131009 => Some(ErrorClass::Permanent),
        // The message was held back as spam or hit a per-pair limit.
        131048 | 131056 | 130429 | 80007 | 4 => Some(ErrorClass::Transient),
        _ => None,
    }
}

/// The numeric code Meta put in an error body.
fn error_code(response: &HttpResponse) -> Option<i64> {
    response.body.get("error")?.get("code")?.as_i64()
}

/// Turn a rejected response into an error that says which class it is.
fn send_error(action: &str, response: &HttpResponse) -> anyhow::Error {
    let code = error_code(response);
    let class = code
        .and_then(classify_error_code)
        .unwrap_or_else(|| response.class());
    let hint = match code {
        Some(131047) => {
            " (the 24-hour customer service window has closed; the customer must write first)"
        }
        Some(131026) => " (the recipient cannot receive this message)",
        Some(190) => " (the access token is invalid or expired)",
        _ => "",
    };
    let label = class.label();
    anyhow!(
        "whatsapp `{action}` rejected ({label}): {}{hint}",
        response.error_message()
    )
}

/// The outbound half.
pub struct WhatsappSender {
    client: reqwest::Client,
    base: String,
    version: String,
    phone_number_id: String,
    access_token: String,
}

impl WhatsappSender {
    fn new(client: reqwest::Client, config: &WhatsappConfig) -> Self {
        Self {
            client,
            base: config.api_base().to_string(),
            version: config.api_version().to_string(),
            phone_number_id: config.phone_number_id.clone(),
            access_token: config.access_token.clone(),
        }
    }

    fn authorization(&self) -> String {
        format!("Bearer {}", self.access_token)
    }

    fn messages_url(&self) -> String {
        format!(
            "{}/{}/{}/messages",
            self.base, self.version, self.phone_number_id
        )
    }

    /// Send one already-chunked message.
    async fn send_chunk(&self, to: &str, text: &str) -> Result<Option<String>> {
        let body = json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": to,
            "type": "text",
            // Links inside a reply are the agent's answer, not an advertisement;
            // letting the platform preview them keeps the reply readable.
            "text": { "preview_url": true, "body": text },
        });
        let authorization = self.authorization();
        let response = send_json(
            &self.client,
            reqwest::Method::POST,
            &self.messages_url(),
            &[("Authorization", authorization.as_str())],
            Some(&body),
            RetryPolicy::default(),
        )
        .await
        .map_err(|error| anyhow!("whatsapp `messages` failed: {error}"))?;
        if !response.is_success() {
            return Err(send_error("messages", &response));
        }
        Ok(message_id_of(&response.body))
    }

    /// Send a reaction to one message. An empty emoji removes it.
    async fn send_reaction(&self, to: &str, message_id: &str, emoji: &str) -> Result<()> {
        let body = json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": to,
            "type": "reaction",
            "reaction": { "message_id": message_id, "emoji": emoji },
        });
        let authorization = self.authorization();
        let response = send_json(
            &self.client,
            reqwest::Method::POST,
            &self.messages_url(),
            &[("Authorization", authorization.as_str())],
            Some(&body),
            RetryPolicy::single_attempt(),
        )
        .await?;
        if !response.is_success() {
            return Err(send_error("reaction", &response));
        }
        Ok(())
    }

    /// Mark one inbound message as read.
    ///
    /// Worth its own call: an unread message in the business inbox is what an
    /// operator sees as unanswered, and the platform turns the receipt into the
    /// customer's own "read" indicator.
    async fn mark_read(&self, message_id: &str) -> Result<()> {
        let body = json!({
            "messaging_product": "whatsapp",
            "status": "read",
            "message_id": message_id,
        });
        let authorization = self.authorization();
        let response = send_json(
            &self.client,
            reqwest::Method::POST,
            &self.messages_url(),
            &[("Authorization", authorization.as_str())],
            Some(&body),
            RetryPolicy::single_attempt(),
        )
        .await?;
        if !response.is_success() {
            return Err(send_error("mark read", &response));
        }
        Ok(())
    }

    /// Fetch the bytes behind one media id.
    ///
    /// Two hops: the media id resolves to a short-lived signed CDN URL, and
    /// that URL still requires the bearer token. The URL expires in minutes, so
    /// it is never cached anywhere.
    async fn fetch_media(
        &self,
        media_id: &str,
    ) -> Result<(Vec<u8>, Option<String>, Option<String>)> {
        let authorization = self.authorization();
        let metadata = send_json(
            &self.client,
            reqwest::Method::GET,
            &format!("{}/{}/{}", self.base, self.version, media_id),
            &[("Authorization", authorization.as_str())],
            None,
            RetryPolicy::default(),
        )
        .await
        .map_err(|error| anyhow!("whatsapp media lookup failed: {error}"))?;
        if !metadata.is_success() {
            return Err(send_error("media", &metadata));
        }
        let url = metadata
            .body
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("whatsapp media lookup returned no url"))?
            .to_string();
        let content_type = metadata
            .body
            .get("mime_type")
            .and_then(Value::as_str)
            .map(str::to_string);
        let filename = metadata
            .body
            .get("file_name")
            .and_then(Value::as_str)
            .map(str::to_string);

        let response = self
            .client
            .get(&url)
            .header("Authorization", authorization)
            .send()
            .await
            .map_err(|error| anyhow!("whatsapp media download failed: {error}"))?;
        if !response.status().is_success() {
            anyhow::bail!(
                "whatsapp media download failed: HTTP {}",
                response.status().as_u16()
            );
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| anyhow!("whatsapp media download was truncated: {error}"))?
            .to_vec();
        if bytes.len() > MAX_DOWNLOAD_BYTES {
            anyhow::bail!(
                "whatsapp media is {} bytes, above the {MAX_DOWNLOAD_BYTES}-byte limit",
                bytes.len()
            );
        }
        Ok((bytes, content_type, filename))
    }

    /// Replace every image placeholder with the real bytes.
    ///
    /// Only images are fetched: the bridge turns any attachment carrying bytes
    /// into model image input, so downloading a document would hand the model a
    /// PDF labelled as a picture. Other kinds stay references.
    ///
    /// A failed download keeps the reference: the words of the message still
    /// matter, and a missing attachment should not cost the user an answer.
    async fn hydrate(&self, media: &mut [MediaRef]) {
        for attachment in media.iter_mut() {
            if attachment.kind != MediaKind::Image {
                continue;
            }
            let Some(media_id) = attachment
                .url
                .as_deref()
                .and_then(|url| url.strip_prefix(MEDIA_URL_PREFIX))
                .map(str::to_string)
            else {
                continue;
            };
            match self.fetch_media(&media_id).await {
                Ok((bytes, content_type, filename)) => {
                    attachment.data = Some(bytes);
                    if attachment.content_type.is_none() {
                        attachment.content_type = content_type;
                    }
                    if attachment.filename.is_none() {
                        attachment.filename = filename;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, media = %media_id, "whatsapp attachment could not be fetched");
                }
            }
        }
    }
}

/// The platform message id out of a successful send.
fn message_id_of(body: &Value) -> Option<String> {
    body.get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| messages.first())
        .and_then(|message| message.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[async_trait]
impl ChannelSender for WhatsappSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        if conversation.id.trim().is_empty() {
            anyhow::bail!("whatsapp conversation has no recipient");
        }
        self.send_chunk(&conversation.id, text).await
    }

    async fn typing(&self, _conversation: &ConversationRef) -> Result<()> {
        // The Cloud API has no typing indicator for a bot; its clients show
        // "typing…" only while a human agent types. Claiming otherwise would
        // turn every turn into a rejected request, so the capability stays off
        // and this is an honest no-op.
        Ok(())
    }

    async fn react(
        &self,
        conversation: &ConversationRef,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        self.send_reaction(&conversation.id, message_id, emoji)
            .await
    }
}

/// The WhatsApp provider.
struct Whatsapp;

#[async_trait]
impl Provider for Whatsapp {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        let config: WhatsappConfig = ctx.config()?;
        config.require_credentials(ctx.id())?;
        Ok(Arc::new(WhatsappSender::new(ctx.http().clone(), &config)))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        let config: WhatsappConfig = ctx.config()?;
        config.require_credentials(ctx.id())?;
        config.require_webhook_secrets(ctx.id())?;
        // The concrete sender is needed for the attachment and receipt calls
        // the bridge does not know about; the same object is handed to the
        // bridge as a `ChannelSender`.
        let sender = Arc::new(WhatsappSender::new(ctx.http().clone(), &config));

        let verify_token = config.verify_token.clone();
        let app_secret = config.app_secret.clone();
        let (addr, path) = (
            config.webhook_addr().to_string(),
            config.webhook_path().to_string(),
        );

        let post_ctx = ctx.clone();
        let server = WebhookServer::bind(&addr)
            .await?
            .on("GET", &path, move |request| {
                let verify_token = verify_token.clone();
                async move { verify_endpoint(&request, &verify_token) }
            })
            .on("POST", &path, move |request| {
                let ctx = post_ctx.clone();
                let sender = sender.clone();
                // The handler is `Fn`: every delivery clones the secret rather
                // than consuming the one captured at startup.
                let app_secret = app_secret.clone();
                async move {
                    if !verify_signature(&request, &app_secret) {
                        tracing::warn!(
                            channel = "whatsapp",
                            "rejected a delivery whose signature did not verify"
                        );
                        return WebhookResponse::unauthorized();
                    }
                    let body = request.json();
                    // Answer first, work after: the platform retries anything
                    // that is not answered quickly, and resolving an attachment
                    // or running a turn takes longer than it will wait.
                    tokio::spawn(async move {
                        deliver(&ctx, &sender, body).await;
                    });
                    WebhookResponse::ok()
                }
            });
        ctx.mark_running();
        tracing::info!(addr = %addr, path = %path, "whatsapp webhook listening");
        server.serve(ctx.shutdown().clone()).await
    }

    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        let config: WhatsappConfig = ctx.config()?;
        config.require_credentials(ctx.id())?;
        let authorization = format!("Bearer {}", config.access_token);
        // Reading the number back is the smallest request that proves the
        // token, the number id and the app belong together.
        let response = send_json(
            ctx.http(),
            reqwest::Method::GET,
            &format!(
                "{}/{}/{}?fields=display_phone_number,verified_name",
                config.api_base(),
                config.api_version(),
                config.phone_number_id
            ),
            &[("Authorization", authorization.as_str())],
            None,
            RetryPolicy::single_attempt(),
        )
        .await?;
        if !response.is_success() {
            return Err(send_error("probe", &response));
        }
        let number = response
            .body
            .get("display_phone_number")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("whatsapp number lookup returned no display_phone_number"))?;
        let name = response
            .body
            .get("verified_name")
            .and_then(Value::as_str)
            .unwrap_or("");
        if name.is_empty() {
            Ok(format!("connected as {number}"))
        } else {
            Ok(format!("connected as {number} ({name})"))
        }
    }
}

/// Turn one verified delivery into the bridge pipeline.
///
/// Runs after the HTTP response, so it may resolve attachments and wait for the
/// agent. The read receipt is sent only once the message was actually accepted,
/// which keeps the platform's unread count honest.
async fn deliver(ctx: &ProviderCtx, outbound: &Arc<WhatsappSender>, body: Value) {
    let sender: Arc<dyn ChannelSender> = outbound.clone();
    for mut inbound in parse_deliveries(&body) {
        let message_id = inbound.message_id.clone();
        outbound.hydrate(&mut inbound.media).await;
        let outcome = ctx.handle(inbound, sender.clone()).await;
        if outcome.is_accepted() {
            if let Err(error) = outbound.mark_read(&message_id).await {
                // A missing receipt is not worth failing the turn over.
                tracing::debug!(%error, "whatsapp read receipt was not accepted");
            }
        }
        tracing::debug!(?outcome, "whatsapp message handled");
    }
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(Whatsapp)
}

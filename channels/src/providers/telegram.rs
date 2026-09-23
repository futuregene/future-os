//! Telegram: a bot on the Bot API, over long polling or a webhook.
//!
//! * **Inbound** — `getUpdates` long polling with a persisted offset (so a
//!   restart does not replay or lose what the platform queued), or a webhook
//!   served by [`crate::transport::webhook`] when the deployment can expose a
//!   public endpoint. Only plain `message` updates are consumed; edited
//!   messages, channel posts and service events are ignored by design.
//! * **Outbound** — `sendMessage` (the bridge splits text to the 4096-character
//!   limit on character boundaries), `editMessageText` for progressive
//!   streaming, `sendChatAction` for the typing signal, `setMessageReaction`
//!   for acknowledgements. Replies keep Markdown readable across platforms, so
//!   markup is escaped to Telegram's MarkdownV2 dialect before sending; a
//!   message Telegram refuses as malformed is retried once as plain text.
//! * **Addressing** — a group message counts as addressed when the bot's
//!   username appears in a mention entity or the message replies to one of the
//!   bot's own messages. Direct messages are always addressed.
//! * **Media** — photos and documents are resolved through `getFile` and
//!   downloaded into the channel's data directory; image bytes become model
//!   input via the bridge.
//! * **Errors** — a 429 is transient and retried by the shared HTTP helper's
//!   backoff; the `retry_after` hint Telegram puts in the response *body* (not
//!   a header) is surfaced in the error text. A 403 means the bot is blocked
//!   or removed and is permanent.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

use crate::bridge::{
    ChatKind, ConversationRef, Inbound, MediaKind, MediaRef, ProviderCtx, SenderRef,
};
use crate::providers::traits::{
    Capabilities, ChannelDefinition, ChannelSender, Maturity, Provider,
};
use crate::transport::http::{send_json, HttpResponse, RetryPolicy};
use crate::transport::webhook::{WebhookRequest, WebhookResponse, WebhookServer};
use crate::transport::LengthUnit;

#[cfg(test)]
#[path = "telegram_tests.rs"]
mod tests;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "telegram",
    display_name: "Telegram",
    description: "Telegram bot: long polling or webhook, streamed replies, group mention gate.",
    docs: "docs/guide/channels-providers.md#telegram",
    maturity: Maturity::Preview,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: true,
        threads: false,
        typing: true,
        reactions: true,
        media_in: true,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 4096,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "bot_token": "",
  "mode": "long_poll",
  "webhook": { "addr": "127.0.0.1:8787", "path": "/telegram", "secret_token": "" },
  "dm_policy": "allowlist",
  "dm_allowlist": [],
  "group_policy": "disabled",
  "group_allowlist": [],
  "require_mention": true
}"#,
    requires: &["a bot token from @BotFather"],
};

/// The default Bot API origin. `api_base` in the config is the test seam; a
/// real deployment should leave it empty.
const DEFAULT_API_BASE: &str = "https://api.telegram.org";
/// Long-poll timeout handed to `getUpdates`; the platform holds the request
/// open this long when nothing is queued.
const LONG_POLL_TIMEOUT_S: u64 = 30;
/// Wait between polls after a failure — long-poll errors are usually the
/// network, not the platform, so a short fixed pause beats hammering.
const POLL_ERROR_BACKOFF: Duration = Duration::from_secs(2);
/// Largest file the Bot API will hand back through `getFile`.
const MAX_DOWNLOAD_BYTES: usize = 20 * 1024 * 1024;
/// File holding the persisted update offset.
const OFFSET_FILE: &str = "offset.json";
/// Where the webhook listens when the config names no address.
const DEFAULT_WEBHOOK_ADDR: &str = "127.0.0.1:8787";
/// The path the webhook answers on when the config names none.
const DEFAULT_WEBHOOK_PATH: &str = "/telegram";

/// The `providers.telegram` block.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TelegramConfig {
    pub bot_token: String,
    /// `long_poll` (default) or `webhook`.
    pub mode: String,
    pub webhook: WebhookConfig,
    /// Test seam: replaces the Bot API origin (mock servers, Bot API test env).
    pub api_base: String,
}

impl TelegramConfig {
    fn api_base(&self) -> &str {
        let base = if self.api_base.is_empty() {
            DEFAULT_API_BASE
        } else {
            self.api_base.as_str()
        };
        base.trim_end_matches('/')
    }

    fn require_token(&self, channel: &str) -> Result<()> {
        if self.bot_token.trim().is_empty() {
            anyhow::bail!("invalid `providers.{channel}` configuration: `bot_token` is required");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WebhookConfig {
    pub addr: String,
    pub path: String,
    /// Registered with `setWebhook`; Telegram repeats it in the
    /// `X-Telegram-Bot-Api-Secret-Token` header of every delivery.
    pub secret_token: String,
}

/// Where the webhook listens.
///
/// Split out of the run loop so both branches — the default and an explicit
/// address — are asserted without depending on whether the default port happens
/// to be free on the machine running the test.
fn webhook_addr(config: &TelegramConfig) -> &str {
    if config.webhook.addr.is_empty() {
        DEFAULT_WEBHOOK_ADDR
    } else {
        config.webhook.addr.as_str()
    }
}

/// The path the webhook answers on, defaulting to `/telegram`.
fn webhook_path(config: &TelegramConfig) -> &str {
    if config.webhook.path.is_empty() {
        DEFAULT_WEBHOOK_PATH
    } else {
        config.webhook.path.as_str()
    }
}

/// One Bot API method URL.
fn method_url(base: &str, token: &str, method: &str) -> String {
    format!("{base}/bot{token}/{method}")
}

/// One file-download URL.
fn file_url(base: &str, token: &str, file_path: &str) -> String {
    format!("{base}/file/bot{token}/{file_path}")
}

/// Escape plain text for Telegram's MarkdownV2 dialect.
///
/// Telegram rejects a message whose markup is malformed; since replies can
/// contain any platform's markup, everything special is escaped and the text
/// renders literally. Inside a pre/code block only backtick and backslash are
/// escaped, per the dialect's rules — this sender never emits those blocks, so
/// the plain-text rules apply throughout.
pub fn escape_markdown_v2(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    for ch in text.chars() {
        match ch {
            '_' | '*' | '[' | ']' | '(' | ')' | '~' | '`' | '>' | '#' | '+' | '-' | '=' | '|'
            | '{' | '}' | '.' | '!' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Whether one Bot API error body means the text was rejected as malformed.
fn is_parse_error(response: &HttpResponse) -> bool {
    response
        .body
        .get("description")
        .and_then(Value::as_str)
        .is_some_and(|description| description.contains("can't parse entities"))
}

/// Whether one Bot API error body means the edit changed nothing — a routine
/// outcome when two streamed snapshots render identically, not a failure.
fn is_edit_no_change(response: &HttpResponse) -> bool {
    response
        .body
        .get("description")
        .and_then(Value::as_str)
        .is_some_and(|description| description.contains("message is not modified"))
}

/// Call one Bot API method and unwrap the envelope (`{"ok":bool,"result":..}`).
///
/// A 429 body carries `retry_after` in JSON rather than a header; it is copied
/// onto the response so the shared helper paces the retry instead of guessing.
async fn call_api(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    method: &str,
    body: &Value,
    policy: RetryPolicy,
) -> Result<Value> {
    let url = method_url(base, token, method);
    let response = send_json(client, reqwest::Method::POST, &url, &[], Some(body), policy)
        .await
        .map_err(|error| anyhow!("telegram `{method}` failed: {error}"))?;
    // Telegram puts its rate-limit hint in the 429 *body* (`parameters.
    // retry_after`), not a header, so the helper's pacing never saw it; the
    // hint is surfaced in the error text instead (see handoff).
    let ok = response
        .body
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !response.is_success() || !ok {
        let mut message = response.error_message();
        if let Some(seconds) = retry_after_hint(&response.body) {
            message = format!("{message} (retry after {seconds}s)");
        }
        let class = response.class();
        return Err(anyhow!(
            "telegram `{method}` rejected ({class:?}): {message}"
        ));
    }
    Ok(response.body.get("result").cloned().unwrap_or(Value::Null))
}

/// The `parameters.retry_after` hint a 429 body carries, when present.
fn retry_after_hint(body: &Value) -> Option<u64> {
    body.get("parameters")
        .and_then(|parameters| parameters.get("retry_after"))
        .and_then(Value::as_u64)
        .map(|seconds| seconds.min(600))
}

/// The Bot API envelope error as an anyhow error that carries the class.
fn send_error(method: &str, response: &HttpResponse) -> anyhow::Error {
    let mut message = response.error_message();
    if let Some(seconds) = retry_after_hint(&response.body) {
        message = format!("{message} (retry after {seconds}s)");
    }
    anyhow!(
        "telegram `{method}` rejected ({:?}): {}",
        response.class(),
        message
    )
}

/// The outbound half: one chat id per conversation, MarkdownV2 on the wire.
pub struct TelegramSender {
    client: reqwest::Client,
    base: String,
    token: String,
}

impl TelegramSender {
    fn new(client: reqwest::Client, config: &TelegramConfig) -> Self {
        Self {
            client,
            base: config.api_base().to_string(),
            token: config.bot_token.clone(),
        }
    }

    /// The chat a reply goes to. Telegram threads by reply, not by channel, so
    /// the conversation id *is* the chat id.
    fn chat_id(conversation: &ConversationRef) -> Result<&str> {
        if conversation.id.trim().is_empty() {
            anyhow::bail!("telegram conversation has no chat id");
        }
        Ok(conversation.id.as_str())
    }

    /// Send one already-chunked message, MarkdownV2-escaped, falling back to
    /// plain text when the platform rejects the markup.
    async fn send_chunk(&self, chat_id: &str, text: &str) -> Result<Option<String>> {
        let escaped = escape_markdown_v2(text);
        let body = json!({
            "chat_id": chat_id,
            "text": escaped,
            "parse_mode": "MarkdownV2",
        });
        let response = send_json(
            &self.client,
            reqwest::Method::POST,
            &method_url(&self.base, &self.token, "sendMessage"),
            &[],
            Some(&body),
            RetryPolicy::default(),
        )
        .await
        .map_err(|error| anyhow!("telegram `sendMessage` failed: {error}"))?;

        if response.is_success() {
            return Ok(message_id_of(&response.body));
        }
        if is_parse_error(&response) {
            // Markup Telegram cannot digest must not kill the reply: send the
            // text literally instead.
            let plain = json!({ "chat_id": chat_id, "text": text });
            let retried = send_json(
                &self.client,
                reqwest::Method::POST,
                &method_url(&self.base, &self.token, "sendMessage"),
                &[],
                Some(&plain),
                RetryPolicy::default(),
            )
            .await
            .map_err(|error| anyhow!("telegram `sendMessage` failed: {error}"))?;
            if retried.is_success() {
                return Ok(message_id_of(&retried.body));
            }
            return Err(send_error("sendMessage", &retried));
        }
        Err(send_error("sendMessage", &response))
    }
}

/// The platform message id out of a successful `sendMessage` result.
fn message_id_of(body: &Value) -> Option<String> {
    body.get("result")
        .and_then(|result| result.get("message_id"))
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
}

#[async_trait]
impl ChannelSender for TelegramSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        let chat_id = Self::chat_id(conversation)?;
        // Telegram's 4096 limit counts *visible* characters; MarkdownV2
        // escaping can add enough backslashes to overflow, so an escaped text
        // that no longer fits is re-chunked at half the limit — enough headroom
        // for any text, since escaping at most doubles a fully-reserved input.
        if escape_markdown_v2(text).chars().count() > DEFINITION.max_text_len {
            let mut last_id = None;
            for piece in
                crate::transport::chunk(text, DEFINITION.max_text_len / 2, LengthUnit::Chars)
            {
                last_id = self.send_chunk(chat_id, &piece).await?;
            }
            return Ok(last_id);
        }
        self.send_chunk(chat_id, text).await
    }

    async fn edit_text(
        &self,
        conversation: &ConversationRef,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let chat_id = Self::chat_id(conversation)?;
        let numeric_id: i64 = message_id
            .parse()
            .map_err(|_| anyhow!("telegram message id `{message_id}` is not numeric"))?;
        let visible = crate::transport::truncate(text, DEFINITION.max_text_len, LengthUnit::Chars);
        let escaped = escape_markdown_v2(&visible);
        let body = json!({
            "chat_id": chat_id,
            "message_id": numeric_id,
            "text": escaped,
            "parse_mode": "MarkdownV2",
        });
        let response = send_json(
            &self.client,
            reqwest::Method::POST,
            &method_url(&self.base, &self.token, "editMessageText"),
            &[],
            Some(&body),
            RetryPolicy::default(),
        )
        .await
        .map_err(|error| anyhow!("telegram `editMessageText` failed: {error}"))?;
        if response.is_success() || is_edit_no_change(&response) {
            return Ok(());
        }
        if is_parse_error(&response) {
            let plain = json!({
                "chat_id": chat_id,
                "message_id": numeric_id,
                "text": visible,
            });
            let retried = send_json(
                &self.client,
                reqwest::Method::POST,
                &method_url(&self.base, &self.token, "editMessageText"),
                &[],
                Some(&plain),
                RetryPolicy::default(),
            )
            .await
            .map_err(|error| anyhow!("telegram `editMessageText` failed: {error}"))?;
            if retried.is_success() || is_edit_no_change(&retried) {
                return Ok(());
            }
            return Err(send_error("editMessageText", &retried));
        }
        Err(send_error("editMessageText", &response))
    }

    async fn typing(&self, conversation: &ConversationRef) -> Result<()> {
        let chat_id = Self::chat_id(conversation)?;
        let body = json!({ "chat_id": chat_id, "action": "typing" });
        // A typing indicator must never take down a turn: one attempt, and an
        // error is the caller's log line, not an exception.
        let response = send_json(
            &self.client,
            reqwest::Method::POST,
            &method_url(&self.base, &self.token, "sendChatAction"),
            &[],
            Some(&body),
            RetryPolicy::single_attempt(),
        )
        .await?;
        if !response.is_success() {
            anyhow::bail!(
                "telegram `sendChatAction` rejected: {}",
                response.error_message()
            );
        }
        Ok(())
    }

    async fn react(
        &self,
        conversation: &ConversationRef,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let chat_id = Self::chat_id(conversation)?;
        let numeric_id: i64 = message_id
            .parse()
            .map_err(|_| anyhow!("telegram message id `{message_id}` is not numeric"))?;
        let body = json!({
            "chat_id": chat_id,
            "message_id": numeric_id,
            "reaction": [{ "type": "emoji", "emoji": emoji }],
        });
        let response = send_json(
            &self.client,
            reqwest::Method::POST,
            &method_url(&self.base, &self.token, "setMessageReaction"),
            &[],
            Some(&body),
            RetryPolicy::single_attempt(),
        )
        .await?;
        if !response.is_success() {
            anyhow::bail!(
                "telegram `setMessageReaction` rejected: {}",
                response.error_message()
            );
        }
        Ok(())
    }
}

/// Read the persisted update offset (0 = start wherever the platform says).
fn load_offset(data_dir: &std::path::Path) -> i64 {
    let path = data_dir.join(OFFSET_FILE);
    let Ok(content) = std::fs::read_to_string(path) else {
        return 0;
    };
    serde_json::from_str::<Value>(&content)
        .ok()
        .and_then(|value| value.get("offset").and_then(Value::as_i64))
        .unwrap_or(0)
}

/// Persist the offset so a restart resumes after the last consumed update.
fn store_offset(data_dir: &std::path::Path, offset: i64) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(OFFSET_FILE);
    std::fs::write(&path, json!({ "offset": offset }).to_string())?;
    Ok(())
}

/// What one update's message text is, stripped of the bot's own leading
/// mention so the prompt the agent sees reads naturally.
///
/// Entity offsets are UTF-16 code units, so the text is walked as UTF-16 to
/// find the cut point.
fn strip_mention(text: &str, mention_len_utf16: usize) -> String {
    if mention_len_utf16 == 0 {
        return text.trim().to_string();
    }
    let mut used = 0usize;
    let mut cut = text.len();
    for (offset, ch) in text.char_indices() {
        if used >= mention_len_utf16 {
            cut = offset;
            break;
        }
        used += ch.len_utf16();
    }
    if used < mention_len_utf16 {
        return text.trim().to_string();
    }
    text[cut..].trim().to_string()
}

/// One Telegram message entity, only the fields addressing needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Mention {
    offset_utf16: usize,
    length_utf16: usize,
    kind: String,
    username: Option<String>,
}

fn parse_entities(entities: Option<&Value>) -> Vec<Mention> {
    entities
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|entity| {
                    Some(Mention {
                        offset_utf16: entity.get("offset")?.as_u64()? as usize,
                        length_utf16: entity.get("length")?.as_u64()? as usize,
                        kind: entity.get("type")?.as_str()?.to_string(),
                        username: entity
                            .get("user")
                            .and_then(|user| user.get("username"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a mention entity addresses *this* bot.
fn mention_targets_bot(text: &str, mention: &Mention, bot_username: &str) -> bool {
    if bot_username.is_empty() {
        return false;
    }
    match mention.kind.as_str() {
        // A textual `@name`: compare the slice the entity covers.
        "mention" => {
            let slice = utf16_slice(text, mention.offset_utf16, mention.length_utf16);
            slice
                .strip_prefix('@')
                .is_some_and(|name| name.eq_ignore_ascii_case(bot_username))
        }
        // A text_mention carries the resolved user; compare usernames.
        "text_mention" => mention
            .username
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(bot_username)),
        _ => false,
    }
}

/// Extract `offset..offset+len` measured in UTF-16 code units.
fn utf16_slice(text: &str, offset_utf16: usize, len_utf16: usize) -> String {
    let mut out = String::new();
    let mut pos = 0usize;
    for ch in text.chars() {
        if pos >= offset_utf16 + len_utf16 {
            break;
        }
        if pos >= offset_utf16 {
            out.push(ch);
        }
        pos += ch.len_utf16();
    }
    out
}

/// Whether the message replies to a message the bot itself sent.
fn replies_to_bot(message: &Value, bot_username: &str, bot_id: i64) -> bool {
    let Some(reply) = message.get("reply_to_message") else {
        return false;
    };
    let from = reply.get("from");
    let by_id = from
        .and_then(|user| user.get("id"))
        .and_then(Value::as_i64)
        .is_some_and(|id| bot_id != 0 && id == bot_id);
    let by_name = from
        .and_then(|user| user.get("username"))
        .and_then(Value::as_str)
        .is_some_and(|name| !bot_username.is_empty() && name.eq_ignore_ascii_case(bot_username));
    by_id || by_name
}

/// The largest photo variant's file id, or a document's.
fn media_file_id(message: &Value) -> Option<(String, MediaKind, Option<String>)> {
    if let Some(photos) = message.get("photo").and_then(Value::as_array) {
        // Variants arrive smallest first; the last is the full-size image.
        if let Some(file_id) = photos
            .last()
            .and_then(|photo| photo.get("file_id"))
            .and_then(Value::as_str)
        {
            return Some((file_id.to_string(), MediaKind::Image, None));
        }
    }
    if let Some(document) = message.get("document") {
        let file_id = document.get("file_id")?.as_str()?.to_string();
        let filename = document
            .get("file_name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let kind = match document
            .get("mime_type")
            .and_then(Value::as_str)
            .unwrap_or("")
        {
            mime if mime.starts_with("image/") => MediaKind::Image,
            _ => MediaKind::Document,
        };
        return Some((file_id, kind, filename));
    }
    None
}

/// Turn one Telegram `message` into a normalized inbound message.
///
/// Returns `None` for shapes that are deliberately ignored (service messages,
/// messages with neither text nor media we can use).
fn parse_message(message: &Value, bot_username: &str, bot_id: i64) -> Option<Inbound> {
    let message_id = message.get("message_id")?.as_i64()?.to_string();
    let chat = message.get("chat")?;
    let chat_id = chat.get("id")?.as_i64()?.to_string();
    let kind = match chat.get("type").and_then(Value::as_str).unwrap_or("") {
        "private" => ChatKind::Direct,
        "group" | "supergroup" => ChatKind::Group,
        "channel" => ChatKind::Channel,
        _ => ChatKind::Group,
    };
    let from = message.get("from").cloned().unwrap_or(Value::Null);
    let sender_id = from
        .get("id")
        .and_then(Value::as_i64)
        .map(|id| id.to_string())
        .unwrap_or_else(|| chat_id.clone());
    let display = from
        .get("username")
        .or_else(|| from.get("first_name"))
        .and_then(Value::as_str)
        .map(str::to_string);

    // A media message carries its words in `caption`, and a quirky payload
    // with an empty `text` must not shadow it.
    let text_raw = message
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .or_else(|| message.get("caption").and_then(Value::as_str))
        .unwrap_or("");
    let entities = message
        .get("entities")
        .filter(|entities| entities.is_array())
        .or_else(|| message.get("caption_entities"));
    let mentions = parse_entities(entities);

    let mut addressed = kind == ChatKind::Direct;
    let mut mention_len = 0usize;
    for mention in &mentions {
        if mention_targets_bot(text_raw, mention, bot_username) {
            addressed = true;
            if mention.offset_utf16 == 0 {
                mention_len = mention.length_utf16;
            }
        }
    }
    if !addressed && kind != ChatKind::Direct {
        addressed = replies_to_bot(message, bot_username, bot_id);
    }

    let text = strip_mention(text_raw, mention_len);
    let media = media_file_id(message)
        .map(|(file_id, kind, filename)| MediaRef {
            kind,
            filename,
            content_type: None,
            // The download needs the bot token, so it happens in the poll
            // loop; the file id rides along as a placeholder scheme.
            url: Some(format!("tgfile:{file_id}")),
            data: None,
        })
        .into_iter()
        .collect::<Vec<_>>();

    if text.is_empty() && media.is_empty() {
        return None;
    }

    Some(Inbound {
        // Scoped by chat: message ids are per-chat and would collide across
        // two groups otherwise.
        message_id: format!("{chat_id}:{message_id}"),
        sender: SenderRef {
            id: sender_id,
            display,
        },
        conversation: ConversationRef {
            id: chat_id,
            thread_id: None,
            kind,
        },
        text,
        media,
        addressed_to_bot: addressed,
        created_at_ms: message
            .get("date")
            .and_then(Value::as_i64)
            .map(|seconds| seconds * 1000),
        raw: Some(message.clone()),
    })
}

/// Resolve a `tgfile:` placeholder into downloaded bytes when possible.
async fn download_media(client: &reqwest::Client, base: &str, token: &str, media: &mut MediaRef) {
    let Some(url) = media.url.clone() else {
        return;
    };
    let Some(file_id) = url.strip_prefix("tgfile:") else {
        return;
    };
    let result = async {
        let info = call_api(
            client,
            base,
            token,
            "getFile",
            &json!({ "file_id": file_id }),
            RetryPolicy::single_attempt(),
        )
        .await?;
        let file_path = info
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("telegram `getFile` returned no file_path"))?
            .to_string();
        let bytes = client
            .get(file_url(base, token, &file_path))
            .send()
            .await?
            .bytes()
            .await?;
        Ok::<_, anyhow::Error>((file_path, bytes))
    }
    .await;
    match result {
        Ok((file_path, bytes)) => {
            let size = bytes.len();
            if size > MAX_DOWNLOAD_BYTES {
                tracing::warn!(bytes = size, "telegram attachment too large; skipping");
                return;
            }
            if media.filename.is_none() {
                media.filename = file_path
                    .rsplit(['/', '\\'])
                    .next()
                    .filter(|name| !name.is_empty())
                    .map(str::to_string);
            }
            if media.content_type.is_none() && media.kind == MediaKind::Image {
                media.content_type = Some("image/jpeg".to_string());
            }
            media.data = Some(bytes.to_vec());
            media.url = None;
        }
        Err(error) => {
            // The text still matters: keep the reference, drop nothing else.
            tracing::warn!(%error, "telegram attachment could not be downloaded");
        }
    }
}

/// The Telegram provider: long poll or webhook, per configuration.
struct Telegram;

#[async_trait]
impl Provider for Telegram {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        let config: TelegramConfig = ctx.config()?;
        config.require_token(ctx.id())?;
        Ok(Arc::new(TelegramSender::new(ctx.http().clone(), &config)))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        let config: TelegramConfig = ctx.config()?;
        config.require_token(ctx.id())?;
        let mode = if config.mode.is_empty() {
            "long_poll"
        } else {
            config.mode.as_str()
        };
        match mode {
            "long_poll" => self.run_long_poll(ctx, &config).await,
            "webhook" => self.run_webhook(ctx, &config).await,
            other => anyhow::bail!(
                "invalid `providers.telegram` configuration: unknown mode `{other}` (long_poll|webhook)"
            ),
        }
    }

    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        let config: TelegramConfig = ctx.config()?;
        config.require_token(ctx.id())?;
        let me = call_api(
            ctx.http(),
            config.api_base(),
            &config.bot_token,
            "getMe",
            &json!({}),
            RetryPolicy::single_attempt(),
        )
        .await?;
        let username = me
            .get("username")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("telegram `getMe` returned no username"))?;
        Ok(format!("connected as @{username}"))
    }
}

/// The bot's own identity, learned from `getMe` and used for mention matching.
struct BotIdentity {
    id: i64,
    username: String,
}

impl Telegram {
    async fn identify(ctx: &ProviderCtx, config: &TelegramConfig) -> Result<BotIdentity> {
        let me = call_api(
            ctx.http(),
            config.api_base(),
            &config.bot_token,
            "getMe",
            &json!({}),
            RetryPolicy::default(),
        )
        .await?;
        let username = me
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let id = me.get("id").and_then(Value::as_i64).unwrap_or_default();
        Ok(BotIdentity { id, username })
    }

    /// Attach downloaded bytes to every media placeholder.
    async fn hydrate(ctx: &ProviderCtx, config: &TelegramConfig, inbound: &mut Inbound) {
        for media in &mut inbound.media {
            download_media(ctx.http(), config.api_base(), &config.bot_token, media).await;
        }
    }

    async fn dispatch(
        ctx: &ProviderCtx,
        config: &TelegramConfig,
        identity: &BotIdentity,
        update: &Value,
        sender: &Arc<dyn ChannelSender>,
    ) {
        // Only fresh `message` updates are consumed: edited messages would
        // re-run a prompt the bridge already answered, and channel posts have
        // no user to hold a conversation with.
        let Some(message) = update.get("message") else {
            return;
        };
        let Some(mut inbound) = parse_message(message, &identity.username, identity.id) else {
            return;
        };
        Self::hydrate(ctx, config, &mut inbound).await;
        let outcome = ctx.handle(inbound, sender.clone()).await;
        tracing::debug!(?outcome, "telegram update handled");
    }

    async fn run_long_poll(&self, ctx: ProviderCtx, config: &TelegramConfig) -> Result<()> {
        let sender = self.sender(&ctx)?;
        let identity = Self::identify(&ctx, config).await?;
        ctx.mark_running();
        let mut offset = load_offset(ctx.data_dir());
        // One long-lived shutdown waiter: a fresh `notified()` per select
        // would miss a `notify_waiters` fired between two selects, and the
        // loop would never stop.
        let shutdown = ctx.shutdown().notified();
        tokio::pin!(shutdown);
        loop {
            let body = json!({
                "timeout": LONG_POLL_TIMEOUT_S,
                "offset": offset,
                // Only what dispatch consumes; everything else never reaches us.
                "allowed_updates": ["message"],
            });
            let polled = tokio::select! {
                result = call_api(
                    ctx.http(),
                    config.api_base(),
                    &config.bot_token,
                    "getUpdates",
                    &body,
                    RetryPolicy::single_attempt(),
                ) => result,
                _ = &mut shutdown => return Ok(()),
            };
            let updates = match polled {
                Ok(result) => result.as_array().cloned().unwrap_or_default(),
                Err(error) => {
                    ctx.mark_failed(&format!("polling failed: {error}"));
                    tracing::warn!(%error, "telegram polling failed; backing off");
                    tokio::select! {
                        _ = tokio::time::sleep(POLL_ERROR_BACKOFF) => {}
                        _ = &mut shutdown => return Ok(()),
                    }
                    continue;
                }
            };
            for update in &updates {
                if let Some(id) = update.get("update_id").and_then(Value::as_i64) {
                    offset = id + 1;
                }
                Self::dispatch(&ctx, config, &identity, update, &sender).await;
            }
            if let Err(error) = store_offset(ctx.data_dir(), offset) {
                tracing::warn!(%error, "telegram update offset could not be persisted");
            }
        }
    }

    async fn run_webhook(&self, ctx: ProviderCtx, config: &TelegramConfig) -> Result<()> {
        let sender = self.sender(&ctx)?;
        let identity = Self::identify(&ctx, config).await?;
        let addr = webhook_addr(config);
        let path = webhook_path(config);
        let secret = config.webhook.secret_token.clone();

        let handler_ctx = ctx.clone();
        let handler_config = config.clone();
        let handler_sender = sender.clone();
        let server = WebhookServer::bind(addr)
            .await?
            .on("POST", path, move |request| {
                let ctx = handler_ctx.clone();
                let config = handler_config.clone();
                let sender = handler_sender.clone();
                let identity = BotIdentity {
                    id: identity.id,
                    username: identity.username.clone(),
                };
                let secret = secret.clone();
                async move {
                    if let Some(response) = verify_secret(&request, &secret) {
                        return response;
                    }
                    let body = request.json();
                    // Answer first, work after: the platform retries any delivery
                    // it does not get a 200 for, and a slow agent turn must not
                    // hold the HTTP request open.
                    tokio::spawn(async move {
                        Telegram::dispatch(&ctx, &config, &identity, &body, &sender).await;
                    });
                    WebhookResponse::ok()
                }
            });
        ctx.mark_running();
        server.serve(ctx.shutdown().clone()).await
    }
}

/// The secret-token gate for webhook deliveries.
///
/// Telegram repeats the token registered with `setWebhook` in a header; when a
/// token is configured, deliveries without it are not ours. With no token
/// configured the gate is open — registering one is the deployment's choice.
fn verify_secret(request: &WebhookRequest, secret: &str) -> Option<WebhookResponse> {
    if secret.is_empty() {
        return None;
    }
    let presented = request.header("x-telegram-bot-api-secret-token");
    if presented == Some(secret) {
        None
    } else {
        Some(WebhookResponse::unauthorized())
    }
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(Telegram)
}

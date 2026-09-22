//! Mattermost: self-hosted teams, REST v4 plus its websocket event stream.
//!
//! * **Inbound** — `/api/v4/websocket` with a bearer-token handshake. The
//!   server answers `hello` and then pushes `posted` events; the post itself
//!   rides in `data.post` as a **JSON-encoded string**, not an object, and
//!   `data.mentions` is likewise a stringified array. Anything that is not a
//!   fresh post (edits, deletes, typing, presence, our own posts) is dropped.
//! * **Outbound** — `POST /api/v4/posts` creates a post, `PUT /api/v4/posts/{id}`
//!   rewrites it for progressive output, and `root_id` carries a threaded
//!   reply (a thread reply without `root_id` silently posts at top level, so
//!   both shapes are explicit in the body builder).
//! * **Addressing** — a channel message counts as addressed when
//!   `data.mentions` contains the bot's user id; DMs (`channel_type` `D`) are
//!   always addressed. Whole channels are gated by `channel_allowlist`
//!   (empty = every channel).
//! * **Identity** — `GET /api/v4/users/me` at startup yields the bot's own
//!   `id`, which is needed both to drop our own posts and for the mention
//!   check; a failure there is a startup error, not a warning, because the
//!   bot would otherwise loop on its own output.
//! * **Errors** — `401`/`403` are permanent (bad token, missing permission),
//!   `429`/`5xx` are retryable (the shared HTTP helper already honours
//!   `Retry-After`). Message splitting rides on the shared chunker at the
//!   platform's 4000-character post limit.

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::bridge::{ChatKind, ConversationRef, Inbound, ProviderCtx, SenderRef};
use crate::providers::traits::{
    Capabilities, ChannelDefinition, ChannelSender, Maturity, Provider,
};
use crate::transport::http::{classify_status, send_json, ErrorClass, HttpResponse, RetryPolicy};
use crate::transport::ws::{self, Backoff};
use crate::transport::LengthUnit;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "mattermost",
    display_name: "Mattermost",
    description: "Mattermost server: websocket events, threaded posts, channel allowlist.",
    docs: "docs/guide/channels-mattermost.md",
    maturity: Maturity::Preview,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: true,
        threads: true,
        typing: false,
        reactions: true,
        media_in: false,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 4000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "base_url": "https://mattermost.example.com",
  "token": "",
  "channel_allowlist": [],
  "dm_policy": "allowlist",
  "dm_allowlist": [],
  "group_policy": "disabled",
  "require_mention": true,
  "streaming": true
}"#,
    requires: &["a Mattermost bot or personal access token"],
};

/// WebSocket events this provider understands. The server emits many more
/// (`typing`, `status_change`, `channel_viewed`, …); everything else is
/// ignored by matching on these names.
const EVENT_POSTED: &str = "posted";
const EVENT_HELLO: &str = "hello";

/// One channel's config block in `~/.future/channels/config.json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MattermostConfig {
    pub enabled: bool,
    /// Server root without a trailing `/api/v4` (added here).
    pub base_url: String,
    /// Bot account token or personal access token, sent as `Authorization:
    /// Bearer` on REST calls and in the websocket `authentication_challenge`.
    pub token: String,
    /// Channels the bot answers in; empty means every channel it can see.
    pub channel_allowlist: Vec<String>,
    /// Explicit API root (tests); derived from `base_url` when empty.
    pub api_base: String,
    /// Explicit websocket URL (tests); derived from `base_url` when empty.
    pub ws_url: String,
}

impl Default for MattermostConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            token: String::new(),
            channel_allowlist: Vec::new(),
            api_base: String::new(),
            ws_url: String::new(),
        }
    }
}

impl MattermostConfig {
    fn api_base(&self) -> String {
        if !self.api_base.is_empty() {
            return self.api_base.trim_end_matches('/').to_string();
        }
        format!("{}/api/v4", self.base_url.trim_end_matches('/'))
    }

    fn ws_url(&self) -> String {
        if !self.ws_url.is_empty() {
            return self.ws_url.clone();
        }
        // http→ws, https→wss; anything else is left alone and the connect
        // error will say why.
        let mut url = self.base_url.trim_end_matches('/').to_string();
        if let Some(rest) = url.strip_prefix("https") {
            url = format!("wss{rest}");
        } else if let Some(rest) = url.strip_prefix("http") {
            url = format!("ws{rest}");
        }
        format!("{url}/api/v4/websocket")
    }
}

/// The REST verbs against `/api/v4`.
#[derive(Clone)]
pub struct MattermostApi {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl MattermostApi {
    fn from_config(config: &MattermostConfig, ctx: &ProviderCtx) -> Self {
        Self {
            base: config.api_base(),
            token: config.token.clone(),
            http: ctx.http().clone(),
        }
    }

    fn authorization(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// One API call with the bearer token, retrying transient failures (the
    /// shared helper honours `Retry-After` on 429).
    async fn call(&self, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<HttpResponse> {
        let url = format!("{}{path}", self.base);
        send_json(
            &self.http,
            method,
            &url,
            &[("Authorization", &self.authorization())],
            body,
            RetryPolicy::default(),
        )
        .await
        .with_context(|| format!("mattermost {path} failed"))
    }

    /// A call that must succeed; the error message names the endpoint and
    /// carries the platform's `message` when it gave one.
    async fn call_checked(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<HttpResponse> {
        let response = self.call(method, path, body).await?;
        if !response.is_success() {
            bail!("{path}: {}", response.error_message());
        }
        Ok(response)
    }

    /// The bot's own identity; also the smallest credential check there is.
    async fn me(&self) -> Result<(String, String)> {
        let response = self
            .call_checked(reqwest::Method::GET, "/users/me", None)
            .await?;
        let id = response
            .body
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let username = response
            .body
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok((id, username))
    }
}

/// Whether a failed API call can ever succeed on retry: `401`/`403` are
/// permanent (the token is wrong or the bot was kicked), everything else
/// follows the shared status classification.
pub(crate) fn classify_failure(status: u16) -> ErrorClass {
    classify_status(status)
}

/// A `posted` event, decoded far enough to normalize.
///
/// `data.post` and `data.mentions` arrive as JSON-encoded strings inside the
/// JSON event (a quirk of the platform's event envelope), so parsing is two
/// steps: the event, then the nested strings.
pub(crate) struct PostedEvent {
    pub post: Value,
    pub mentions: Vec<String>,
    pub channel_type: String,
    pub sender_name: String,
}

/// Parse one websocket event into a `posted` payload, or `None` when the
/// event is something else or malformed.
pub(crate) fn parse_ws_event(raw: &Value) -> Option<PostedEvent> {
    if raw.get("event").and_then(Value::as_str) != Some(EVENT_POSTED) {
        return None;
    }
    let data = raw.get("data")?;
    // `post` is a JSON string on the wire; tolerate an object too so a
    // future envelope change does not silently drop every message.
    let post = match data.get("post") {
        Some(Value::String(encoded)) => serde_json::from_str::<Value>(encoded).ok()?,
        Some(post @ Value::Object(_)) => post.clone(),
        _ => return None,
    };
    let mentions = match data.get("mentions") {
        Some(Value::String(encoded)) => serde_json::from_str::<Vec<String>>(encoded)
            .ok()
            .unwrap_or_default(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    };
    let channel_type = data
        .get("channel_type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let sender_name = data
        .get("sender_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Some(PostedEvent {
        post,
        mentions,
        channel_type,
        sender_name,
    })
}

/// Normalize one `posted` event into an inbound message.
///
/// Returns `Ok(None)` for everything that is not a fresh user prompt: posts
/// the bot wrote itself (otherwise its own replies loop back as prompts),
/// edits (an edited post arrives as `post_edited`, but a `root_id` post by
/// someone else is still a fresh prompt), empty messages, and channels the
/// allowlist excludes.
pub(crate) fn parse_posted(
    event: &PostedEvent,
    bot_user_id: &str,
    allowlist: &HashSet<String>,
) -> Result<Option<Inbound>> {
    let post = &event.post;
    let post_id = post
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let Some(post_id) = post_id else {
        return Ok(None);
    };
    let user_id = post
        .get("user_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if user_id.is_empty() || user_id == bot_user_id {
        return Ok(None);
    }
    let channel_id = post
        .get("channel_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !allowlist.is_empty() && !allowlist.contains(channel_id) {
        return Ok(None);
    }
    let text = post
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(None);
    }

    let kind = match event.channel_type.as_str() {
        // `D` is a direct message, `G` a group DM, `O` a public channel,
        // `P` a private one; anything but a DM must be addressed.
        "D" => ChatKind::Direct,
        "G" => ChatKind::Group,
        _ => ChatKind::Channel,
    };
    // A thread is its own conversation: replies to a root post land in the
    // root's session. `root_id` empty means a top-level post.
    let thread_id = post
        .get("root_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|root| !root.is_empty());
    let addressed_to_bot = kind == ChatKind::Direct
        || (!bot_user_id.is_empty() && event.mentions.iter().any(|m| m == bot_user_id));
    let created_at_ms = post.get("create_at").and_then(Value::as_i64);
    let display = (!event.sender_name.is_empty()).then(|| event.sender_name.clone());

    Ok(Some(Inbound {
        message_id: post_id.to_string(),
        sender: SenderRef {
            id: user_id.to_string(),
            display,
        },
        conversation: ConversationRef {
            id: channel_id.to_string(),
            thread_id,
            kind,
        },
        text,
        media: Vec::new(),
        addressed_to_bot,
        created_at_ms,
        raw: None,
    }))
}

/// Build the `POST /posts` body. A threaded reply must carry `root_id`;
/// posting a reply without it lands at the top level and splits the
/// conversation, so both shapes are built explicitly here.
pub(crate) fn create_post_body(conversation: &ConversationRef, text: &str) -> Value {
    let mut body = json!({
        "channel_id": conversation.id,
        "message": text,
    });
    if let Some(root) = conversation
        .thread_id
        .as_deref()
        .filter(|root| !root.is_empty())
    {
        body["root_id"] = json!(root);
    }
    body
}

/// The outbound half: create/update posts and add reactions.
pub struct MattermostSender {
    api: MattermostApi,
}

#[async_trait]
impl ChannelSender for MattermostSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        let body = create_post_body(conversation, text);
        let response = self
            .api
            .call_checked(reqwest::Method::POST, "/posts", Some(&body))
            .await?;
        Ok(response
            .body
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string))
    }

    async fn edit_text(
        &self,
        _conversation: &ConversationRef,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let body = json!({ "id": message_id, "message": text });
        let path = format!("/posts/{message_id}");
        self.api
            .call_checked(reqwest::Method::PUT, &path, Some(&body))
            .await?;
        Ok(())
    }

    async fn react(
        &self,
        conversation: &ConversationRef,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        // Reactions are per-user, so the reaction is posted as the bot
        // itself; the conversation id is only needed to stay trait-shaped.
        let _ = conversation;
        let me = self.api.me().await?.0;
        let body = json!({
            "user_id": me,
            "post_id": message_id,
            "emoji_name": emoji,
        });
        self.api
            .call_checked(reqwest::Method::POST, "/reactions", Some(&body))
            .await?;
        Ok(())
    }
}

/// The Mattermost server.
struct Mattermost;

#[async_trait]
impl Provider for Mattermost {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        let config = ctx.config::<MattermostConfig>()?;
        if config.token.is_empty() {
            bail!("providers.mattermost.token is required");
        }
        Ok(Arc::new(MattermostSender {
            api: MattermostApi::from_config(&config, ctx),
        }))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        let config = ctx.config::<MattermostConfig>()?;
        if config.token.is_empty() {
            bail!("providers.mattermost.token is required");
        }
        let api = MattermostApi::from_config(&config, &ctx);
        // Without our own user id we cannot drop our own posts (prompt loop)
        // or check mentions, so this is a startup error, not a warning.
        let (bot_user_id, _) = api.me().await?;
        let sender: Arc<dyn ChannelSender> = Arc::new(MattermostSender { api: api.clone() });
        let allowlist: HashSet<String> = config.channel_allowlist.iter().cloned().collect();
        let ws_url = config.ws_url();
        let token = config.token.clone();

        ws::supervise("mattermost", ctx.shutdown(), Backoff::gateway(), || {
            let sender = sender.clone();
            let bot_user_id = bot_user_id.clone();
            let allowlist = allowlist.clone();
            let ws_url = ws_url.clone();
            let token = token.clone();
            let ctx = ctx.clone();
            async move {
                // The bearer header authenticates the HTTP upgrade; the
                // challenge frame authenticates the event stream itself.
                let socket = ws::connect(&ws_url, &[("Authorization", &format!("Bearer {token}"))])
                    .await?;
                ctx.mark_running();
                websocket_session(&ctx, sender, &token, &bot_user_id, &allowlist, socket).await
            }
        })
        .await
    }

    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        let config = ctx.config::<MattermostConfig>()?;
        if config.token.is_empty() {
            bail!("providers.mattermost.token is required");
        }
        let api = MattermostApi::from_config(&config, ctx);
        let (id, username) = api.me().await?;
        if username.is_empty() {
            Ok(format!("connected (user id {id})"))
        } else {
            Ok(format!("connected as @{username}"))
        }
    }
}

/// One connected websocket session: send the authentication challenge, then
/// read events until shutdown or the platform drops the socket. Reconnecting
/// (with a fresh `hello`) is the outer supervisor's job.
async fn websocket_session(
    ctx: &ProviderCtx,
    sender: Arc<dyn ChannelSender>,
    token: &str,
    bot_user_id: &str,
    allowlist: &HashSet<String>,
    socket: ws::Socket,
) -> Result<()> {
    let (mut sink, mut stream) = socket.split();
    // The token handshake is an explicit challenge frame; the bearer header
    // on the upgrade request alone does not authenticate the event stream.
    let challenge = json!({
        "seq": 1,
        "action": "authentication_challenge",
        "data": { "token": token }
    });
    if sink
        .send(WsMessage::Text(challenge.to_string()))
        .await
        .is_err()
    {
        bail!("mattermost websocket is not writable");
    }
    loop {
        let message = tokio::select! {
            message = stream.next() => message,
            _ = ctx.shutdown().notified() => {
                tracing::info!(channel = "mattermost", "websocket session stopped");
                return Ok(());
            }
        };
        let Some(message) = message else {
            bail!("mattermost websocket closed by the server");
        };
        match message {
            Ok(WsMessage::Text(text)) => {
                let event: Value = match serde_json::from_str(&text) {
                    Ok(event) => event,
                    Err(error) => {
                        tracing::debug!(channel = "mattermost", %error, "ignoring a non-JSON frame");
                        continue;
                    }
                };
                match event.get("event").and_then(Value::as_str) {
                    Some(EVENT_HELLO) => {
                        tracing::debug!(channel = "mattermost", "websocket authenticated");
                    }
                    Some(EVENT_POSTED) => {
                        if let Some(posted) = parse_ws_event(&event) {
                            dispatch_posted(ctx, &sender, &posted, bot_user_id, allowlist).await;
                        }
                    }
                    _ => {}
                }
            }
            Ok(WsMessage::Ping(data)) => {
                if sink.send(WsMessage::Pong(data)).await.is_err() {
                    bail!("mattermost websocket is not writable");
                }
            }
            Ok(WsMessage::Close(_)) => bail!("mattermost websocket closed by the server"),
            Ok(_) => {}
            Err(error) => bail!("mattermost websocket read failed: {error}"),
        }
    }
}

/// Normalize one `posted` event and hand it to the bridge.
async fn dispatch_posted(
    ctx: &ProviderCtx,
    sender: &Arc<dyn ChannelSender>,
    posted: &PostedEvent,
    bot_user_id: &str,
    allowlist: &HashSet<String>,
) {
    let inbound = match parse_posted(posted, bot_user_id, allowlist) {
        Ok(Some(inbound)) => inbound,
        Ok(None) => return,
        Err(error) => {
            tracing::debug!(channel = "mattermost", %error, "dropping an unparseable post");
            return;
        }
    };
    let message_id = inbound.message_id.clone();
    let conversation = inbound.conversation.clone();
    let outcome = ctx.handle(inbound, sender.clone()).await;
    tracing::debug!(channel = "mattermost", ?outcome, "post handled");
    if outcome.is_accepted() {
        // 👀 is the lightweight "picked up" signal; the name is the
        // platform's colon-free emoji name.
        sender.react(&conversation, &message_id, "eyes").await.ok();
    }
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(Mattermost)
}

#[cfg(test)]
#[path = "mattermost_tests.rs"]
mod tests;

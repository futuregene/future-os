//! Signal: a chat client driven through a local `signal-cli` daemon.
//!
//! Signal has no bot API, so the supported shape is a `signal-cli` instance
//! the user runs themselves; this channel is a client of its HTTP interface.
//! The requirement is declared in the definition so nobody has to guess.
//!
//! * **Inbound** — poll `/api/v1/receive/<number>` (falling back to
//!   `/v1/receive/<number>`); the daemon holds the request open for the
//!   `timeout` we ask for, so a poll is cheap. Only `dataMessage` envelopes
//!   with text or attachments are consumed; reactions, receipts, typing
//!   indicators and our own linked-device traffic carry no prompt and are
//!   dropped. A group message is its own conversation (a group id is not a
//!   phone number, so the conversation id is `group:<id>`).
//! * **Outbound** — `POST /v2/send` (falling back to `/api/v1/send`) with
//!   `recipients` for a direct message and `group-id` for a group. The bridge
//!   splits long answers against the 4000-character limit; the daemon's own
//!   limit is larger, so no second split is needed.
//! * **Addressing** — a group message counts as addressed when the bot is in
//!   the message's `mentions` or the message quotes one of the bot's own.
//!   Our own uuid is optional configuration: it is only used to recognise a
//!   mention when the daemon cannot resolve the mention to a phone number.
//! * **Media** — attachments are recorded and images are downloaded from the
//!   daemon so they become model input. A download that fails (an older
//!   daemon without the attachment route) degrades to a URL reference rather
//!   than dropping the message.
//! * **Errors** — a daemon that is not running yet is transient (it may be
//!   starting); an unregistered account or rejected credentials is permanent.
//!
//! Two caveats this build cannot settle on its own: the daemon **removes a
//! message from its queue as it hands it over**, so an interrupted bridge can
//! lose an unprocessed message, and the typing/reaction sends below are
//! best-effort against the daemon's response shapes rather than verified
//! against a live daemon. That is why the channel publishes
//! [`Maturity::Preview`].

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
use crate::transport::http::{classify_status, send_json, ErrorClass, HttpResponse, RetryPolicy};
use crate::transport::LengthUnit;

#[cfg(test)]
#[path = "signal_tests.rs"]
mod tests;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "signal",
    display_name: "Signal",
    description: "Signal via a local signal-cli daemon (HTTP interface).",
    docs: "docs/guide/channels-signal.md",
    maturity: Maturity::Preview,
    capabilities: Capabilities {
        receive: true,
        send: true,
        edit: false,
        threads: false,
        typing: true,
        reactions: true,
        media_in: true,
        media_out: false,
        mention_gate: true,
    },
    max_text_len: 4000,
    length_unit: LengthUnit::Chars,
    config_example: r#"{
  "enabled": true,
  "http_url": "http://127.0.0.1:8080",
  "number": "+15550001234",
  "uuid": "",
  "receive_timeout_s": 25,
  "poll_interval_ms": 1000,
  "dm_policy": "allowlist",
  "dm_allowlist": [],
  "group_policy": "disabled",
  "group_allowlist": [],
  "require_mention": true
}"#,
    requires: &["a running signal-cli daemon with a registered number"],
};

/// Send paths a daemon may expose, newest first. The route was renamed once,
/// so a 404 on the first is answered by trying the second instead of failing a
/// message the daemon would have accepted.
const SEND_PATHS: &[&str] = &["/v2/send", "/api/v1/send"];
/// Receive paths, in the same spirit as [`SEND_PATHS`].
const RECEIVE_PATHS: &[&str] = &["/api/v1/receive", "/v1/receive"];
/// Best-effort signals (typing, reactions) the daemon exposes over JSON-RPC.
const JSONRPC_PATH: &str = "/api/v1/rpc";
/// Attachment download route, relative to the daemon's base URL.
const ATTACHMENTS_PATH: &str = "/api/v1/attachments";

/// Server-side long-poll ceiling. The shared HTTP client gives one call 60
/// seconds, and the client budget is this value plus slack, so the poll has to
/// stay comfortably below it — asking for longer would make every poll look
/// like a send failure.
const MAX_RECEIVE_TIMEOUT_S: u64 = 30;
/// Pause after a fault so a broken daemon is not hammered.
const ERROR_BACKOFF: Duration = Duration::from_secs(5);
/// One send or JSON-RPC call. A long poll is given its own budget.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Images above this are referenced, not downloaded — the bridge would refuse
/// to turn them into model input anyway.
const MAX_MEDIA_BYTES: usize = 10 * 1024 * 1024;

/// Distinguishes the (unlikely) envelopes that arrive without a timestamp, so
/// two of them cannot share a dedup key.
static UNTIMED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The marker that makes a group conversation id distinguishable from a phone
/// number: a Signal group id is opaque, and a session key must never confuse
/// the two.
pub(crate) const GROUP_PREFIX: &str = "group:";

/// The `providers.signal` block.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SignalConfig {
    /// Base URL of the daemon, without a trailing slash.
    pub http_url: String,
    /// The account the daemon sends as, in E.164 form.
    pub number: String,
    /// Optional: this account's uuid, used only to recognise a mention the
    /// daemon could not resolve to a phone number.
    pub uuid: String,
    /// Seconds the daemon should hold a receive call open.
    pub receive_timeout_s: u64,
    /// Pause after an empty poll — a daemon that ignores the long-poll timeout
    /// would otherwise turn this loop into a busy one.
    pub poll_interval_ms: u64,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            http_url: "http://127.0.0.1:8080".into(),
            number: String::new(),
            uuid: String::new(),
            receive_timeout_s: 25,
            poll_interval_ms: 1_000,
        }
    }
}

impl SignalConfig {
    /// The daemon base URL, without a trailing slash.
    pub(crate) fn base(&self) -> &str {
        self.http_url.trim_end_matches('/')
    }

    /// The long-poll timeout actually requested.
    fn receive_timeout_s(&self) -> u64 {
        self.receive_timeout_s.min(MAX_RECEIVE_TIMEOUT_S)
    }

    fn validate(&self, channel: &str) -> Result<()> {
        let mut missing = Vec::new();
        if self.http_url.trim().is_empty() {
            missing.push("http_url");
        }
        if self.number.trim().is_empty() {
            missing.push("number");
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
        Ok(())
    }
}

/// The account this bridge sends as, and what counts as being addressed.
#[derive(Debug, Clone)]
pub struct AccountIdentity {
    number: String,
    uuid: String,
}

impl AccountIdentity {
    fn new(config: &SignalConfig) -> Self {
        Self {
            number: config.number.clone(),
            uuid: config.uuid.clone(),
        }
    }

    /// Whether a mention or quote author refers to this account.
    ///
    /// Only the phone number is normally resolvable, so a missing number is
    /// not a `false`: an unresolvable mention would otherwise leave a user who
    /// clearly addressed the bot unanswered.
    fn is_me(&self, number: Option<&str>, uuid: Option<&str>) -> bool {
        let by_number = match number {
            Some(number) => !self.number.is_empty() && number == self.number,
            None => false,
        };
        let by_uuid = match uuid {
            Some(uuid) => !self.uuid.is_empty() && uuid.eq_ignore_ascii_case(&self.uuid),
            None => false,
        };
        by_number || by_uuid
    }
}

/// Where a reply goes: one Signal user, or a group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Direct(String),
    Group(String),
}

impl Target {
    /// Read a conversation id back into a target.
    pub(crate) fn parse(conversation_id: &str) -> Self {
        match conversation_id.strip_prefix(GROUP_PREFIX) {
            Some(group) => Target::Group(group.to_string()),
            None => Target::Direct(conversation_id.to_string()),
        }
    }

    /// The conversation id this channel uses for the target.
    pub(crate) fn conversation_id(&self) -> String {
        match self {
            Target::Direct(recipient) => recipient.clone(),
            Target::Group(group) => format!("{GROUP_PREFIX}{group}"),
        }
    }

    /// The send body for one message. A direct send lists `recipients`; a
    /// group send names the group instead.
    fn send_body(&self, number: &str, text: &str) -> Value {
        match self {
            Target::Direct(recipient) => json!({
                "message": text,
                "number": number,
                "recipients": [recipient],
            }),
            Target::Group(group) => json!({
                "message": text,
                "number": number,
                "group-id": group,
            }),
        }
    }

    /// The `recipient`/`groupId` parameter pair the daemon's JSON-RPC methods
    /// take for this target.
    fn rpc_params(&self, number: &str) -> Value {
        match self {
            Target::Direct(recipient) => json!({ "account": number, "recipient": [recipient] }),
            Target::Group(group) => json!({ "account": number, "groupId": group }),
        }
    }
}

/// Percent-encode one path segment (phone numbers carry a leading `+`, which
/// some HTTP servers decode as a space when it is left raw).
pub(crate) fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

/// The outbound half: talks to the daemon, nothing else.
pub struct SignalSender {
    http: reqwest::Client,
    base: String,
    number: String,
}

impl SignalSender {
    fn new(ctx: &ProviderCtx, config: &SignalConfig) -> Self {
        Self {
            http: ctx.http().clone(),
            base: config.base().to_string(),
            number: config.number.clone(),
        }
    }

    /// POST one send body, tolerating the daemon's two route shapes.
    async fn post_send(&self, body: &Value) -> Result<HttpResponse> {
        let mut missing_routes: Vec<&str> = Vec::new();
        for path in SEND_PATHS {
            let url = format!("{}{path}", self.base);
            let response = send_json(
                &self.http,
                reqwest::Method::POST,
                &url,
                &[],
                Some(body),
                RetryPolicy::default(),
            )
            .await?;
            // A 404/405 is the route, not the message: try the other shape
            // before giving up on a send the daemon would have accepted.
            if response.status == 404 || response.status == 405 {
                missing_routes.push(path);
                continue;
            }
            return Ok(response);
        }
        Err(anyhow!(
            "signal-cli at {} exposes none of the send routes {}",
            self.base,
            missing_routes.join(", ")
        ))
    }

    /// Best-effort JSON-RPC call for the signals the REST routes do not carry.
    async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        let url = format!("{}{JSONRPC_PATH}", self.base);
        let body = json!({ "jsonrpc": "2.0", "id": method, "method": method, "params": params });
        let response = send_json(
            &self.http,
            reqwest::Method::POST,
            &url,
            &[],
            Some(&body),
            RetryPolicy::single_attempt(),
        )
        .await?;
        if !response.is_success() {
            return Err(describe_failure(method, &response));
        }
        Ok(response.body)
    }

    /// One receive poll, tolerating the daemon's two route shapes.
    ///
    /// The daemon holds the request open for `timeout` seconds; the client
    /// budget is that timeout plus slack, because a reply that takes longer
    /// than the client waits would look like a dropped connection.
    async fn receive(&self, config: &SignalConfig) -> Result<Vec<Value>> {
        let timeout = config.receive_timeout_s();
        let url_timeout = if timeout == 0 { 1 } else { timeout };
        let url = |path: &str| {
            format!(
                "{}{path}/{}?timeout={url_timeout}",
                self.base,
                encode_segment(&self.number)
            )
        };
        let budget = Duration::from_secs(url_timeout + 15);
        let mut last_error: Option<anyhow::Error> = None;
        for path in RECEIVE_PATHS {
            let request = self
                .http
                .get(url(path))
                .timeout(budget)
                .header("accept", "application/json");
            let response = match request.send().await {
                Ok(response) => response,
                // A connection-level failure is the daemon, not the route.
                Err(error) => {
                    return Err(anyhow!("cannot reach signal-cli at {}: {error}", self.base))
                }
            };
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            if status == 404 || status == 405 {
                last_error = Some(anyhow!("signal-cli at {} has no {path} route", self.base));
                continue;
            }
            if !(200..300).contains(&status) {
                return Err(describe_failure("receive", &classify(status, &text)));
            }
            return Ok(parse_envelopes(&text));
        }
        Err(last_error.unwrap_or_else(|| anyhow!("signal-cli answered no receive route")))
    }

    /// Fetch one attachment's bytes from the URL the inbound message carries.
    async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let response = self.http.get(url).timeout(REQUEST_TIMEOUT).send().await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "signal-cli answered {} for an attachment",
                response.status()
            );
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_MEDIA_BYTES as u64)
        {
            anyhow::bail!("attachment is larger than the {MAX_MEDIA_BYTES}-byte limit");
        }
        let bytes = response.bytes().await?;
        if bytes.len() > MAX_MEDIA_BYTES {
            anyhow::bail!("attachment is larger than the {MAX_MEDIA_BYTES}-byte limit");
        }
        Ok(bytes.to_vec())
    }
}

#[async_trait]
impl ChannelSender for SignalSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        let target = Target::parse(&conversation.id);
        let mut last_id = None;
        // The bridge splits against the same limit; splitting again here keeps
        // a direct caller (`future channel send`) inside it too.
        for piece in crate::transport::chunk(text, DEFINITION.max_text_len, DEFINITION.length_unit)
        {
            let response = self
                .post_send(&target.send_body(&self.number, &piece))
                .await?;
            if !response.is_success() {
                return Err(describe_failure("send", &response));
            }
            // The daemon answers with the timestamp it assigned; the bridge
            // cannot edit messages on Signal, so the id is informational.
            last_id = response
                .body
                .get("timestamp")
                .and_then(Value::as_i64)
                .map(|timestamp| timestamp.to_string());
        }
        Ok(last_id)
    }

    /// A typing indicator is a courtesy; a daemon without the method answering
    /// an error is not worth failing a turn over.
    async fn typing(&self, conversation: &ConversationRef) -> Result<()> {
        let target = Target::parse(&conversation.id);
        let mut params = target.rpc_params(&self.number);
        if let Some(object) = params.as_object_mut() {
            object.insert("stop".into(), Value::Bool(false));
        }
        self.rpc("sendTyping", params).await.map(|_| ())
    }

    async fn react(
        &self,
        conversation: &ConversationRef,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let Ok(timestamp) = message_id.parse::<i64>() else {
            anyhow::bail!("signal needs the numeric timestamp of the message to react to it");
        };
        let target = Target::parse(&conversation.id);
        let mut params = target.rpc_params(&self.number);
        if let Some(object) = params.as_object_mut() {
            // A reaction is addressed to the message's author; the only message
            // this channel can react to is one it sent itself.
            object.insert("reaction".into(), Value::String(emoji.to_string()));
            object.insert("targetAuthor".into(), Value::String(self.number.clone()));
            object.insert("targetTimestamp".into(), json!(timestamp));
        }
        self.rpc("sendReaction", params).await.map(|_| ())
    }
}

/// The receiving half.
struct Signal;

#[async_trait]
impl Provider for Signal {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        let config: SignalConfig = ctx.config()?;
        config.validate(DEFINITION.id)?;
        Ok(Arc::new(SignalSender::new(ctx, &config)))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        let config: SignalConfig = ctx.config()?;
        config.validate(DEFINITION.id)?;
        let sender = Arc::new(SignalSender::new(&ctx, &config));
        let identity = AccountIdentity::new(&config);

        // Prove the daemon is there and the account is registered before
        // claiming to be running: a channel that never receives anything
        // should not look healthy. A failed check is published rather than
        // fatal — a daemon that rejects the check but still answers the
        // receive route (an older daemon without the account-list method)
        // should not leave the channel dead.
        match self.probe(&ctx).await {
            Ok(summary) => {
                tracing::info!(channel = DEFINITION.id, %summary, "signal is ready");
                ctx.mark_running();
            }
            Err(error) => {
                tracing::warn!(%error, "signal startup check failed; receiving anyway");
                ctx.mark_failed(&error.to_string());
            }
        }

        // One long-lived shutdown waiter: a fresh `notified()` per iteration
        // would miss a `notify_waiters` fired between two selects.
        let shutdown = ctx.shutdown().notified();
        tokio::pin!(shutdown);
        let interval = Duration::from_millis(config.poll_interval_ms.max(50));
        loop {
            let polled = tokio::select! {
                result = sender.receive(&config) => result,
                _ = &mut shutdown => return Ok(()),
            };
            let envelopes = match polled {
                Ok(envelopes) => envelopes,
                Err(error) => {
                    // A daemon that is down may come back; only the account
                    // being wrong is permanent, and that is caught by `probe`.
                    ctx.mark_failed(&format!("receive failed: {error}"));
                    tracing::warn!(%error, "signal receive failed; backing off");
                    tokio::select! {
                        _ = tokio::time::sleep(ERROR_BACKOFF) => {}
                        _ = &mut shutdown => return Ok(()),
                    }
                    continue;
                }
            };
            // A poll that came back is the real proof the channel is working
            // (and it clears any error the startup check published).
            ctx.mark_running();
            if envelopes.is_empty() {
                // Either the long poll timed out with nothing waiting, or the
                // daemon ignored the timeout: pause so we never busy-loop.
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = &mut shutdown => return Ok(()),
                }
                continue;
            }
            for raw in &envelopes {
                let Some(mut inbound) = parse_envelope(raw, &identity, &sender.base) else {
                    continue;
                };
                hydrate_media(&sender, &mut inbound).await;
                let outcome = ctx.handle(inbound, sender.clone()).await;
                tracing::debug!(?outcome, "signal envelope handled");
            }
        }
    }

    /// A request that proves the daemon is up **and** the account is known.
    ///
    /// Deliberately reads the account list: a receive call would *consume*
    /// queued messages, and a probe must never eat a user's message.
    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        let config: SignalConfig = ctx.config()?;
        config.validate(DEFINITION.id)?;
        let sender = SignalSender::new(ctx, &config);
        let url = format!("{}{}", sender.base, JSONRPC_PATH);
        let body = json!({ "jsonrpc": "2.0", "id": "listAccounts", "method": "listAccounts", "params": {} });
        let response = send_json(
            &sender.http,
            reqwest::Method::POST,
            &url,
            &[],
            Some(&body),
            RetryPolicy::single_attempt(),
        )
        .await;
        let accounts = match response {
            Ok(response) if response.is_success() => accounts_of(&response.body),
            Ok(response) if response.status == 404 || response.status == 405 => {
                anyhow::bail!(
                    "signal-cli at {} does not expose {JSONRPC_PATH}; check that it runs in HTTP mode",
                    sender.base
                )
            }
            Ok(response) => return Err(describe_failure("listAccounts", &response)),
            Err(error) => {
                return Err(anyhow!(
                    "cannot reach signal-cli at {}: {error}",
                    sender.base
                ));
            }
        };
        match accounts {
            Some(accounts) => {
                if accounts.is_empty() {
                    anyhow::bail!(
                        "signal-cli at {} has no registered account; register {} with it first",
                        sender.base,
                        config.number
                    );
                }
                if !accounts
                    .iter()
                    .any(|account| account == &config.number || account == &config.uuid)
                {
                    anyhow::bail!(
                        "signal-cli at {} does not know the account {}; it reports {}",
                        sender.base,
                        config.number,
                        accounts.join(", ")
                    );
                }
                Ok(format!(
                    "connected to signal-cli at {} as {}",
                    sender.base, config.number
                ))
            }
            None => Ok(format!(
                "connected to signal-cli at {} (it did not list accounts to compare)",
                sender.base
            )),
        }
    }
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(Signal)
}

/// The account numbers (and uuids) a `listAccounts` answer names.
///
/// `None` means the answer did not carry a list at all, which is not the same
/// as an empty account list.
pub(crate) fn accounts_of(body: &Value) -> Option<Vec<String>> {
    let list = body
        .get("result")
        .filter(|result| result.is_array())
        .or_else(|| body.as_array().map(|_| body))?;
    let entries = list.as_array()?;
    let mut accounts = Vec::new();
    for entry in entries {
        match entry {
            // Newer daemons list objects; older ones list bare numbers.
            Value::Object(_) => {
                for key in ["number", "uuid"] {
                    if let Some(value) = entry.get(key).and_then(Value::as_str) {
                        accounts.push(value.to_string());
                    }
                }
            }
            Value::String(number) => accounts.push(number.clone()),
            _ => {}
        }
    }
    Some(accounts)
}

/// The JSON array from a receive answer.
///
/// A daemon answers with an array of envelopes; a single envelope (or an
/// object wrapper) is accepted too, because the shapes drift between versions
/// and dropping a message over a wrapper would be silent.
pub(crate) fn parse_envelopes(text: &str) -> Vec<Value> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(envelopes)) => envelopes,
        Ok(value @ Value::Object(_)) => vec![value],
        _ => Vec::new(),
    }
}

/// Turn one receive element into an inbound message.
///
/// `None` means the element carries no prompt: a reaction, a receipt, a typing
/// indicator, a group-state update, or a copy of something one of the user's
/// own devices sent.
pub(crate) fn parse_envelope(
    raw: &Value,
    identity: &AccountIdentity,
    base: &str,
) -> Option<Inbound> {
    let envelope = raw.get("envelope").unwrap_or(raw);
    let data = envelope.get("dataMessage")?;

    let text = data
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let attachments: Vec<Value> = data
        .get("attachments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // A reaction, an expiration update and a "delete for everyone" all arrive
    // as a `dataMessage` with neither text nor attachments.
    if text.trim().is_empty() && attachments.is_empty() {
        return None;
    }

    let source = envelope
        .get("sourceNumber")
        .and_then(Value::as_str)
        .or_else(|| envelope.get("sourceUuid").and_then(Value::as_str))
        .or_else(|| envelope.get("source").and_then(Value::as_str))?
        .to_string();
    let timestamp = data
        .get("timestamp")
        .and_then(Value::as_i64)
        .or_else(|| envelope.get("timestamp").and_then(Value::as_i64));

    let group = group_id(data);
    let conversation = match &group {
        Some(group) => ConversationRef {
            id: format!("{GROUP_PREFIX}{group}"),
            thread_id: None,
            kind: ChatKind::Group,
        },
        None => ConversationRef {
            id: source.clone(),
            thread_id: None,
            kind: ChatKind::Direct,
        },
    };
    let addressed_to_bot = match &group {
        // A direct message is addressed by definition; in a group, only an
        // explicit mention or a reply to the bot counts.
        None => true,
        Some(_) => mentions_us(data, identity) || quotes_us(data, identity),
    };

    Some(Inbound {
        message_id: match timestamp {
            // The timestamp alone repeats when two senders write in the same
            // millisecond; the source keeps the dedup key unique.
            Some(timestamp) => format!("{timestamp}:{source}"),
            None => {
                let sequence = UNTIMED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                format!("{}:{sequence}", crate::bridge::dedup::now_ms())
            }
        },
        sender: SenderRef {
            id: source,
            display: envelope
                .get("sourceName")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
        conversation,
        text: clean_text(text),
        media: attachment_refs(&attachments, base),
        addressed_to_bot,
        created_at_ms: timestamp,
        raw: Some(raw.clone()),
    })
}

/// Replace the placeholder a mention leaves in the text.
///
/// The platform puts U+FFFC where the mention was; passing that to a model is
/// noise, and it is not a character anyone typed.
pub(crate) fn clean_text(text: &str) -> String {
    text.replace('\u{fffc}', "").trim().to_string()
}

/// The group id of a message, from either of the two shapes a daemon reports.
pub(crate) fn group_id(data: &Value) -> Option<String> {
    data.get("groupInfo")
        .and_then(|info| info.get("groupId"))
        .and_then(Value::as_str)
        .or_else(|| {
            data.get("groupV2")
                .and_then(|info| info.get("id"))
                .and_then(Value::as_str)
        })
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// Whether the message mentions this account.
pub(crate) fn mentions_us(data: &Value, identity: &AccountIdentity) -> bool {
    data.get("mentions")
        .and_then(Value::as_array)
        .is_some_and(|mentions| {
            mentions.iter().any(|mention| {
                identity.is_me(
                    mention.get("number").and_then(Value::as_str),
                    mention.get("uuid").and_then(Value::as_str),
                )
            })
        })
}

/// Whether the message quotes one of this account's own messages.
pub(crate) fn quotes_us(data: &Value, identity: &AccountIdentity) -> bool {
    let Some(quote) = data.get("quote") else {
        return false;
    };
    identity.is_me(
        quote.get("authorNumber").and_then(Value::as_str),
        quote.get("authorUuid").and_then(Value::as_str),
    )
}

/// The media kinds of one message's attachments, each pointing at the daemon
/// route that serves its bytes.
pub(crate) fn attachment_refs(attachments: &[Value], base: &str) -> Vec<MediaRef> {
    attachments
        .iter()
        .filter_map(|attachment| {
            let content_type = attachment
                .get("contentType")
                .and_then(Value::as_str)
                .map(str::to_string);
            let kind = content_type
                .as_deref()
                .map(media_kind)
                .unwrap_or(MediaKind::Unknown);
            let id = attachment.get("id").and_then(Value::as_str)?;
            Some(MediaRef {
                kind,
                filename: attachment
                    .get("filename")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                content_type,
                // The bridge turns the bytes into model input when it has them
                // and otherwise leaves the reference for the prompt.
                url: Some(format!("{base}{ATTACHMENTS_PATH}/{}", encode_segment(id))),
                data: None,
            })
        })
        .collect()
}

/// The media kind one content type describes.
pub(crate) fn media_kind(content_type: &str) -> MediaKind {
    let lowered = content_type.to_ascii_lowercase();
    if lowered.starts_with("image/") {
        MediaKind::Image
    } else if lowered.starts_with("audio/") {
        MediaKind::Audio
    } else if lowered.starts_with("video/") {
        MediaKind::Video
    } else {
        MediaKind::Document
    }
}

/// Download the attachments a model can use.
///
/// Images are the only kind the bridge feeds to the model, so they are the
/// only kind worth a round trip. A failure is not fatal: the URL reference
/// stays in the message so nothing is silently dropped.
async fn hydrate_media(sender: &SignalSender, inbound: &mut Inbound) {
    for media in inbound.media.iter_mut() {
        if media.kind != MediaKind::Image || media.data.is_some() {
            continue;
        }
        let Some(url) = media.url.clone() else {
            continue;
        };
        match sender.download(&url).await {
            Ok(bytes) => media.data = Some(bytes),
            Err(error) => {
                tracing::debug!(%error, "signal attachment not downloaded; keeping the reference");
            }
        }
    }
}

/// Classify a send/receive failure the way the delivery queue needs it.
///
/// The strings below are the daemon's own vocabulary: a phone number the
/// daemon cannot encrypt to, or an account it does not hold, will never work.
pub(crate) fn classify_failure(status: u16, message: &str) -> ErrorClass {
    let lowered = message.to_ascii_lowercase();
    const TRANSIENT: &[&str] = &[
        "timeout",
        "timed out",
        "busy",
        "rate limit",
        "too many",
        "temporarily",
        "connection refused",
        "connection reset",
    ];
    if TRANSIENT.iter().any(|pattern| lowered.contains(pattern)) {
        return ErrorClass::Transient;
    }
    const PERMANENT: &[&str] = &[
        "unregistered",
        "not registered",
        "not a valid",
        "invalid number",
        "invalid recipient",
        "unknown group",
        "group not found",
        "no such account",
        "unauthorized",
        "forbidden",
        "not a member",
    ];
    if PERMANENT.iter().any(|pattern| lowered.contains(pattern)) {
        return ErrorClass::Permanent;
    }
    classify_status(status)
}

/// A status and body as the HTTP response type the classifier takes.
fn classify(status: u16, text: &str) -> HttpResponse {
    HttpResponse {
        status,
        text: text.to_string(),
        body: serde_json::from_str(text).unwrap_or(Value::Null),
        headers: std::collections::HashMap::new(),
        retry_after: None,
    }
}

/// One failure, in the vocabulary that decides whether it is worth retrying.
fn describe_failure(operation: &str, response: &HttpResponse) -> anyhow::Error {
    let message = response.error_message();
    let class = classify_failure(response.status, &message);
    let label = match class {
        ErrorClass::Permanent => "permanent",
        ErrorClass::Transient => "transient",
    };
    anyhow!("signal {operation} failed: {message} ({label} error)")
}

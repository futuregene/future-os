//! WeCom (企业微信): the self-built app callback.
//!
//! WeCom delivers messages by calling *us*, and it encrypts the body. Both
//! halves of that are platform-specific and both are security-relevant, so they
//! are the whole of this module's inbound side:
//!
//! * the callback carries a `msg_signature` — SHA-1 over the token, the
//!   timestamp, the nonce and the ciphertext, sorted and concatenated. It is
//!   checked **before** anything is decrypted, because a ciphertext from the
//!   network is attacker-chosen input;
//! * the body is then decrypted (AES-256-CBC, see [`crate::transport::cipher`]).
//!   The plaintext ends with the intended receiver, which must be this corp:
//!   that is what stops a callback replayed from another tenant.
//!
//! * **Inbound** — a [`WebhookServer`] answering the one-time URL verification
//!   (`GET`, echo the decrypted `echostr`) and the message callback (`POST`).
//!   The answer is an empty body, which is the platform's documented "no
//!   passive reply": the platform then stops retrying, and the bridge sends the
//!   real answer through the active-send API instead.
//! * **Outbound** — `message/send` on the WeCom app API, with the access token
//!   cached until shortly before it expires (fetching it per message would hit
//!   the platform's own rate limit). Every reply is a new message; WeCom's app
//!   API cannot rewrite one that was already sent.
//! * **Addressing** — a self-built app only receives direct messages from
//!   members, so every conversation is a direct one and the DM policy applies.
//!   Group chats are a different integration (a group robot webhook, which is
//!   send-only), so they are out of scope here.
//! * **Errors** — WeCom answers **HTTP 200 for failures**, with the real status
//!   in `errcode`. Treating 200 as success would turn a rejected send into
//!   silence, so `errcode` is checked on every call and classified where its
//!   meaning is unambiguous.

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bridge::{ChatKind, ConversationRef, Inbound, ProviderCtx, SenderRef};
use crate::providers::traits::{
    Capabilities, ChannelDefinition, ChannelSender, Maturity, Provider,
};
use crate::transport::cipher::{self, AesKey};
use crate::transport::http::{send_json, ErrorClass, HttpResponse, RetryPolicy};
use crate::transport::signature::{constant_time_eq, verify_hex};
use crate::transport::webhook::{WebhookRequest, WebhookResponse, WebhookServer};
use crate::transport::LengthUnit;

#[cfg(test)]
#[path = "wecom_tests.rs"]
mod tests;

pub static DEFINITION: ChannelDefinition = ChannelDefinition {
    id: "wecom",
    display_name: "WeCom (企业微信)",
    description: "WeCom self-built app: encrypted callback inbound, app message API outbound.",
    docs: "docs/guide/channels-providers.md#wecom",
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
        mention_gate: false,
    },
    max_text_len: 2048,
    length_unit: LengthUnit::Bytes,
    config_example: r#"{
  "enabled": true,
  "corp_id": "",
  "agent_id": 0,
  "secret": "",
  "token": "",
  "encoding_aes_key": "",
  "webhook": { "addr": "127.0.0.1:8790", "path": "/webhooks/wecom" },
  "dm_policy": "allowlist",
  "dm_allowlist": []
}"#,
    requires: &["a WeCom self-built app whose callback URL reaches this host"],
};

/// The app API origin; `api_base` in the config is the test seam.
const DEFAULT_API_BASE: &str = "https://qyapi.weixin.qq.com";
const DEFAULT_WEBHOOK_ADDR: &str = "127.0.0.1:8790";
const DEFAULT_WEBHOOK_PATH: &str = "/webhooks/wecom";
/// Renew the access token this long before it expires, so a send never races
/// the expiry.
const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(120);
/// The envelope prefix before the message: 16 random bytes and a 4-byte length.
const ENVELOPE_PREFIX: usize = 20;

/// The `providers.wecom` block.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WecomConfig {
    /// Corp id, `ww…`. The callback's receive id is checked against it.
    pub corp_id: String,
    /// Agent id of the self-built app; every send has to name it.
    pub agent_id: i64,
    /// The app's secret, used to fetch the access token.
    pub secret: String,
    /// Callback token, configured alongside the callback URL.
    pub token: String,
    /// 43-character callback encoding key.
    pub encoding_aes_key: String,
    pub webhook: WebhookConfig,
    /// Test seam: replaces the app API origin.
    pub api_base: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WebhookConfig {
    pub addr: String,
    pub path: String,
}

impl WecomConfig {
    fn api_base(&self) -> &str {
        let base = if self.api_base.is_empty() {
            DEFAULT_API_BASE
        } else {
            self.api_base.as_str()
        };
        base.trim_end_matches('/')
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

    /// Sending needs the corp, the app it is sent as, and a secret to fetch a
    /// token with.
    fn require_credentials(&self, channel: &str) -> Result<()> {
        if self.corp_id.trim().is_empty() || self.secret.trim().is_empty() || self.agent_id == 0 {
            bail!(
                "invalid `providers.{channel}` configuration: `corp_id`, `secret` and a non-zero `agent_id` are required"
            );
        }
        Ok(())
    }

    /// Receiving needs all three parts of the callback handshake. Refusing to
    /// start without them is deliberate: an unverified callback would let
    /// anyone who can reach the port drive the agent.
    fn require_callback_secrets(&self, channel: &str) -> Result<()> {
        if self.token.trim().is_empty() || self.encoding_aes_key.trim().is_empty() {
            bail!(
                "invalid `providers.{channel}` configuration: `token` and `encoding_aes_key` are required to receive"
            );
        }
        Ok(())
    }
}

/// Lowercase hex, the form both the platform and `verify_hex` expect.
fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// The platform's callback signature.
///
/// SHA-1 over the token, the timestamp, the nonce and the ciphertext, sorted
/// lexicographically and concatenated — pinned by
/// `the_signature_is_sha1_over_the_sorted_values` against a vector computed
/// outside this crate.
fn callback_signature(token: &str, timestamp: &str, nonce: &str, payload: &str) -> String {
    let mut parts = [token, timestamp, nonce, payload];
    parts.sort_unstable();
    let mut hasher = Sha1::new();
    for part in parts {
        hasher.update(part.as_bytes());
    }
    to_hex(&hasher.finalize())
}

/// Whether a callback's signature is genuine.
fn verify_callback(
    token: &str,
    timestamp: &str,
    nonce: &str,
    payload: &str,
    presented: &str,
) -> bool {
    if token.is_empty() || presented.is_empty() {
        return false;
    }
    verify_hex(
        &callback_signature(token, timestamp, nonce, payload),
        presented,
    )
}

/// The text of one XML element, unwrapping the `<![CDATA[…]]>` the platform
/// puts around every string field.
///
/// A hand-rolled reader rather than an XML dependency: the payload is a flat
/// list of known tags, and a full parser would be a larger surface to trust
/// with attacker-reachable input.
fn xml_tag(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let raw = &xml[start..end];
    let value = raw
        .strip_prefix("<![CDATA[")
        .and_then(|rest| rest.strip_suffix("]]>"))
        .unwrap_or(raw);
    Some(value.to_string())
}

/// The message inside a decrypted envelope.
///
/// The envelope is 16 random bytes, a 4-byte big-endian length, the message,
/// and finally the id of whoever it was meant for. Checking that last part is
/// what makes a captured callback useless to anyone else, so a mismatch is an
/// error rather than a warning.
fn decode_envelope(plaintext: &[u8], corp_id: &str) -> Result<String> {
    if plaintext.len() < ENVELOPE_PREFIX {
        bail!(
            "the decrypted envelope is {} bytes, shorter than its {ENVELOPE_PREFIX}-byte prefix",
            plaintext.len()
        );
    }
    let declared = u32::from_be_bytes(
        plaintext[16..ENVELOPE_PREFIX]
            .try_into()
            .expect("four bytes were sliced"),
    ) as usize;
    let end = ENVELOPE_PREFIX + declared;
    if end > plaintext.len() {
        bail!(
            "the envelope declares {declared} bytes of message but only {} follow the prefix",
            plaintext.len() - ENVELOPE_PREFIX
        );
    }
    let message = std::str::from_utf8(&plaintext[ENVELOPE_PREFIX..end])
        .map_err(|error| anyhow!("the decrypted message is not UTF-8: {error}"))?
        .to_string();
    let receive_id = &plaintext[end..];
    if receive_id.is_empty() {
        bail!("the envelope carries no receive id");
    }
    if !constant_time_eq(receive_id, corp_id.as_bytes()) {
        bail!("the envelope was addressed to another corp, not to this one");
    }
    Ok(message)
}

/// Decrypt one callback payload and return the message it carries.
fn decrypt_callback(key: &AesKey, encrypted: &str, corp_id: &str) -> Result<String> {
    use base64::Engine;
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(encrypted.trim())
        .map_err(|error| anyhow!("the callback ciphertext is not base64: {error}"))?;
    let plaintext = cipher::decrypt(key, &ciphertext)?;
    decode_envelope(&plaintext, corp_id)
}

/// One decrypted message as the bridge's inbound envelope.
///
/// Only `text` is a prompt. The platform also sends `image`, `voice`, `video`
/// and `location`, none of which this provider carries into the pipeline, so
/// they are dropped rather than turned into an empty one — the alternative is
/// the agent answering a message it cannot see.
fn parse_message(xml: &str) -> Option<Inbound> {
    if xml_tag(xml, "MsgType").as_deref() != Some("text") {
        return None;
    }
    let sender = xml_tag(xml, "FromUserName").filter(|id| !id.is_empty())?;
    let text = xml_tag(xml, "Content").unwrap_or_default();
    if text.trim().is_empty() {
        return None;
    }
    let created_at_ms = xml_tag(xml, "CreateTime")
        .and_then(|seconds| seconds.parse::<i64>().ok())
        .map(|seconds| seconds * 1000);
    // `MsgId` is stable across the platform's retries, which is what makes the
    // bridge's deduplication work. A message without one still needs an id, so
    // it falls back to the pair that identifies it.
    let message_id = xml_tag(xml, "MsgId")
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| format!("{sender}:{}", created_at_ms.unwrap_or_default()));

    Some(Inbound {
        message_id,
        sender: SenderRef {
            id: sender.clone(),
            display: None,
        },
        conversation: ConversationRef {
            // A self-built app receives members' direct messages, so the
            // member *is* the conversation.
            id: sender,
            thread_id: None,
            kind: ChatKind::Direct,
        },
        text,
        media: Vec::new(),
        // The app is addressed by construction.
        addressed_to_bot: true,
        created_at_ms,
        raw: Some(json!({ "format": "wecom-callback" })),
    })
}

/// `errcode` from a response body, when the platform sent one.
fn errcode_of(body: &Value) -> Option<i64> {
    body.get("errcode").and_then(Value::as_i64)
}

/// Codes that mean "the token in hand cannot be used".
///
/// They are transient by nature — another `gettoken` fixes them — but the
/// cached token has to be dropped first, or every retry reuses the dead one.
const TOKEN_ERROR_CODES: [i64; 4] = [40014, 42001, 42007, 42009];

/// Whether an error is about the access token rather than the request.
fn is_token_error(code: Option<i64>) -> bool {
    code.is_some_and(|code| TOKEN_ERROR_CODES.contains(&code))
}

/// Whether an `errcode` is worth another attempt.
///
/// Only the codes whose meaning is unambiguous are listed; anything else falls
/// back to the HTTP status.
fn classify_error_code(code: i64) -> Option<ErrorClass> {
    match code {
        // The token was rotated or expired under us. Another call fetches one.
        _ if TOKEN_ERROR_CODES.contains(&code) => Some(ErrorClass::Transient),
        // Rate limits and a busy platform.
        45009 | 45047 | -1 => Some(ErrorClass::Transient),
        // Wrong corp id, wrong secret, or no token at all.
        40001 | 40013 | 41001 => Some(ErrorClass::Permanent),
        // The app is not allowed to use this API, or not from this address.
        60011 | 60020 => Some(ErrorClass::Permanent),
        // The recipient is not a member of this corp.
        81013 | 60111 => Some(ErrorClass::Permanent),
        // The payload was rejected: too long, or blocked by content rules.
        45002 | 45003 | 86001 => Some(ErrorClass::Permanent),
        _ => None,
    }
}

/// The platform's own words for a failure.
fn error_text(body: &Value) -> Option<String> {
    body.get("errmsg")
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
}

/// Turn a rejected response into an error that says which class it is.
///
/// The class is stamped into the text with [`ErrorClass::label`] because the
/// durable delivery queue stores the message and nothing else; a classification
/// that never reaches the text is a classification the queue cannot use.
///
/// `errcode` is included when the platform sent one. It is the identifier an
/// operator searches for, and without it the message would read "HTTP 200:
/// …" — because these failures arrive with a success status.
fn send_error(action: &str, response: &HttpResponse) -> anyhow::Error {
    let code = errcode_of(&response.body);
    let class = code
        .and_then(classify_error_code)
        .unwrap_or_else(|| response.class());
    let hint = match code {
        Some(40001) | Some(40013) => " (the corp id or the app secret is wrong)",
        Some(40014) | Some(42001) => " (the access token was rejected and will be fetched again)",
        Some(60011) | Some(60020) => " (the app may not call this API from this address)",
        Some(81013) => " (the recipient is not a member of this corp)",
        Some(45009) => " (the app has reached the platform's call limit)",
        _ => "",
    };
    let detail = match (code, error_text(&response.body)) {
        (Some(code), Some(message)) => format!("errcode {code}: {message}"),
        (Some(code), None) => format!("errcode {code}"),
        // No errcode: this is a transport-level failure, so the shared
        // description (which names the HTTP status) is the useful one.
        (None, _) => response.error_message(),
    };
    anyhow!(
        "wecom `{action}` rejected ({}): {detail}{hint}",
        class.label()
    )
}

/// WeCom answers HTTP 200 for failures, so `errcode` is the real status.
fn check_ok(action: &str, response: &HttpResponse) -> Result<()> {
    if !response.is_success() {
        return Err(send_error(action, response));
    }
    match errcode_of(&response.body) {
        // No `errcode` at all is outside the documented shape but not evidence
        // of failure, so it is not treated as one.
        Some(0) | None => Ok(()),
        Some(_) => Err(send_error(action, response)),
    }
}

/// The access token, and the moment it stops being usable.
struct CachedToken {
    value: String,
    expires_at: Instant,
}

impl Default for CachedToken {
    fn default() -> Self {
        Self {
            value: String::new(),
            // The unix epoch, so the first call always fetches.
            expires_at: Instant::now(),
        }
    }
}

/// The outbound half.
pub struct WecomSender {
    client: reqwest::Client,
    base: String,
    corp_id: String,
    agent_id: i64,
    secret: String,
    token: tokio::sync::Mutex<CachedToken>,
}

impl WecomSender {
    fn new(client: reqwest::Client, config: &WecomConfig) -> Self {
        Self {
            client,
            base: config.api_base().to_string(),
            corp_id: config.corp_id.clone(),
            agent_id: config.agent_id,
            secret: config.secret.clone(),
            token: tokio::sync::Mutex::new(CachedToken::default()),
        }
    }

    /// The URL the platform's calls go to.
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// A usable access token, fetching one only when the cached one is spent.
    ///
    /// The lock is held across the fetch on purpose: several sends arriving at
    /// once must not each spend a token request, because fetching in a loop is
    /// itself rate-limited by the platform.
    async fn access_token(&self) -> Result<String> {
        let mut cached = self.token.lock().await;
        if !cached.value.is_empty() && Instant::now() < cached.expires_at {
            return Ok(cached.value.clone());
        }
        let url = format!(
            "{}?corpid={}&corpsecret={}",
            self.url("/cgi-bin/gettoken"),
            self.corp_id,
            self.secret
        );
        let response = send_json(
            &self.client,
            reqwest::Method::GET,
            &url,
            &[],
            None,
            RetryPolicy::single_attempt(),
        )
        .await
        .map_err(|error| anyhow!("wecom `gettoken` failed: {error}"))?;
        check_ok("gettoken", &response)?;
        let token = response
            .body
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| anyhow!("wecom `gettoken` returned no access_token"))?
            .to_string();
        let expires_in = response
            .body
            .get("expires_in")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        // A shorter-than-advertised lifetime is possible, and a zero would mean
        // "expires now" and refetch forever, so the margin is applied to what
        // the platform promised with a floor of one second.
        let lifetime = Duration::from_secs(expires_in.max(1) as u64);
        cached.value = token.clone();
        cached.expires_at = Instant::now() + lifetime.saturating_sub(TOKEN_REFRESH_MARGIN);
        Ok(token)
    }

    /// Send one already-chunked message.
    async fn send_chunk(&self, to: &str, text: &str) -> Result<Option<String>> {
        let token = self.access_token().await?;
        let url = format!("{}?access_token={token}", self.url("/cgi-bin/message/send"));
        let body = json!({
            "touser": to,
            "msgtype": "text",
            "agentid": self.agent_id,
            "text": { "content": text },
        });
        let response = send_json(
            &self.client,
            reqwest::Method::POST,
            &url,
            &[],
            Some(&body),
            RetryPolicy::default(),
        )
        .await
        .map_err(|error| anyhow!("wecom `message/send` failed: {error}"))?;
        if let Err(error) = check_ok("message/send", &response) {
            if is_token_error(errcode_of(&response.body)) {
                // Keeping a rejected token would make every retry fail the same
                // way, so the next attempt fetches a fresh one.
                self.token.lock().await.value.clear();
            }
            return Err(error);
        }
        Ok(message_id_of(&response.body))
    }
}

/// The platform message id out of a successful send.
fn message_id_of(body: &Value) -> Option<String> {
    body.get("msgid")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

#[async_trait]
impl ChannelSender for WecomSender {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    async fn send_text(
        &self,
        conversation: &ConversationRef,
        text: &str,
    ) -> Result<Option<String>> {
        if conversation.id.trim().is_empty() {
            bail!("wecom conversation has no recipient");
        }
        self.send_chunk(&conversation.id, text).await
    }
}

/// The WeCom provider.
struct Wecom;

#[async_trait]
impl Provider for Wecom {
    fn definition(&self) -> &'static ChannelDefinition {
        &DEFINITION
    }

    fn sender(&self, ctx: &ProviderCtx) -> Result<Arc<dyn ChannelSender>> {
        let config: WecomConfig = ctx.config()?;
        config.require_credentials(ctx.id())?;
        Ok(Arc::new(WecomSender::new(ctx.http().clone(), &config)))
    }

    async fn run(&self, ctx: ProviderCtx) -> Result<()> {
        let config: WecomConfig = ctx.config()?;
        config.require_credentials(ctx.id())?;
        config.require_callback_secrets(ctx.id())?;
        // Built once, at startup: a bad key is a configuration error worth
        // reporting before the port is open, not a per-callback failure.
        let key = Arc::new(AesKey::parse(&config.encoding_aes_key)?);
        let sender = Arc::new(WecomSender::new(ctx.http().clone(), &config));

        let token = config.token.clone();
        let corp_id = config.corp_id.clone();
        let (addr, path) = (
            config.webhook_addr().to_string(),
            config.webhook_path().to_string(),
        );

        let verify_key = key.clone();
        let verify_token = token.clone();
        let verify_corp = corp_id.clone();
        let message_ctx = ctx.clone();
        let message_token = token.clone();
        let server = WebhookServer::bind(&addr)
            .await?
            .on("GET", &path, move |request| {
                let key = verify_key.clone();
                let token = verify_token.clone();
                let corp_id = verify_corp.clone();
                async move { verify_endpoint(&request, &token, &key, &corp_id) }
            })
            .on("POST", &path, move |request| {
                let ctx = message_ctx.clone();
                let sender = sender.clone();
                let key = key.clone();
                let token = message_token.clone();
                let corp_id = corp_id.clone();
                async move {
                    match read_callback(&request, &token, &key, &corp_id) {
                        Ok(message) => {
                            let ctx = ctx.clone();
                            let sender = sender.clone();
                            // Answer first, work after: the platform waits five
                            // seconds and retries three times, and running a
                            // turn takes longer.
                            tokio::spawn(async move {
                                deliver(&ctx, &sender, &message).await;
                            });
                            // The documented "no passive reply": an empty body.
                            // The answer itself goes out through the send API.
                            WebhookResponse::text(200, "")
                        }
                        Err(response) => response,
                    }
                }
            });
        ctx.mark_running();
        tracing::info!(addr = %addr, path = %path, "wecom callback listening");
        server.serve(ctx.shutdown().clone()).await
    }

    async fn probe(&self, ctx: &ProviderCtx) -> Result<String> {
        let config: WecomConfig = ctx.config()?;
        config.require_credentials(ctx.id())?;
        // Fetching the token proves the corp id and the secret; reading the
        // agent back proves the agent id belongs to the same app. Together they
        // are the smallest request set that exercises every credential.
        let sender = WecomSender::new(ctx.http().clone(), &config);
        let token = sender.access_token().await?;
        let url = format!(
            "{}?access_token={token}&agentid={}",
            sender.url("/cgi-bin/agent/get"),
            config.agent_id
        );
        let response = send_json(
            ctx.http(),
            reqwest::Method::GET,
            &url,
            &[],
            None,
            RetryPolicy::single_attempt(),
        )
        .await?;
        check_ok("agent/get", &response)?;
        let name = response
            .body
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("");
        if name.is_empty() {
            Ok(format!("connected as agent {}", config.agent_id))
        } else {
            Ok(format!("connected as {name} (agent {})", config.agent_id))
        }
    }
}

/// The verified, decrypted message in one callback.
///
/// Split out of the request handler so the whole security path — element
/// extraction, signature, decryption, receiver check — can be exercised without
/// a socket. The error is the response to send back, so the caller cannot
/// accidentally answer a rejected callback with anything but a rejection.
fn read_callback(
    request: &WebhookRequest,
    token: &str,
    key: &AesKey,
    corp_id: &str,
) -> std::result::Result<String, WebhookResponse> {
    let body = String::from_utf8_lossy(&request.body).to_string();
    let Some(encrypted) = xml_tag(&body, "Encrypt") else {
        return Err(WebhookResponse::bad_request("no Encrypt element"));
    };
    let timestamp = request.query_param("timestamp").unwrap_or_default();
    let nonce = request.query_param("nonce").unwrap_or_default();
    let presented = request.query_param("msg_signature").unwrap_or_default();
    // Verified before decryption: nothing below this line runs on a payload
    // that did not come from the platform.
    if !verify_callback(token, &timestamp, &nonce, &encrypted, &presented) {
        tracing::warn!(
            channel = "wecom",
            "rejected a callback whose signature did not verify"
        );
        return Err(WebhookResponse::unauthorized());
    }
    decrypt_callback(key, &encrypted, corp_id).map_err(|error| {
        // The signature was good, so this is a key mismatch or a platform
        // change, not an attack.
        tracing::warn!(channel = "wecom", %error, "callback could not be decrypted");
        WebhookResponse::bad_request("undecryptable callback")
    })
}

/// The one-time callback URL verification.
///
/// The platform sends `echoStr` — itself an encrypted envelope whose message is
/// the challenge — and expects the challenge back as plain text.
fn verify_endpoint(
    request: &WebhookRequest,
    token: &str,
    key: &AesKey,
    corp_id: &str,
) -> WebhookResponse {
    if request.method != "GET" {
        return WebhookResponse::not_found();
    }
    let timestamp = request.query_param("timestamp").unwrap_or_default();
    let nonce = request.query_param("nonce").unwrap_or_default();
    let echo = request.query_param("echostr").unwrap_or_default();
    let presented = request.query_param("msg_signature").unwrap_or_default();
    if echo.is_empty() {
        return WebhookResponse::bad_request("no echostr");
    }
    if !verify_callback(token, &timestamp, &nonce, &echo, &presented) {
        return WebhookResponse::unauthorized();
    }
    match decrypt_callback(key, &echo, corp_id) {
        Ok(challenge) => WebhookResponse::text(200, challenge),
        Err(error) => {
            tracing::warn!(%error, "the verification challenge could not be decrypted");
            WebhookResponse::bad_request("undecryptable challenge")
        }
    }
}

/// Turn one decrypted callback into the bridge pipeline.
///
/// Runs after the HTTP response, so it may wait for a turn. A message the
/// provider cannot turn into a prompt is dropped with a reason rather than
/// becoming an empty one.
async fn deliver(ctx: &ProviderCtx, outbound: &Arc<WecomSender>, message: &str) {
    let Some(inbound) = parse_message(message) else {
        tracing::debug!(channel = "wecom", "callback carried no text prompt");
        return;
    };
    let sender: Arc<dyn ChannelSender> = outbound.clone();
    let outcome = ctx.handle(inbound, sender).await;
    tracing::debug!(?outcome, "wecom message handled");
}

pub fn provider() -> Box<dyn Provider> {
    Box::new(Wecom)
}

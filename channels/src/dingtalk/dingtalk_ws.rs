//! DingTalk WebSocket stream client.
//! Connects to DingTalk Gateway via Stream Mode (no OAuth2 needed).
//! Reference: https://github.com/open-dingtalk/dingtalk-stream-sdk-python

use anyhow::{anyhow, Result};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::Arc;
use tokio::time::Duration;
use tokio_tungstenite::{connect_async_tls_with_config, tungstenite::Message as WsMessage};
use tracing::{debug, info, warn};

/// Event received from DingTalk stream.
#[derive(Debug, Clone)]
pub struct DingtalkEvent {
    pub event_type: String,
    pub message_id: Option<String>,
    pub chat_id: Option<String>,
    pub chat_type: Option<String>,
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    pub msg_type: Option<String>,
    pub content: Option<String>,
    pub create_time_ms: Option<i64>,
    /// URL for replying to this message (POST to this URL with access token).
    pub session_webhook: Option<String>,
    /// The bot's own user ID in this conversation.
    pub chatbot_user_id: Option<String>,
    pub raw: Value,
}

const PING_INTERVAL_SECS: u64 = 20;
/// UA string sent when opening the connection.
const UA: &str = "future-os/1.0 dingtalk-stream-sdk/1.0";

pub struct DingtalkWsClient {
    client_id: String,
    client_secret: String,
    domain: String,
    ping_interval_secs: u64,
}

impl DingtalkWsClient {
    pub fn new(domain: &str, client_id: &str, client_secret: &str) -> Self {
        Self {
            domain: domain.to_string(),
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            ping_interval_secs: PING_INTERVAL_SECS,
        }
    }

    /// Test seam: shrink the keepalive timer so the ping path is reachable
    /// in real-time tests.
    #[cfg(test)]
    pub(crate) fn with_test_ping_interval(mut self, secs: u64) -> Self {
        self.ping_interval_secs = secs;
        self
    }

    /// Open a Stream Mode connection by POSTing credentials directly
    /// (no OAuth2 token). Returns the WebSocket endpoint and ticket.
    async fn open_connection(&self) -> Result<(String, String)> {
        let client = crate::tls::http_client();
        let url = format!(
            "{}/v1.0/gateway/connections/open",
            super::config::base_url(&self.domain)
        );

        let body = serde_json::json!({
            "clientId": self.client_id,
            "clientSecret": self.client_secret,
            "subscriptions": [
                {"type": "CALLBACK", "topic": "/v1.0/im/bot/messages/get"}
            ],
            "ua": UA,
            "localIp": "127.0.0.1",
        });

        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "DingTalk open connection failed (HTTP {}): {}",
                status.as_u16(),
                text
            ));
        }

        let raw: Value = resp.json().await?;
        let endpoint = raw
            .get("endpoint")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing endpoint in gateway response: {}", raw))?;
        let ticket = raw
            .get("ticket")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing ticket in gateway response: {}", raw))?;

        info!(
            "DingTalk Gateway endpoint={} ticket={:.16}...",
            endpoint,
            &ticket[..ticket.len().min(16)]
        );
        Ok((endpoint.to_string(), ticket.to_string()))
    }

    /// Connect to the DingTalk WebSocket and start processing events.
    /// Reconnects on connection loss (caller should invoke in a loop).
    pub async fn connect_and_listen<F>(&self, mut on_event: F) -> Result<()>
    where
        F: FnMut(DingtalkEvent),
    {
        let (endpoint, ticket) = self.open_connection().await?;
        let ws_url = format!("{}?ticket={}", endpoint, urlencoding(&ticket));
        info!("DingTalk WebSocket connecting: {}", ws_url);

        let (ws_stream, _response) =
            connect_async_tls_with_config(&ws_url, None, false, Some(crate::tls::ws_connector()))
                .await
                .map_err(|e| anyhow!("DingTalk WebSocket connection failed: {}", e))?;

        info!("DingTalk WebSocket connected.");

        // Split so keepalive and ACK sends don't block the read loop.
        // Matches official SDK: keepalive is a separate asyncio.Task,
        // and ACKs are sent from background_task coroutines.
        let (ws_sink, mut ws_stream) = ws_stream.split();
        let ws_sink = Arc::new(tokio::sync::Mutex::new(ws_sink));

        // Spawn keepalive — matches SDK's create_task(self.keepalive(websocket))
        // plus Python websockets library built-in ping_interval=20.
        let keepalive = tokio::spawn(keepalive_loop(ws_sink.clone(), self.ping_interval_secs));

        loop {
            // tungstenite yields None only after a completed close handshake
            // (EOF without one surfaces as Err) — map it to the Close arm.
            let msg = ws_stream.next().await.unwrap_or(Ok(WsMessage::Close(None)));
            match msg {
                Ok(WsMessage::Text(text)) => {
                    match serde_json::from_str::<Value>(&text) {
                        Ok(msg_data) => {
                            let msg_type =
                                msg_data.get("type").and_then(|v| v.as_str()).unwrap_or("");

                            info!("DingTalk WS raw: {}", text);
                            match msg_type {
                                "PONG" => debug!("DingTalk pong"),
                                "SYSTEM" => {
                                    let headers = msg_data.get("headers");
                                    let topic = headers
                                        .and_then(|h| h.get("topic"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    info!("DingTalk SYSTEM topic={}", topic);
                                    // Spawn ACK to avoid blocking read loop (matches SDK's background_task).
                                    let ack_sink = ws_sink.clone();
                                    let ack_data = msg_data.clone();
                                    tokio::spawn(async move {
                                        let mut sink = ack_sink.lock().await;
                                        let _ =
                                            send_ack_inner(&mut *sink, &ack_data, 200, "").await;
                                    });
                                    if topic == "disconnect" {
                                        info!("DingTalk server requested disconnect");
                                        keepalive.abort();
                                        return Ok(());
                                    }
                                }
                                "EVENT" => {
                                    if let Some(event) = parse_dingtalk_event(&msg_data) {
                                        on_event(event);
                                    }
                                    let ack_sink = ws_sink.clone();
                                    let ack_data = msg_data.clone();
                                    tokio::spawn(async move {
                                        let mut sink = ack_sink.lock().await;
                                        let _ =
                                            send_ack_inner(&mut *sink, &ack_data, 200, "").await;
                                    });
                                }
                                "CALLBACK" => {
                                    let topic = msg_data
                                        .get("headers")
                                        .and_then(|h| h.get("topic"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    info!("DingTalk CALLBACK topic={}", topic);
                                    dispatch_bot_callback(&msg_data, topic, &mut on_event);
                                    let ack_sink = ws_sink.clone();
                                    let ack_data = msg_data.clone();
                                    tokio::spawn(async move {
                                        let mut sink = ack_sink.lock().await;
                                        let _ =
                                            send_ack_inner(&mut *sink, &ack_data, 200, "").await;
                                    });
                                }
                                other => {
                                    debug!("DingTalk unknown type: {}", other);
                                }
                            }
                        }
                        Err(e) => warn!("DingTalk JSON parse error: {}", e),
                    }
                }
                Ok(WsMessage::Ping(data)) => {
                    let pong_sink = ws_sink.clone();
                    tokio::spawn(async move {
                        let mut sink = pong_sink.lock().await;
                        let _ = sink.send(WsMessage::Pong(data)).await;
                    });
                }
                Ok(WsMessage::Close(_)) => {
                    info!("DingTalk WebSocket closed by server");
                    keepalive.abort();
                    return Ok(());
                }
                Err(e) => {
                    keepalive.abort();
                    return Err(anyhow!("DingTalk WebSocket error: {}", e));
                }
                _ => {}
            }
        }
    }
}

/// Keepalive task body: ping every `ping_secs`, exiting when the sink fails
/// (connection gone). Free function so tests can drive it to completion.
async fn keepalive_loop<S>(sink: Arc<tokio::sync::Mutex<S>>, ping_secs: u64)
where
    S: futures::Sink<WsMessage, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    loop {
        tokio::time::sleep(Duration::from_secs(ping_secs)).await;
        let mut sink = sink.lock().await;
        if sink.send(WsMessage::Ping(vec![])).await.is_err() {
            break;
        }
    }
}

/// URL-encode a string (RFC 3986), matching Python's quote_plus behavior.
fn urlencoding(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => result.push('+'),
            _ => {
                result.push('%');
                result.push(hex_char(byte >> 4));
                result.push(hex_char(byte & 0x0f));
            }
        }
    }
    result
}

fn hex_char(b: u8) -> char {
    match b {
        0..=9 => (b'0' + b) as char,
        _ => (b'A' + (b - 10)) as char,
    }
}

/// Send an ACK response back to DingTalk Stream.
/// The ACK must include messageId and contentType in headers (matching Python SDK).
async fn send_ack_inner(
    ws: &mut (impl futures::Sink<WsMessage, Error = tokio_tungstenite::tungstenite::Error> + Unpin),
    msg: &Value,
    code: u32,
    message: &str,
) {
    let message_id = msg
        .get("headers")
        .and_then(|h| h.get("messageId"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // Parse the incoming data field (a JSON string) into a Value,
    // then re-serialize it — matching Python SDK's json.loads → json.dumps roundtrip.
    let data_val: Value = msg
        .get("data")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::json!({}));
    let ack = serde_json::json!({
        "code": code,
        "headers": {
            "messageId": message_id,
            "contentType": "application/json",
        },
        "message": message,
        "data": serde_json::to_string(&data_val).unwrap_or("{}".to_string()),
    });
    // The send only fails when the connection is already gone — the caller
    // doesn't care, so neither do we.
    let _ = ws.send(WsMessage::Text(ack.to_string())).await;
}

/// Parse a DingTalk event from a Stream protocol frame (EVENT or CALLBACK).
/// The event data is nested: { headers: { eventType, ... }, data: "<JSON string>" }
/// First present string field among camelCase/snake_case key variants.
/// Shared closures: each fires across the parse test suite.
fn str_field(body: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| body.get(k).and_then(|v| v.as_str()))
        .map(|s| s.to_string())
}

/// Dispatch a CALLBACK frame on the bot-messages topics to the event handler.
fn dispatch_bot_callback(msg_data: &Value, topic: &str, on_event: &mut impl FnMut(DingtalkEvent)) {
    if topic != "/v1.0/im/bot/messages/get" && topic != "/v1.0/im/bot/messages/delegate" {
        return;
    }
    // parse cannot fail here: its only None edge is missing headers, but the
    // topic was just read out of the headers. inspect (not if-let) because
    // rustfmt explodes single-line if-lets and the dead edge would leave an
    // uncovered brace line.
    let _ = parse_dingtalk_event(msg_data).inspect(|event| on_event(event.clone()));
}

fn parse_dingtalk_event(msg: &Value) -> Option<DingtalkEvent> {
    let headers = msg.get("headers")?;
    let msg_type_str = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");

    // Data field is a JSON-encoded string containing the actual event/message body
    let data_str = msg.get("data").and_then(|v| v.as_str()).unwrap_or("{}");
    let body: Value = serde_json::from_str(data_str).unwrap_or_default();

    let event_type = headers
        .get("eventType")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // For CALLBACK bot messages, extract fields from body directly (ChatbotMessage format)
    let sender_id = body
        .get("senderId")
        .or_else(|| body.get("sender_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let chat_id = body
        .get("conversationId")
        .or_else(|| body.get("conversation_id"))
        .or_else(|| body.get("openConversationId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let chat_type = body
        .get("conversationType")
        .or_else(|| body.get("conversation_type"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // For CALLBACK: text can be {content: "..."} or a plain string
    let text_content = body
        .get("text")
        .and_then(|v| v.get("content"))
        .and_then(|v| v.as_str())
        .or_else(|| body.get("text").and_then(|v| v.as_str()))
        .or_else(|| body.get("content").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let msg_type = body
        .get("msgType")
        .or_else(|| body.get("msgtype"))
        .or_else(|| body.get("message_type"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let content = text_content.or_else(|| {
        // Reached only when `content` is not a plain string (that case is
        // already captured in text_content above) — stringify objects.
        body.get("content").map(|v| v.to_string())
    });
    let message_id = headers
        .get("messageId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| str_field(&body, &["messageId", "message_id"]));
    let sender_name = body
        .get("senderNick")
        .or_else(|| body.get("sender_nick"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let create_time_ms = body
        .get("createAt")
        .or_else(|| body.get("create_at"))
        .and_then(|v| v.as_i64());
    let session_webhook = body
        .get("sessionWebhook")
        .or_else(|| body.get("session_webhook"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let chatbot_user_id = body
        .get("chatbotUserId")
        .or_else(|| body.get("chatbot_user_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Some(DingtalkEvent {
        event_type: if event_type.is_empty() {
            msg_type_str.to_string()
        } else {
            event_type
        },
        message_id,
        chat_id,
        chat_type,
        sender_id,
        sender_name,
        msg_type,
        content,
        create_time_ms,
        session_webhook,
        chatbot_user_id,
        raw: msg.clone(),
    })
}

/// Extract text content from a DingTalk message.
pub fn extract_text_content(content: &str, msg_type: &str) -> Option<String> {
    match msg_type {
        "text" => {
            let parsed: Value = serde_json::from_str(content).ok()?;
            parsed["content"].as_str().map(|s| s.to_string())
        }
        _ => Some(content.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shared no-op callback: a plain fn item leaves no uncalled-closure
    /// region on the call lines (unlike `|_| {}` at every site).
    fn ignore_event(_: DingtalkEvent) {}

    #[test]
    fn ignore_event_is_callable() {
        ignore_event(parse_dingtalk_event(&bot_message_frame()).expect("must parse"));
    }

    /// A realistic CALLBACK frame as delivered by DingTalk Stream Mode:
    /// `data` is a JSON *string* (not an object) holding the ChatbotMessage.
    fn bot_message_frame() -> Value {
        let data = serde_json::json!({
            "senderId": "user-1",
            "senderNick": "Alice",
            "conversationId": "cid-1",
            "conversationType": "1",
            "msgtype": "text",
            "text": {"content": "hello bot"},
            "sessionWebhook": "https://oapi.dingtalk.com/robot/sendBySession?session=abc",
            "chatbotUserId": "bot-1",
            "createAt": 1700000000000i64
        });
        serde_json::json!({
            "type": "CALLBACK",
            "headers": {
                "messageId": "mid-123",
                "topic": "/v1.0/im/bot/messages/get"
            },
            "data": data.to_string()
        })
    }

    #[test]
    fn parses_callback_chatbot_message() {
        let ev = parse_dingtalk_event(&bot_message_frame()).expect("must parse");
        assert_eq!(ev.event_type, "CALLBACK");
        assert_eq!(ev.message_id.as_deref(), Some("mid-123"));
        assert_eq!(ev.sender_id.as_deref(), Some("user-1"));
        assert_eq!(ev.sender_name.as_deref(), Some("Alice"));
        assert_eq!(ev.chat_id.as_deref(), Some("cid-1"));
        assert_eq!(ev.msg_type.as_deref(), Some("text"));
        // text as {content: "..."} object form must be unwrapped.
        assert_eq!(ev.content.as_deref(), Some("hello bot"));
        assert_eq!(
            ev.session_webhook.as_deref(),
            Some("https://oapi.dingtalk.com/robot/sendBySession?session=abc")
        );
        assert_eq!(ev.chatbot_user_id.as_deref(), Some("bot-1"));
        assert_eq!(ev.create_time_ms, Some(1700000000000));
    }

    #[test]
    fn parses_snake_case_and_plain_text_variants() {
        // Some payloads use snake_case keys and a plain-string text field.
        let data = serde_json::json!({
            "sender_id": "user-2",
            "conversation_id": "cid-2",
            "msg_type": "text",
            "text": "plain string text"
        });
        let frame = serde_json::json!({
            "type": "CALLBACK",
            "headers": {"messageId": "mid-9"},
            "data": data.to_string()
        });
        let ev = parse_dingtalk_event(&frame).expect("must parse");
        assert_eq!(ev.sender_id.as_deref(), Some("user-2"));
        assert_eq!(ev.content.as_deref(), Some("plain string text"));
    }

    #[test]
    fn missing_headers_returns_none() {
        assert!(parse_dingtalk_event(&serde_json::json!({"data": "{}"})).is_none());
    }

    #[test]
    fn malformed_data_string_yields_event_with_empty_fields() {
        // A non-JSON data string must not panic — fields degrade to None.
        let frame = serde_json::json!({
            "type": "EVENT",
            "headers": {"messageId": "mid-x", "eventType": "topic"},
            "data": "not json"
        });
        let ev = parse_dingtalk_event(&frame).expect("headers exist → Some");
        assert_eq!(ev.event_type, "topic");
        assert_eq!(ev.sender_id, None);
    }

    #[test]
    fn extract_text_prefers_structured_content() {
        assert_eq!(
            extract_text_content(r#"{"content":"hi"}"#, "text").as_deref(),
            Some("hi")
        );
        // Non-text types pass the raw payload through unchanged.
        assert_eq!(
            extract_text_content("raw payload", "picture").as_deref(),
            Some("raw payload")
        );
        // Invalid JSON for a text message → None (the .ok()? edge).
        assert_eq!(extract_text_content("not json", "text"), None);
    }

    // ─── Mock-server-backed tests ────────────────────────────────────────────

    use crate::test_support::{self as ts, HttpRoute, WsAction};
    use tokio_tungstenite::tungstenite::Message as WsMsg;

    fn gateway_route(ws_url: &str) -> HttpRoute {
        HttpRoute::json(
            "/v1.0/gateway/connections/open",
            200,
            &serde_json::json!({"endpoint": ws_url, "ticket": "ticket-1"}).to_string(),
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn open_connection_ok_and_errors() {
        ts::ensure_crypto_provider();
        let (base, recorded) = ts::spawn_http(vec![gateway_route("wss://gw/")]).await;
        let c = DingtalkWsClient::new(&base, "id", "secret");
        let (endpoint, ticket) = c.open_connection().await.unwrap();
        assert_eq!(endpoint, "wss://gw/");
        assert_eq!(ticket, "ticket-1");
        // The subscription registers the bot-messages CALLBACK topic.
        let calls = ts::requests_to(&recorded, "/v1.0/gateway/connections/open");
        let body = calls[0].body_string();
        assert!(body.contains("/v1.0/im/bot/messages/get"));
        assert!(body.contains("\"clientId\":\"id\""));

        // HTTP error status.
        let (base, _) = ts::spawn_http(vec![HttpRoute::json(
            "/v1.0/gateway/connections/open",
            500,
            "gateway down",
        )])
        .await;
        let err = DingtalkWsClient::new(&base, "id", "s")
            .open_connection()
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("HTTP 500"), "{err}");
        assert!(err.to_string().contains("gateway down"), "{err}");

        // Non-JSON 200 body → resp.json() error edge.
        let (base, _) = ts::spawn_http(vec![HttpRoute::json(
            "/v1.0/gateway/connections/open",
            200,
            "this is not json",
        )])
        .await;
        let err = DingtalkWsClient::new(&base, "id", "s")
            .open_connection()
            .await
            .err()
            .unwrap();
        assert!(!err.to_string().is_empty());

        // Missing endpoint / missing ticket.
        let (base, _) = ts::spawn_http(vec![HttpRoute::json(
            "/v1.0/gateway/connections/open",
            200,
            r#"{"ticket":"t"}"#,
        )])
        .await;
        let err = DingtalkWsClient::new(&base, "id", "s")
            .open_connection()
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("Missing endpoint"), "{err}");

        let (base, _) = ts::spawn_http(vec![HttpRoute::json(
            "/v1.0/gateway/connections/open",
            200,
            r#"{"endpoint":"wss://gw/"}"#,
        )])
        .await;
        let err = DingtalkWsClient::new(&base, "id", "s")
            .open_connection()
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("Missing ticket"), "{err}");

        // Transport failure.
        let err = DingtalkWsClient::new("http://127.0.0.1:1", "id", "s")
            .open_connection()
            .await
            .err()
            .unwrap();
        assert!(!err.to_string().is_empty());
    }

    fn callback_frame() -> String {
        bot_message_frame().to_string()
    }

    // current_thread: the tracing subscriber is thread-local — the event
    // construction regions in connect_and_listen only count when the log
    // calls run on this thread.
    #[tokio::test(flavor = "current_thread")]
    async fn connect_listen_full_frame_flow() {
        let _sub = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .finish(),
        );
        ts::ensure_crypto_provider();
        let event_frame = serde_json::json!({
            "type": "EVENT",
            "headers": {"messageId": "mid-ev", "eventType": "some.event"},
            "data": "{\"senderId\":\"u-9\"}"
        })
        .to_string();
        let (ws_url, received) = ts::spawn_ws(vec![
            WsAction::SendText(serde_json::json!({"type": "PONG"}).to_string()),
            // SYSTEM with a benign topic → ACK, keep going.
            WsAction::SendText(
                serde_json::json!({"type": "SYSTEM", "headers": {"messageId": "sys-1", "topic": "heartbeat"}, "data": "{}"})
                    .to_string(),
            ),
            // EVENT → on_event + ACK.
            WsAction::SendText(event_frame),
            // EVENT that doesn't parse (no headers) → ACK only.
            WsAction::SendText(serde_json::json!({"type": "EVENT", "data": "{}"}).to_string()),
            // CALLBACK on the bot-messages topic → on_event + ACK.
            WsAction::SendText(callback_frame()),
            // CALLBACK on another topic → ACK only.
            WsAction::SendText(
                serde_json::json!({"type": "CALLBACK", "headers": {"messageId": "cb-9", "topic": "/other"}, "data": "{}"})
                    .to_string(),
            ),
            // CALLBACK delegate topic → on_event.
            WsAction::SendText(
                serde_json::json!({
                    "type": "CALLBACK",
                    "headers": {"messageId": "mid-del", "topic": "/v1.0/im/bot/messages/delegate"},
                    "data": "{\"senderId\":\"u-del\"}"
                })
                .to_string(),
            ),
            // Unknown type → debug only.
            WsAction::SendText(serde_json::json!({"type": "MYSTERY"}).to_string()),
            // Invalid JSON → warn.
            WsAction::SendText("not json at all".to_string()),
            // WS protocol ping → client pongs.
            WsAction::SendPing(b"dt".to_vec()),
            WsAction::Delay(Duration::from_millis(400)),
            WsAction::SendClose,
        ])
        .await;
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/stream", ws_url))]).await;
        let client = DingtalkWsClient::new(&base, "id", "secret");
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let events_clone = events.clone();
        client
            .connect_and_listen(move |ev| events_clone.lock().unwrap().push(ev))
            .await
            .expect("clean close");
        let events = events.lock().unwrap();
        // EVENT (eventType some.event) + CALLBACK bot + CALLBACK delegate.
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, "some.event");
        assert_eq!(events[1].message_id.as_deref(), Some("mid-123"));
        assert_eq!(events[2].sender_id.as_deref(), Some("u-del"));
        drop(events);

        // ACKs were sent for SYSTEM/EVENT/CALLBACK frames; a WS Pong answered
        // the protocol ping.
        let got = received.lock().unwrap();
        let acks: Vec<_> = got
            .iter()
            .filter_map(|m| match m {
                WsMsg::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert!(acks
            .iter()
            .any(|a| a.contains("\"code\":200") && a.contains("sys-1")));
        assert!(acks.iter().any(|a| a.contains("mid-ev")));
        assert!(acks.iter().any(|a| a.contains("mid-123")));
        assert!(acks.iter().any(|a| a.contains("cb-9")));
        assert!(
            got.iter()
                .any(|m| matches!(m, WsMsg::Pong(p) if p == b"dt")),
            "client must answer WS ping"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connect_listen_disconnect_and_errors() {
        ts::ensure_crypto_provider();
        // SYSTEM disconnect → clean Ok.
        let (ws_url, _) = ts::spawn_ws(vec![WsAction::SendText(
            serde_json::json!({"type": "SYSTEM", "headers": {"messageId": "d-1", "topic": "disconnect"}, "data": "{}"})
                .to_string(),
        )])
        .await;
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/stream", ws_url))]).await;
        DingtalkWsClient::new(&base, "id", "s")
            .connect_and_listen(ignore_event)
            .await
            .expect("disconnect is Ok");

        // Dead endpoint → connection failed.
        let (base, _) = ts::spawn_http(vec![gateway_route("ws://127.0.0.1:1/")]).await;
        let err = DingtalkWsClient::new(&base, "id", "s")
            .connect_and_listen(ignore_event)
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("connection failed"), "{err}");

        // Protocol garbage → WebSocket error arm.
        let (ws_url, _) = ts::spawn_ws(vec![
            WsAction::SendRawBytes(vec![0x83, 0x00]),
            WsAction::Delay(Duration::from_millis(300)),
        ])
        .await;
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/stream", ws_url))]).await;
        let err = DingtalkWsClient::new(&base, "id", "s")
            .connect_and_listen(ignore_event)
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("WebSocket error"), "{err}");

        // EOF without close handshake → same error arm.
        let (ws_url, _) = ts::spawn_ws(vec![]).await;
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/stream", ws_url))]).await;
        let err = DingtalkWsClient::new(&base, "id", "s")
            .connect_and_listen(ignore_event)
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("WebSocket error"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connect_listen_sends_keepalive_ping() {
        ts::ensure_crypto_provider();
        let (ws_url, received) = ts::spawn_ws(vec![
            WsAction::Delay(Duration::from_millis(1500)),
            WsAction::SendClose,
        ])
        .await;
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/stream", ws_url))]).await;
        let client = DingtalkWsClient::new(&base, "id", "s").with_test_ping_interval(1);
        client
            .connect_and_listen(ignore_event)
            .await
            .expect("close after keepalive");
        let got = received.lock().unwrap();
        assert!(
            got.iter().any(|m| matches!(m, WsMsg::Ping(_))),
            "keepalive must send WS pings"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn keepalive_loop_exits_when_sink_fails() {
        use tokio::io::AsyncReadExt as _;
        let (client, mut peer) = tokio::io::duplex(4096);
        let ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
            client,
            tokio_tungstenite::tungstenite::protocol::Role::Client,
            None,
        )
        .await;
        let (sink, _stream) = ws.split();
        let sink = Arc::new(tokio::sync::Mutex::new(sink));
        let driver = tokio::spawn(keepalive_loop(sink, 0));
        // Read part of one ping frame so at least one send succeeds…
        let mut header = [0u8; 2];
        peer.read_exact(&mut header).await.unwrap();
        assert_eq!(header[0] & 0x0f, 0x9, "opcode 9 = ping");
        // …then drop the peer: the next send fails and the loop exits.
        drop(peer);
        tokio::time::timeout(Duration::from_secs(2), driver)
            .await
            .expect("keepalive exits on send failure")
            .expect("task not panicked");
    }

    // ─── urlencoding ─────────────────────────────────────────────────────────

    #[test]
    fn urlencoding_rules() {
        assert_eq!(urlencoding("abcXYZ019-._~"), "abcXYZ019-._~");
        assert_eq!(urlencoding("a b"), "a+b");
        assert_eq!(urlencoding("a/b?c=d"), "a%2Fb%3Fc%3Dd");
        assert_eq!(urlencoding("票"), "%E7%A5%A8");
        assert_eq!(urlencoding(""), "");
    }

    #[test]
    fn hex_char_digits_and_letters() {
        assert_eq!(hex_char(0), '0');
        assert_eq!(hex_char(9), '9');
        assert_eq!(hex_char(10), 'A');
        assert_eq!(hex_char(15), 'F');
    }

    // ─── parse_dingtalk_event residual arms ──────────────────────────────────

    #[test]
    fn parses_body_fallback_fields() {
        // message_id/chat_id/sender from the body, object content stringified.
        let data = serde_json::json!({
            "senderId": "u-1",
            "openConversationId": "oc-9",
            "conversation_type": "2",
            "message_id": "body-mid",
            "sender_nick": "Nick",
            "create_at": 123i64,
            "session_webhook": "http://hook",
            "chatbot_user_id": "bot-9",
            "content": {"rich": "object"}
        });
        let frame = serde_json::json!({
            "type": "EVENT",
            "headers": {},
            "data": data.to_string()
        });
        let ev = parse_dingtalk_event(&frame).expect("parses");
        // Empty eventType falls back to the frame type.
        assert_eq!(ev.event_type, "EVENT");
        assert_eq!(ev.message_id.as_deref(), Some("body-mid"));
        assert_eq!(ev.chat_id.as_deref(), Some("oc-9"));
        assert_eq!(ev.chat_type.as_deref(), Some("2"));
        assert_eq!(ev.sender_name.as_deref(), Some("Nick"));
        assert_eq!(ev.create_time_ms, Some(123));
        assert_eq!(ev.session_webhook.as_deref(), Some("http://hook"));
        assert_eq!(ev.chatbot_user_id.as_deref(), Some("bot-9"));
        // Object content is stringified.
        assert!(ev.content.as_deref().unwrap().contains("rich"));
    }

    #[test]
    fn send_ack_roundtrips_data_payload() {
        // Covered through the WS flow, but pin the data-string roundtrip:
        // a non-JSON data string degrades to "{}" in the ACK.
        let msg = serde_json::json!({
            "headers": {"messageId": "m-1"},
            "data": "not json"
        });
        let data_val: Value = msg
            .get("data")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::json!({}));
        assert_eq!(data_val, serde_json::json!({}));
    }

    #[test]
    fn parses_plain_string_content() {
        // content as a plain string passes through (not stringified).
        let frame = serde_json::json!({
            "type": "CALLBACK",
            "headers": {"messageId": "m-2"},
            "data": "{\"senderId\":\"u-1\",\"content\":\"plain text body\"}"
        });
        let ev = parse_dingtalk_event(&frame).expect("parses");
        assert_eq!(ev.content.as_deref(), Some("plain text body"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gateway_log_line_evaluated_with_subscriber() {
        // The endpoint/ticket info! only evaluates its args when a tracing
        // subscriber is installed.
        let _sub = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .finish(),
        );
        ts::ensure_crypto_provider();
        let (ws_url, _) = ts::spawn_ws(vec![WsAction::SendClose]).await;
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/s", ws_url))]).await;
        DingtalkWsClient::new(&base, "id", "s")
            .connect_and_listen(ignore_event)
            .await
            .expect("ok");
    }

    // current_thread + subscriber: the spawned ACK task stays on this thread,
    // and the warn! event region only evaluates under a subscriber.
    #[tokio::test(flavor = "current_thread")]
    async fn ack_send_after_reset_only_warns() {
        let _sub = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .finish(),
        );
        ts::ensure_crypto_provider();
        // CALLBACK frame then RST: the spawned ACK task's send hits the reset
        // connection (warn arm) — the read loop then errors out.
        let (ws_url, _) = ts::spawn_ws(vec![
            WsAction::SendText(bot_message_frame().to_string()),
            WsAction::ResetTcp,
        ])
        .await;
        let (base, _) = ts::spawn_http(vec![gateway_route(&format!("{}/s", ws_url))]).await;
        let err = DingtalkWsClient::new(&base, "id", "s")
            .connect_and_listen(ignore_event)
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("WebSocket error"), "{err}");
    }
}

//! DingTalk WebSocket stream client.
//! Connects to DingTalk Gateway via Stream Mode (no OAuth2 needed).
//! Reference: https://github.com/open-dingtalk/dingtalk-stream-sdk-python

use anyhow::{anyhow, Result};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::Arc;
use tokio::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
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
}

impl DingtalkWsClient {
    pub fn new(domain: &str, client_id: &str, client_secret: &str) -> Self {
        Self {
            domain: domain.to_string(),
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
        }
    }

    /// Open a Stream Mode connection by POSTing credentials directly
    /// (no OAuth2 token). Returns the WebSocket endpoint and ticket.
    async fn open_connection(&self) -> Result<(String, String)> {
        let client = reqwest::Client::new();
        let url = format!(
            "https://{}/v1.0/gateway/connections/open",
            self.domain
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
        let ws_url = format!(
            "{}?ticket={}",
            endpoint,
            urlencoding(&ticket)
        );
        info!("DingTalk WebSocket connecting: {}", ws_url);

        let (ws_stream, _response) = connect_async(&ws_url)
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
        let keepalive_sink = ws_sink.clone();
        let keepalive = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(PING_INTERVAL_SECS)).await;
                let mut sink = keepalive_sink.lock().await;
                if sink.send(WsMessage::Ping(vec![])).await.is_err() {
                    break;
                }
            }
        });

        while let Some(msg) = ws_stream.next().await {
            match msg {
                Ok(WsMessage::Text(text)) => {
                    match serde_json::from_str::<Value>(&text) {
                        Ok(msg_data) => {
                            let msg_type = msg_data
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");

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
                                        let _ = send_ack_inner(&mut *sink, &ack_data, 200, "").await;
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
                                        let _ = send_ack_inner(&mut *sink, &ack_data, 200, "").await;
                                    });
                                }
                                "CALLBACK" => {
                                    let topic = msg_data
                                        .get("headers")
                                        .and_then(|h| h.get("topic"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    info!("DingTalk CALLBACK topic={}", topic);
                                    if topic == "/v1.0/im/bot/messages/get" || topic == "/v1.0/im/bot/messages/delegate" {
                                        if let Some(event) = parse_dingtalk_event(&msg_data) {
                                            on_event(event);
                                        }
                                    }
                                    let ack_sink = ws_sink.clone();
                                    let ack_data = msg_data.clone();
                                    tokio::spawn(async move {
                                        let mut sink = ack_sink.lock().await;
                                        let _ = send_ack_inner(&mut *sink, &ack_data, 200, "").await;
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

        keepalive.abort();
        info!("DingTalk WebSocket stream ended");
        Ok(())
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
        "data": serde_json::to_string(&data_val).unwrap_or_else(|_| "{}".to_string()),
    });
    if let Err(e) = ws.send(WsMessage::Text(ack.to_string())).await {
        warn!("DingTalk ACK send failed: {}", e);
    }
}

/// Parse a DingTalk event from a Stream protocol frame (EVENT or CALLBACK).
/// The event data is nested: { headers: { eventType, ... }, data: "<JSON string>" }
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
        .or_else(|| body.get("content").and_then(|v| {
            if let Some(s) = v.as_str() { Some(s) } else { None }
        }))
        .map(|s| s.to_string());
    let msg_type = body
        .get("msgType")
        .or_else(|| body.get("msgtype"))
        .or_else(|| body.get("message_type"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let content = text_content.or_else(|| body.get("content").map(|v| {
        if let Some(s) = v.as_str() { s.to_string() } else { v.to_string() }
    }));
    let message_id = headers
        .get("messageId")
        .and_then(|v| v.as_str())
        .or_else(|| body.get("messageId").and_then(|v| v.as_str()))
        .or_else(|| body.get("message_id").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
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
        event_type: if event_type.is_empty() { msg_type_str.to_string() } else { event_type },
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

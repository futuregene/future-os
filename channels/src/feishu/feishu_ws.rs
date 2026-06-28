//! Feishu/Lark WebSocket long connection client.
//!
//! Uses the pbbp2 protobuf binary frame protocol over WebSocket.
//! Connection flow: POST /callback/ws/endpoint (app credentials) → get WS URL → connect.

use anyhow::{anyhow, Result};
use futures::{SinkExt, StreamExt};
use prost::Message;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tracing::{debug, info, warn};

// Generated from proto/feishu_ws.proto
mod feishu_pb {
    include!(concat!(env!("OUT_DIR"), "/feishu_ws.rs"));
}

use feishu_pb::{Header, WsFrame};

/// Event received from Feishu WebSocket.
#[derive(Debug, Clone)]
pub struct FeishuEvent {
    pub event_type: String,
    pub message_id: Option<String>,
    pub chat_id: Option<String>,
    pub chat_type: Option<String>,
    pub sender_open_id: Option<String>,
    pub msg_type: Option<String>,
    pub content: Option<String>,
    pub root_id: Option<String>,
    pub parent_id: Option<String>,
    pub tenant_key: Option<String>,
    pub app_id: Option<String>,
    /// Message create time in milliseconds since epoch.
    /// Used to filter out stale messages replayed on reconnect.
    pub create_time_ms: Option<i64>,
    /// Mentions from the message object (Feishu API v2 puts mentions here,
    /// not inside the content JSON).
    pub mentions: Option<Vec<serde_json::Value>>,
    pub raw: serde_json::Value,
}

/// Bootstrap response from /callback/ws/endpoint.
#[derive(Debug, serde::Deserialize)]
struct BootstrapResponse {
    code: i64,
    #[serde(default)]
    msg: String,
    data: Option<BootstrapData>,
}

#[derive(Debug, serde::Deserialize)]
struct BootstrapData {
    #[serde(rename = "URL", default)]
    url: String,
    #[serde(rename = "ClientConfig")]
    client_config: Option<ClientConfig>,
}

#[derive(Debug, serde::Deserialize)]
struct ClientConfig {
    #[serde(rename = "PingInterval", default)]
    ping_interval: Option<i64>,
    #[serde(rename = "ReconnectInterval", default)]
    reconnect_interval: Option<i64>,
    #[serde(rename = "ReconnectNonce", default)]
    reconnect_nonce: Option<i64>,
    #[serde(rename = "ReconnectCount", default)]
    reconnect_count: Option<i64>,
}

/// Default keepalive ping interval (seconds).  Feishu server idle timeout
/// is typically ~90 s; we ping well inside that window.  The server may also
/// send a ``ping_interval`` hint in the bootstrap response — if it does we
/// honour it, clamped to a safe maximum.
const DEFAULT_PING_INTERVAL: u64 = 30;

/// Maximum time without receiving *any* frame before declaring the connection
/// dead and reconnecting.
const HEARTBEAT_TIMEOUT: u64 = 120;

pub struct FeishuWsClient {
    app_id: String,
    app_secret: String,
    domain: String,
    ping_interval: Arc<RwLock<u64>>,
}

impl FeishuWsClient {
    pub fn new(domain: &str, app_id: &str, app_secret: &str) -> Self {
        Self {
            domain: domain.to_string(),
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
            ping_interval: Arc::new(RwLock::new(DEFAULT_PING_INTERVAL)),
        }
    }

    /// Call POST /callback/ws/endpoint to get the WebSocket URL.
    async fn bootstrap_ws(&self) -> Result<(String, Option<ClientConfig>)> {
        let client = reqwest::Client::new();
        let url = format!("{}/callback/ws/endpoint", self.domain);
        let resp: BootstrapResponse = client
            .post(&url)
            .header("locale", "zh")
            .json(&serde_json::json!({
                "AppID": self.app_id,
                "AppSecret": self.app_secret,
            }))
            .send()
            .await?
            .json()
            .await?;

        if resp.code != 0 {
            return Err(anyhow!(
                "WS bootstrap failed: {} (code {})",
                resp.msg,
                resp.code
            ));
        }

        let data = resp
            .data
            .ok_or_else(|| anyhow!("WS bootstrap response missing data"))?;
        if data.url.is_empty() {
            return Err(anyhow!("WS bootstrap response missing URL"));
        }

        info!("WS bootstrap success, URL: {}", data.url);
        Ok((data.url, data.client_config))
    }

    /// Connect to Feishu WebSocket and process events.
    pub async fn connect_and_listen<F>(&self, mut on_event: F) -> Result<()>
    where
        F: FnMut(FeishuEvent),
    {
        // Bootstrap: get WebSocket URL from /callback/ws/endpoint
        let (ws_url, client_config) = self.bootstrap_ws().await?;

        // Apply server-provided client config, clamped to reasonable bounds.
        if let Some(ref cfg) = client_config {
            if let Some(pi) = cfg.ping_interval {
                if pi > 0 {
                    let clamped = (pi as u64).max(15).min(60);
                    *self.ping_interval.write().await = clamped;
                    info!("Server ping interval: {}s (clamped to {}s)", pi, clamped);
                }
            }
        }

        info!("Connecting to Feishu WebSocket...");

        let (mut ws_stream, _response) = connect_async(&ws_url)
            .await
            .map_err(|e| anyhow!("WebSocket connection failed: {}", e))?;

        info!("WebSocket connected. Waiting for events...");

        let ping_interval_arc = self.ping_interval.clone();
        let interval_secs = *ping_interval_arc.read().await;
        let mut ping_timer = interval(Duration::from_secs(interval_secs));
        let mut last_recv = Instant::now();
        let mut seq_id: i64 = 0;

        loop {
            tokio::select! {
                _ = ping_timer.tick() => {
                    if last_recv.elapsed().as_secs() > HEARTBEAT_TIMEOUT {
                        return Err(anyhow!("WebSocket heartbeat timeout"));
                    }
                    // Use WebSocket protocol ping (matches lark_oapi SDK which
                    // delegates to Python websockets library ping_interval=20).
                    // The server's WS stack responds with a protocol pong
                    // automatically — no pbbp2 frame encoding needed.
                    if let Err(e) = ws_stream.send(WsMessage::Ping(vec![])).await {
                        warn!("Failed to send ping: {}", e);
                        return Err(anyhow!("WebSocket send error: {}", e));
                    }
                }

                msg = ws_stream.next() => {
                    match msg {
                        Some(Ok(WsMessage::Ping(data))) => {
                            // Auto-respond to server WS protocol pings.
                            last_recv = Instant::now();
                            let _ = ws_stream.send(WsMessage::Pong(data)).await;
                        }
                        Some(Ok(WsMessage::Pong(_))) => {
                            last_recv = Instant::now();
                            debug!("Received WS pong");
                        }
                        Some(Ok(WsMessage::Binary(data))) => {
                            last_recv = Instant::now();

                            match WsFrame::decode(data.as_ref()) {
                                Ok(frame) => {
                                    let frame_type = frame.headers.iter()
                                        .find(|h| h.key == "type")
                                        .map(|h| h.value.as_str())
                                        .unwrap_or("unknown");

                                    match frame_type {
                                        "ping" => {
                                            debug!("Received ping, sending pong");
                                            seq_id += 1;
                                            let pong_frame = WsFrame {
                                                seq_id,
                                                log_id: frame.log_id,
                                                service: 0,
                                                method: 0,
                                                headers: vec![
                                                    Header { key: "type".into(), value: "pong".into() },
                                                ],
                                                payload: vec![],
                                                payload_encoding: String::new(),
                                                payload_type: String::new(),
                                                log_id_new: String::new(),
                                            };
                                            let mut buf = Vec::new();
                                            if let Err(e) = pong_frame.encode(&mut buf) {
                                                warn!("Failed to encode pong: {}", e);
                                            } else if let Err(e) = ws_stream.send(WsMessage::Binary(buf)).await {
                                                warn!("Failed to send pong: {}", e);
                                            }
                                        }
                                        "pong" => {
                                            debug!("Received pong");
                                        }
                                        "event" => {
                                            let payload_str = String::from_utf8_lossy(&frame.payload);
                                            match serde_json::from_str::<serde_json::Value>(&payload_str) {
                                                Ok(event_data) => {
                                                    if let Some(event) = parse_feishu_event(&event_data) {
                                                        on_event(event);
                                                    }
                                                }
                                                Err(e) => {
                                                    warn!("Failed to parse event payload: {}", e);
                                                }
                                            }
                                        }
                                        other => {
                                            debug!("Unknown frame type: {}", other);
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to decode WsFrame: {}", e);
                                }
                            }
                        }
                        Some(Ok(WsMessage::Close(_))) => {
                            info!("WebSocket closed by server");
                            return Ok(());
                        }
                        Some(Err(e)) => {
                            return Err(anyhow!("WebSocket error: {}", e));
                        }
                        None => {
                            info!("WebSocket stream ended");
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

/// Parse a Feishu event JSON into a structured FeishuEvent.
fn parse_feishu_event(data: &serde_json::Value) -> Option<FeishuEvent> {
    let header = data.get("header")?;
    let event = data.get("event")?;

    let event_type = header.get("event_type")?.as_str()?;

    // Card action event (button click on approval card)
    if event_type == "card.action.trigger" {
        return parse_card_action_event(header, event);
    }

    let message = event.get("message")?;

    if event_type != "im.message.receive_v1" {
        return None;
    }

    let sender = event.get("sender")?;
    let sender_id = sender.get("sender_id")?;
    let sender_open_id = sender_id
        .get("open_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Feishu API v2 uses "message_type", older API uses "msg_type"
    let msg_type = message
        .get("message_type")
        .or_else(|| message.get("msg_type"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let content = message
        .get("content")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let message_id = message
        .get("message_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let chat_id = message
        .get("chat_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let chat_type = message
        .get("chat_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let root_id = message
        .get("root_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let parent_id = message
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Extract mentions from the message object (API v2 format)
    let mentions = message
        .get("mentions")
        .and_then(|v| v.as_array())
        .map(|arr| arr.to_vec());

    let tenant_key = header
        .get("tenant_key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let app_id = header
        .get("app_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Parse create_time from message (millisecond timestamp string)
    let create_time_ms = message
        .get("create_time")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok());

    Some(FeishuEvent {
        event_type: event_type.to_string(),
        message_id,
        chat_id,
        chat_type,
        sender_open_id,
        msg_type,
        content,
        root_id,
        parent_id,
        tenant_key,
        app_id,
        create_time_ms,
        mentions,
        raw: data.clone(),
    })
}

/// Extract text from a Feishu message content JSON.
pub fn extract_text_content(content: &str, msg_type: &str) -> Option<String> {
    match msg_type {
        "text" => {
            let parsed: serde_json::Value = serde_json::from_str(content).ok()?;
            parsed["text"].as_str().map(|s| s.to_string())
        }
        "post" => {
            let parsed: serde_json::Value = serde_json::from_str(content).ok()?;
            let mut texts = Vec::new();
            if let Some(content_blocks) = parsed["content"].as_array() {
                for block in content_blocks {
                    if let Some(elements) = block.as_array() {
                        for element in elements {
                            if let Some("text") = element.get("tag").and_then(|t| t.as_str()) {
                                if let Some(text) = element.get("text").and_then(|t| t.as_str()) {
                                    texts.push(text.to_string());
                                }
                            } else if let Some("at") = element.get("tag").and_then(|t| t.as_str()) {
                                if let Some(uid) = element.get("user_id").and_then(|t| t.as_str()) {
                                    texts.push(format!("@{}", uid));
                                }
                            }
                        }
                    }
                }
            }
            if texts.is_empty() {
                None
            } else {
                Some(texts.join(""))
            }
        }
        _ => None,
    }
}

/// Extract image_key from an image message.
pub fn extract_image_key(content: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(content).ok()?;
    parsed["image_key"].as_str().map(|s| s.to_string())
}

/// Extract file_key from a file/media message.
pub fn extract_file_key(content: &str) -> (Option<String>, Option<String>) {
    let parsed: Option<serde_json::Value> = serde_json::from_str(content).ok();
    match parsed {
        Some(ref p) => (
            p["file_key"].as_str().map(|s| s.to_string()),
            p["file_name"].as_str().map(|s| s.to_string()),
        ),
        None => (None, None),
    }
}

/// Check if the bot was @mentioned in the content JSON (old Feishu API).
/// In the old API, mentions are embedded inside the content JSON string.
pub fn is_bot_mentioned(content: &str, msg_type: &str, bot_open_id: &str) -> bool {
    /// Extract the open_id from a mention's id field, which can be either:
    ///   - a plain string: "ou_xxx"
    ///   - an object: {"open_id": "ou_xxx", "user_id": "xxx"}
    fn mention_id_matches(mention: &serde_json::Value, bot_open_id: &str) -> bool {
        match mention.get("id") {
            // Object format: {"open_id": "ou_xxx", ...}
            Some(id_obj) if id_obj.is_object() => {
                id_obj.get("open_id").and_then(|v| v.as_str()) == Some(bot_open_id)
                    || id_obj.get("user_id").and_then(|v| v.as_str()) == Some(bot_open_id)
            }
            // String format: "ou_xxx"
            Some(id_str) => id_str.as_str() == Some(bot_open_id),
            None => false,
        }
    }

    match msg_type {
        "text" => {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) {
                if let Some(mentions) = parsed["mentions"].as_array() {
                    return mentions.iter().any(|m| mention_id_matches(m, bot_open_id));
                }
            }
            false
        }
        "post" => {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) {
                if let Some(blocks) = parsed["content"].as_array() {
                    for block in blocks {
                        if let Some(elements) = block.as_array() {
                            for element in elements {
                                if element.get("tag").and_then(|t| t.as_str()) == Some("at") {
                                    // user_id can be a string or an object with open_id
                                    let user_id_match = match element.get("user_id") {
                                        Some(uid) if uid.is_object() => {
                                            uid.get("open_id").and_then(|v| v.as_str())
                                                == Some(bot_open_id)
                                        }
                                        Some(uid) => uid.as_str() == Some(bot_open_id),
                                        None => false,
                                    };
                                    if user_id_match {
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            false
        }
        _ => false,
    }
}

/// Parse a card.action.trigger event (button click on an interactive card).
/// Returns a FeishuEvent with event_type="card.action.trigger" and the action
/// value stored in the content field as JSON.
fn parse_card_action_event(
    header: &serde_json::Value,
    event: &serde_json::Value,
) -> Option<FeishuEvent> {
    let action = event.get("action")?;
    let action_value = action.get("value")?;

    let message_id = event
        .get("context")
        .and_then(|c| c.get("open_message_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let chat_id = event
        .get("context")
        .and_then(|c| c.get("chat_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let sender_open_id = event
        .get("operator")
        .and_then(|o| o.get("open_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let tenant_key = header
        .get("tenant_key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let app_id = header
        .get("app_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Some(FeishuEvent {
        event_type: "card.action.trigger".to_string(),
        message_id,
        chat_id,
        chat_type: None,
        sender_open_id,
        msg_type: Some("card_action".to_string()),
        content: Some(action_value.to_string()),
        root_id: None,
        parent_id: None,
        tenant_key,
        app_id,
        create_time_ms: None,
        mentions: None,
        raw: event.clone(),
    })
}

/// Check if the bot is mentioned in the event-level mentions array (Feishu API v2).
/// In API v2, mentions are in `message.mentions` rather than inside the content JSON.
/// Each mention has an `id` object with `open_id`, `union_id`, etc.
pub fn is_bot_mentioned_in_mentions(mentions: &[serde_json::Value], bot_open_id: &str) -> bool {
    mentions.iter().any(|m| match m.get("id") {
        Some(id_obj) if id_obj.is_object() => {
            id_obj.get("open_id").and_then(|v| v.as_str()) == Some(bot_open_id)
        }
        Some(id_str) => id_str.as_str() == Some(bot_open_id),
        None => false,
    })
}

//! Feishu/Lark Open API REST client.
//! Uses reqwest for all HTTP calls.

use anyhow::{anyhow, Result};
use base64::Engine;
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct FeishuRestClient {
    http: reqwest::Client,
    api_base: String,
    app_id: String,
    app_secret: String,
    token: Arc<RwLock<CachedToken>>,
}

struct CachedToken {
    value: String,
    expires_at: Instant,
}

impl FeishuRestClient {
    pub fn new(api_base: &str, app_id: &str, app_secret: &str) -> Self {
        Self {
            http: crate::tls::http_client_builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| crate::tls::http_client()),
            api_base: api_base.to_string(),
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
            token: Arc::new(RwLock::new(CachedToken {
                value: String::new(),
                expires_at: Instant::now(),
            })),
        }
    }

    /// Get tenant access token, with caching and auto-refresh.
    pub async fn get_token(&self) -> Result<String> {
        {
            let cached = self.token.read().await;
            if cached.expires_at > Instant::now() + std::time::Duration::from_secs(60) {
                return Ok(cached.value.clone());
            }
        }

        let url = format!("{}/auth/v3/tenant_access_token/internal", self.api_base);
        let resp: Value = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "app_id": self.app_id,
                "app_secret": self.app_secret,
            }))
            .send()
            .await?
            .json()
            .await?;

        let code = resp["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            return Err(anyhow!(
                "Failed to get tenant token: {} (code {})",
                resp["msg"].as_str().unwrap_or("unknown"),
                code
            ));
        }

        let token = resp["tenant_access_token"]
            .as_str()
            .ok_or_else(|| anyhow!("Token not found in response"))?
            .to_string();
        let expire = resp["expire"].as_i64().unwrap_or(7200);

        let mut cached = self.token.write().await;
        *cached = CachedToken {
            value: token.clone(),
            expires_at: Instant::now() + std::time::Duration::from_secs((expire - 60) as u64),
        };
        Ok(token)
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let token = self.get_token().await?;
        let url = format!("{}{}", self.api_base, path);
        let resp: Value = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(body)
            .send()
            .await?
            .json()
            .await?;

        let code = resp["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            let msg = resp["msg"].as_str().unwrap_or("unknown error");
            return Err(anyhow!("API error ({}): {}", code, msg));
        }
        Ok(resp)
    }

    async fn put_json(&self, path: &str, body: &Value) -> Result<Value> {
        let token = self.get_token().await?;
        let url = format!("{}{}", self.api_base, path);
        let resp: Value = self
            .http
            .put(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(body)
            .send()
            .await?
            .json()
            .await?;

        let code = resp["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            let msg = resp["msg"].as_str().unwrap_or("unknown error");
            return Err(anyhow!("API error ({}): {}", code, msg));
        }
        Ok(resp)
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let token = self.get_token().await?;
        let url = format!("{}{}", self.api_base, path);
        let resp: Value = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json()
            .await?;

        let code = resp["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            let msg = resp["msg"].as_str().unwrap_or("unknown error");
            return Err(anyhow!("API error ({}): {}", code, msg));
        }
        Ok(resp)
    }

    /// Send a message to a user or group.
    /// receive_id_type: "open_id" or "chat_id"
    pub async fn send_message(
        &self,
        receive_id: &str,
        receive_id_type: &str,
        msg_type: &str,
        content: &str,
    ) -> Result<SendMessageResponse> {
        let path = format!("/im/v1/messages?receive_id_type={}", receive_id_type);
        let resp = self
            .post(
                &path,
                &serde_json::json!({
                    "receive_id": receive_id,
                    "msg_type": msg_type,
                    "content": content,
                    "uuid": uuid::Uuid::new_v4().to_string(),
                }),
            )
            .await?;

        Ok(SendMessageResponse {
            message_id: resp["data"]["message_id"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        })
    }

    /// Reply to a message.
    pub async fn reply_message(
        &self,
        message_id: &str,
        msg_type: &str,
        content: &str,
    ) -> Result<SendMessageResponse> {
        let path = format!("/im/v1/messages/{}/reply", message_id);
        let resp = self
            .post(
                &path,
                &serde_json::json!({
                    "content": content,
                    "msg_type": msg_type,
                    "uuid": uuid::Uuid::new_v4().to_string(),
                }),
            )
            .await?;

        Ok(SendMessageResponse {
            message_id: resp["data"]["message_id"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        })
    }

    /// Upload an image. Returns image_key.
    pub async fn upload_image(&self, data: &[u8], mime_type: &str) -> Result<String> {
        let token = self.get_token().await?;
        let url = format!("{}/im/v1/images", self.api_base);

        let ext = mime_type.split('/').next_back().unwrap_or("png");
        let filename = format!("image.{}", ext);

        let form = reqwest::multipart::Form::new()
            .text("image_type", "message")
            .part(
                "image",
                reqwest::multipart::Part::bytes(data.to_vec())
                    .file_name(filename)
                    .mime_str(mime_type)?,
            );

        let resp: Value = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await?
            .json()
            .await?;

        let code = resp["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            return Err(anyhow!("Upload image failed: {}", resp["msg"]));
        }

        resp["data"]["image_key"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("image_key not found in upload response"))
    }

    /// Upload a file. Returns file_key.
    pub async fn upload_file(
        &self,
        data: &[u8],
        file_type: &str,
        filename: &str,
    ) -> Result<String> {
        let token = self.get_token().await?;
        let url = format!("{}/im/v1/files", self.api_base);

        let form = reqwest::multipart::Form::new()
            .text("file_type", file_type.to_string())
            .text("file_name", filename.to_string())
            .part(
                "file",
                reqwest::multipart::Part::bytes(data.to_vec())
                    .file_name(filename.to_string())
                    .mime_str("application/octet-stream")?,
            );

        let resp: Value = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await?
            .json()
            .await?;

        let code = resp["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            return Err(anyhow!("Upload file failed: {}", resp["msg"]));
        }

        resp["data"]["file_key"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("file_key not found in upload response"))
    }

    /// Download a message resource (image/file).
    pub async fn download_resource(
        &self,
        message_id: &str,
        file_key: &str,
        resource_type: &str,
    ) -> Result<Vec<u8>> {
        let token = self.get_token().await?;
        let url = format!(
            "{}/im/v1/messages/{}/resources/{}?type={}",
            self.api_base, message_id, file_key, resource_type
        );

        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow!("Download resource failed: HTTP {}", resp.status()));
        }

        Ok(resp.bytes().await?.to_vec())
    }

    /// Get message content.
    pub async fn get_message(&self, message_id: &str) -> Result<Value> {
        let path = format!("/im/v1/messages/{}", message_id);
        self.get(&path).await
    }

    /// Get bot's own information (used to get the bot's open_id for mention detection).
    /// Calls GET /open-apis/bot/v3/info
    pub async fn get_bot_info(&self) -> Result<BotInfo> {
        let token = self.get_token().await?;
        let url = format!("{}/bot/v3/info", self.api_base);
        let resp: Value = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json()
            .await?;

        let code = resp["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            return Err(anyhow!(
                "Failed to get bot info: {} (code {})",
                resp["msg"].as_str().unwrap_or("unknown"),
                code
            ));
        }

        let bot = &resp["bot"];
        Ok(BotInfo {
            open_id: bot["open_id"].as_str().unwrap_or("").to_string(),
            app_name: bot["app_name"].as_str().unwrap_or("").to_string(),
            app_id: bot["app_id"].as_str().unwrap_or("").to_string(),
            avatar_url: bot["avatar_url"].as_str().unwrap_or("").to_string(),
        })
    }

    /// Create a CardKit card entity. Returns the card_id for later operations.
    pub async fn create_cardkit_card(&self, card: &Value) -> Result<String> {
        let resp = self
            .post(
                "/cardkit/v1/cards",
                &serde_json::json!({
                    "type": "card_json",
                    "data": card.to_string(),
                }),
            )
            .await?;
        resp["data"]["card_id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("card_id not found in cardkit create response"))
    }

    /// Send an interactive message that references a CardKit card by card_id.
    pub async fn send_card_by_card_id(
        &self,
        receive_id: &str,
        receive_id_type: &str,
        card_id: &str,
    ) -> Result<SendMessageResponse> {
        let path = format!("/im/v1/messages?receive_id_type={}", receive_id_type);
        let resp = self.post(&path, &serde_json::json!({
            "receive_id": receive_id,
            "msg_type": "interactive",
            "content": serde_json::json!({"type": "card", "data": {"card_id": card_id}}).to_string(),
            "uuid": uuid::Uuid::new_v4().to_string(),
        })).await?;
        Ok(SendMessageResponse {
            message_id: resp["data"]["message_id"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        })
    }

    /// Reply to a message with a CardKit card reference.
    pub async fn reply_with_card_id(
        &self,
        message_id: &str,
        card_id: &str,
    ) -> Result<SendMessageResponse> {
        let path = format!("/im/v1/messages/{}/reply", message_id);
        let resp = self.post(&path, &serde_json::json!({
            "content": serde_json::json!({"type": "card", "data": {"card_id": card_id}}).to_string(),
            "msg_type": "interactive",
            "uuid": uuid::Uuid::new_v4().to_string(),
        })).await?;
        Ok(SendMessageResponse {
            message_id: resp["data"]["message_id"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        })
    }

    /// Update a card element's content (for streaming text via CardKit).
    pub async fn update_card_element(
        &self,
        card_id: &str,
        element_id: &str,
        content: &str,
        sequence: u64,
    ) -> Result<()> {
        let path = format!(
            "/cardkit/v1/cards/{}/elements/{}/content",
            card_id, element_id
        );
        self.put_json(
            &path,
            &serde_json::json!({
                "content": content,
                "sequence": sequence,
            }),
        )
        .await?;
        Ok(())
    }

    /// Replace the full card content (for final state after streaming).
    pub async fn update_cardkit_card(
        &self,
        card_id: &str,
        card: &Value,
        sequence: u64,
    ) -> Result<()> {
        let path = format!("/cardkit/v1/cards/{}", card_id);
        self.put_json(
            &path,
            &serde_json::json!({
                "card": {"type": "card_json", "data": card.to_string()},
                "sequence": sequence,
            }),
        )
        .await?;
        Ok(())
    }

    /// Set streaming mode on/off for a CardKit card.
    /// Uses PATCH /cardkit/v1/cards/{card_id}/settings
    /// Returns empty body on success — use raw HTTP status check.
    pub async fn set_card_streaming_mode(
        &self,
        card_id: &str,
        streaming_mode: bool,
        sequence: u64,
    ) -> Result<()> {
        let token = self.get_token().await?;
        let url = format!("{}/cardkit/v1/cards/{}/settings", self.api_base, card_id);
        let resp = self.http
            .patch(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&serde_json::json!({
                "settings": serde_json::json!({"config": {"streaming_mode": streaming_mode}}).to_string(),
                "sequence": sequence,
            }))
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("HTTP {}: {}", status.as_u16(), body));
        }
        Ok(())
    }

    /// Get chat info (for group chats).
    pub async fn get_chat_info(&self, chat_id: &str) -> Result<Value> {
        let path = format!("/im/v1/chats/{}", chat_id);
        self.get(&path).await
    }

    /// Get user info.
    pub async fn get_user_info(&self, open_id: &str) -> Result<UserInfo> {
        let path = format!("/contact/v3/users/{}?user_id_type=open_id", open_id);
        let resp = self.get(&path).await?;
        let user = &resp["data"]["user"];
        Ok(UserInfo {
            open_id: open_id.to_string(),
            name: user["name"].as_str().unwrap_or("Unknown").to_string(),
            avatar_url: user["avatar"]["avatar_origin"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        })
    }

    /// React to a message with an emoji (used as ACK).
    /// Returns the reaction_id on success.
    pub async fn react_to_message(&self, message_id: &str, emoji_type: &str) -> Result<String> {
        let path = format!("/im/v1/messages/{}/reactions", message_id);
        let resp = self
            .post(
                &path,
                &serde_json::json!({
                    "reaction_type": {"emoji_type": emoji_type}
                }),
            )
            .await?;
        resp["data"]["reaction_id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("reaction_id not found in response"))
    }

    /// Remove a reaction from a message.
    pub async fn remove_reaction(&self, message_id: &str, reaction_id: &str) -> Result<()> {
        let token = self.get_token().await?;
        let url = format!(
            "{}/im/v1/messages/{}/reactions/{}",
            self.api_base, message_id, reaction_id
        );
        let resp: Value = self
            .http
            .delete(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json()
            .await?;
        let code = resp["code"].as_i64().unwrap_or(-1);
        if code != 0 {
            let msg = resp["msg"].as_str().unwrap_or("unknown error");
            return Err(anyhow!("Remove reaction failed ({}): {}", code, msg));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SendMessageResponse {
    pub message_id: String,
}

#[derive(Debug, Clone)]
pub struct UserInfo {
    pub open_id: String,
    pub name: String,
    pub avatar_url: String,
}

#[derive(Debug, Clone)]
pub struct BotInfo {
    pub open_id: String,
    pub app_name: String,
    pub app_id: String,
    pub avatar_url: String,
}

/// Convert raw bytes to base64 data URL form for agent input.
pub fn bytes_to_base64_data(data: &[u8], mime_type: &str) -> String {
    format!(
        "data:{};base64,{}",
        mime_type,
        base64::engine::general_purpose::STANDARD.encode(data)
    )
}

/// Detect MIME type from file extension.
pub fn mime_from_ext(filename: &str) -> &str {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "ogg" | "opus" => "audio/ogg",
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

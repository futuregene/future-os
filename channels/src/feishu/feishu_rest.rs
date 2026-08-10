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

#[cfg(test)]
mod tests {
    use super::*;

    // ─── bytes_to_base64_data ────────────────────────────────────────────────

    #[test]
    fn base64_data_url_format() {
        let data = b"hello";
        let result = bytes_to_base64_data(data, "image/png");
        assert!(result.starts_with("data:image/png;base64,"));
        // Verify the base64 payload decodes back to the original
        let b64_part = result.strip_prefix("data:image/png;base64,").unwrap();
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64_part)
            .unwrap();
        assert_eq!(decoded, b"hello");
    }

    #[test]
    fn base64_data_empty_input() {
        let result = bytes_to_base64_data(b"", "text/plain");
        assert_eq!(result, "data:text/plain;base64,");
    }

    #[test]
    fn base64_data_binary_content() {
        let data = vec![0u8, 255, 128, 1, 0];
        let result = bytes_to_base64_data(&data, "application/octet-stream");
        assert!(result.starts_with("data:application/octet-stream;base64,"));
    }

    // ─── mime_from_ext ───────────────────────────────────────────────────────

    #[test]
    fn mime_image_extensions() {
        assert_eq!(mime_from_ext("photo.png"), "image/png");
        assert_eq!(mime_from_ext("photo.jpg"), "image/jpeg");
        assert_eq!(mime_from_ext("photo.jpeg"), "image/jpeg");
        assert_eq!(mime_from_ext("photo.gif"), "image/gif");
        assert_eq!(mime_from_ext("photo.webp"), "image/webp");
        assert_eq!(mime_from_ext("photo.bmp"), "image/bmp");
        assert_eq!(mime_from_ext("icon.svg"), "image/svg+xml");
    }

    #[test]
    fn mime_media_extensions() {
        assert_eq!(mime_from_ext("video.mp4"), "video/mp4");
        assert_eq!(mime_from_ext("audio.mp3"), "audio/mpeg");
        assert_eq!(mime_from_ext("audio.ogg"), "audio/ogg");
        assert_eq!(mime_from_ext("audio.opus"), "audio/ogg");
    }

    #[test]
    fn mime_document_extensions() {
        assert_eq!(mime_from_ext("report.pdf"), "application/pdf");
        assert_eq!(mime_from_ext("report.doc"), "application/msword");
        assert_eq!(
            mime_from_ext("report.docx"),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(mime_from_ext("data.xls"), "application/vnd.ms-excel");
        assert_eq!(
            mime_from_ext("data.xlsx"),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
    }

    #[test]
    fn mime_unknown_extension_falls_back() {
        assert_eq!(mime_from_ext("archive.zip"), "application/octet-stream");
        assert_eq!(mime_from_ext("noext"), "application/octet-stream");
        assert_eq!(mime_from_ext(""), "application/octet-stream");
    }

    #[test]
    fn mime_extension_case_insensitive() {
        assert_eq!(mime_from_ext("PHOTO.PNG"), "image/png");
        assert_eq!(mime_from_ext("Photo.Jpeg"), "image/jpeg");
        assert_eq!(mime_from_ext("FILE.PDF"), "application/pdf");
    }

    #[test]
    fn mime_multiple_dots_uses_last() {
        assert_eq!(mime_from_ext("archive.tar.gz"), "application/octet-stream");
        assert_eq!(mime_from_ext("image.backup.png"), "image/png");
    }

    // ─── Struct field types ─────────────────────────────────────────────────

    #[test]
    fn send_message_response_fields() {
        let resp = SendMessageResponse {
            message_id: "om_abc".to_string(),
        };
        assert_eq!(resp.message_id, "om_abc");
    }

    #[test]
    fn user_info_fields() {
        let info = UserInfo {
            open_id: "ou_123".to_string(),
            name: "Alice".to_string(),
            avatar_url: "https://example.com/avatar.png".to_string(),
        };
        assert_eq!(info.name, "Alice");
        assert_eq!(info.open_id, "ou_123");
    }

    #[test]
    fn bot_info_fields() {
        let info = BotInfo {
            open_id: "ou_bot".to_string(),
            app_name: "FutureBot".to_string(),
            app_id: "cli_abc".to_string(),
            avatar_url: "".to_string(),
        };
        assert_eq!(info.app_name, "FutureBot");
    }

    // ─── HTTP methods against the mock server ────────────────────────────────

    use crate::test_support::HttpRoute;

    const TOKEN_ROUTE: &str = "/auth/v3/tenant_access_token/internal";

    fn token_ok() -> HttpRoute {
        HttpRoute::json(
            TOKEN_ROUTE,
            200,
            r#"{"code":0,"msg":"ok","tenant_access_token":"tok-1","expire":7200}"#,
        )
    }

    fn client(base: &str) -> FeishuRestClient {
        crate::test_support::ensure_crypto_provider();
        FeishuRestClient::new(base, "app-id", "app-secret")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_token_caches_and_sends_credentials() {
        let (base, recorded) =
            crate::test_support::spawn_http(vec![token_ok()]).await;
        let c = client(&base);
        let t1 = c.get_token().await.unwrap();
        let t2 = c.get_token().await.unwrap();
        assert_eq!(t1, "tok-1");
        assert_eq!(t2, "tok-1");
        let token_calls = crate::test_support::requests_to(&recorded, TOKEN_ROUTE);
        assert_eq!(token_calls.len(), 1, "second call must hit the cache");
        assert!(token_calls[0].body_string().contains("app-id"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_token_refreshes_near_expiry() {
        let route = HttpRoute::sequence(
            TOKEN_ROUTE,
            vec![
                (200, r#"{"code":0,"tenant_access_token":"tok-old","expire":61}"#),
                (200, r#"{"code":0,"tenant_access_token":"tok-new","expire":7200}"#),
            ],
        );
        let (base, recorded) = crate::test_support::spawn_http(vec![route]).await;
        let c = client(&base);
        assert_eq!(c.get_token().await.unwrap(), "tok-old");
        // expire=61 → cache retires almost immediately → refetch
        assert_eq!(c.get_token().await.unwrap(), "tok-new");
        assert_eq!(
            crate::test_support::requests_to(&recorded, TOKEN_ROUTE).len(),
            2
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_token_error_code_and_missing_token() {
        let err_route = HttpRoute::json(TOKEN_ROUTE, 200, r#"{"code":999,"msg":"bad app"}"#);
        let (base, _) = crate::test_support::spawn_http(vec![err_route]).await;
        let err = client(&base).get_token().await.unwrap_err();
        assert!(err.to_string().contains("bad app"), "{err}");

        let no_token = HttpRoute::json(TOKEN_ROUTE, 200, r#"{"code":0}"#);
        let (base, _) = crate::test_support::spawn_http(vec![no_token]).await;
        let err = client(&base).get_token().await.unwrap_err();
        assert!(err.to_string().contains("Token not found"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_token_http_failure() {
        let (base, _) = crate::test_support::spawn_http(vec![]).await; // 404 {}
        let err = client(&base).get_token().await.unwrap_err();
        assert!(err.to_string().contains("Failed to get tenant token"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn send_message_and_reply_message() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/im/v1/messages", 200, r#"{"code":0,"data":{"message_id":"om_1"}}"#),
            HttpRoute::json("/im/v1/messages/om_x/reply", 200, r#"{"code":0,"data":{"message_id":"om_2"}}"#),
        ];
        let (base, recorded) = crate::test_support::spawn_http(routes).await;
        let c = client(&base);
        let r = c.send_message("ou_1", "open_id", "text", "{}").await.unwrap();
        assert_eq!(r.message_id, "om_1");
        let r = c.reply_message("om_x", "text", "{}").await.unwrap();
        assert_eq!(r.message_id, "om_2");
        // Query string preserved, auth header set.
        let sent = crate::test_support::requests_to(&recorded, "/im/v1/messages");
        assert!(sent[0].target.contains("receive_id_type=open_id"));
        assert_eq!(sent[0].header("authorization"), Some("Bearer tok-1"));
        // Missing message_id → empty string (no error).
        let routes = vec![
            HttpRoute::json(TOKEN_ROUTE, 200, r#"{"code":0,"tenant_access_token":"t","expire":7200}"#),
            HttpRoute::json("/im/v1/messages", 200, r#"{"code":0,"data":{}}"#),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let r = client(&base).send_message("ou_1", "open_id", "text", "{}").await.unwrap();
        assert_eq!(r.message_id, "");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn api_error_code_propagates() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/im/v1/messages", 200, r#"{"code":230001,"msg":"msg too long"}"#),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let err = client(&base)
            .send_message("ou_1", "open_id", "text", "{}")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("230001"), "{err}");
        assert!(err.to_string().contains("msg too long"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn upload_image_and_file() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/im/v1/images", 200, r#"{"code":0,"data":{"image_key":"img_k1"}}"#),
            HttpRoute::json("/im/v1/files", 200, r#"{"code":0,"data":{"file_key":"file_k1"}}"#),
        ];
        let (base, recorded) = crate::test_support::spawn_http(routes).await;
        let c = client(&base);
        let key = c.upload_image(b"pngbytes", "image/png").await.unwrap();
        assert_eq!(key, "img_k1");
        let key = c.upload_file(b"data", "stream", "report.pdf").await.unwrap();
        assert_eq!(key, "file_k1");
        // Multipart bodies actually arrived.
        let up = crate::test_support::requests_to(&recorded, "/im/v1/images");
        assert!(up[0].body_string().contains("pngbytes"));
        assert!(up[0].body_string().contains("image.png"));
        let up = crate::test_support::requests_to(&recorded, "/im/v1/files");
        assert!(up[0].body_string().contains("report.pdf"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn upload_error_arms() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/im/v1/images", 200, r#"{"code":1,"msg":"too big"}"#),
            HttpRoute::json("/im/v1/files", 200, r#"{"code":2,"msg":"bad type"}"#),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let c = client(&base);
        assert!(c.upload_image(b"x", "image/png").await.unwrap_err().to_string().contains("too big"));
        assert!(c.upload_file(b"x", "stream", "f.bin").await.unwrap_err().to_string().contains("bad type"));

        let routes = vec![
            HttpRoute::json(TOKEN_ROUTE, 200, r#"{"code":0,"tenant_access_token":"t","expire":7200}"#),
            HttpRoute::json("/im/v1/images", 200, r#"{"code":0,"data":{}}"#),
            HttpRoute::json("/im/v1/files", 200, r#"{"code":0,"data":{}}"#),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let c = client(&base);
        assert!(c.upload_image(b"x", "image/png").await.unwrap_err().to_string().contains("image_key not found"));
        assert!(c.upload_file(b"x", "stream", "f.bin").await.unwrap_err().to_string().contains("file_key not found"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn download_resource_ok_and_http_error() {
        let routes = vec![
            token_ok(),
            HttpRoute::binary("/im/v1/messages/om_1/resources/img_k", 200, b"\x89PNG".to_vec()),
        ];
        let (base, recorded) = crate::test_support::spawn_http(routes).await;
        let c = client(&base);
        let data = c.download_resource("om_1", "img_k", "image").await.unwrap();
        assert_eq!(data, b"\x89PNG");
        let dl = crate::test_support::requests_to(&recorded, "/im/v1/messages/om_1/resources/img_k");
        assert!(dl[0].target.contains("type=image"));

        let routes = vec![
            HttpRoute::json(TOKEN_ROUTE, 200, r#"{"code":0,"tenant_access_token":"t","expire":7200}"#),
            HttpRoute::json("/im/v1/messages/om_1/resources/img_k", 500, "{}"),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let err = client(&base).download_resource("om_1", "img_k", "image").await.unwrap_err();
        assert!(err.to_string().contains("HTTP 500"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_message_and_chat_info() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/im/v1/messages/om_9", 200, r#"{"code":0,"data":{"items":[]}}"#),
            HttpRoute::json("/im/v1/chats/oc_1", 200, r#"{"code":0,"data":{"name":"g"}}"#),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let c = client(&base);
        let v = c.get_message("om_9").await.unwrap();
        assert!(v["data"]["items"].is_array());
        let v = c.get_chat_info("oc_1").await.unwrap();
        assert_eq!(v["data"]["name"], "g");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_error_code_propagates() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/im/v1/messages/om_9", 200, r#"{"code":44,"msg":"not found"}"#),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let err = client(&base).get_message("om_9").await.unwrap_err();
        assert!(err.to_string().contains("44"), "{err}");
        assert!(err.to_string().contains("not found"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_bot_info_ok_and_error() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/bot/v3/info", 200, r#"{"code":0,"bot":{"open_id":"ou_bot","app_name":"Bot","app_id":"cli_1","avatar_url":"u"}}"#),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let info = client(&base).get_bot_info().await.unwrap();
        assert_eq!(info.open_id, "ou_bot");
        assert_eq!(info.app_name, "Bot");

        let routes = vec![
            HttpRoute::json(TOKEN_ROUTE, 200, r#"{"code":0,"tenant_access_token":"t","expire":7200}"#),
            HttpRoute::json("/bot/v3/info", 200, r#"{"code":55,"msg":"no scope"}"#),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let err = client(&base).get_bot_info().await.unwrap_err();
        assert!(err.to_string().contains("no scope"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cardkit_full_flow() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/cardkit/v1/cards", 200, r#"{"code":0,"data":{"card_id":"card_1"}}"#),
            HttpRoute::json("/im/v1/messages", 200, r#"{"code":0,"data":{"message_id":"om_1"}}"#),
            HttpRoute::json("/im/v1/messages/om_1/reply", 200, r#"{"code":0,"data":{"message_id":"om_2"}}"#),
            HttpRoute::json("/cardkit/v1/cards/card_1/elements/stream_out/content", 200, r#"{"code":0}"#),
            HttpRoute::json("/cardkit/v1/cards/card_1", 200, r#"{"code":0}"#),
            HttpRoute::json("/cardkit/v1/cards/card_1/settings", 200, ""),
        ];
        let (base, recorded) = crate::test_support::spawn_http(routes).await;
        let c = client(&base);
        let card = serde_json::json!({"schema":"2.0"});
        let cid = c.create_cardkit_card(&card).await.unwrap();
        assert_eq!(cid, "card_1");
        let r = c.send_card_by_card_id("oc_1", "chat_id", &cid).await.unwrap();
        assert_eq!(r.message_id, "om_1");
        let r = c.reply_with_card_id("om_1", &cid).await.unwrap();
        assert_eq!(r.message_id, "om_2");
        c.update_card_element(&cid, "stream_out", "hello", 1).await.unwrap();
        c.update_cardkit_card(&cid, &card, 2).await.unwrap();
        c.set_card_streaming_mode(&cid, false, 3).await.unwrap();
        // The card payload travels as a string under data.card_id.
        let sent = crate::test_support::requests_to(&recorded, "/im/v1/messages");
        assert!(sent[0].body_string().contains("card_1"));
        let settings = crate::test_support::requests_to(&recorded, "/cardkit/v1/cards/card_1/settings");
        assert_eq!(settings[0].method, "PATCH");
        assert!(settings[0].body_string().contains("streaming_mode"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cardkit_error_arms() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/cardkit/v1/cards", 200, r#"{"code":0,"data":{}}"#),
            HttpRoute::json("/cardkit/v1/cards/c1/settings", 500, "boom"),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let c = client(&base);
        let err = c.create_cardkit_card(&serde_json::json!({})).await.unwrap_err();
        assert!(err.to_string().contains("card_id not found"), "{err}");
        let err = c.set_card_streaming_mode("c1", true, 1).await.unwrap_err();
        assert!(err.to_string().contains("HTTP 500"), "{err}");

        // PUT element update API error code arm.
        let routes = vec![
            HttpRoute::json(TOKEN_ROUTE, 200, r#"{"code":0,"tenant_access_token":"t","expire":7200}"#),
            HttpRoute::json("/cardkit/v1/cards/c1/elements/e/content", 200, r#"{"code":300302,"msg":"update_multi"}"#),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let err = client(&base).update_card_element("c1", "e", "x", 1).await.unwrap_err();
        assert!(err.to_string().contains("300302"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_user_info_parses_and_defaults() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/contact/v3/users/ou_1", 200,
                r#"{"code":0,"data":{"user":{"name":"Alice","avatar":{"avatar_origin":"http://a"}}}}"#),
            HttpRoute::json("/contact/v3/users/ou_2", 200, r#"{"code":0,"data":{"user":{}}}"#),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let c = client(&base);
        let u = c.get_user_info("ou_1").await.unwrap();
        assert_eq!(u.name, "Alice");
        assert_eq!(u.avatar_url, "http://a");
        let u = c.get_user_info("ou_2").await.unwrap();
        assert_eq!(u.name, "Unknown");
        assert_eq!(u.avatar_url, "");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reactions_add_and_remove() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/im/v1/messages/om_1/reactions", 200, r#"{"code":0,"data":{"reaction_id":"rid_1"}}"#),
            HttpRoute::json("/im/v1/messages/om_1/reactions/rid_1", 200, r#"{"code":0}"#),
        ];
        let (base, recorded) = crate::test_support::spawn_http(routes).await;
        let c = client(&base);
        let rid = c.react_to_message("om_1", "Typing").await.unwrap();
        assert_eq!(rid, "rid_1");
        c.remove_reaction("om_1", "rid_1").await.unwrap();
        let del = crate::test_support::requests_to(&recorded, "/im/v1/messages/om_1/reactions/rid_1");
        assert_eq!(del[0].method, "DELETE");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reaction_error_arms() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/im/v1/messages/om_1/reactions", 200, r#"{"code":0,"data":{}}"#),
            HttpRoute::json("/im/v1/messages/om_1/reactions/rid_1", 200, r#"{"code":7,"msg":"gone"}"#),
        ];
        let (base, _) = crate::test_support::spawn_http(routes).await;
        let c = client(&base);
        let err = c.react_to_message("om_1", "Typing").await.unwrap_err();
        assert!(err.to_string().contains("reaction_id not found"), "{err}");
        let err = c.remove_reaction("om_1", "rid_1").await.unwrap_err();
        assert!(err.to_string().contains("gone"), "{err}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn constructor_falls_back_when_builder_fails() {
        // FeishuRestClient::new has an unwrap_or_else fallback to
        // tls::http_client(); normal construction exercises the primary arm.
        crate::test_support::ensure_crypto_provider();
        let c = FeishuRestClient::new("http://127.0.0.1:1", "a", "s");
        let cloned = c.clone();
        let _ = format!("{:?}", cloned.app_id);
    }
}

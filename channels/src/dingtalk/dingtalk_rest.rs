//! DingTalk REST API client.
//! Handles access token acquisition and message sending.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct DingtalkRestClient {
    domain: String,
    client_id: String,
    client_secret: String,
    token: Arc<RwLock<Option<String>>>,
}

impl DingtalkRestClient {
    pub fn new(domain: &str, client_id: &str, client_secret: &str) -> Self {
        Self {
            domain: domain.to_string(),
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            token: Arc::new(RwLock::new(None)),
        }
    }

    async fn get_token(&self) -> Result<String> {
        if let Some(ref t) = *self.token.read().await {
            return Ok(t.clone());
        }
        let client = reqwest::Client::new();
        let url = format!("https://{}/v1.0/oauth2/accessToken", self.domain);
        let resp: Value = client
            .post(&url)
            .json(&json!({
                "appKey": self.client_id,
                "appSecret": self.client_secret,
            }))
            .send()
            .await?
            .json()
            .await?;
        let t = resp
            .get("accessToken")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Failed to get access token: {}", resp))?
            .to_string();
        *self.token.write().await = Some(t.clone());
        Ok(t)
    }

    /// Get a fresh access token (for AI Card usage).
    pub async fn get_token_internal(&self) -> Result<String> {
        self.get_token().await
    }

    /// Get the client ID / robot code.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Reply via a sessionWebhook with markdown content.
    pub async fn reply_webhook_markdown(&self, webhook_url: &str, title: &str, markdown: &str) -> Result<()> {
        let token = self.get_token().await?;
        let client = reqwest::Client::new();
        let body = json!({
            "msgtype": "markdown",
            "markdown": {"title": title, "text": markdown},
        });
        client
            .post(webhook_url)
            .header("x-acs-dingtalk-access-token", &token)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;
        Ok(())
    }
}

//! DingTalk REST API client.
//! Handles access token acquisition and message sending.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct DingtalkRestClient {
    domain: String,
    client_id: String,
    client_secret: String,
    token: Arc<RwLock<CachedToken>>,
}

struct CachedToken {
    value: String,
    expires_at: Instant,
}

impl DingtalkRestClient {
    pub fn new(domain: &str, client_id: &str, client_secret: &str) -> Self {
        Self {
            domain: domain.to_string(),
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            token: Arc::new(RwLock::new(CachedToken {
                value: String::new(),
                expires_at: Instant::now(),
            })),
        }
    }

    async fn get_token(&self) -> Result<String> {
        {
            let cached = self.token.read().await;
            if cached.expires_at > Instant::now() + std::time::Duration::from_secs(60) {
                return Ok(cached.value.clone());
            }
        }
        let client = crate::tls::http_client();
        let url = format!(
            "{}/v1.0/oauth2/accessToken",
            super::config::base_url(&self.domain)
        );
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
        // Tokens live ~7200s server-side; retire the cache 60s early so an
        // in-flight request never races the expiry (same rule as Feishu).
        let expire_in = resp
            .get("expireIn")
            .and_then(|v| v.as_i64())
            .unwrap_or(7200);
        *self.token.write().await = CachedToken {
            value: t.clone(),
            expires_at: Instant::now()
                + std::time::Duration::from_secs((expire_in - 60).max(0) as u64),
        };
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
    pub async fn reply_webhook_markdown(
        &self,
        webhook_url: &str,
        title: &str,
        markdown: &str,
    ) -> Result<()> {
        let token = self.get_token().await?;
        let client = crate::tls::http_client();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{self as ts, HttpRoute};

    const TOKEN_ROUTE: &str = "/v1.0/oauth2/accessToken";

    fn token_ok() -> HttpRoute {
        HttpRoute::json(
            TOKEN_ROUTE,
            200,
            r#"{"accessToken":"dt-tok","expireIn":7200}"#,
        )
    }

    fn client(base: &str) -> DingtalkRestClient {
        ts::ensure_crypto_provider();
        DingtalkRestClient::new(base, "client-id", "client-secret")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_token_caches_and_sends_app_credentials() {
        let (base, recorded) = ts::spawn_http(vec![token_ok()]).await;
        let c = client(&base);
        assert_eq!(c.get_token().await.unwrap(), "dt-tok");
        assert_eq!(c.get_token_internal().await.unwrap(), "dt-tok");
        let calls = ts::requests_to(&recorded, TOKEN_ROUTE);
        assert_eq!(calls.len(), 1, "second call must hit the cache");
        assert!(calls[0].body_string().contains("\"appKey\":\"client-id\""));
        assert!(calls[0]
            .body_string()
            .contains("\"appSecret\":\"client-secret\""));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_token_refreshes_when_near_expiry() {
        let route = HttpRoute::sequence(
            TOKEN_ROUTE,
            vec![
                (200, r#"{"accessToken":"old","expireIn":61}"#),
                (200, r#"{"accessToken":"new","expireIn":7200}"#),
            ],
        );
        let (base, _) = ts::spawn_http(vec![route]).await;
        let c = client(&base);
        assert_eq!(c.get_token().await.unwrap(), "old");
        assert_eq!(c.get_token().await.unwrap(), "new");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_token_missing_field_and_default_expire() {
        // Missing accessToken → error carrying the raw response.
        let (base, _) = ts::spawn_http(vec![HttpRoute::json(TOKEN_ROUTE, 200, "{}")]).await;
        let err = client(&base).get_token().await.unwrap_err();
        assert!(
            err.to_string().contains("Failed to get access token"),
            "{err}"
        );

        // Missing expireIn → 7200 default (token stays cached).
        let (base, recorded) = ts::spawn_http(vec![HttpRoute::json(
            TOKEN_ROUTE,
            200,
            r#"{"accessToken":"t1"}"#,
        )])
        .await;
        let c = client(&base);
        assert_eq!(c.get_token().await.unwrap(), "t1");
        assert_eq!(c.get_token().await.unwrap(), "t1");
        assert_eq!(ts::requests_to(&recorded, TOKEN_ROUTE).len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_token_zero_or_negative_expire_clamps() {
        // expireIn below the 60s safety margin clamps to 0 → always refetch.
        let route = HttpRoute::sequence(
            TOKEN_ROUTE,
            vec![
                (200, r#"{"accessToken":"a","expireIn":30}"#),
                (200, r#"{"accessToken":"b","expireIn":7200}"#),
            ],
        );
        let (base, _) = ts::spawn_http(vec![route]).await;
        let c = client(&base);
        assert_eq!(c.get_token().await.unwrap(), "a");
        assert_eq!(c.get_token().await.unwrap(), "b");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_id_getter() {
        let c = client("http://127.0.0.1:1");
        assert_eq!(c.client_id(), "client-id");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reply_webhook_markdown_posts_with_token_header() {
        let routes = vec![
            token_ok(),
            HttpRoute::json("/robot/sendBySession", 200, "{}"),
        ];
        let (base, recorded) = ts::spawn_http(routes).await;
        let c = client(&base);
        let webhook = format!("{}/robot/sendBySession?session=abc", base);
        c.reply_webhook_markdown(&webhook, "Title", "**bold** reply")
            .await
            .unwrap();
        let calls = ts::requests_to(&recorded, "/robot/sendBySession");
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].header("x-acs-dingtalk-access-token"),
            Some("dt-tok")
        );
        assert!(calls[0].body_string().contains("\"msgtype\":\"markdown\""));
        assert!(calls[0].body_string().contains("**bold** reply"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reply_webhook_send_failure() {
        let (base, _) = ts::spawn_http(vec![token_ok()]).await;
        let c = client(&base);
        // Token succeeds; webhook host is dead.
        let err = c
            .reply_webhook_markdown("http://127.0.0.1:1/hook", "T", "m")
            .await
            .unwrap_err();
        assert!(!err.to_string().is_empty());
        drop(base);
    }
}

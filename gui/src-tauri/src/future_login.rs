//! GUI-native FutureGene device-code OAuth (see gui/LOGIN.md).
//!
//! Mirrors the CLI protocol (`cli/src/commands/auth.ts`) but is fully
//! self-contained: it requests a device code, opens the verification page, then
//! exchanges the device code for an API key which is written to the `future`
//! entry of `~/.future/agent/auth.json` via [`crate::auth_store`]. Polling is
//! driven by the frontend (one short request per call); this module is
//! stateless.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::AppError;

const CLIENT_NAME: &str = "Future OS GUI";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FutureLoginStart {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    /// Server-suggested poll interval, in seconds.
    pub interval: u64,
    /// Lifetime of the device code, in seconds.
    pub expires_in: u64,
    pub device_code: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FutureLoginPoll {
    /// One of: `pending`, `slow_down`, `authorized`, `denied`, `expired`, `error`.
    pub status: String,
    pub message: Option<String>,
}

impl FutureLoginPoll {
    fn of(status: &str) -> Self {
        FutureLoginPoll {
            status: status.to_string(),
            message: None,
        }
    }

    fn with_message(status: &str, message: impl Into<String>) -> Self {
        FutureLoginPoll {
            status: status.to_string(),
            message: Some(message.into()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: Option<String>,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct DeviceTokenResponse {
    api_key: Option<String>,
    token_type: Option<String>,
}

fn client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| AppError::Message(format!("无法创建 HTTP 客户端：{error}")))
}

fn base_url() -> String {
    let auth = Value::Object(crate::auth_store::read().unwrap_or_default());
    crate::agent_providers::resolve_future_base_url(&auth)
}

/// Begin device authorization: fetch a device/user code and open the
/// verification page. Returns the codes for the dialog to display and poll.
pub async fn start() -> Result<FutureLoginStart, AppError> {
    let base = base_url();
    let response = client()?
        .post(format!("{base}/v1/oauth/device/code"))
        .json(&json!({ "client_name": CLIENT_NAME }))
        .send()
        .await
        .map_err(|error| AppError::Message(format!("请求设备码失败：{error}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let message = error_message_from_body(response.json::<Value>().await.ok())
            .unwrap_or_else(|| format!("请求设备码失败（HTTP {}）", status.as_u16()));
        return Err(AppError::Message(message));
    }

    let device: DeviceCodeResponse = response
        .json()
        .await
        .map_err(|error| AppError::Message(format!("解析设备码响应失败：{error}")))?;

    if device.device_code.trim().is_empty() || device.user_code.trim().is_empty() {
        return Err(AppError::Message("设备码响应缺少必要字段。".to_string()));
    }
    if device.expires_in == 0 || device.interval == 0 {
        return Err(AppError::Message(
            "设备码响应的过期时间或轮询间隔无效。".to_string(),
        ));
    }

    // Prefer the "complete" URL (carries the user code); fall back to the bare
    // verification URI. Validate before doing anything with it.
    let verification = device
        .verification_uri_complete
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| device.verification_uri.clone())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::Message("设备码响应缺少授权链接。".to_string()))?;
    validate_browser_url(&verification)?;

    // Best-effort: failure is fine, the dialog shows a copyable link.
    open_browser(&verification);

    Ok(FutureLoginStart {
        user_code: device.user_code,
        verification_uri: device
            .verification_uri
            .unwrap_or_else(|| verification.clone()),
        verification_uri_complete: verification,
        interval: device.interval,
        expires_in: device.expires_in,
        device_code: device.device_code,
    })
}

/// Exchange the device code for an API key once. On success the key is written
/// to `auth.json`; the returned status drives the frontend poll loop.
pub async fn poll(device_code: &str) -> Result<FutureLoginPoll, AppError> {
    let base = base_url();
    let response = client()?
        .post(format!("{base}/v1/oauth/device/token"))
        .json(&json!({ "device_code": device_code }))
        .send()
        .await
        .map_err(|error| AppError::Message(format!("轮询授权状态失败：{error}")))?;

    let success = response.status().is_success();
    let body: Value = response
        .json()
        .await
        .map_err(|error| AppError::Message(format!("解析授权响应失败：{error}")))?;

    if success {
        let token: DeviceTokenResponse = serde_json::from_value(body)
            .map_err(|error| AppError::Message(format!("解析授权响应失败：{error}")))?;
        let key = token.api_key.unwrap_or_default();
        if key.trim().is_empty() {
            return Ok(FutureLoginPoll::with_message(
                "error",
                "授权响应未包含 API key。",
            ));
        }
        if token
            .token_type
            .as_deref()
            .map(|kind| kind != "api_key")
            .unwrap_or(false)
        {
            return Ok(FutureLoginPoll::with_message(
                "error",
                "授权响应的凭证类型不受支持。",
            ));
        }
        // Only report success after the key is durably written.
        crate::auth_store::set_future_key(key.trim())?;
        return Ok(FutureLoginPoll::of("authorized"));
    }

    let error = body
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let message = error_message_from_body(Some(body.clone()));
    Ok(match error {
        "authorization_pending" => FutureLoginPoll::of("pending"),
        "slow_down" => FutureLoginPoll::of("slow_down"),
        "access_denied" => FutureLoginPoll::with_message(
            "denied",
            message.unwrap_or_else(|| "授权被拒绝。".to_string()),
        ),
        "expired_token" => FutureLoginPoll::with_message(
            "expired",
            message.unwrap_or_else(|| "授权码已过期，请重试。".to_string()),
        ),
        _ => FutureLoginPoll::with_message(
            "error",
            message.unwrap_or_else(|| "授权失败。".to_string()),
        ),
    })
}

fn error_message_from_body(body: Option<Value>) -> Option<String> {
    let body = body?;
    body.get("message")
        .or_else(|| body.get("error_description"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

/// Allow opening only `http(s)` URLs, rejecting `file:` / `javascript:` /
/// `data:` / custom schemes. The host is intentionally NOT pinned to the API
/// host: the verification page legitimately lives on a different host (a web
/// console / login page), so requiring same-origin would reject the real URL.
/// This matches the CLI, which opens the returned URL directly.
fn validate_browser_url(target: &str) -> Result<(), AppError> {
    let url =
        reqwest::Url::parse(target).map_err(|_| AppError::Message("授权链接无效。".to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::Message("授权链接的协议不被允许。".to_string()));
    }
    Ok(())
}

/// Open a URL in the default browser, detached and best-effort (mirrors the
/// CLI). Caller must validate the URL first (see [`validate_browser_url`]).
fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let (command, args): (&str, Vec<&str>) = ("open", vec![url]);
    #[cfg(target_os = "windows")]
    let (command, args): (&str, Vec<&str>) = ("cmd", vec!["/c", "start", "", url]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let (command, args): (&str, Vec<&str>) = ("xdg-open", vec![url]);

    let _ = std::process::Command::new(command).args(args).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_url_accepts_http_and_https_any_host() {
        // The verification page can live on a different host than the API.
        assert!(validate_browser_url("http://api.example.com/oauth/authorize?code=ABCD").is_ok());
        assert!(validate_browser_url("https://console.example.org/device").is_ok());
    }

    #[test]
    fn browser_url_rejects_non_web_schemes() {
        assert!(validate_browser_url("javascript:alert(1)").is_err());
        assert!(validate_browser_url("file:///etc/passwd").is_err());
        assert!(validate_browser_url("data:text/html,<script>").is_err());
        assert!(validate_browser_url("not a url").is_err());
    }

    #[test]
    fn error_message_prefers_message_then_description() {
        assert_eq!(
            error_message_from_body(Some(json!({ "message": "boom" }))).as_deref(),
            Some("boom")
        );
        assert_eq!(
            error_message_from_body(Some(json!({ "error_description": "desc" }))).as_deref(),
            Some("desc")
        );
        assert_eq!(error_message_from_body(Some(json!({ "error": "x" }))), None);
    }
}

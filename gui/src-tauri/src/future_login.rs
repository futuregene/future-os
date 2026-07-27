//! GUI-native FutureGene device-code OAuth (see gui/ER.md §6.9).
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

/// The signed-in account, as returned by `{platform}/client/v1/account/profile`.
/// Deserialized from the platform's snake_case payload; serialized to camelCase
/// for the frontend. Mirrors the CLI's `future account profile`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct FutureProfile {
    pub email: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub email_verified: bool,
    #[serde(default)]
    pub created_at: Option<String>,
}

fn client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| AppError::Message(format!("Failed to create HTTP client: {error}")))
}

/// Begin device authorization: fetch a device/user code and open the
/// verification page. Returns the codes for the dialog to display and poll.
///
/// Device-code OAuth lives on the platform root (`{platform}/client/v1/...`),
/// not the model API base — mirror the CLI (`cli/src/commands/auth.ts`).
pub async fn start() -> Result<FutureLoginStart, AppError> {
    let platform = crate::future_platform::current_platform_url();
    let response = client()?
        .post(format!("{platform}/client/v1/oauth/device/code"))
        .json(&json!({ "client_name": CLIENT_NAME }))
        .send()
        .await
        .map_err(|error| AppError::Message(format!("Failed to request device code: {error}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let message = error_message_from_body(response.json::<Value>().await.ok())
            .unwrap_or_else(|| format!("Device code request failed (HTTP {})", status.as_u16()));
        return Err(AppError::Message(message));
    }

    let device: DeviceCodeResponse = response.json().await.map_err(|error| {
        AppError::Message(format!("Failed to parse device code response: {error}"))
    })?;

    if device.device_code.trim().is_empty() || device.user_code.trim().is_empty() {
        return Err(AppError::Message(
            "Device code response is missing required fields.".to_string(),
        ));
    }
    if device.expires_in == 0 || device.interval == 0 {
        return Err(AppError::Message(
            "Device code response has an invalid expiry or polling interval.".to_string(),
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
        .ok_or_else(|| {
            AppError::Message("Device code response is missing the authorization URL.".to_string())
        })?;
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
    let platform = crate::future_platform::current_platform_url();
    let response = client()?
        .post(format!("{platform}/client/v1/oauth/device/token"))
        .json(&json!({ "device_code": device_code }))
        .send()
        .await
        .map_err(|error| {
            AppError::Message(format!("Failed to poll authorization status: {error}"))
        })?;

    let success = response.status().is_success();
    let body: Value = response.json().await.map_err(|error| {
        AppError::Message(format!("Failed to parse authorization response: {error}"))
    })?;

    if success {
        let token: DeviceTokenResponse = serde_json::from_value(body).map_err(|error| {
            AppError::Message(format!("Failed to parse authorization response: {error}"))
        })?;
        let key = token.api_key.unwrap_or_default();
        if key.trim().is_empty() {
            return Ok(FutureLoginPoll::with_message(
                "error",
                "Authorization response did not contain an API key.",
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
                "The credential type in the authorization response is not supported.",
            ));
        }
        // Only report success after the key is durably written. Pin `base_url`
        // to the resolved platform (`{platform}/api`), exactly as the CLI does,
        // so a GUI login and a CLI login leave identical `auth.json` state.
        crate::auth_store::set_future_login(key.trim(), &format!("{platform}/api"))?;
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
            message.unwrap_or_else(|| "Authorization was denied.".to_string()),
        ),
        "expired_token" => FutureLoginPoll::with_message(
            "expired",
            message
                .unwrap_or_else(|| "Authorization code has expired; please try again.".to_string()),
        ),
        _ => FutureLoginPoll::with_message(
            "error",
            message.unwrap_or_else(|| "Authorization failed.".to_string()),
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

/// The stored FutureGene API key, or an error when signed out. Mirrors the CLI's
/// precedence trivially: the GUI only ever writes the key to the `future` entry.
pub(crate) fn future_api_key() -> Result<String, AppError> {
    crate::auth_store::read()?
        .get(crate::auth_store::FUTURE_PROVIDER_ID)
        .and_then(|entry| entry.get("key"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .ok_or_else(|| AppError::Message("Not signed in to FutureOS.".to_string()))
}

/// Fetch the signed-in account profile (`GET {platform}/client/v1/account/profile`,
/// Bearer the stored `future` key) — mirrors the CLI's `future account profile`
/// (`cli/src/commands/account.ts`). Errors when signed out or on a failed request.
pub async fn fetch_profile() -> Result<FutureProfile, AppError> {
    let key = future_api_key()?;
    let platform = crate::future_platform::current_platform_url();
    let response = client()?
        .get(format!("{platform}/client/v1/account/profile"))
        .bearer_auth(&key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| AppError::Message(format!("Failed to fetch account profile: {error}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let message =
            error_message_from_body(response.json::<Value>().await.ok()).unwrap_or_else(|| {
                format!("Account profile request failed (HTTP {})", status.as_u16())
            });
        return Err(AppError::Message(message));
    }

    response
        .json::<FutureProfile>()
        .await
        .map_err(|error| AppError::Message(format!("Failed to parse account profile: {error}")))
}

/// Allow opening only `http(s)` URLs, rejecting `file:` / `javascript:` /
/// `data:` / custom schemes. The host is intentionally NOT pinned to the API
/// host: the verification page legitimately lives on a different host (a web
/// console / login page), so requiring same-origin would reject the real URL.
/// This matches the CLI, which opens the returned URL directly.
fn validate_browser_url(target: &str) -> Result<(), AppError> {
    let url = reqwest::Url::parse(target)
        .map_err(|_| AppError::Message("Authorization URL is invalid.".to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::Message(
            "Authorization URL scheme is not permitted.".to_string(),
        ));
    }
    Ok(())
}

/// Open a URL in the default browser, detached and best-effort (mirrors the
/// CLI). Caller must validate the URL first (see [`validate_browser_url`]).
/// Uses the `open` crate (ShellExecuteW on Windows) — never `cmd /c start`,
/// which re-parses the URL so a `&` truncates it and `&cmd`-style payloads
/// from a hostile platform host would execute.
fn open_browser(url: &str) {
    let _ = open::that_detached(url);
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

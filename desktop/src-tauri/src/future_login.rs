//! Desktop FutureGene device-code OAuth, shared by GUI and headless mode.
//!
//! Requests a device code and optionally opens the verification page. On
//! authorization the Agent durably commits the key in the `future` entry of
//! its auth configuration. The caller (WebView or foreground terminal) owns
//! polling and cancellation; this module is stateless.

use std::{error::Error as StdError, sync::OnceLock, time::Duration};

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
    /// One of: `pending`, `slow_down`, `retry`, `malformed`, `authorized`,
    /// `denied`, `expired`, `error`.
    pub status: String,
    pub message: Option<String>,
    /// Server-requested delay for a retryable response such as HTTP 429.
    pub retry_after_seconds: Option<u64>,
}

impl FutureLoginPoll {
    fn of(status: &str) -> Self {
        FutureLoginPoll {
            status: status.to_string(),
            message: None,
            retry_after_seconds: None,
        }
    }

    fn with_message(status: &str, message: impl Into<String>) -> Self {
        FutureLoginPoll {
            status: status.to_string(),
            message: Some(message.into()),
            retry_after_seconds: None,
        }
    }

    fn retry(retry_after_seconds: Option<u64>) -> Self {
        FutureLoginPoll {
            status: "retry".to_string(),
            message: None,
            retry_after_seconds,
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

/// Credit balance, as returned by `{platform}/client/v1/account/balance`.
/// Deserialized from the platform's snake_case payload; serialized to camelCase
/// for the frontend. Mirrors the CLI's `future account balance`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct FutureBalance {
    /// Credits as a human-readable number (already divided by the internal unit,
    /// `balance_credits / 10_000_000_000`).
    pub credits: f64,
}

/// Authoritative account-session state for Desktop UI consumers. A stored key
/// is only configuration; `authenticated` means the platform accepted it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FutureAuthState {
    pub status: FutureAuthStatus,
    pub profile: Option<FutureProfile>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FutureAuthStatus {
    SignedOut,
    Authenticated,
    Invalid,
    Unavailable,
}

pub fn is_auth_rejection(error: &AppError) -> bool {
    matches!(
        error,
        AppError::Remote {
            status: 401 | 403,
            ..
        }
    )
}

/// Verify the stored FutureOS credential against the account profile endpoint.
/// Only a platform authorization rejection invalidates the session; transport,
/// server, and response errors remain a recoverable `unavailable` state.
pub async fn check_auth_state() -> FutureAuthState {
    match future_api_key() {
        Ok(_) => {}
        Err(AppError::Message(_)) => {
            return FutureAuthState {
                status: FutureAuthStatus::SignedOut,
                profile: None,
            };
        }
        Err(error) => {
            eprintln!("FutureOS account credential unavailable: {error}");
            return FutureAuthState {
                status: FutureAuthStatus::Unavailable,
                profile: None,
            };
        }
    }

    match fetch_profile().await {
        Ok(profile) => FutureAuthState {
            status: FutureAuthStatus::Authenticated,
            profile: Some(profile),
        },
        Err(error) if is_auth_rejection(&error) => FutureAuthState {
            status: FutureAuthStatus::Invalid,
            profile: None,
        },
        Err(error) => {
            eprintln!("FutureOS account verification unavailable: {error}");
            FutureAuthState {
                status: FutureAuthStatus::Unavailable,
                profile: None,
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct BalanceResponse {
    balance_credits: i64,
}

/// The internal unit: 1 credit = 10_000_000_000 internal units. Mirrors the CLI.
const CREDIT_UNIT: f64 = 10_000_000_000.0;

/// Fetch the signed-in account credit balance
/// (`GET {platform}/client/v1/account/balance`, Bearer the stored `future` key)
/// — mirrors the CLI's `future account balance`. Errors when signed out or on a
/// failed request.
pub async fn fetch_balance() -> Result<FutureBalance, AppError> {
    let key = future_api_key()?;
    let platform = crate::future_platform::current_platform_url();
    let response = client()
        .get(format!("{platform}/client/v1/account/balance"))
        .bearer_auth(&key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| AppError::Message(format!("Failed to fetch account balance: {error}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let message =
            error_message_from_body(response.json::<Value>().await.ok()).unwrap_or_else(|| {
                format!("Account balance request failed (HTTP {})", status.as_u16())
            });
        return Err(AppError::Remote {
            status: status.as_u16(),
            code: None,
            message,
        });
    }

    let raw = response
        .json::<BalanceResponse>()
        .await
        .map_err(|error| AppError::Message(format!("Failed to parse account balance: {error}")))?;

    Ok(FutureBalance {
        credits: ((raw.balance_credits as f64 / CREDIT_UNIT) * 1000.0).trunc() / 1000.0,
    })
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

    // `Client::builder().timeout().build()` only fails for an invalid config;
    // the default config here is constant, so a failure is an invariant break.
    CLIENT.get_or_init(|| {
        crate::install_rustls_provider();
        reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("default reqwest client config cannot fail to build")
    })
}

/// Begin device authorization: fetch a device/user code and open the
/// verification page. Returns the codes for the dialog to display and poll.
///
/// Device-code OAuth lives on the platform root (`{platform}/client/v1/...`),
/// not the model API base — mirror the CLI (`cli/src/commands/auth.ts`).
pub async fn start() -> Result<FutureLoginStart, AppError> {
    start_with_browser(true).await
}

/// The same device authorization flow without launching a browser on an SSH host.
pub(crate) async fn start_with_browser(open: bool) -> Result<FutureLoginStart, AppError> {
    let platform = crate::future_platform::current_platform_url();
    let response = client()
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
    let base = device
        .verification_uri_complete
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| device.verification_uri.clone())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            AppError::Message("Device code response is missing the authorization URL.".to_string())
        })?;
    validate_browser_url(&base)?;

    // Tag the page we open with the current platform so the authorization page
    // can tailor itself. The user code is no longer shown to the user — the page
    // opens straight away and the poll loop is unchanged.
    let verification = append_login_platform(base);
    let verification_uri = device
        .verification_uri
        .unwrap_or_else(|| verification.clone());

    // Best-effort: failure is fine, the dialog shows a copyable link.
    if open {
        open_browser(&verification);
    }

    Ok(FutureLoginStart {
        user_code: device.user_code,
        verification_uri,
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
    let response = match client()
        .post(format!("{platform}/client/v1/oauth/device/token"))
        .json(&json!({ "device_code": device_code }))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            log_poll_transport_error(&error);
            return Ok(FutureLoginPoll::retry(None));
        }
    };

    let status = response.status();
    if is_retryable_poll_status(status) {
        let retry_after_seconds = retry_after_seconds(&response);
        eprintln!(
            "FutureOS device authorization poll will retry after HTTP {}{}",
            status.as_u16(),
            retry_after_seconds
                .map(|seconds| format!(" (Retry-After: {seconds}s)"))
                .unwrap_or_default()
        );
        return Ok(FutureLoginPoll::retry(retry_after_seconds));
    }

    let success = status.is_success();
    let body: Value = match response.json().await {
        Ok(body) => body,
        Err(error) if is_retryable_poll_error(&error) => {
            log_poll_transport_error(&error);
            return Ok(FutureLoginPoll::retry(None));
        }
        Err(error) if success => {
            eprintln!(
                "FutureOS device authorization returned an invalid success response: {error}"
            );
            return Ok(FutureLoginPoll::of("malformed"));
        }
        Err(error) => {
            return Ok(FutureLoginPoll::with_message(
                "error",
                format!(
                    "Authorization request failed (HTTP {}): {error}",
                    status.as_u16()
                ),
            ));
        }
    };

    if success {
        let token: DeviceTokenResponse = match serde_json::from_value(body) {
            Ok(token) => token,
            Err(error) => {
                eprintln!(
                    "FutureOS device authorization returned an invalid token response: {error}"
                );
                return Ok(FutureLoginPoll::of("malformed"));
            }
        };
        let key = token.api_key.unwrap_or_default();
        if key.trim().is_empty() {
            eprintln!("FutureOS device authorization response did not contain an API key");
            return Ok(FutureLoginPoll::of("malformed"));
        }
        if token
            .token_type
            .as_deref()
            .map(|kind| kind != "api_key")
            .unwrap_or(false)
        {
            eprintln!("FutureOS device authorization returned an unsupported credential type");
            return Ok(FutureLoginPoll::of("malformed"));
        }
        // Only report success after the key is durably written. Pin `base_url`
        // to the resolved platform (`{platform}/api`), exactly as the CLI does,
        // so a GUI login and a CLI login leave identical `auth.json` state.
        // The Agent is the sole writer. Authorization is reported complete
        // only after it durably stores the key and refreshes live sessions.
        let base_url = format!("{platform}/api");
        crate::agent_bridge::config::future_login(key.trim(), &base_url).await?;
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

fn is_retryable_poll_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn retry_after_seconds(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after)
}

fn parse_retry_after(value: &str) -> Option<u64> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds);
    }
    let retry_at = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let seconds = retry_at
        .timestamp()
        .saturating_sub(chrono::Utc::now().timestamp());
    Some(seconds.max(0) as u64)
}

fn is_retryable_poll_error(error: &reqwest::Error) -> bool {
    error.is_connect()
        || error.is_timeout()
        || error.is_request()
        || error.is_body()
        || error.is_redirect()
}

fn log_poll_transport_error(error: &reqwest::Error) {
    let mut causes = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        causes.push(cause.to_string());
        source = cause.source();
    }
    eprintln!(
        "FutureOS device authorization poll transport error (connect={}, timeout={}, request={}, body={}, redirect={}): {}{}",
        error.is_connect(),
        error.is_timeout(),
        error.is_request(),
        error.is_body(),
        error.is_redirect(),
        error,
        if causes.is_empty() {
            String::new()
        } else {
            format!("; caused by: {}", causes.join("; caused by: "))
        }
    );
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
    let response = client()
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
        return Err(AppError::Remote {
            status: status.as_u16(),
            code: None,
            message,
        });
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
    // The opener is swappable so tests can assert the URL is passed through
    // without launching a real browser (`open::that_detached` is a real side
    // effect that would escape the test process).
    let opener = BROWSER_OPENER.get_or_init(|| |url: &str| open::that_detached(url));
    let _ = opener(url);
}

/// Swappable default-browser opener; production uses the default `open` crate
/// launcher, tests install a no-op.
static BROWSER_OPENER: std::sync::OnceLock<fn(&str) -> std::io::Result<()>> =
    std::sync::OnceLock::new();

#[cfg(target_os = "macos")]
const LOGIN_PLATFORM: &str = "macOS";
#[cfg(target_os = "windows")]
const LOGIN_PLATFORM: &str = "Windows";
#[cfg(target_os = "linux")]
const LOGIN_PLATFORM: &str = "Linux";
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
const LOGIN_PLATFORM: &str = "";

/// Map the compile target to the platform label the authorization page expects
/// (`macOS` / `Windows` / `Linux`); any other target yields an empty value.
fn login_platform() -> &'static str {
    LOGIN_PLATFORM
}

/// Append `platform=<login_platform>` to the authorization URL so the opened page
/// knows the client OS. The existing query (e.g. the embedded user code) is
/// preserved; an unparseable URL is returned unchanged (it already passed
/// validation, so this is only a defensive fallback).
fn append_login_platform(url: String) -> String {
    match reqwest::Url::parse(&url) {
        Ok(mut parsed) => {
            parsed
                .query_pairs_mut()
                .append_pair("platform", login_platform());
            parsed.to_string()
        }
        Err(_) => url,
    }
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

    #[test]
    fn append_login_platform_preserves_query_and_tags_platform() {
        let out =
            append_login_platform("https://example.com/device?user_code=ABCD-1234".to_string());
        let parsed = reqwest::Url::parse(&out).expect("valid url");
        let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            pairs.get("user_code").map(String::as_str),
            Some("ABCD-1234")
        );
        assert_eq!(
            pairs.get("platform").map(String::as_str),
            Some(login_platform())
        );
    }

    #[test]
    fn append_login_platform_adds_param_without_existing_query() {
        let out = append_login_platform("https://example.com/device".to_string());
        let parsed = reqwest::Url::parse(&out).expect("valid url");
        let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(
            pairs.get("platform").map(String::as_str),
            Some(login_platform())
        );
    }

    #[test]
    fn append_login_platform_returns_unparseable_url_unchanged() {
        assert_eq!(append_login_platform("not a url".to_string()), "not a url");
    }

    #[test]
    fn poll_constructors_set_status_and_message() {
        let pending = FutureLoginPoll::of("pending");
        assert_eq!(pending.status, "pending");
        assert!(pending.message.is_none());
        assert!(pending.retry_after_seconds.is_none());

        let denied = FutureLoginPoll::with_message("denied", "nope");
        assert_eq!(denied.status, "denied");
        assert_eq!(denied.message.as_deref(), Some("nope"));

        let retry = FutureLoginPoll::retry(Some(9));
        assert_eq!(retry.status, "retry");
        assert_eq!(retry.retry_after_seconds, Some(9));
    }

    #[test]
    fn http_client_is_reused() {
        assert!(std::ptr::eq(client(), client()));
    }

    #[test]
    fn retryable_poll_statuses_are_bounded_to_transient_failures() {
        for status in [408, 429, 500, 502, 503, 504] {
            assert!(is_retryable_poll_status(
                reqwest::StatusCode::from_u16(status).unwrap()
            ));
        }
        for status in [400, 401, 403, 404, 422] {
            assert!(!is_retryable_poll_status(
                reqwest::StatusCode::from_u16(status).unwrap()
            ));
        }
    }

    #[test]
    fn retry_after_accepts_seconds_and_http_dates() {
        assert_eq!(parse_retry_after("12"), Some(12));
        assert!(parse_retry_after("Wed, 21 Oct 2099 07:28:00 GMT").unwrap() > 0);
        assert_eq!(parse_retry_after("not-a-delay"), None);
    }

    #[test]
    fn future_api_key_errors_when_signed_out() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-key-signedout");
        assert!(future_api_key().is_err());
    }

    #[test]
    fn future_api_key_reads_stored_key() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-key-signedin");
        crate::auth_store::set_future_login("sekret", "https://future-os.cn/api").unwrap();
        assert_eq!(future_api_key().unwrap(), "sekret");
    }

    #[test]
    fn open_browser_uses_injected_opener() {
        let _ = BROWSER_OPENER.set(|_| Ok(()));
        open_browser("https://example.com/device");
    }

    #[tokio::test]
    async fn broken_agent_endpoint_restores_unset_env() {
        std::env::remove_var("FUTURE_AGENT_GRPC_ADDR");
        let value = with_broken_agent_endpoint(|| async { 42 }).await;
        assert_eq!(value, 42);
        assert!(std::env::var("FUTURE_AGENT_GRPC_ADDR").is_err());
    }

    // ─── async OAuth + account calls against a mock HTTP server ───────────

    fn mock_http_server(responses: Vec<(u16, &'static str, Vec<u8>)>) -> String {
        mock_http_server_with_headers(
            responses
                .into_iter()
                .map(|(status, content_type, body)| (status, content_type, Vec::new(), body))
                .collect(),
        )
    }

    type MockHttpResponse = (
        u16,
        &'static str,
        Vec<(&'static str, &'static str)>,
        Vec<u8>,
    );

    fn mock_http_server_with_headers(responses: Vec<MockHttpResponse>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for (status, content_type, headers, body) in responses {
                let (mut stream, _) = listener.accept().expect("mock accept");
                let mut sink = [0u8; 8192];
                let _ = stream.read(&mut sink);
                let extra_headers = headers
                    .into_iter()
                    .map(|(name, value)| format!("{name}: {value}\r\n"))
                    .collect::<String>();
                let header = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len(),
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn point_auth(url: &str) {
        crate::auth_store::set_future_base_url(&format!("{url}/api")).unwrap();
    }

    fn point_auth_with_key(url: &str) {
        crate::auth_store::set_future_login("sekret", &format!("{url}/api")).unwrap();
    }

    /// Run a closure with `FUTURE_AGENT_GRPC_ADDR` pointed at an unparseable
    /// endpoint so `connect_agent` fails deterministically, then restore the
    /// previous value (or remove it when it was unset).
    async fn with_broken_agent_endpoint<F, Fut, T>(f: F) -> T
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        let previous = std::env::var("FUTURE_AGENT_GRPC_ADDR").ok();
        std::env::set_var("FUTURE_AGENT_GRPC_ADDR", "http://[::1");
        let result = f().await;
        match previous {
            Some(value) => std::env::set_var("FUTURE_AGENT_GRPC_ADDR", value),
            None => std::env::remove_var("FUTURE_AGENT_GRPC_ADDR"),
        }
        result
    }

    #[tokio::test]
    async fn fetch_balance_parses_credits() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-balance");
        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"balance_credits\":10000000000}".to_vec(),
        )]);
        point_auth_with_key(&url);
        let balance = fetch_balance().await.unwrap();
        assert_eq!(balance.credits, 1.0);
    }

    #[tokio::test]
    async fn fetch_balance_http_error_uses_body_message() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-balance-err");
        let url = mock_http_server(vec![(
            500,
            "application/json",
            b"{\"message\":\"boom\"}".to_vec(),
        )]);
        point_auth_with_key(&url);
        assert!(fetch_balance()
            .await
            .unwrap_err()
            .to_string()
            .contains("boom"));
    }

    #[tokio::test]
    async fn fetch_balance_preserves_auth_rejection_status() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-balance-auth");
        let url = mock_http_server(vec![(
            401,
            "application/json",
            b"{\"message\":\"expired\"}".to_vec(),
        )]);
        point_auth_with_key(&url);
        let error = fetch_balance().await.unwrap_err();
        assert!(is_auth_rejection(&error));
    }

    #[tokio::test]
    async fn fetch_balance_parse_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-balance-bad");
        let url = mock_http_server(vec![(200, "application/json", b"not json".to_vec())]);
        point_auth_with_key(&url);
        assert!(fetch_balance()
            .await
            .unwrap_err()
            .to_string()
            .contains("parse"));
    }

    #[tokio::test]
    async fn fetch_balance_requires_key() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-balance-out");
        assert!(fetch_balance()
            .await
            .unwrap_err()
            .to_string()
            .contains("Not signed in"));
    }

    #[tokio::test]
    async fn fetch_profile_parses_account() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-profile");
        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"email\":\"a@b.c\",\"user_id\":\"u1\"}".to_vec(),
        )]);
        point_auth_with_key(&url);
        let profile = fetch_profile().await.unwrap();
        assert_eq!(profile.email, "a@b.c");
        assert_eq!(profile.user_id, "u1");
    }

    #[tokio::test]
    async fn fetch_profile_http_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-profile-err");
        let url = mock_http_server(vec![(
            500,
            "application/json",
            b"{\"message\":\"nope\"}".to_vec(),
        )]);
        point_auth_with_key(&url);
        assert!(fetch_profile()
            .await
            .unwrap_err()
            .to_string()
            .contains("nope"));
    }

    #[tokio::test]
    async fn fetch_profile_parse_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-profile-bad");
        let url = mock_http_server(vec![(200, "application/json", b"not json".to_vec())]);
        point_auth_with_key(&url);
        assert!(fetch_profile()
            .await
            .unwrap_err()
            .to_string()
            .contains("parse"));
    }

    #[tokio::test]
    async fn fetch_profile_requires_key() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-profile-out");
        assert!(fetch_profile()
            .await
            .unwrap_err()
            .to_string()
            .contains("Not signed in"));
    }

    #[tokio::test]
    async fn auth_state_requires_platform_validation() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-auth-state");
        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"email\":\"a@b.c\",\"user_id\":\"u1\"}".to_vec(),
        )]);
        point_auth_with_key(&url);

        let state = check_auth_state().await;
        assert!(matches!(state.status, FutureAuthStatus::Authenticated));
        assert_eq!(state.profile.expect("profile").email, "a@b.c");
    }

    #[tokio::test]
    async fn auth_state_marks_rejected_key_invalid() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-auth-invalid");
        let url = mock_http_server(vec![(
            401,
            "application/json",
            b"{\"message\":\"expired\"}".to_vec(),
        )]);
        point_auth_with_key(&url);

        let state = check_auth_state().await;
        assert!(matches!(state.status, FutureAuthStatus::Invalid));
        assert!(state.profile.is_none());
    }

    #[tokio::test]
    async fn auth_state_keeps_server_failure_recoverable() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-auth-unavailable");
        let url = mock_http_server(vec![(
            503,
            "application/json",
            b"{\"message\":\"try later\"}".to_vec(),
        )]);
        point_auth_with_key(&url);

        let state = check_auth_state().await;
        assert!(matches!(state.status, FutureAuthStatus::Unavailable));
        assert!(state.profile.is_none());
    }

    #[tokio::test]
    async fn headless_start_returns_browser_authorization_without_writing_a_key() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-headless-start");
        let body = r#"{"device_code":"private-device-code","user_code":"ABCD","verification_uri_complete":"https://example.com/device?code=ABCD","expires_in":300,"interval":2}"#;
        let url = mock_http_server(vec![(200, "application/json", body.as_bytes().to_vec())]);
        point_auth(&url);
        let login = start_with_browser(false).await.unwrap();
        assert_eq!(login.user_code, "ABCD");
        assert_eq!(login.device_code, "private-device-code");
        assert!(login
            .verification_uri_complete
            .starts_with("https://example.com/device?code=ABCD"));
        assert!(future_api_key().is_err());
    }

    #[tokio::test]
    async fn start_returns_device_codes() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-start");
        // The success path reaches `open_browser`; install a no-op opener so it
        // doesn't launch a real browser.
        let _ = BROWSER_OPENER.set(|_| Ok(()));
        let body = r#"{"device_code":"dc-123","user_code":"uc-456","verification_uri":"https://example.com/device","verification_uri_complete":"https://example.com/device?code=uc-456","expires_in":300,"interval":5}"#;
        let url = mock_http_server(vec![(200, "application/json", body.as_bytes().to_vec())]);
        point_auth(&url);
        let out = start().await.unwrap();
        assert_eq!(out.device_code, "dc-123");
        assert_eq!(out.user_code, "uc-456");
        assert_eq!(out.interval, 5);
        assert_eq!(out.expires_in, 300);
    }

    #[tokio::test]
    async fn start_http_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-start-err");
        let url = mock_http_server(vec![(
            500,
            "application/json",
            b"{\"message\":\"down\"}".to_vec(),
        )]);
        point_auth(&url);
        assert!(start().await.unwrap_err().to_string().contains("down"));
    }

    #[tokio::test]
    async fn start_parse_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-start-bad");
        let url = mock_http_server(vec![(200, "application/json", b"not json".to_vec())]);
        point_auth(&url);
        assert!(start().await.unwrap_err().to_string().contains("parse"));
    }

    #[tokio::test]
    async fn start_missing_required_fields() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-start-missing");
        let body = r#"{"device_code":"","user_code":"uc","verification_uri":"https://e.com","verification_uri_complete":"https://e.com/c","expires_in":300,"interval":5}"#;
        let url = mock_http_server(vec![(200, "application/json", body.as_bytes().to_vec())]);
        point_auth(&url);
        assert!(start()
            .await
            .unwrap_err()
            .to_string()
            .contains("missing required"));
    }

    #[tokio::test]
    async fn start_invalid_expiry() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-start-expiry");
        let body = r#"{"device_code":"dc","user_code":"uc","verification_uri":"https://e.com","verification_uri_complete":"https://e.com/c","expires_in":0,"interval":5}"#;
        let url = mock_http_server(vec![(200, "application/json", body.as_bytes().to_vec())]);
        point_auth(&url);
        assert!(start()
            .await
            .unwrap_err()
            .to_string()
            .contains("invalid expiry"));
    }

    #[tokio::test]
    async fn start_missing_authorization_url() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-start-nourl");
        let body = r#"{"device_code":"dc","user_code":"uc","expires_in":300,"interval":5}"#;
        let url = mock_http_server(vec![(200, "application/json", body.as_bytes().to_vec())]);
        point_auth(&url);
        assert!(start()
            .await
            .unwrap_err()
            .to_string()
            .contains("authorization URL"));
    }

    #[tokio::test]
    async fn start_rejects_non_web_url_scheme() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-start-scheme");
        let body = r#"{"device_code":"dc","user_code":"uc","verification_uri":"javascript:alert(1)","verification_uri_complete":"javascript:alert(1)","expires_in":300,"interval":5}"#;
        let url = mock_http_server(vec![(200, "application/json", body.as_bytes().to_vec())]);
        point_auth(&url);
        assert!(start().await.unwrap_err().to_string().contains("scheme"));
    }

    #[tokio::test]
    async fn start_network_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-start-net");
        crate::auth_store::set_future_base_url("http://127.0.0.1:1/api").unwrap();
        assert!(start()
            .await
            .unwrap_err()
            .to_string()
            .contains("Failed to request device code"));
    }

    #[tokio::test]
    async fn poll_error_statuses() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-poll-errors");
        let cases = [
            ("authorization_pending", "pending"),
            ("slow_down", "slow_down"),
            ("access_denied", "denied"),
            ("expired_token", "expired"),
            ("weird", "error"),
        ];
        for (code, expected) in cases {
            let body = format!(r#"{{"error":"{code}","message":"msg"}}"#);
            let url = mock_http_server(vec![(400, "application/json", body.into_bytes())]);
            point_auth(&url);
            let poll = poll("dc").await.unwrap();
            assert_eq!(poll.status, expected, "code {code}");
        }
    }

    #[tokio::test]
    async fn poll_parse_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-poll-bad");
        let url = mock_http_server(vec![(200, "application/json", b"not json".to_vec())]);
        point_auth(&url);
        assert_eq!(poll("dc").await.unwrap().status, "malformed");
    }

    #[tokio::test]
    async fn poll_network_error() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-poll-net");
        crate::auth_store::set_future_base_url("http://127.0.0.1:1/api").unwrap();
        assert_eq!(poll("dc").await.unwrap().status, "retry");
    }

    #[tokio::test]
    async fn poll_retries_transient_http_status() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-poll-503");
        let url = mock_http_server(vec![(
            503,
            "application/json",
            b"{\"message\":\"try later\"}".to_vec(),
        )]);
        point_auth(&url);
        let out = poll("dc").await.unwrap();
        assert_eq!(out.status, "retry");
        assert!(out.message.is_none());
    }

    #[tokio::test]
    async fn poll_honors_retry_after_header() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-poll-429");
        let url = mock_http_server_with_headers(vec![(
            429,
            "application/json",
            vec![("Retry-After", "7")],
            b"{\"message\":\"slow down\"}".to_vec(),
        )]);
        point_auth(&url);
        let out = poll("dc").await.unwrap();
        assert_eq!(out.status, "retry");
        assert_eq!(out.retry_after_seconds, Some(7));
    }

    #[tokio::test]
    async fn fetch_balance_http_error_without_message() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-balance-err2");
        let url = mock_http_server(vec![(500, "application/json", b"{}".to_vec())]);
        point_auth_with_key(&url);
        assert!(fetch_balance()
            .await
            .unwrap_err()
            .to_string()
            .contains("HTTP 500"));
    }

    #[tokio::test]
    async fn fetch_profile_http_error_without_message() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-profile-err2");
        let url = mock_http_server(vec![(500, "application/json", b"{}".to_vec())]);
        point_auth_with_key(&url);
        assert!(fetch_profile()
            .await
            .unwrap_err()
            .to_string()
            .contains("HTTP 500"));
    }

    #[tokio::test]
    async fn poll_missing_api_key() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-poll-nokey");
        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"api_key\":\"\",\"token_type\":\"api_key\"}".to_vec(),
        )]);
        point_auth(&url);
        let out = poll("dc").await.unwrap();
        assert_eq!(out.status, "malformed");
        assert!(out.message.is_none());
    }

    #[tokio::test]
    async fn poll_unsupported_token_type() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-poll-badtype");
        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"api_key\":\"k\",\"token_type\":\"bearer\"}".to_vec(),
        )]);
        point_auth(&url);
        let out = poll("dc").await.unwrap();
        assert_eq!(out.status, "malformed");
        assert!(out.message.is_none());
    }

    #[tokio::test]
    async fn poll_malformed_token_body() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-poll-malformed");
        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"api_key\":123}".to_vec(),
        )]);
        point_auth(&url);
        assert_eq!(poll("dc").await.unwrap().status, "malformed");
    }

    #[tokio::test]
    async fn poll_does_not_report_authorized_when_agent_write_fails() {
        let _home = crate::auth_store::test_support::HomeGuard::new("fl-poll-ok");
        // Force `connect_agent` to fail deterministically with an unparseable
        // endpoint — `Endpoint::from_shared` fails before the latched channel
        // is consulted. Restore the env var after
        // instead of re-pointing at the shared mock: starting the mock here
        // would re-order the process-wide agent-channel latch and break the
        // agent_bridge mock tests.
        let url = mock_http_server(vec![(
            200,
            "application/json",
            b"{\"api_key\":\"sekret\",\"token_type\":\"api_key\"}".to_vec(),
        )]);
        point_auth(&url);

        let error = with_broken_agent_endpoint(|| poll("dc"))
            .await
            .expect_err("authorization must wait for the Agent write");

        assert!(error.to_string().contains("not saved"));
        assert!(crate::auth_store::read()
            .unwrap()
            .get("future")
            .and_then(|entry| entry.get("key"))
            .is_none());
    }
}

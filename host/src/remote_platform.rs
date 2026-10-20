//! Shared host-side HTTP adaptation for both Remote roles and relay control.
//! Application TLS setup and error presentation stay outside the role libraries.
use serde_json::Value;
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub(crate) fn http_client() -> Result<reqwest::Client, crate::Error> {
    crate::install_rustls_provider();
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| crate::Error::Message(format!("Failed to create HTTP client: {error}")))
}

/// Build the error for a failed `.send()`: a [`crate::Error::RemoteTransport`] when no HTTP
/// response was possible (so the UI can say "check your network"), a plain
/// [`crate::Error::Message`] otherwise. The original reqwest detail is preserved in both for
/// logs.
pub(crate) fn transport_or_message(action: &str, error: reqwest::Error) -> crate::Error {
    let message = format!("Failed to {action}: {error}");
    if is_transport_error(&error) {
        crate::Error::RemoteTransport(message)
    } else {
        crate::Error::Message(message)
    }
}

/// A send failure that never produced an HTTP response — i.e. the network path
/// itself failed (offline, DNS, connection refused, timeout). Status / body /
/// decode errors mean we *did* reach the server, so they aren't transport
/// failures.
pub(crate) fn is_transport_error(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_timeout() || error.is_request() || error.is_redirect()
}

pub(crate) async fn parse_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    action: &str,
) -> Result<T, crate::Error> {
    if !response.status().is_success() {
        return Err(response_error(response, action).await);
    }
    response.json::<T>().await.map_err(|error| {
        crate::Error::Message(format!("Failed to parse {action} response: {error}"))
    })
}

pub(crate) async fn response_error(response: reqwest::Response, action: &str) -> crate::Error {
    let status = response.status();
    let body = response.json::<Value>().await.ok();
    // The platform error body is `{error: <machine code>, message: <human text>}`.
    let code = body
        .as_ref()
        .and_then(|value| value.get("error"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|code| !code.trim().is_empty());
    let message = body
        .as_ref()
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Failed to {action} (HTTP {})", status.as_u16()));
    // Error bodies can reflect credentials. Keep logs to the action/status;
    // the typed UI error still retains the platform code and message.
    eprintln!("remote: {action} failed: HTTP {}", status.as_u16());
    crate::Error::Remote {
        status: status.as_u16(),
        code,
        message,
    }
}

pub(crate) fn jwt_expiry(jwt: &str) -> Result<i64, crate::Error> {
    future_remote_protocol::jwt::expiry(jwt)
        .map_err(|error| crate::Error::Message(error.to_string()))
}

//! Persisted desktop identity and the platform pairing/JWT control plane.
//!
//! The NKey seed never leaves this desktop. The platform receives only the
//! public user key and returns a short-lived, pair-scoped NATS user JWT.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCreds {
    pub pair_id: String,
    pub desktop_id: String,
    pub nkey_seed: String,
    pub user_jwt: String,
    pub nats_url: String,
    pub nats_ws_url: String,
    pub jwt_expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct CreatePairCodeResponse {
    pair_id: String,
    pairing_code: String,
    user_jwt: String,
    nats_url: String,
    nats_ws_url: String,
}

#[derive(Debug, Deserialize)]
struct RefreshTokenResponse {
    user_jwt: String,
    nats_url: String,
    nats_ws_url: String,
}

fn pairing_path() -> Result<PathBuf, crate::AppError> {
    let home = crate::home_dir().ok_or_else(|| {
        crate::AppError::Message("HOME/USERPROFILE environment variable is not set.".to_string())
    })?;
    Ok(PathBuf::from(home)
        .join(".future")
        .join("remote_pairing.json"))
}

pub fn load_creds() -> Option<PairingCreds> {
    let path = pairing_path().ok()?;
    let value = crate::config_io::read_json_object(&path).ok()?;
    serde_json::from_value(value).ok()
}

pub fn save_creds(creds: &PairingCreds) -> Result<(), crate::AppError> {
    let path = pairing_path()?;
    let value = serde_json::to_value(creds)
        .map_err(|error| crate::AppError::Message(format!("encode pairing creds: {error}")))?;
    crate::config_io::write_json_atomic(&path, &value, true)
}

pub fn clear_creds() -> Result<(), crate::AppError> {
    let path = pairing_path()?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|error| crate::AppError::Message(format!("remove pairing creds: {error}")))?;
    }
    Ok(())
}

/// Whether a refresh failure means the server has revoked (or no longer
/// recognizes) this pairing. Transport, login, and other transient failures
/// must not be treated as a revocation: keeping the persisted pairing lets a
/// later retry recover normally. Matched on the server's machine-readable
/// error code (`invalid_remote_credential`), not the human message.
pub fn is_invalid_or_revoked_error(error: &crate::AppError) -> bool {
    matches!(
        error,
        crate::AppError::Remote {
            code: Some(code),
            ..
        } if code == "invalid_remote_credential"
    )
}

/// Map a remote-control failure to a stable, machine-readable category the UI
/// localizes (`error.<code>`). Returns `None` for errors that aren't
/// remote-control failures (local IO, NKey, SQLite) — those keep surfacing as a
/// raw string. Never sniff the human message: only the variant / server code is
/// authoritative.
///
/// - `network` — the call never got a response (offline, DNS, refused, timeout).
/// - `revoked` — the server rejected the credential (web unpair / re-pair).
/// - `server`  — the server responded with an error status.
pub fn error_code(error: &crate::AppError) -> Option<&'static str> {
    match error {
        crate::AppError::RemoteTransport(_) => Some("network"),
        crate::AppError::Remote { code, .. } => match code.as_deref() {
            Some("invalid_remote_credential") => Some("revoked"),
            _ => Some("server"),
        },
        _ => None,
    }
}

pub async fn create_pairing() -> Result<(PairingCreds, String, Option<i64>), crate::AppError> {
    let key_pair = nkeys::KeyPair::new_user();
    let nkey_seed = key_pair
        .seed()
        .map_err(|error| crate::AppError::Message(format!("generate desktop NKey: {error}")))?;
    let desktop_id = load_creds()
        .map(|creds| creds.desktop_id)
        .unwrap_or_else(new_device_id);
    let platform = crate::future_platform::current_platform_url();
    let response = http_client()?
        .post(format!("{platform}/client/v1/remote/pair/code"))
        .bearer_auth(crate::future_login::future_api_key()?)
        .json(&json!({
            "desktop_id": desktop_id,
            "desktop_public_key": key_pair.public_key(),
            "desktop_name": "FutureOS GUI",
        }))
        .send()
        .await
        .map_err(|error| transport_or_message("create pairing code", error))?;
    let response: CreatePairCodeResponse = parse_response(response, "create pairing code").await?;
    let jwt_expires_at = jwt_expiry(&response.user_jwt)?;
    let code_expires_at = pairing_code_expiry(&response.pairing_code);
    let creds = PairingCreds {
        pair_id: response.pair_id,
        desktop_id,
        nkey_seed,
        user_jwt: response.user_jwt,
        nats_url: response.nats_url,
        nats_ws_url: response.nats_ws_url,
        jwt_expires_at,
    };
    Ok((creds, response.pairing_code, code_expires_at))
}

/// Decode a v2 pairing code's `exp` (unix seconds). Self-contained — the
/// expiry travels inside the code payload the web client also validates, so
/// there's a single source of truth. `None` if the code can't be decoded.
pub fn pairing_code_expiry(code: &str) -> Option<i64> {
    let bytes = URL_SAFE_NO_PAD.decode(code).ok()?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()?
        .get("exp")
        .and_then(Value::as_i64)
}

pub async fn refresh_bridge_jwt(mut creds: PairingCreds) -> Result<PairingCreds, crate::AppError> {
    let key_pair = nkeys::KeyPair::from_seed(&creds.nkey_seed)
        .map_err(|error| crate::AppError::Message(format!("read desktop NKey: {error}")))?;
    let platform = crate::future_platform::current_platform_url();
    let response = http_client()?
        .post(format!("{platform}/client/v1/remote/auth/token"))
        .bearer_auth(crate::future_login::future_api_key()?)
        .json(&json!({
            "pair_id": creds.pair_id,
            "device_id": creds.desktop_id,
            "public_key": key_pair.public_key(),
            "role": "bridge",
        }))
        .send()
        .await
        .map_err(|error| transport_or_message("refresh remote credential", error))?;
    let response: RefreshTokenResponse =
        parse_response(response, "refresh remote credential").await?;
    creds.jwt_expires_at = jwt_expiry(&response.user_jwt)?;
    creds.user_jwt = response.user_jwt;
    creds.nats_url = response.nats_url;
    creds.nats_ws_url = response.nats_ws_url;
    Ok(creds)
}

pub async fn revoke_pairing(creds: &PairingCreds) -> Result<(), crate::AppError> {
    let platform = crate::future_platform::current_platform_url();
    let response = http_client()?
        .post(format!("{platform}/client/v1/remote/pair/revoke"))
        .bearer_auth(crate::future_login::future_api_key()?)
        .json(&json!({ "pair_id": creds.pair_id }))
        .send()
        .await
        .map_err(|error| transport_or_message("revoke remote pairing", error))?;
    if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    Err(response_error(response, "revoke remote pairing").await)
}

pub fn new_device_id() -> String {
    let key = nkeys::KeyPair::new_user().public_key();
    format!("desktop_{}", &key[1..17])
}

pub fn refresh_delay(creds: &PairingCreds) -> std::time::Duration {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    std::time::Duration::from_secs(
        creds
            .jwt_expires_at
            .saturating_sub(now)
            .saturating_sub(60)
            .max(5) as u64,
    )
}

fn http_client() -> Result<reqwest::Client, crate::AppError> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|error| crate::AppError::Message(format!("Failed to create HTTP client: {error}")))
}

/// Build the error for a failed `.send()`: a [`RemoteTransport`] when no HTTP
/// response was possible (so the UI can say "check your network"), a plain
/// [`Message`] otherwise. The original reqwest detail is preserved in both for
/// logs.
fn transport_or_message(action: &str, error: reqwest::Error) -> crate::AppError {
    let message = format!("Failed to {action}: {error}");
    if is_transport_error(&error) {
        crate::AppError::RemoteTransport(message)
    } else {
        crate::AppError::Message(message)
    }
}

/// A send failure that never produced an HTTP response — i.e. the network path
/// itself failed (offline, DNS, connection refused, timeout). Status / body /
/// decode errors mean we *did* reach the server, so they aren't transport
/// failures.
fn is_transport_error(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_timeout() || error.is_request() || error.is_redirect()
}

async fn parse_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    action: &str,
) -> Result<T, crate::AppError> {
    if !response.status().is_success() {
        return Err(response_error(response, action).await);
    }
    response.json::<T>().await.map_err(|error| {
        crate::AppError::Message(format!("Failed to parse {action} response: {error}"))
    })
}

async fn response_error(response: reqwest::Response, action: &str) -> crate::AppError {
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
    crate::AppError::Remote {
        status: status.as_u16(),
        code,
        message,
    }
}

fn jwt_expiry(jwt: &str) -> Result<i64, crate::AppError> {
    let payload = jwt.split('.').nth(1).ok_or_else(|| {
        crate::AppError::Message("Remote server returned an invalid JWT.".to_string())
    })?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| {
        crate::AppError::Message("Remote server returned an invalid JWT.".to_string())
    })?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| value.get("exp").and_then(Value::as_i64))
        .ok_or_else(|| crate::AppError::Message("Remote JWT is missing its expiry.".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_revoked_credential_error() {
        assert!(is_invalid_or_revoked_error(&crate::AppError::Remote {
            status: 401,
            code: Some("invalid_remote_credential".to_string()),
            message: "Remote credential is invalid or revoked.".to_string(),
        }));
        // A human message that merely reads like a revocation must NOT match —
        // only the machine code is authoritative.
        assert!(!is_invalid_or_revoked_error(&crate::AppError::Remote {
            status: 401,
            code: Some("unauthorized".to_string()),
            message: "Remote credential is invalid or revoked.".to_string(),
        }));
        assert!(!is_invalid_or_revoked_error(&crate::AppError::Message(
            "Failed to refresh remote credential: timeout".to_string(),
        )));
    }

    #[test]
    fn classifies_error_codes_without_sniffing_messages() {
        // Offline / unreachable → network, regardless of the human message.
        assert_eq!(
            error_code(&crate::AppError::RemoteTransport(
                "Failed to create pairing code: error sending request".to_string(),
            )),
            Some("network"),
        );
        // Server revocation is read from the machine code, not the message.
        assert_eq!(
            error_code(&crate::AppError::Remote {
                status: 401,
                code: Some("invalid_remote_credential".to_string()),
                message: "gone".to_string(),
            }),
            Some("revoked"),
        );
        // Any other server status → generic server category.
        assert_eq!(
            error_code(&crate::AppError::Remote {
                status: 500,
                code: None,
                message: "boom".to_string(),
            }),
            Some("server"),
        );
        // Local failures carry no remote category.
        assert_eq!(
            error_code(&crate::AppError::Message("disk full".to_string())),
            None,
        );
    }
}

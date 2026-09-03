//! Platform pairing/JWT control plane.
//!
//! The NKey seed never leaves this desktop. The platform receives only the
//! public user key and returns a short-lived, pair-scoped NATS user JWT. The
//! installation-wide device id is owned by `device_identity`; pairing stores a
//! copy because the server credential is bound to that exact identity.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCreds {
    #[serde(default)]
    pub handshake_version: u32,
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

/// One-shot injected failure for `save_creds` (tests only): the credential
/// file lives in the test HOME, which tests fully control, but the
/// credential-refresh loop saves under the global STATE lock mid-iteration —
/// no interleaving point exists to break the filesystem at exactly that
/// moment, so the log-and-continue arm is exercised through this seam.
#[cfg(test)]
pub(crate) static INJECT_SAVE_FAILURE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn save_creds(creds: &PairingCreds) -> Result<(), crate::AppError> {
    #[cfg(test)]
    if INJECT_SAVE_FAILURE.swap(false, std::sync::atomic::Ordering::Relaxed) {
        return Err(crate::AppError::Message(
            "injected save failure".to_string(),
        ));
    }
    let path = pairing_path()?;
    let value = serde_json::to_value(creds)
        .map_err(|error| crate::AppError::Message(format!("encode pairing creds: {error}")))?;
    crate::config_io::write_json_atomic(&path, &value, true)
}

pub fn public_key(creds: &PairingCreds) -> Result<String, crate::AppError> {
    nkeys::KeyPair::from_seed(&creds.nkey_seed)
        .map(|key_pair| key_pair.public_key())
        .map_err(|error| crate::AppError::Message(format!("read desktop NKey: {error}")))
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
        crate::AppError::RemoteAuthorization(_) => Some("service_authorization"),
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
    let desktop_id = crate::device_identity::device_id()?;
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
        handshake_version: 1,
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
    crate::install_rustls_provider();
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
    eprintln!(
        "remote: {action} failed: HTTP {}, code={:?}, message={}, body={}",
        status.as_u16(),
        code,
        message,
        body.as_ref()
            .map(serde_json::to_string)
            .transpose()
            .unwrap_or_default()
            .unwrap_or_default(),
    );
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

#[cfg(test)]
mod http_tests {
    use super::super::test_support::{
        jwt, now_secs, pairing_code, sign_in, HomeGuard, MockPlatform,
    };
    use super::*;
    use serde_json::json;

    fn fixture_creds() -> PairingCreds {
        let key_pair = nkeys::KeyPair::new_user();
        PairingCreds {
            handshake_version: 1,
            pair_id: "pair_test".to_string(),
            desktop_id: "desktop_test".to_string(),
            nkey_seed: key_pair.seed().unwrap().to_string(),
            user_jwt: jwt(now_secs() + 3600),
            nats_url: "nats://127.0.0.1:1".to_string(),
            nats_ws_url: "ws://127.0.0.1:1".to_string(),
            jwt_expires_at: now_secs() + 3600,
        }
    }

    #[test]
    fn creds_save_load_clear_roundtrip() {
        let _home = HomeGuard::new("pairing-roundtrip");
        assert!(load_creds().is_none(), "no file → no creds");

        let creds = fixture_creds();
        save_creds(&creds).unwrap();
        let loaded = load_creds().expect("saved creds load back");
        assert_eq!(loaded.pair_id, "pair_test");
        assert_eq!(loaded.handshake_version, 1);
        assert_eq!(public_key(&loaded).unwrap(), {
            nkeys::KeyPair::from_seed(&creds.nkey_seed)
                .unwrap()
                .public_key()
        });

        clear_creds().unwrap();
        assert!(load_creds().is_none());
        // Clearing with no file present is a no-op success.
        clear_creds().unwrap();
    }

    #[test]
    fn load_creds_returns_none_on_corrupt_file() {
        let _home = HomeGuard::new("pairing-corrupt");
        let path = pairing_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        assert!(load_creds().is_none());
        // A valid JSON file that does not match the creds shape also → None.
        std::fs::write(&path, json!({ "unrelated": true }).to_string()).unwrap();
        assert!(load_creds().is_none());
    }

    #[test]
    fn public_key_rejects_a_bad_seed() {
        let _home = HomeGuard::new("pairing-bad-seed");
        let mut creds = fixture_creds();
        creds.nkey_seed = "not-a-seed".to_string();
        assert!(public_key(&creds).is_err());
    }

    #[test]
    fn pairing_path_requires_a_home() {
        let _home = HomeGuard::new("pairing-no-home");
        std::env::remove_var("HOME");
        std::env::remove_var("USERPROFILE");
        assert!(pairing_path().is_err());
        assert!(load_creds().is_none());
        assert!(clear_creds().is_err());
        assert!(save_creds(&fixture_creds()).is_err());
    }

    #[tokio::test]
    async fn create_pairing_success() {
        let _home = HomeGuard::new("pairing-create");
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        let code = platform.respond_pair_code("nats://127.0.0.1:4222");

        let (creds, returned_code, code_expires_at) = create_pairing().await.unwrap();
        assert_eq!(returned_code, code);
        assert!(creds.pair_id.starts_with("pair_mock-"));
        assert!(creds.desktop_id.starts_with("desktop_"));
        assert_eq!(creds.nats_url, "nats://127.0.0.1:4222");
        assert!(creds.jwt_expires_at > now_secs());
        // The mock stamps `exp = now + 600` when it builds the pair-code
        // response; a second boundary can pass before this assertion runs, so
        // allow the one-second skew instead of an exact `now + 600`.
        let expires_at = code_expires_at.expect("code expiry");
        let delta = expires_at - now_secs();
        assert!((599..=600).contains(&delta), "expiry delta {delta}");

        let requests = platform.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, "/client/v1/remote/pair/code");
        let body: Value = serde_json::from_str(&requests[0].2).unwrap();
        assert_eq!(body["desktop_id"], json!(creds.desktop_id));
        assert_eq!(body["desktop_name"], json!("FutureOS GUI"));
    }

    #[tokio::test]
    async fn create_pairing_reuses_the_persisted_desktop_id() {
        let _home = HomeGuard::new("pairing-create-reuse");
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        save_creds(&fixture_creds()).unwrap();
        platform.respond_pair_code("nats://127.0.0.1:4222");

        let (creds, _, _) = create_pairing().await.unwrap();
        assert_eq!(creds.desktop_id, "desktop_test");
    }

    #[tokio::test]
    async fn create_pairing_maps_network_and_server_failures() {
        let _home = HomeGuard::new("pairing-create-errors");
        // Nothing listening → transport failure.
        sign_in("http://127.0.0.1:9");
        let error = create_pairing().await.unwrap_err();
        assert!(matches!(error, crate::AppError::RemoteTransport(_)));
        assert_eq!(error_code(&error), Some("network"));

        // HTTP 500 with a platform error body → categorized remote error.
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        platform.push(
            "/client/v1/remote/pair/code",
            500,
            json!({ "error": "boom", "message": "server exploded" }),
        );
        let error = create_pairing().await.unwrap_err();
        assert_eq!(error_code(&error), Some("server"));
        match error {
            crate::AppError::Remote {
                status,
                code,
                message,
            } => {
                assert_eq!(status, 500);
                assert_eq!(code.as_deref(), Some("boom"));
                assert_eq!(message, "server exploded");
            }
            other => panic!("expected Remote, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_pairing_rejects_an_invalid_jwt() {
        let _home = HomeGuard::new("pairing-create-bad-jwt");
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        platform.push(
            "/client/v1/remote/pair/code",
            200,
            json!({
                "pair_id": "pair_x",
                "pairing_code": pairing_code(now_secs() + 60),
                "user_jwt": "not-a-jwt",
                "nats_url": "nats://127.0.0.1:4222",
                "nats_ws_url": "ws://127.0.0.1:4222",
            }),
        );
        let error = create_pairing().await.unwrap_err();
        assert!(error.to_string().contains("invalid JWT"));
    }

    #[tokio::test]
    async fn create_pairing_rejects_an_undecodable_response() {
        let _home = HomeGuard::new("pairing-create-bad-body");
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        platform.push(
            "/client/v1/remote/pair/code",
            200,
            json!({ "unexpected": 1 }),
        );
        let error = create_pairing().await.unwrap_err();
        assert!(error
            .to_string()
            .contains("Failed to parse create pairing code"));
    }

    #[tokio::test]
    async fn refresh_bridge_jwt_success_and_failures() {
        let _home = HomeGuard::new("pairing-refresh");
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        let creds = fixture_creds();

        platform.respond_refresh("nats://127.0.0.1:4223");
        let refreshed = refresh_bridge_jwt(creds.clone()).await.unwrap();
        assert_eq!(refreshed.nats_url, "nats://127.0.0.1:4223");
        assert!(refreshed.jwt_expires_at > now_secs());

        // Revocation is recognized via the machine code.
        platform.respond_refresh_revoked();
        let error = refresh_bridge_jwt(creds.clone()).await.unwrap_err();
        assert!(is_invalid_or_revoked_error(&error));
        assert_eq!(error_code(&error), Some("revoked"));

        // A server error without the code is NOT a revocation.
        platform.push(
            "/client/v1/remote/auth/token",
            500,
            json!({ "message": "kaput" }),
        );
        let error = refresh_bridge_jwt(creds.clone()).await.unwrap_err();
        assert!(!is_invalid_or_revoked_error(&error));

        // Transport failure keeps the pairing retryable.
        sign_in("http://127.0.0.1:9");
        let error = refresh_bridge_jwt(creds).await.unwrap_err();
        assert!(matches!(error, crate::AppError::RemoteTransport(_)));
        assert!(!is_invalid_or_revoked_error(&error));
    }

    #[tokio::test]
    async fn revoke_pairing_success_not_found_and_error() {
        let _home = HomeGuard::new("pairing-revoke");
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        let creds = fixture_creds();

        platform.push("/client/v1/remote/pair/revoke", 200, json!({}));
        revoke_pairing(&creds).await.unwrap();

        // An already-unknown pairing is a successful unpair.
        platform.push("/client/v1/remote/pair/revoke", 404, json!({}));
        revoke_pairing(&creds).await.unwrap();

        platform.push(
            "/client/v1/remote/pair/revoke",
            500,
            json!({ "error": "boom", "message": "no" }),
        );
        assert!(revoke_pairing(&creds).await.is_err());
    }

    #[test]
    fn pairing_code_expiry_decodes_only_valid_codes() {
        assert_eq!(pairing_code_expiry(&pairing_code(1234)), Some(1234));
        assert_eq!(pairing_code_expiry("!!! not base64 !!!"), None);
        assert_eq!(
            pairing_code_expiry(&URL_SAFE_NO_PAD.encode(json!({ "nope": 1 }).to_string())),
            None
        );
    }

    #[test]
    fn refresh_delay_clamps_between_floor_and_expiry() {
        let mut creds = fixture_creds();
        creds.jwt_expires_at = now_secs() + 3600;
        let delay = refresh_delay(&creds);
        assert!(delay.as_secs() > 3500 && delay.as_secs() <= 3540);
        // Already expired → the 5s floor.
        creds.jwt_expires_at = now_secs() - 10;
        assert_eq!(refresh_delay(&creds).as_secs(), 5);
    }

    #[test]
    fn jwt_expiry_rejects_malformed_tokens() {
        assert!(jwt_expiry("one-segment").is_err());
        assert!(jwt_expiry("two.segments").is_err()); // decodes, but not JSON
        assert!(jwt_expiry("a.!!!.b").is_err()); // invalid base64 payload → decode failure
        let no_exp = URL_SAFE_NO_PAD.encode(json!({ "sub": 1 }).to_string());
        assert!(jwt_expiry(&format!("h.{no_exp}.s")).is_err());
        assert_eq!(jwt_expiry(&jwt(42)).unwrap(), 42);
    }

    #[tokio::test]
    async fn transport_or_message_distinguishes_failure_kinds() {
        let _home = HomeGuard::new("pairing-transport-class");
        // A connection-refused send error is a transport failure.
        let error = reqwest::Client::new()
            .get("http://127.0.0.1:9/")
            .send()
            .await
            .unwrap_err();
        assert!(is_transport_error(&error));
        assert!(matches!(
            transport_or_message("probe", error),
            crate::AppError::RemoteTransport(_)
        ));

        // A body-decode error DID reach the server → plain message.
        let platform = MockPlatform::start().await;
        platform.push("/anything", 200, json!("not the expected shape"));
        #[derive(Debug, serde::Deserialize)]
        struct Strict {
            #[allow(dead_code)]
            definitely_missing: String,
        }
        let error = reqwest::Client::new()
            .get(format!("{}/anything", platform.url()))
            .send()
            .await
            .unwrap()
            .json::<Strict>()
            .await
            .unwrap_err();
        assert!(!is_transport_error(&error));
        assert!(matches!(
            transport_or_message("probe", error),
            crate::AppError::Message(_)
        ));
    }

    /// Destructure a remote error; panics (with the actual variant) otherwise.
    fn remote_error_parts(error: crate::AppError) -> (u16, Option<String>, String) {
        match error {
            crate::AppError::Remote {
                status,
                code,
                message,
            } => (status, code, message),
            other => panic!("expected Remote, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn response_error_defaults_and_filters() {
        let _home = HomeGuard::new("pairing-response-error");
        let platform = MockPlatform::start().await;
        sign_in(platform.url());

        // No error/message fields → synthesized message, no code.
        platform.push("/client/v1/remote/pair/code", 500, json!({}));
        let (status, code, message) = remote_error_parts(create_pairing().await.unwrap_err());
        assert_eq!(status, 500);
        assert!(code.is_none());
        assert!(message.contains("HTTP 500"));

        // Whitespace-only fields are filtered out.
        platform.push(
            "/client/v1/remote/pair/code",
            500,
            json!({ "error": "  ", "message": " " }),
        );
        let (status, code, message) = remote_error_parts(create_pairing().await.unwrap_err());
        assert_eq!(status, 500);
        assert!(code.is_none());
        assert!(message.contains("HTTP 500"));
    }

    #[test]
    #[should_panic(expected = "expected Remote")]
    fn remote_error_parts_panics_on_non_remote() {
        let _ = remote_error_parts(crate::AppError::Message("x".to_string()));
    }
}

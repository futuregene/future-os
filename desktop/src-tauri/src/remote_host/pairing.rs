//! Platform pairing/JWT control plane.
//!
//! The NKey seed never leaves this desktop. The platform receives only the
//! public user key and returns a short-lived, pair-scoped NATS user JWT. The
//! installation-wide device id is owned by `device_identity`; pairing stores a
//! copy because the server credential is bound to that exact identity.

#[cfg(test)]
use crate::remote::pairing::{
    error_code, is_invalid_or_revoked_error, public_key, refresh_delay, REFRESH_LEAD,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub(crate) use crate::remote::protocol::PairingCreds;

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

/// Serializes revoke delivery with invitation creation.
///
/// A revoke and the invitation that replaces it change the same binding, and the
/// platform reuses the pair id for a known desktop. An in-flight revoke must
/// finish — and its queued entry be retired — before `create_pairing` hands out
/// the replacement, or the revoke that was meant for the old binding would land
/// on the new one.
static REVOKE_DELIVERY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn pairing_path() -> Result<PathBuf, crate::AppError> {
    Ok(crate::store::future_dir()?.join("remote_pairing.json"))
}

/// Where the multi-device credential book lives.
///
/// A separate file from [`pairing_path`], deliberately: the legacy file keeps
/// describing exactly one pairing in the shape older builds parse, so a
/// downgrade still finds a pairing and does not mint a new invitation (which
/// would revoke the one the book holds). The old file is therefore read as a
/// book with one slot, and left on disk until an explicit unpair.
fn book_path() -> Result<PathBuf, crate::AppError> {
    Ok(crate::store::future_dir()?.join("remote_pairings.json"))
}

const BOOK_VERSION: u32 = 2;

/// Every device this desktop has paired, plus which one is in control.
///
/// One slot per `pair_id`. Each carries its own NKey and secure identity — two
/// slots must never share a key: the platform derives a NATS user from the
/// public key and hashes its claims into the JWT `jti`, so one key in two
/// pairings is a single NATS user with two namespaces, and the broker's
/// per-pair isolation stops meaning anything.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairingBook {
    #[serde(default = "book_version")]
    version: u32,
    /// The device currently driving this desktop. `None` when nothing has
    /// claimed control yet.
    #[serde(default)]
    active: Option<String>,
    #[serde(default)]
    slots: Vec<PairingCreds>,
}

fn book_version() -> u32 {
    BOOK_VERSION
}

impl PairingBook {
    /// The slot a single-slot caller means: the one in control, else the first.
    ///
    /// Until the per-slot runtime lands, every caller is a single-slot caller,
    /// so this is the whole of the compatibility story.
    fn primary(&self) -> Option<&PairingCreds> {
        match self.active.as_deref() {
            Some(pair_id) => self.slots.iter().find(|slot| slot.pair_id == pair_id),
            None => None,
        }
        .or_else(|| self.slots.first())
    }

    fn slot(&self, pair_id: &str) -> Option<&PairingCreds> {
        self.slots.iter().find(|slot| slot.pair_id == pair_id)
    }

    /// Insert or replace by `pair_id`; a re-pair of the same device keeps its
    /// position, so the list does not reshuffle when a token is renewed.
    fn upsert(&mut self, creds: PairingCreds) {
        match self
            .slots
            .iter_mut()
            .find(|slot| slot.pair_id == creds.pair_id)
        {
            Some(slot) => *slot = creds,
            None => self.slots.push(creds),
        }
    }
}

pub fn load_creds() -> Option<PairingCreds> {
    load_creds_checked().ok().flatten()
}

/// Terminal startup must not turn corrupt saved credentials into a new pairing.
pub(crate) fn load_creds_checked() -> Result<Option<PairingCreds>, crate::AppError> {
    Ok(load_book_checked()?.primary().cloned())
}

fn load_book_checked() -> Result<PairingBook, crate::AppError> {
    let path = book_path()?;
    if path.try_exists()? {
        let value = crate::config_io::read_json_object(&path)?;
        return Ok(serde_json::from_value(value)?);
    }
    // An installation paired before multi-device kept one credential here. It
    // becomes the book's only slot — and the one in control, because a single
    // device is trivially the one driving the machine.
    let legacy = pairing_path()?;
    if !legacy.try_exists()? {
        return Ok(PairingBook {
            version: BOOK_VERSION,
            active: None,
            slots: Vec::new(),
        });
    }
    let creds: PairingCreds = serde_json::from_value(crate::config_io::read_json_object(&legacy)?)?;
    Ok(PairingBook {
        version: BOOK_VERSION,
        active: Some(creds.pair_id.clone()),
        slots: vec![creds],
    })
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
    let path = book_path()?;
    crate::config_io::with_config_lock(&path, || {
        let mut book = load_book_checked()?;
        let mut creds = creds.clone();
        // Credential refresh may have started before pairing committed. Never
        // overwrite a pinned identity with that stale pending invitation.
        if let Some(stored) = book.slot(&creds.pair_id) {
            if stored
                .secure
                .as_ref()
                .is_some_and(|identity| identity.peer_public_key.is_some())
            {
                creds.secure = stored.secure.clone();
            }
        }
        book.upsert(creds);
        // Something has to be in control, or a single-device desktop would
        // report itself as paired but unclaimed. The takeover rules that decide
        // *which* device that is arrive with the per-slot runtime; until then
        // the only slot there is.
        if book.active.is_none() {
            book.active = book.slots.first().map(|slot| slot.pair_id.clone());
        }
        let value = serde_json::to_value(&book)
            .map_err(|error| crate::AppError::Message(format!("encode pairing creds: {error}")))?;
        crate::config_io::write_json_atomic(&path, &value, true)
    })
}

/// Forget every device. This is the unpair / re-pair path, whose published
/// meaning is "this machine has no devices": clearing some of them would leave
/// a pairing the user was told was gone.
pub fn clear_creds() -> Result<(), crate::AppError> {
    for path in [book_path()?, pairing_path()?] {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|error| {
                crate::AppError::Message(format!("remove pairing creds: {error}"))
            })?;
        }
    }
    Ok(())
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
    // The platform hands a desktop it already knows the same `pair_id` for a
    // fresh invitation, and this invitation replaces that binding. A revoke still
    // queued from the unpair that preceded it is therefore obsolete: delivering it
    // later would delete the pairing just minted. Retiring it here — as soon as
    // the new pair id is known, and before returning the invitation — is what
    // keeps the compensation loop from revoking the code a `--re-pair` prints.
    // The lock waits out any revoke already in flight for this binding.
    {
        let _delivery = REVOKE_DELIVERY.lock().await;
        retire_pending_revoke(&response.pair_id, &platform);
    }
    let jwt_expires_at = jwt_expiry(&response.user_jwt)?;
    let code_expires_at = pairing_code_expiry(&response.pairing_code);
    let creds = PairingCreds {
        handshake_version: 2,
        secure: Some(crate::remote::secure::PairingIdentity::new(
            code_expires_at
                .ok_or_else(|| crate::AppError::Message("pairing_expiry_required".into()))?,
        )?),
        pair_id: response.pair_id,
        desktop_id,
        nkey_seed,
        user_jwt: response.user_jwt,
        nats_url: response.nats_url,
        nats_ws_url: response.nats_ws_url,
        jwt_expires_at,
    };
    let identity = creds.secure.as_ref().expect("new secure identity");
    let mut invitation = reqwest::Url::parse("futureos://remote/pair").expect("constant URL");
    invitation
        .query_pairs_mut()
        .append_pair("v", "2")
        .append_pair("code", &response.pairing_code)
        .append_pair("desktopId", &creds.desktop_id)
        .append_pair("desktopKey", &key_pair.public_key())
        .append_pair("secureKey", &identity.public_key)
        .append_pair(
            "secret",
            identity.secret.as_deref().expect("new invitation secret"),
        );
    Ok((creds, invitation.into(), code_expires_at))
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

#[cfg(test)]
pub async fn revoke_pairing(creds: &PairingCreds) -> Result<(), crate::AppError> {
    revoke_pair_id(&creds.pair_id).await
}

async fn revoke_pair_id(pair_id: &str) -> Result<(), crate::AppError> {
    let platform = crate::future_platform::current_platform_url();
    let response = http_client()?
        .post(format!("{platform}/client/v1/remote/pair/revoke"))
        .bearer_auth(crate::future_login::future_api_key()?)
        .json(&json!({ "pair_id": pair_id }))
        .send()
        .await
        .map_err(|error| transport_or_message("revoke remote pairing", error))?;
    if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    Err(response_error(response, "revoke remote pairing").await)
}

/// Compensation contains no connection credential or account token. It can
/// only use the signed-in account against the original platform environment.
pub(crate) fn queue_revoke(creds: &PairingCreds) -> Result<(), crate::AppError> {
    let path = pairing_path()?.with_file_name("remote_pending_revokes.json");
    crate::config_io::with_config_lock(&path, || {
        let mut pending = crate::config_io::read_json_object(&path)?;
        pending[&creds.pair_id] = json!(crate::future_platform::current_platform_url());
        crate::config_io::write_json_atomic(&path, &pending, true)
    })
}

/// Retire a queued revoke for the pair id a new invitation just took over.
///
/// Only the entry for the platform that minted the invitation is retired: a
/// queued revoke for the same pair id on another platform targets a different
/// binding and must still be delivered there. A local storage failure is logged
/// rather than returned — the invitation exists and is usable, and failing the
/// pairing over bookkeeping would cost the user the code they asked for.
fn retire_pending_revoke(pair_id: &str, platform: &str) {
    let result = (|| -> Result<(), crate::AppError> {
        let path = pairing_path()?.with_file_name("remote_pending_revokes.json");
        crate::config_io::with_config_lock(&path, || {
            let mut pending = crate::config_io::read_json_object(&path)?;
            if pending.get(pair_id).and_then(Value::as_str) != Some(platform) {
                return Ok(());
            }
            pending
                .as_object_mut()
                .expect("read_json_object returns an object")
                .remove(pair_id);
            crate::config_io::write_json_atomic(&path, &pending, true)
        })
    })();
    if let Err(error) = result {
        eprintln!(
            "remote: could not retire the queued revoke for the replacement invitation: {error}"
        );
    }
}

pub(crate) async fn retry_pending_revokes() -> Result<(), crate::AppError> {
    // Hold the delivery lock for the whole pass so an invitation is never minted
    // while a revoke for its pair id is on the wire (see `REVOKE_DELIVERY`).
    let _delivery = REVOKE_DELIVERY.lock().await;
    static BLOCKED: std::sync::LazyLock<
        std::sync::Mutex<std::collections::HashSet<(String, String)>>,
    > = std::sync::LazyLock::new(Default::default);
    let path = pairing_path()?.with_file_name("remote_pending_revokes.json");
    let pending = crate::config_io::read_json_object(&path)?;
    let platform = crate::future_platform::current_platform_url();
    for (pair_id, original_platform) in pending.as_object().expect("read_json_object") {
        if original_platform.as_str() != Some(platform.as_str()) {
            continue;
        }
        let key = (pair_id.clone(), platform.clone());
        if BLOCKED.lock().unwrap().contains(&key) {
            continue;
        }
        // Authorization failures need an account change/relaunch, not periodic retries.
        if let Err(error) = revoke_pair_id(pair_id).await {
            if matches!(
                error,
                crate::AppError::Remote {
                    status: 400..=499,
                    ..
                }
            ) && !matches!(
                error,
                crate::AppError::Remote {
                    status: 408 | 429,
                    ..
                }
            ) {
                BLOCKED.lock().unwrap().insert(key);
            }
            continue;
        }
        crate::config_io::with_config_lock(&path, || {
            let mut latest = crate::config_io::read_json_object(&path)?;
            if latest.get(pair_id) == Some(original_platform) {
                latest
                    .as_object_mut()
                    .expect("read_json_object")
                    .remove(pair_id);
            }
            crate::config_io::write_json_atomic(&path, &latest, true)
        })?;
    }
    Ok(())
}

// The four helpers below are `pub(crate)` because the *client* role
// (`remote_peer`) speaks to the same platform API and must fail the same way:
// one HTTP/TLS policy per process, one reading of the platform's
// `{error, message}` body, one JWT-expiry rule. Duplicating them per role is
// how the two would drift into disagreeing about, say, whether a 401 is
// "revoked" or "retry".
pub(crate) fn http_client() -> Result<reqwest::Client, crate::AppError> {
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
pub(crate) fn transport_or_message(action: &str, error: reqwest::Error) -> crate::AppError {
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

pub(crate) async fn parse_response<T: serde::de::DeserializeOwned>(
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

pub(crate) fn jwt_expiry(jwt: &str) -> Result<i64, crate::AppError> {
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
    fn platform_account_rejection_is_not_a_retryable_network_error() {
        for status in [401, 403] {
            let error = crate::AppError::Remote {
                status,
                code: None,
                message: "rejected".into(),
            };
            assert_eq!(error_code(&error), Some("account_authorization"));
            assert!(!is_invalid_or_revoked_error(&error));
        }
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
        // A missing account credential is the account's problem, and the type
        // says so — no message is consulted.
        assert_eq!(
            error_code(&crate::AppError::NotSignedIn),
            Some("account_authorization"),
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
    use super::*;
    use crate::remote::test_support::{
        init_store, jwt, now_secs, pairing_code, sign_in, HomeGuard, MockPlatform,
    };
    use serde_json::json;

    fn fixture_creds() -> PairingCreds {
        let key_pair = nkeys::KeyPair::new_user();
        PairingCreds {
            handshake_version: 1,
            secure: None,
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
        assert!(
            load_creds_checked().is_err(),
            "headless startup must not create a new invitation on corrupt data"
        );
        // A valid JSON file that does not match the creds shape also → None.
        std::fs::write(&path, json!({ "unrelated": true }).to_string()).unwrap();
        assert!(load_creds().is_none());
        assert!(load_creds_checked().is_err());
    }

    #[test]
    fn a_legacy_single_pairing_is_read_as_the_book_s_only_slot() {
        let _home = HomeGuard::new("pairing-legacy-book");
        let legacy = pairing_path().unwrap();
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, serde_json::to_vec(&fixture_creds()).unwrap()).unwrap();

        let creds = load_creds().expect("a legacy credential still loads");
        assert_eq!(creds.pair_id, "pair_test");

        save_creds(&creds).unwrap();
        // The legacy file survives the migration. An older build reads it and
        // finds a pairing; deleting it here would make that build mint a fresh
        // invitation, which revokes the pairing the book holds.
        assert!(legacy.exists(), "migration must not delete the legacy file");
        let book: serde_json::Value =
            serde_json::from_slice(&std::fs::read(book_path().unwrap()).unwrap()).unwrap();
        assert_eq!(book["version"], json!(2));
        assert_eq!(
            book["active"],
            json!("pair_test"),
            "a single device is trivially the one in control"
        );
        assert_eq!(book["slots"][0]["pairId"], json!("pair_test"));
        assert_eq!(load_creds().unwrap().pair_id, "pair_test");
    }

    #[test]
    fn the_book_holds_one_slot_per_pairing_and_keeps_their_order() {
        let _home = HomeGuard::new("pairing-book-slots");
        let first = fixture_creds();
        save_creds(&first).unwrap();
        let mut second = fixture_creds();
        second.pair_id = "pair_second".to_string();
        save_creds(&second).unwrap();
        // A credential refresh of the first device must neither append a
        // duplicate nor move it behind the second.
        let mut renewed = first.clone();
        renewed.jwt_expires_at += 60;
        save_creds(&renewed).unwrap();

        let book = load_book_checked().unwrap();
        assert_eq!(book.slots.len(), 2, "one slot per pair id");
        assert_eq!(book.slots[0].pair_id, "pair_test");
        assert_eq!(book.slots[1].pair_id, "pair_second");
        assert_eq!(book.slots[0].jwt_expires_at, renewed.jwt_expires_at);
        assert_eq!(book.active.as_deref(), Some("pair_test"));
        assert_eq!(load_creds().unwrap().pair_id, "pair_test");
    }

    #[test]
    fn clearing_removes_the_book_and_the_legacy_file() {
        let _home = HomeGuard::new("pairing-clear-all");
        let legacy = pairing_path().unwrap();
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, serde_json::to_vec(&fixture_creds()).unwrap()).unwrap();
        save_creds(&fixture_creds()).unwrap();
        assert!(book_path().unwrap().exists());

        clear_creds().unwrap();
        assert!(!book_path().unwrap().exists());
        assert!(!legacy.exists(), "unpair must not leave a file behind");
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
        let invitation = reqwest::Url::parse(&returned_code).unwrap();
        let fields: std::collections::HashMap<_, _> = invitation.query_pairs().collect();
        assert_eq!(fields["code"], code);
        assert_eq!(fields["v"], "2");
        assert_eq!(
            fields["secureKey"],
            creds.secure.as_ref().unwrap().public_key
        );
        assert_eq!(
            fields["secret"],
            creds.secure.as_ref().unwrap().secret.as_deref().unwrap()
        );
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
        assert!(!requests[0]
            .2
            .contains(creds.secure.as_ref().unwrap().secret.as_deref().unwrap()));
        assert!(!requests[0]
            .2
            .contains(&creds.secure.as_ref().unwrap().private_key));
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
    fn refresh_delay_is_the_time_until_the_lead_before_expiry() {
        let mut creds = fixture_creds();
        // Renewal starts REFRESH_LEAD before expiry (minutes, not the previous
        // 60 seconds): the HTTPS round trip, TLS connect and transport rebuild
        // have to fit inside the old credential's lifetime.
        creds.jwt_expires_at = now_secs() + 3600;
        let delay = refresh_delay(&creds).as_secs();
        let expected = 3600 - REFRESH_LEAD.as_secs();
        assert!(
            delay <= expected && delay + 5 >= expected,
            "delay was {delay}"
        );
        // Already expired → due now, so the renewal loop retries at its fast
        // cadence instead of idling out a tick with the bridge unable to publish.
        creds.jwt_expires_at = now_secs() - 10;
        assert_eq!(refresh_delay(&creds), std::time::Duration::ZERO);
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

    /// A platform that answers a pair-code request whose code carries no
    /// readable expiry is not something the desktop can serve: the invitation it
    /// would mint could never be shown to expire. It must be refused by name
    /// rather than silently minted with an expiry the desktop invented.
    #[tokio::test]
    async fn a_pairing_code_without_an_expiry_is_refused_by_name() {
        let _home = HomeGuard::new("pairing-create-no-expiry");
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        platform.push(
            "/client/v1/remote/pair/code",
            200,
            json!({
                "pair_id": "pair_no_expiry",
                // Not decodable as the `{exp}` payload every client validates.
                "pairing_code": "!!!not-a-pairing-code!!!",
                "user_jwt": jwt(now_secs() + 3600),
                "nats_url": "nats://127.0.0.1:4222",
                "nats_ws_url": "ws://127.0.0.1:4222",
            }),
        );
        let error = create_pairing().await.unwrap_err();
        assert!(
            error.to_string().contains("pairing_expiry_required"),
            "got {error}"
        );
    }

    /// Compensation is retried until the platform accepts it, but only against
    /// the environment that issued the pairing: an entry recorded for another
    /// platform address must be left alone, and an authorization failure (4xx,
    /// except the two retryable statuses) stops the periodic retry instead of
    /// hammering it. A throttled revoke stays pending and is retried.
    #[tokio::test]
    async fn pending_revokes_are_skipped_blocked_or_retried_by_error_class() {
        let _home = HomeGuard::new("pairing-retry-revokes");
        init_store();
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        let current = crate::future_platform::current_platform_url();
        let file = pairing_path()
            .expect("pairing path")
            .with_file_name("remote_pending_revokes.json");
        crate::config_io::write_json_atomic(
            &file,
            &json!({
                // Different platform environment: never revoked here.
                "pair_foreign": "https://api.example.invalid",
                // Authorization rejection: blocked after the first attempt.
                "pair_forbidden": current,
                // Throttled: retried on every pass.
                "pair_throttled": current,
            }),
            true,
        )
        .unwrap();

        let revoked = |requests: &[(String, String, String)]| -> Vec<String> {
            let mut ids: Vec<String> = requests
                .iter()
                .map(|(_, path, body)| {
                    assert_eq!(path, "/client/v1/remote/pair/revoke");
                    serde_json::from_str::<Value>(body).unwrap()["pair_id"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect();
            ids.sort();
            ids
        };

        for pair_id in ["pair_forbidden", "pair_throttled"] {
            platform.push(
                "/client/v1/remote/pair/revoke",
                if pair_id == "pair_forbidden" {
                    400
                } else {
                    429
                },
                json!({ "error": "forbidden", "message": "no" }),
            );
        }
        retry_pending_revokes().await.unwrap();
        assert_eq!(
            revoked(&platform.requests()),
            vec!["pair_forbidden", "pair_throttled"],
            "a foreign-platform entry must not be sent to this platform"
        );

        // Nothing was revoked, so every entry stays pending.
        let pending: Value = crate::config_io::read_json_object(&file).unwrap();
        assert_eq!(pending.as_object().unwrap().len(), 3);

        // Second pass: the 4xx entry is blocked (no new request), the throttled
        // one is retried.
        platform.push(
            "/client/v1/remote/pair/revoke",
            429,
            json!({ "error": "throttled", "message": "slow down" }),
        );
        retry_pending_revokes().await.unwrap();
        let requests = platform.requests();
        assert_eq!(requests.len(), 3, "the blocked entry was not retried");
        assert_eq!(
            revoked(&requests)[2],
            "pair_throttled",
            "the throttled entry must be retried once more"
        );
    }
}

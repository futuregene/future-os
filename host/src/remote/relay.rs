//! The self-hosted relay (host role).
//!
//! Remote v2's transport does not know which relay carries it: the Noise
//! handshake, the `FRE2` records, the subjects and the JSON commands are the
//! same. What differs is where a *pairing* comes from. On the official relay
//! the platform mints a pair id, an invitation code and a short-lived NATS JWT
//! for an account. Here the desktop does all three itself against
//! `future-relay`:
//!
//! * it picks a random pair id and an invitation token,
//! * it registers the pair's *room* with a request signed by its pairing NKey,
//! * it rotates its own NATS credential with the same signature.
//!
//! Nothing about this touches `future-os.cn`, and a machine that also has
//! official pairings keeps serving them: the choice is recorded per pairing in
//! [`PairingCreds::control_url`], not globally. A machine can hold both at
//! once, in separate slots, and they never meet.
//!
//! The relay is not trusted for content. It learns the pair id, the device
//! public keys and the traffic shape; it never sees the Noise static keys, the
//! invitation PSK or a single plaintext byte. See
//! `docs/guide/self-hosted-relay.md`.

use std::path::PathBuf;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use nkeys::KeyPair;
use serde_json::json;
use sha2::{Digest, Sha256};

use future_remote_server::PairingCreds;

/// How long an invitation stays usable. Short on purpose: the QR is a bearer
/// credential, and the relay keeps only its hash.
pub const INVITATION_TTL_SECONDS: i64 = 300;

/// The exact string a relay request signs.
///
/// Mirrors `relay/src/sign.rs::canonical`: method, path, timestamp and the
/// body digest, so a captured signature cannot be moved to another route or
/// another room.
pub fn canonical_request(method: &str, path: &str, timestamp: i64, body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!(
        "futureos-relay-v1\n{}\n{}\n{}\n{}",
        method.to_ascii_uppercase(),
        path,
        timestamp,
        hex
    )
}

/// The relay *this launch* points at, as `--relay` / `--relay-ca` gave it.
///
/// Deliberately not persisted. The flag answers "where do the invitations this
/// process mints go", and a Desktop that writes that answer down turns one
/// debugging launch into a machine that silently stops using the official relay
/// — including every launch the user never typed a flag into. So a launch with
/// no flag means the official relay, always, and leaving a self-hosted relay
/// behind is just not passing the flag again. Existing pairings are unaffected
/// either way: each records the relay that issued it in `controlUrl`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayLaunch {
    /// `Some` means new invitations are minted against a self-hosted relay.
    pub url: Option<String>,
    /// Extra TLS root (PEM) for a relay with a private CA.
    pub ca: Option<String>,
}

/// `None` until a launch applies its flags. A `Mutex` rather than a `OnceLock`
/// because both entry points apply the same parsed flags (the GUI from
/// `launch.rs`, `future headless` from its own parser): identical values must be
/// harmless rather than an "already set" error.
static LAUNCH: std::sync::Mutex<Option<RelayLaunch>> = std::sync::Mutex::new(None);

/// Record this launch's relay flags. Absent flags leave the official relay in
/// place — the same thing an empty `--relay` means.
pub fn set_launch(relay: Option<&str>, relay_ca: Option<&str>) {
    let normalized = |value: Option<&str>| {
        value
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty())
    };
    let launch = RelayLaunch {
        url: normalized(relay),
        ca: normalized(relay_ca),
    };
    *LAUNCH.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(launch);
}

fn launch() -> RelayLaunch {
    LAUNCH
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
        .unwrap_or_default()
}

/// Apply the entry's parsed relay options before reading pairing state.
///
/// Called by both entry points, so the choice means the same thing whether it
/// arrived on the GUI's command line or through `future headless`. Nothing is
/// written down: these flags decide where *this launch* mints its invitations,
/// and a launch that names no relay uses the Future platform for new pairings.
/// Existing pairings keep the relay that issued them either way, which is why an
/// empty `--relay` needs no bookkeeping to undo.
pub fn apply_launch_settings(relay: Option<&str>, relay_ca: Option<&str>) {
    if relay.is_none() && relay_ca.is_none() {
        return;
    }
    set_launch(relay, relay_ca);
}

/// The relay endpoint this launch points at, normalized without a trailing
/// slash.
pub fn endpoint() -> Option<String> {
    // The environment wins over the flags so a test or a container can point at
    // a relay without going through argv; an empty value means "official".
    if let Ok(value) = std::env::var("FUTURE_RELAY_URL") {
        let value = value.trim().trim_end_matches('/').to_string();
        return (!value.is_empty()).then_some(value);
    }
    launch().url
}

/// The extra CA bundle for the relay's TLS, when this launch named one.
pub fn ca_path() -> Option<PathBuf> {
    let from_env = std::env::var("FUTURE_RELAY_CA").ok();
    let value = from_env.or(launch().ca)?;
    let value = value.trim().to_string();
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// Whether a relay is configured. Only the tests ask this: the runtime code
/// always needs the URL itself (`endpoint()`), so a separate predicate would be
/// a second, silently divergent answer.
#[cfg(any(test, feature = "test-support"))]
pub fn enabled() -> bool {
    endpoint().is_some()
}

/// What a relay answers when it hands out a credential.
pub struct Grant {
    pub user_jwt: String,
    pub jwt_expires_at: i64,
    pub nats_url: String,
    pub nats_ws_url: String,
}

/// A random pair id that satisfies both the subject grammar and the Noise
/// prologue's identifier rule.
pub fn new_pair_id() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// The one-time token that lets a phone join this room.
pub fn new_join_token() -> String {
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn hex_sha256(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn signed_headers(
    key: &KeyPair,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<[(&'static str, String); 3], crate::Error> {
    let timestamp = super::unix_timestamp() as i64;
    let signature = key
        .sign(canonical_request(method, path, timestamp, body).as_bytes())
        .map_err(|error| crate::Error::Message(format!("sign relay request: {error}")))?;
    Ok([
        ("x-future-public-key", key.public_key()),
        ("x-future-timestamp", timestamp.to_string()),
        ("x-future-signature", URL_SAFE_NO_PAD.encode(signature)),
    ])
}

fn relay_url(base: &str, path: &str) -> String {
    format!("{}{path}", base.trim_end_matches('/'))
}

/// Register (or refresh) this pairing's room and take a bridge credential.
///
/// `join_token` opens an invitation; `None` only refreshes the credential for a
/// desktop that is already paired, which is the path a restart takes.
pub async fn register(
    base: &str,
    pair_id: &str,
    desktop_id: &str,
    host_name: &str,
    key: &KeyPair,
    join_token: Option<&str>,
) -> Result<Grant, crate::Error> {
    let body = serde_json::to_vec(&json!({
        "pairId": pair_id,
        "hostDeviceId": desktop_id,
        "hostName": host_name,
        "joinHash": join_token.map(hex_sha256),
    }))
    .map_err(|error| crate::Error::Message(format!("encode relay request: {error}")))?;
    let headers = signed_headers(key, "POST", "/v1/rooms", &body)?;
    let mut request = crate::remote_platform::http_client()?
        .post(relay_url(base, "/v1/rooms"))
        .header("content-type", "application/json")
        .body(body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await.map_err(|error| {
        crate::remote_platform::transport_or_message("register relay room", error)
    })?;
    let body: serde_json::Value =
        crate::remote_platform::parse_response(response, "register relay room").await?;
    grant_from(body, "register relay room")
}

/// Rotate the bridge credential.
///
/// A `404` is not a failure to report to the user: it means the relay has no
/// record of this room (a fresh process, a wiped registry). The caller responds
/// by re-registering, which the signature makes possible without any account.
pub async fn refresh(creds: &PairingCreds) -> Result<Grant, crate::Error> {
    let base = creds
        .control_url
        .as_deref()
        .ok_or_else(|| crate::Error::Message("no relay is configured for this pairing".into()))?;
    rotate(base, creds).await
}

async fn rotate(base: &str, creds: &PairingCreds) -> Result<Grant, crate::Error> {
    let key = KeyPair::from_seed(&creds.nkey_seed)
        .map_err(|error| crate::Error::Message(format!("read desktop NKey: {error}")))?;
    rotate_with(base, creds, &key).await
}

async fn rotate_with(
    base: &str,
    creds: &PairingCreds,
    key: &KeyPair,
) -> Result<Grant, crate::Error> {
    let body = serde_json::to_vec(&json!({
        "pair_id": creds.pair_id,
        "device_id": creds.desktop_id,
        "public_key": key.public_key(),
        "role": "bridge",
    }))
    .map_err(|error| crate::Error::Message(format!("encode relay request: {error}")))?;
    let path = "/client/v1/remote/auth/token";
    let headers = signed_headers(key, "POST", path, &body)?;
    let mut request = crate::remote_platform::http_client()?
        .post(relay_url(base, path))
        .header("content-type", "application/json")
        .body(body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await.map_err(|error| {
        crate::remote_platform::transport_or_message("refresh relay credential", error)
    })?;
    let body: serde_json::Value =
        crate::remote_platform::parse_response(response, "refresh relay credential").await?;
    grant_from(body, "refresh relay credential")
}

/// Delete this pairing's room. Best effort: a desktop that unpairs while the
/// relay is unreachable still stops locally, and a room with no bridge
/// connected grants access to a namespace nobody serves.
pub async fn revoke(creds: &PairingCreds) -> Result<(), crate::Error> {
    let Some(base) = creds.control_url.as_deref() else {
        return Ok(());
    };
    let key = KeyPair::from_seed(&creds.nkey_seed)
        .map_err(|error| crate::Error::Message(format!("read desktop NKey: {error}")))?;
    let path = format!("/v1/rooms/{}", creds.pair_id);
    let headers = signed_headers(&key, "DELETE", &path, &[])?;
    let mut request = crate::remote_platform::http_client()?.delete(relay_url(base, &path));
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await.map_err(|error| {
        crate::remote_platform::transport_or_message("revoke relay room", error)
    })?;
    if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    Err(crate::remote_platform::response_error(response, "revoke relay room").await)
}

/// Whether an error means "this relay has no such room".
pub fn is_unknown_room(error: &crate::Error) -> bool {
    matches!(error, crate::Error::Remote { status: 404, .. })
}

fn grant_from(body: serde_json::Value, action: &str) -> Result<Grant, crate::Error> {
    let field = |name: &str| -> Result<String, crate::Error> {
        body.get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| crate::Error::Message(format!("{action}: the relay returned no {name}")))
    };
    let user_jwt = field("userJwt")?;
    let jwt_expires_at = crate::remote_platform::jwt_expiry(&user_jwt)?;
    Ok(Grant {
        user_jwt,
        jwt_expires_at,
        nats_url: field("natsUrl")?,
        nats_ws_url: field("natsWsUrl")?,
    })
}

#[cfg(test)]
mod tests {
    /// The flags decide where *this process* mints invitations and are not
    /// written down, so the next launch — one that names no relay — is back on
    /// the Future platform without anyone remembering to undo anything.
    #[test]
    fn relay_flags_apply_to_this_launch_only() {
        let _home = crate::auth_store::test_support::HomeGuard::new("launch-relay-settings");

        apply_launch_settings(Some("https://relay.example/"), Some("/etc/ca.pem"));
        assert_eq!(
            crate::remote::relay::endpoint().as_deref(),
            Some("https://relay.example")
        );
        assert_eq!(
            crate::remote::relay::ca_path().as_deref(),
            Some(std::path::Path::new("/etc/ca.pem"))
        );
        // Nothing was written: a fresh home has no `relay.json` at all.
        let relay_file = crate::store::future_dir()
            .expect("future dir")
            .join("relay.json");
        assert!(!relay_file.exists(), "{} was written", relay_file.display());

        // An empty `--relay` is this launch saying "the Future platform", which
        // is the same answer as not passing the flag.
        apply_launch_settings(Some(""), None);
        assert!(!crate::remote::relay::enabled());
        assert_eq!(crate::remote::relay::ca_path(), None);

        // A launch with neither flag leaves the official relay in place.
        apply_launch_settings(None, None);
        assert!(!crate::remote::relay::enabled());
    }

    /// A launch that never names a relay speaks for the Future platform, even
    /// when an older build left `relay.json` in the data root: that file is the
    /// trap this rule exists to remove, so it must not come back to life.
    #[test]
    fn a_leftover_relay_file_does_not_choose_the_relay() {
        let _home = crate::auth_store::test_support::HomeGuard::new("relay-file-ignored");
        crate::store::initialize_app_store().expect("store init");
        let path = crate::store::future_dir()
            .expect("future dir")
            .join("relay.json");
        std::fs::write(
            &path,
            r#"{"url":"http://192.168.1.2:8099","ca":"/etc/private-ca.pem"}"#,
        )
        .expect("write legacy file");

        apply_launch_settings(None, None);
        assert!(!crate::remote::relay::enabled());
        assert_eq!(crate::remote::relay::ca_path(), None);
    }

    use super::*;

    #[test]
    fn the_canonical_string_matches_the_relay_side() {
        // Kept byte-identical to `relay/src/sign.rs::canonical`; the two crates
        // cannot share code, so this is the pin.
        let body = br#"{"pairId":"PAIR1"}"#;
        let expected = format!(
            "futureos-relay-v1\nPOST\n/v1/rooms\n1700000000\n{}",
            hex_sha256(std::str::from_utf8(body).unwrap())
        );
        assert_eq!(
            canonical_request("POST", "/v1/rooms", 1_700_000_000, body),
            expected
        );
        assert_eq!(
            canonical_request("post", "/v1/rooms", 1_700_000_000, body),
            expected
        );
    }

    #[test]
    fn a_pair_id_is_subject_and_prologue_safe() {
        let pair = new_pair_id();
        assert_eq!(pair.len(), 32);
        assert!(pair.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(pair, new_pair_id());
        assert!(future_remote_crypto::prologue(&pair, "desktop_test").is_ok());
    }

    #[test]
    fn a_join_token_is_unguessable() {
        let token = new_join_token();
        assert!(token.len() >= 40, "{token}");
        assert_ne!(token, new_join_token());
    }

    #[test]
    fn the_launch_flags_are_the_whole_choice() {
        let _home = crate::remote::test_support::HomeGuard::new("relay-settings");
        set_launch(Some("http://127.0.0.1:8099/"), None);
        assert_eq!(endpoint().as_deref(), Some("http://127.0.0.1:8099"));
        assert!(enabled());
        // An empty `--relay` is "the official relay for this launch".
        set_launch(Some(""), None);
        assert!(!enabled());
    }

    #[test]
    fn a_404_is_an_unknown_room_and_nothing_else_is() {
        let unknown = crate::Error::Remote {
            status: 404,
            code: Some("room_unknown".into()),
            message: "No such room.".into(),
        };
        assert!(is_unknown_room(&unknown));
        let denied = crate::Error::Remote {
            status: 403,
            code: Some("room_owned_by_another_host".into()),
            message: "no".into(),
        };
        assert!(!is_unknown_room(&denied));
        assert!(!is_unknown_room(&crate::Error::Message("x".into())));
    }
}

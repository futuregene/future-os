//! Platform pairing/JWT control plane.
//!
//! The NKey seed never leaves this desktop. The platform receives only the
//! public user key and returns a short-lived, pair-scoped NATS user JWT. The
//! installation-wide device id is owned by `device_identity`; pairing stores a
//! copy because the server credential is bound to that exact identity.

#[cfg(test)]
use crate::remote::pairing::{error_code, is_invalid_or_revoked_error, public_key};
#[cfg(test)]
use crate::remote_platform::is_transport_error;
use crate::remote_platform::{
    http_client, jwt_expiry, parse_response, response_error, transport_or_message,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

pub(crate) use future_remote_server::services::Invitation;
pub(crate) use future_remote_server::PairingCreds;

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
pub(crate) struct PairingBook {
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
    /// Every device, in the order they were first paired.
    pub(crate) fn slots(&self) -> &[PairingCreds] {
        &self.slots
    }

    /// Which device drives this machine, if any.
    pub(crate) fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

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
        let saved_pair_id = creds.pair_id.clone();
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
        crate::config_io::write_json_atomic(&path, &value, true)?;
        // Tell the control rules this device exists, so no command has to read
        // the book to find out.
        crate::remote::note_known_device(&saved_pair_id);
        Ok(())
    })
}

/// Forget every device. This is the unpair / re-pair path, whose published
/// meaning is "this machine has no devices": clearing some of them would leave
/// a pairing the user was told was gone.
pub fn clear_creds() -> Result<(), crate::AppError> {
    for slot in load_book_checked()
        .map(|book| book.slots)
        .unwrap_or_default()
    {
        crate::remote::forget_known_device(&slot.pair_id);
    }
    for path in [book_path()?, pairing_path()?] {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|error| {
                crate::AppError::Message(format!("remove pairing creds: {error}"))
            })?;
        }
    }
    Ok(())
}

/// Every device this desktop has paired.
///
/// The runtime starts one bridge per slot and keeps them all connected; which
/// one drives the machine is [`PairingBook::active`]. Callers that only pair and
/// unpair want [`load_creds`] ("the device in control") instead.
pub fn load_book() -> Option<PairingBook> {
    load_book_checked()
        .ok()
        .filter(|book| !book.slots.is_empty())
}

/// Persist which device drives this machine.
///
/// Written through the same lock and atomic replace as the credentials
/// themselves, because a control pointer that survived a crash but disagreed
/// with the slots would describe a device this machine no longer has.
pub fn set_active(pair_id: &str) -> Result<bool, crate::AppError> {
    let path = book_path()?;
    crate::config_io::with_config_lock(&path, || {
        let mut book = load_book_checked()?;
        if !book.slots.iter().any(|slot| slot.pair_id == pair_id) {
            return Ok(false);
        }
        if book.active.as_deref() == Some(pair_id) {
            return Ok(false);
        }
        book.active = Some(pair_id.to_string());
        let value = serde_json::to_value(&book)
            .map_err(|error| crate::AppError::Message(format!("encode pairing creds: {error}")))?;
        crate::config_io::write_json_atomic(&path, &value, true)?;
        Ok(true)
    })
}

/// Forget one device, leaving the rest paired.
///
/// When the slot being removed is the one in control, the pointer is cleared in
/// the same write. There is no promotion: the remaining devices are other
/// people's connections to this machine, and picking one for them would decide
/// who is sitting in front of the computer.
pub fn remove_slot(pair_id: &str) -> Result<(), crate::AppError> {
    let path = book_path()?;
    let book = crate::config_io::with_config_lock(&path, || {
        let mut book = load_book_checked()?;
        book.slots.retain(|slot| slot.pair_id != pair_id);
        if book.active.as_deref() == Some(pair_id) {
            book.active = None;
        }
        let value = serde_json::to_value(&book)
            .map_err(|error| crate::AppError::Message(format!("encode pairing creds: {error}")))?;
        crate::config_io::write_json_atomic(&path, &value, true).map(|()| book)
    })?;
    // Nothing is left. Both files go: an empty book would still answer "this
    // machine has a credential file", and the legacy file would keep
    // describing a device that was just unpaired.
    crate::remote::forget_known_device(pair_id);
    if book.slots.is_empty() {
        for path in [book_path()?, pairing_path()?] {
            if path.exists() {
                std::fs::remove_file(&path).map_err(|error| {
                    crate::AppError::Message(format!("remove pairing creds: {error}"))
                })?;
            }
        }
    }
    Ok(())
}

/// Publish a fresh invitation.
///
/// `keep_existing` is forwarded verbatim to the platform, where `false` replaces
/// every device this desktop holds and `true` adds one beside them. The wording
/// matters: this function has no opinion, because the platform owns the cap.
///
/// `shown` names the pair ids whose invitation this desktop is displaying. They
/// matter only when the mint is refused for the cap (see
/// [`clear_stale_invitations`]): filling that cap with invitations no phone can
/// claim is this desktop's own doing, and `shown` is what keeps the cure from
/// killing a code someone is holding.
pub async fn create_pairing(
    keep_existing: bool,
    shown: &[String],
) -> Result<Invitation, crate::AppError> {
    // A self-hosted relay replaces the platform in exactly one place: whatever
    // mints the pair id, the invitation and the bridge credential. Everything
    // the invitation carries about the endpoints is the relay's, not ours.
    if let Some(base) = crate::remote::relay::endpoint() {
        let (creds, code, expires_at) = create_pairing_via_relay(&base, keep_existing).await?;
        return Ok(Invitation {
            creds,
            code,
            expires_at,
            cleared: Vec::new(),
        });
    }
    let desktop_id = crate::device_identity::device_id()?;
    let key_pair = nkeys::KeyPair::new_user();
    let nkey_seed = key_pair
        .seed()
        .map_err(|error| crate::AppError::Message(format!("generate desktop NKey: {error}")))?;
    let (response, cleared) = match mint_pair_code(&desktop_id, &key_pair, keep_existing).await {
        Ok(response) => (response, Vec::new()),
        // The cap is the platform's rule and this desktop cannot raise it. What
        // it *can* do is stop spending its own allowance on invitations it lost
        // track of: an `add` whose answer never arrived minted a row no phone
        // has ever seen a code for, and that row keeps the slot for minutes.
        // Clearing those is attempted only now — the refusal already happened,
        // so nothing is revoked while a mint could still have succeeded — and if
        // it frees nothing the platform's own answer is passed through verbatim.
        Err(error) if refused_for_the_cap(&error) => {
            let cleared = clear_stale_invitations(&desktop_id, shown).await;
            if cleared.is_empty() {
                return Err(error);
            }
            (
                mint_pair_code(&desktop_id, &key_pair, keep_existing).await?,
                cleared,
            )
        }
        Err(error) => return Err(error),
    };
    let platform = crate::future_platform::current_platform_url();
    // The platform hands a desktop it already knows the same `pair_id` for a
    // fresh invitation, and this invitation replaces that binding. A revoke still
    // queued from the unpair that preceded it is therefore obsolete: delivering it
    // later would delete the pairing just minted. Retiring it here — as soon as
    // the new pair id is known, and before returning the invitation — is what
    // keeps the compensation loop from revoking the code just handed to the user.
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
        control_url: None,
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
    Ok(Invitation {
        creds,
        code: invitation.into(),
        expires_at: code_expires_at,
        cleared,
    })
}

/// Ask the platform for an invitation code.
///
/// Split out because the cap refusal is answered by retrying this exact call
/// once the allowance has been made room in; the retry must be the same request
/// in every other respect.
async fn mint_pair_code(
    desktop_id: &str,
    key_pair: &nkeys::KeyPair,
    keep_existing: bool,
) -> Result<CreatePairCodeResponse, crate::AppError> {
    let platform = crate::future_platform::current_platform_url();
    let response = http_client()?
        .post(format!("{platform}/client/v1/remote/pair/code"))
        .bearer_auth(crate::future_login::future_api_key()?)
        .json(&json!({
            "desktop_id": desktop_id,
            "desktop_public_key": key_pair.public_key(),
            "desktop_name": "FutureOS GUI",
            "keep_existing": keep_existing,
        }))
        .send()
        .await
        .map_err(|error| transport_or_message("create pairing code", error))?;
    parse_response(response, "create pairing code").await
}

/// Whether the platform refused a mint because this desktop already holds its
/// allowance of devices.
///
/// Only this machine code: another 409 is a different refusal, and revoking
/// invitations to "cure" it would delete codes to solve the wrong problem.
fn refused_for_the_cap(error: &crate::AppError) -> bool {
    matches!(
        error,
        crate::AppError::Remote {
            code: Some(code),
            ..
        } if code == "pairing_limit_reached"
    )
}

/// How long an unclaimed invitation must have existed before this desktop may
/// forget it to make room for a new one.
///
/// Measured rather than assumed: across the nine pairings this account ever
/// claimed, mint → claim ran 2.4 s at the fastest and 18.0 s at the slowest
/// (median 8.5 s). The floor sits an order of magnitude above that slowest
/// case, because getting it wrong deletes a code somebody is holding, while the
/// only cost of being slow is that the sweep frees nothing and the platform's
/// own refusal stands.
const STALE_INVITATION_FLOOR_SECS: i64 = 180;

/// One pairing row the platform holds for this account.
#[derive(Debug, serde::Deserialize)]
struct PlatformDevice {
    pair_id: String,
    #[serde(default)]
    desktop_id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    activated_at: Option<String>,
    #[serde(default)]
    client_device_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct DevicesResponse {
    #[serde(default)]
    devices: Vec<PlatformDevice>,
}

/// Forget the invitations this desktop minted and never heard back about.
///
/// An `add` whose answer was lost — a dropped connection, a Desktop that exited
/// mid-request — still left a row on the platform, and that row counts against
/// the per-desktop cap until its nonce expires even though no phone has ever
/// seen its code. Nothing local records those rows, so the platform's own list
/// is the only place to find them.
///
/// A platform fault is reported and yields "cleared nothing": housekeeping must
/// not fail the pairing that needs the room.
async fn clear_stale_invitations(desktop_id: &str, shown: &[String]) -> Vec<String> {
    let devices = match list_devices().await {
        Ok(devices) => devices,
        Err(error) => {
            eprintln!("remote: could not list invitations to clear: {error}");
            return Vec::new();
        }
    };
    let now = chrono::Utc::now();
    let mut cleared = Vec::new();
    for device in devices {
        if !is_sweepable(&device, desktop_id, shown, now) {
            continue;
        }
        match delete_device(&device.pair_id).await {
            Ok(()) => cleared.push(device.pair_id),
            Err(error) => {
                eprintln!("remote: could not clear a stale invitation: {error}");
            }
        }
    }
    if !cleared.is_empty() {
        eprintln!(
            "remote: cleared {} stale invitation(s) to make room for a device",
            cleared.len()
        );
    }
    cleared
}

async fn list_devices() -> Result<Vec<PlatformDevice>, crate::AppError> {
    let platform = crate::future_platform::current_platform_url();
    let response = http_client()?
        .get(format!("{platform}/client/v1/remote/devices"))
        .bearer_auth(crate::future_login::future_api_key()?)
        .send()
        .await
        .map_err(|error| transport_or_message("list remote devices", error))?;
    let response: DevicesResponse = parse_response(response, "list remote devices").await?;
    Ok(response.devices)
}

/// Revoke one pairing with account permission.
async fn delete_device(pair_id: &str) -> Result<(), crate::AppError> {
    let platform = crate::future_platform::current_platform_url();
    let response = http_client()?
        .delete(format!("{platform}/client/v1/remote/devices/{pair_id}"))
        .bearer_auth(crate::future_login::future_api_key()?)
        .send()
        .await
        .map_err(|error| transport_or_message("clear a stale invitation", error))?;
    if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    Err(response_error(response, "clear a stale invitation").await)
}

/// Whether `device` is an invitation this desktop may forget.
///
/// Every clause is a refusal, and each answers a way this goes wrong: another
/// desktop's rows are not this desktop's to delete and are usually in use; a
/// claimed one is somebody's device, not an invitation; a `shown` code may be
/// under a phone right now; and an invitation younger than the measured scan
/// ceiling is not stale whatever else it says.
fn is_sweepable(
    device: &PlatformDevice,
    desktop_id: &str,
    shown: &[String],
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if device.desktop_id != desktop_id
        || device.status != "pending"
        || device.activated_at.is_some()
        || device.client_device_id.is_some()
        || shown.iter().any(|pair| pair == &device.pair_id)
    {
        return false;
    }
    let Some(created_at) = device.created_at.as_deref() else {
        return false;
    };
    // A timestamp that cannot be read is left alone: guessing "old enough" is
    // how a live code gets revoked.
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|created| {
            (now - created.with_timezone(&chrono::Utc)).num_seconds() >= STALE_INVITATION_FLOOR_SECS
        })
        .unwrap_or(false)
}

/// Mint an invitation against a self-hosted relay.
///
/// `keep_existing` has no meaning here: the official platform owns a cap and a
/// row per desktop, while a relay room *is* the pairing. Every invitation opens
/// a new room, and the old ones keep serving until they are unpaired.
async fn create_pairing_via_relay(
    base: &str,
    _keep_existing: bool,
) -> Result<(PairingCreds, String, Option<i64>), crate::AppError> {
    let key_pair = nkeys::KeyPair::new_user();
    let nkey_seed = key_pair
        .seed()
        .map_err(|error| crate::AppError::Message(format!("generate desktop NKey: {error}")))?;
    let desktop_id = crate::device_identity::device_id()?;
    let pair_id = crate::remote::relay::new_pair_id();
    let join = crate::remote::relay::new_join_token();
    let grant = crate::remote::relay::register(
        base,
        &pair_id,
        &desktop_id,
        HOST_NAME,
        &key_pair,
        Some(&join),
    )
    .await?;
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
        + crate::remote::relay::INVITATION_TTL_SECONDS;
    let creds = PairingCreds {
        handshake_version: 2,
        secure: Some(crate::remote::secure::PairingIdentity::new(expires_at)?),
        pair_id,
        desktop_id,
        nkey_seed,
        user_jwt: grant.user_jwt,
        nats_url: grant.nats_url,
        nats_ws_url: grant.nats_ws_url,
        jwt_expires_at: grant.jwt_expires_at,
        control_url: Some(base.trim_end_matches('/').to_string()),
    };
    let identity = creds.secure.as_ref().expect("new secure identity");
    // The payload keeps the official v2 *shape* (`nonce`, `claim_url`, `exp`)
    // so the phone's claim path is unchanged; `relay` is the addition that
    // tells the client this claim endpoint is not the Future platform. The URL
    // version is `3` because a v2 reader must refuse to send its join token to
    // a host its allowlist does not know.
    let code = serde_json::to_vec(&json!({
        "v": 2,
        "nonce": join,
        "claim_url": format!("{}/client/v1/remote/pair/claim", base.trim_end_matches('/')),
        "exp": expires_at,
        "relay": relay_origin(base)?,
    }))
    .map_err(|error| crate::AppError::Message(format!("encode relay invitation: {error}")))?;
    let mut invitation = reqwest::Url::parse("futureos://remote/pair").expect("constant URL");
    invitation
        .query_pairs_mut()
        .append_pair("v", "3")
        .append_pair("code", &URL_SAFE_NO_PAD.encode(code))
        .append_pair("desktopId", &creds.desktop_id)
        .append_pair("desktopKey", &key_pair.public_key())
        .append_pair("secureKey", &identity.public_key)
        .append_pair(
            "secret",
            identity.secret.as_deref().expect("new invitation secret"),
        );
    Ok((creds, invitation.into(), Some(expires_at)))
}

/// The friend name every self-hosted invitation declares itself with. The
/// platform's room label, not a security decision: the QR is the credential.
const HOST_NAME: &str = "FutureOS";

/// `scheme://host[:port]` — what a client compares its claim URL against when
/// the invitation says "this is a relay, not the Future platform".
fn relay_origin(base: &str) -> Result<String, crate::AppError> {
    let url = reqwest::Url::parse(base)
        .map_err(|error| crate::AppError::Message(format!("invalid relay URL: {error}")))?;
    let host = url
        .host_str()
        .ok_or_else(|| crate::AppError::Message("relay URL has no host".into()))?;
    Ok(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
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
    if creds.control_url.is_some() {
        return refresh_relay_jwt(creds).await;
    }
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

/// Rotate a credential at the relay that issued it.
///
/// A relay keeps its rooms in a file, so it usually remembers this pairing. A
/// `404` still means "no such room" (a wiped registry, a replaced relay), and
/// the right answer is not an error the user sees: the desktop owns the
/// pairing, so it re-registers the same room from its own credentials and
/// tries once. That is what makes the relay disposable.
async fn refresh_relay_jwt(mut creds: PairingCreds) -> Result<PairingCreds, crate::AppError> {
    let base = creds.control_url.clone().expect("checked by the caller");
    let key_pair = nkeys::KeyPair::from_seed(&creds.nkey_seed)
        .map_err(|error| crate::AppError::Message(format!("read desktop NKey: {error}")))?;
    let grant = match crate::remote::relay::refresh(&creds).await {
        Ok(grant) => grant,
        Err(error) if crate::remote::relay::is_unknown_room(&error) => {
            crate::remote::relay::register(
                &base,
                &creds.pair_id,
                &creds.desktop_id,
                HOST_NAME,
                &key_pair,
                None,
            )
            .await?
        }
        Err(error) => return Err(error),
    };
    creds.user_jwt = grant.user_jwt;
    creds.jwt_expires_at = grant.jwt_expires_at;
    creds.nats_url = grant.nats_url;
    creds.nats_ws_url = grant.nats_ws_url;
    Ok(creds)
}

#[cfg(test)]
pub async fn revoke_pairing(creds: &PairingCreds) -> Result<(), crate::AppError> {
    if creds.control_url.is_some() {
        return crate::remote::relay::revoke(creds).await;
    }
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
    if creds.control_url.is_some() {
        // There is no account to authenticate a queued retry with, so the room
        // is deleted now with a signed request, detached so an unreachable
        // relay cannot make "unpair" hang. A room that survives is a namespace
        // with no bridge connected: useless without the desktop, and pruned
        // when it expires.
        let creds = creds.clone();
        crate::runtime::spawn(async move {
            if let Err(error) = crate::remote::relay::revoke(&creds).await {
                eprintln!("remote: could not delete the relay room: {error}");
            }
        });
        return Ok(());
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A book written before the `version` field existed (or by any writer that
    /// omitted it) must decode as the current version rather than being refused:
    /// the field exists to *detect* an old file, and `serde(default)` is what
    /// turns "absent" into a number. The supplier is the only thing standing
    /// between an older book and a hard parse error, so it is pinned directly.
    #[test]
    fn a_book_without_a_version_decodes_as_the_current_one() {
        let book: PairingBook = serde_json::from_value(serde_json::json!({
            "active": null,
            "slots": [],
        }))
        .expect("a book without a version decodes");
        assert!(book.slots().is_empty());
        assert_eq!(book.active(), None);
        assert_eq!(
            serde_json::to_value(&book).expect("serializes")["version"],
            serde_json::json!(BOOK_VERSION),
            "the default is the constant the writer uses"
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
            control_url: None,
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

        let Invitation {
            creds,
            code: returned_code,
            expires_at: code_expires_at,
            cleared,
        } = create_pairing(false, &[]).await.unwrap();
        assert!(cleared.is_empty(), "an ordinary mint clears nothing");
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

        let Invitation { creds, .. } = create_pairing(false, &[]).await.unwrap();
        assert_eq!(creds.desktop_id, "desktop_test");
    }

    /// Adding a device must reach the platform as "keep the ones I have". The
    /// default is the opposite, and getting it wrong would silently cut every
    /// already-paired device off the moment a second one is added.
    #[tokio::test]
    async fn adding_a_device_asks_the_platform_to_keep_the_existing_ones() {
        let _home = HomeGuard::new("pairing-keep-existing");
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        platform.respond_pair_code("nats://127.0.0.1:4222");
        platform.respond_pair_code("nats://127.0.0.1:4222");
        create_pairing(true, &[]).await.unwrap();
        create_pairing(false, &[]).await.unwrap();

        let bodies: Vec<Value> = platform
            .requests()
            .iter()
            .filter(|(_, path, _)| path.ends_with("/client/v1/remote/pair/code"))
            .map(|(_, _, body)| serde_json::from_str(body).unwrap())
            .collect();
        assert_eq!(bodies.len(), 2);
        assert_eq!(
            bodies[0]["keep_existing"],
            json!(true),
            "adding a device keeps the paired ones"
        );
        assert_eq!(
            bodies[1]["keep_existing"],
            json!(false),
            "re-pairing replaces the machine's devices"
        );
    }

    /// One row as `GET /client/v1/remote/devices` returns it.
    fn device_row(
        pair_id: &str,
        desktop_id: &str,
        status: &str,
        age_secs: i64,
        claimed: bool,
    ) -> Value {
        let created = chrono::Utc::now() - chrono::Duration::seconds(age_secs);
        let stamp = created.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        json!({
            "pair_id": pair_id,
            "desktop_id": desktop_id,
            "status": status,
            "created_at": stamp,
            "activated_at": claimed.then(|| stamp.clone()),
            "client_device_id": claimed.then(|| "dev_claimed".to_string()),
        })
    }

    /// Script the per-desktop cap refusal the platform answers with.
    fn refuse_for_the_cap(platform: &MockPlatform) {
        platform.push(
            "/client/v1/remote/pair/code",
            409,
            json!({
                "error": "pairing_limit_reached",
                "message": "This desktop already holds 5 devices. Unpair one of them before adding another.",
            }),
        );
    }

    /// The cap counts invitations no phone ever claimed, and an `add` whose
    /// answer was lost leaves one behind. When the platform refuses for that
    /// reason the desktop clears its own abandoned invitations — and only
    /// those — then asks again.
    #[tokio::test]
    async fn a_full_desktop_clears_its_own_unclaimed_invitations_and_mints() {
        let _home = HomeGuard::new("pairing-sweep-mint");
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        let desktop = crate::device_identity::device_id().unwrap();
        refuse_for_the_cap(&platform);
        platform.respond_pair_code("nats://127.0.0.1:4222");
        platform.push(
            "/client/v1/remote/devices",
            200,
            json!({ "devices": [
                // An `add` whose answer never arrived: nothing can claim these.
                device_row("pair_stale_a", &desktop, "pending", 600, false),
                device_row("pair_stale_b", &desktop, "pending", 900, false),
                // Younger than any scan could take — a phone may be reading it.
                device_row("pair_fresh", &desktop, "pending", 30, false),
                // Somebody's device, not an invitation.
                device_row("pair_claimed", &desktop, "active", 600, true),
                // Another machine's row, none of this desktop's business.
                device_row("pair_elsewhere", "desktop_other", "pending", 600, false),
            ]}),
        );
        for pair in ["pair_stale_a", "pair_stale_b"] {
            platform.push(
                &format!("/client/v1/remote/devices/{pair}"),
                200,
                json!({ "revoked": true }),
            );
        }

        let invitation = create_pairing(true, &[]).await.unwrap();
        assert_eq!(
            invitation.cleared,
            vec!["pair_stale_a".to_string(), "pair_stale_b".to_string()],
            "only the abandoned invitations go"
        );
        let link = reqwest::Url::parse(&invitation.code).unwrap();
        let fields: std::collections::HashMap<_, _> = link.query_pairs().collect();
        assert_eq!(fields["v"], "2");

        let requests = platform.requests();
        assert_eq!(
            requests
                .iter()
                .filter(|(_, path, _)| path.ends_with("/pair/code"))
                .count(),
            2,
            "the mint is asked again once the room is made"
        );
        assert_eq!(
            requests[1].1, "/client/v1/remote/devices",
            "the list is read after the refusal, not before"
        );
        let deleted: Vec<&str> = requests
            .iter()
            .filter(|(method, _, _)| method == "DELETE")
            .map(|(_, path, _)| path.as_str())
            .collect();
        assert_eq!(
            deleted,
            vec![
                "/client/v1/remote/devices/pair_stale_a",
                "/client/v1/remote/devices/pair_stale_b",
            ],
            "a fresh invitation, a claimed device and another desktop all survive"
        );
    }

    /// The code this desktop is showing may be under a phone right now. Making
    /// room must never take it, however old it is.
    #[tokio::test]
    async fn a_full_desktop_never_clears_the_invitation_it_is_showing() {
        let _home = HomeGuard::new("pairing-sweep-shown");
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        let desktop = crate::device_identity::device_id().unwrap();
        refuse_for_the_cap(&platform);
        platform.respond_pair_code("nats://127.0.0.1:4222");
        platform.push(
            "/client/v1/remote/devices",
            200,
            json!({ "devices": [
                // The oldest row here, and still on screen: past the age floor,
                // so only the explicit exclusion can save it.
                device_row("pair_on_screen", &desktop, "pending", 240, false),
                device_row("pair_abandoned", &desktop, "pending", 300, false),
            ]}),
        );
        platform.push(
            "/client/v1/remote/devices/pair_abandoned",
            200,
            json!({ "revoked": true }),
        );

        let invitation = create_pairing(true, &["pair_on_screen".to_string()])
            .await
            .unwrap();
        assert_eq!(invitation.cleared, vec!["pair_abandoned".to_string()]);
        assert!(
            !platform
                .requests()
                .iter()
                .any(|(_, path, _)| path.ends_with("/pair_on_screen")),
            "the invitation on screen must survive the sweep"
        );
    }

    /// With nothing to clear, the platform's refusal is the answer — machine
    /// code and wording untouched, because that is what the UI classifies on
    /// and what sends the user to unpair a device.
    #[tokio::test]
    async fn a_refusal_with_nothing_to_clear_passes_through_unchanged() {
        let _home = HomeGuard::new("pairing-sweep-empty");
        let platform = MockPlatform::start().await;
        sign_in(platform.url());
        refuse_for_the_cap(&platform);
        platform.respond_pair_code("nats://127.0.0.1:4222");
        platform.push("/client/v1/remote/devices", 200, json!({ "devices": [] }));

        let error = create_pairing(true, &[]).await.map(|_| ()).unwrap_err();
        match error {
            crate::AppError::Remote {
                status,
                code,
                message,
            } => {
                assert_eq!(status, 409);
                assert_eq!(code.as_deref(), Some("pairing_limit_reached"));
                assert!(message.contains("Unpair one of them"), "{message}");
            }
            other => panic!("expected the platform refusal, got {other:?}"),
        }
        assert_eq!(
            platform
                .requests()
                .iter()
                .filter(|(_, path, _)| path.ends_with("/pair/code"))
                .count(),
            1,
            "nothing was freed, so the mint is not retried"
        );
    }

    #[tokio::test]
    async fn create_pairing_maps_network_and_server_failures() {
        let _home = HomeGuard::new("pairing-create-errors");
        // Nothing listening → transport failure.
        sign_in("http://127.0.0.1:9");
        let error = create_pairing(false, &[]).await.map(|_| ()).unwrap_err();
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
        let error = create_pairing(false, &[]).await.map(|_| ()).unwrap_err();
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
        let error = create_pairing(false, &[]).await.map(|_| ()).unwrap_err();
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
        let error = create_pairing(false, &[]).await.map(|_| ()).unwrap_err();
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
        let (status, code, message) =
            remote_error_parts(create_pairing(false, &[]).await.map(|_| ()).unwrap_err());
        assert_eq!(status, 500);
        assert!(code.is_none());
        assert!(message.contains("HTTP 500"));

        // Whitespace-only fields are filtered out.
        platform.push(
            "/client/v1/remote/pair/code",
            500,
            json!({ "error": "  ", "message": " " }),
        );
        let (status, code, message) =
            remote_error_parts(create_pairing(false, &[]).await.map(|_| ()).unwrap_err());
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
        let error = create_pairing(false, &[]).await.map(|_| ()).unwrap_err();
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

//! The relay's HTTP API.
//!
//! Two families of routes, deliberately different:
//!
//! * **Pairing** (`/client/v1/remote/...`) speaks the *official platform's*
//!   JSON shapes and paths. That is the whole point: the phone and the
//!   desktop's client role already know how to claim an invitation and rotate a
//!   credential, so going through a self-hosted relay changes where the request
//!   goes, not what it says. Only the *host* half — creating a pairing — is new,
//!   because on the official relay that half is the account API.
//! * **Rooms** (`/v1/rooms...`) are the host's half: create/refresh the pairing
//!   namespace and delete it. Both require a signature by the pairing's NKey,
//!   which is what lets a desktop re-register after a relay restart without
//!   holding any long-lived bearer token.
//!
//! Subject allow lists are minted here and enforced by the broker; see
//! [`crate::jwt`].

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use nkeys::KeyPair;
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::relay::jwt::{self, Role};
use crate::relay::room::{hash_join, is_sha256_hex, JoinOutcome, RoomError, RoomStore};
use crate::relay::sign;
use crate::relay::RelayConfig;

pub struct Relay {
    pub config: RelayConfig,
    account: KeyPair,
    pub rooms: Arc<RoomStore>,
}

impl Relay {
    /// `rooms_path` is where the registry is persisted. `None` keeps it in
    /// memory (tests): a served relay always names a file, because losing the
    /// registry means every phone has to re-pair.
    pub fn new(config: RelayConfig, rooms_path: Option<&std::path::Path>) -> anyhow::Result<Self> {
        let account = KeyPair::from_seed(config.account_seed.trim())
            .map_err(|error| anyhow::anyhow!("account_seed is not a valid NKey seed: {error}"))?;
        if account.key_pair_type() != nkeys::KeyPairType::Account {
            anyhow::bail!("account_seed must be an account seed (`SA…`), not a user seed");
        }
        let rooms = match rooms_path {
            Some(path) => RoomStore::open(path)?,
            None => RoomStore::new(),
        };
        Ok(Self {
            config,
            account,
            rooms: Arc::new(rooms),
        })
    }

    pub fn account_public_key(&self) -> String {
        self.account.public_key()
    }

    /// A handle the periodic purge can hold without borrowing the relay.
    pub fn rooms_probe(&self) -> Arc<RoomStore> {
        self.rooms.clone()
    }

    fn issue(
        &self,
        pair_id: &str,
        device_id: &str,
        role: Role,
        user_public_key: &str,
        name: &str,
    ) -> (String, i64) {
        let now = now();
        let exp = now + self.config.jwt_ttl_seconds;
        let token = jwt::sign_user(
            &self.account,
            user_public_key,
            name,
            jwt::permissions(pair_id, device_id, role),
            now,
            exp,
            self.config.subs_limit,
            self.config.payload_limit,
        )
        .expect("signing claims cannot fail for a parsed account key");
        (token, exp)
    }

    /// The platform's `claim` response shape. `pair_id` is upper-case hex and
    /// `user_jwt` is a NATS user JWT, exactly as the official service produces
    /// them, so neither client has to care which relay answered.
    fn claim_response(
        &self,
        outcome: &JoinOutcome,
        public_key: &str,
        name: &str,
        refresh: &str,
    ) -> Value {
        let (user_jwt, jwt_expires_at) = self.issue(
            &outcome.pair_id,
            &outcome.device_id,
            Role::Client,
            public_key,
            name,
        );
        json!({
            "pair_id": outcome.pair_id,
            "user_jwt": user_jwt,
            "refresh_token": refresh,
            "nats_url": self.config.nats_url,
            "nats_ws_url": self.config.nats_ws_url,
            "jwt_expires_at": jwt_expires_at,
        })
    }
}

pub fn router(relay: Arc<Relay>) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/rooms", post(register_room))
        .route("/v1/rooms/{pair}", get(describe_room).delete(revoke_room))
        .route("/client/v1/remote/pair/claim", post(claim))
        .route("/client/v1/remote/auth/token", post(rotate_token))
        .route("/client/v1/remote/pair/revoke", post(client_revoke))
        .with_state(relay)
}

async fn health(State(relay): State<Arc<Relay>>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "rooms": relay.rooms.len(),
        "account": relay.account_public_key(),
    }))
}

// ── Host half ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegisterRequest {
    pair_id: String,
    host_device_id: String,
    #[serde(default)]
    host_name: String,
    /// Hex SHA-256 of the invitation token. Absent when the desktop is already
    /// paired and only needs a credential.
    #[serde(default)]
    join_hash: Option<String>,
}

async fn register_room(
    State(relay): State<Arc<Relay>>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let caller = match verify(&headers, "POST", uri.path(), &body) {
        Ok(key) => key,
        Err(response) => return response,
    };
    let request: RegisterRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return error(400, "invalid_request", "Malformed room request."),
    };
    if request
        .join_hash
        .as_deref()
        .is_some_and(|hash| !is_sha256_hex(hash))
    {
        return error(
            400,
            "invalid_request",
            "joinHash must be 64 hex characters.",
        );
    }
    let now = now();
    match relay.rooms.register(
        &request.pair_id,
        &caller,
        &request.host_device_id,
        &request.host_name,
        request.join_hash.as_deref(),
        now,
        relay.config.invitation_ttl_seconds,
        relay.config.room_ttl_seconds,
    ) {
        Ok(_) => {
            let (user_jwt, jwt_expires_at) = relay.issue(
                &request.pair_id,
                &request.host_device_id,
                Role::Bridge,
                &caller,
                &format!("bridge:{}", display_host(&request.host_name)),
            );
            (
                StatusCode::OK,
                Json(json!({
                    "pairId": request.pair_id,
                    "userJwt": user_jwt,
                    "jwtExpiresAt": jwt_expires_at,
                    "natsUrl": relay.config.nats_url,
                    "natsWsUrl": relay.config.nats_ws_url,
                })),
            )
        }
        Err(failure) => room_error(failure),
    }
}

/// Who is in this room. Host-signed: the roster is the desktop's business, and
/// an unauthenticated read would let anyone enumerate pairings.
async fn describe_room(
    State(relay): State<Arc<Relay>>,
    Path(pair_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> (StatusCode, Json<Value>) {
    let caller = match verify(&headers, "GET", uri.path(), &[]) {
        Ok(key) => key,
        Err(response) => return response,
    };
    let Some(room) = relay.rooms.describe(&pair_id, now()) else {
        return room_error(RoomError::Unknown);
    };
    if room.host_public_key != caller {
        return room_error(RoomError::HostMismatch);
    }
    match serde_json::to_value(&room) {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(_) => error(500, "internal", "Could not describe the room."),
    }
}

async fn revoke_room(
    State(relay): State<Arc<Relay>>,
    Path(pair_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let caller = match verify(&headers, "DELETE", uri.path(), &body) {
        Ok(key) => key,
        Err(response) => return response,
    };
    match relay.rooms.revoke(&pair_id, &caller) {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(failure) => room_error(failure),
    }
}

// ── Pairing half, in the shape the shipping clients already speak ────────────

#[derive(Deserialize)]
struct ClaimRequest {
    #[serde(default)]
    nonce: String,
    #[serde(default)]
    device_id: String,
    #[serde(default)]
    device_public_key: String,
    #[serde(default)]
    device_name: String,
}

async fn claim(State(relay): State<Arc<Relay>>, body: Bytes) -> (StatusCode, Json<Value>) {
    let request: ClaimRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return error(400, "invalid_request", "Malformed claim request."),
    };
    if !request.device_public_key.starts_with('U') {
        return error(
            400,
            "invalid_request",
            "device_public_key must be an NKey user key.",
        );
    }
    let Some(pair_id) = relay.rooms.room_for_join(&request.nonce) else {
        // One answer for "no such room" and "not that token": a claim endpoint
        // that distinguishes them is an oracle for other people's pairings.
        return error(403, "pairing_code_invalid", "This invitation is not valid.");
    };
    let name = if request.device_name.trim().is_empty() {
        "FutureOS device".to_string()
    } else {
        request.device_name.clone()
    };
    let refresh = refresh_token();
    let now = now();
    match relay.rooms.join(
        &pair_id,
        &request.nonce,
        &request.device_id,
        &request.device_public_key,
        &name,
        &hash_join(&refresh),
        now,
        relay.config.room_ttl_seconds,
    ) {
        Ok(outcome) => (
            StatusCode::OK,
            Json(relay.claim_response(
                &outcome,
                &request.device_public_key,
                &format!("client:{}", display_host(&name)),
                &refresh,
            )),
        ),
        Err(failure) => room_error(failure),
    }
}

#[derive(Deserialize)]
struct TokenRequest {
    #[serde(default)]
    pair_id: String,
    #[serde(default)]
    device_id: String,
    #[serde(default)]
    public_key: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    refresh_token: String,
}

async fn rotate_token(
    State(relay): State<Arc<Relay>>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
    let request: TokenRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return error(400, "invalid_request", "Malformed token request."),
    };
    let Some(role) = Role::parse(request.role.trim()) else {
        return error(400, "invalid_request", "role must be `bridge` or `client`.");
    };
    let now = now();
    match role {
        Role::Bridge => {
            // The desktop host proves itself with its pairing NKey; there is no
            // account to present and no bearer token to steal.
            let caller = match verify(&headers, "POST", uri.path(), &body) {
                Ok(key) => key,
                Err(response) => return response,
            };
            if caller != request.public_key {
                return error(
                    403,
                    "signature_invalid",
                    "The signature does not match the credential.",
                );
            }
            match relay.rooms.authorize(
                &request.pair_id,
                &caller,
                Role::Bridge,
                now,
                relay.config.room_ttl_seconds,
            ) {
                Ok(device_id) => {
                    let (user_jwt, jwt_expires_at) = relay.issue(
                        &request.pair_id,
                        &device_id,
                        Role::Bridge,
                        &caller,
                        "bridge:relay",
                    );
                    credential_response(&relay, user_jwt, jwt_expires_at)
                }
                Err(failure) => room_error(failure),
            }
        }
        Role::Client => match relay.rooms.authorize_refresh(
            &request.pair_id,
            &request.device_id,
            &request.public_key,
            &request.refresh_token,
            now,
            relay.config.room_ttl_seconds,
        ) {
            Ok(outcome) => {
                let (user_jwt, jwt_expires_at) = relay.issue(
                    &outcome.pair_id,
                    &outcome.device_id,
                    Role::Client,
                    &request.public_key,
                    "client:relay",
                );
                credential_response(&relay, user_jwt, jwt_expires_at)
            }
            Err(failure) => room_error(failure),
        },
    }
}

#[derive(Deserialize)]
struct ClientRevokeRequest {
    #[serde(default)]
    pair_id: String,
    #[serde(default)]
    device_id: String,
    #[serde(default)]
    public_key: String,
    #[serde(default)]
    refresh_token: String,
}

async fn client_revoke(State(relay): State<Arc<Relay>>, body: Bytes) -> (StatusCode, Json<Value>) {
    let request: ClientRevokeRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return error(400, "invalid_request", "Malformed revoke request."),
    };
    match relay.rooms.authorize_refresh(
        &request.pair_id,
        &request.device_id,
        &request.public_key,
        &request.refresh_token,
        now(),
        relay.config.room_ttl_seconds,
    ) {
        Ok(outcome) => {
            relay
                .rooms
                .forget_device(&request.pair_id, &outcome.device_id);
            (StatusCode::OK, Json(json!({ "ok": true })))
        }
        // A credential that is already gone is the outcome the caller wanted.
        Err(RoomError::DeviceUnknown | RoomError::RefreshInvalid | RoomError::Unknown) => {
            (StatusCode::OK, Json(json!({ "ok": true })))
        }
        Err(failure) => room_error(failure),
    }
}

// ── Shared ───────────────────────────────────────────────────────────────────

fn credential_response(
    relay: &Relay,
    user_jwt: String,
    jwt_expires_at: i64,
) -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(json!({
            "user_jwt": user_jwt,
            "jwt_expires_at": jwt_expires_at,
            "nats_url": relay.config.nats_url,
            "nats_ws_url": relay.config.nats_ws_url,
        })),
    )
}

fn verify(
    headers: &HeaderMap,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<String, (StatusCode, Json<Value>)> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    sign::verify(
        method,
        path,
        body,
        header("x-future-public-key").as_deref(),
        header("x-future-timestamp").as_deref(),
        header("x-future-signature").as_deref(),
        now(),
    )
    .map_err(|failure| {
        // "Not signed by its caller" is untrue for the one failure that is about
        // *time*: the signature is the caller's, and the two machines disagree
        // about the clock. The skew is in the header this request carried, so the
        // answer can name it instead of sending the reader to look for a forged
        // request that does not exist.
        let message = match failure {
            sign::VerifyError::Stale => clock_skew_message(header("x-future-timestamp")),
            _ => "This request is not signed by its caller.".to_string(),
        };
        error(401, failure.code(), &message)
    })
}

/// Why a correctly signed request was refused, in the terms the reader can fix.
///
/// The relay compares the request's timestamp with its own clock
/// (`sign::CLOCK_SKEW_SECONDS`), which is right — a signature that never expires
/// is a signature that can be replayed — but the advice it used to give
/// ("not signed by its caller") pointed at the wrong problem entirely. A machine
/// whose clock is minutes off cannot reach the relay at all, and the fix is to
/// correct its clock, not to re-pair.
fn clock_skew_message(timestamp: Option<String>) -> String {
    let limit = sign::CLOCK_SKEW_SECONDS;
    match timestamp
        .and_then(|value| value.trim().parse::<i64>().ok())
        .map(|stamp| now() - stamp)
    {
        Some(skew) if skew > 0 => format!(
            "This request's timestamp is {skew}s behind the relay's clock (the limit is {limit}s). \
             The two machines disagree about the current time — check that both are set from a \
             time service, then retry."
        ),
        Some(skew) => format!(
            "This request's timestamp is {}s ahead of the relay's clock (the limit is {limit}s). \
             The two machines disagree about the current time — check that both are set from a \
             time service, then retry.",
            -skew
        ),
        None => format!(
            "This request's timestamp is more than {limit}s away from the relay's clock, or is not \
             a Unix time. The two machines disagree about the current time — check that both are \
             set from a time service, then retry."
        ),
    }
}

fn room_error(failure: RoomError) -> (StatusCode, Json<Value>) {
    let status = StatusCode::from_u16(failure.status()).unwrap_or(StatusCode::BAD_REQUEST);
    let message = match failure {
        RoomError::Unknown => "No such room.",
        RoomError::Expired => "This room has expired.",
        RoomError::InvitationExpired => {
            "This invitation has expired. Create a new one on the desktop."
        }
        RoomError::HostMismatch => "This room belongs to another desktop.",
        RoomError::JoinInvalid => "This invitation is not valid for this room.",
        RoomError::JoinUsed => "This invitation has already been used.",
        RoomError::DeviceUnknown => "This device is not paired with that room.",
        RoomError::RefreshInvalid => "This device credential is no longer valid.",
        RoomError::BadIdentifier => "One of the identifiers is not usable.",
    };
    error(status.as_u16(), failure.code(), message)
}

fn error(status: u16, code: &str, message: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
        Json(json!({ "error": code, "message": message })),
    )
}

/// A NATS subject cannot carry arbitrary text, and `hostName`/`deviceName` are
/// free-form labels people type. They only ever appear in the JWT's `name`
/// claim (not in a subject), but keeping them printable keeps logs honest.
fn display_host(name: &str) -> String {
    let trimmed: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect::<String>()
        .trim()
        .to_string();
    if trimmed.is_empty() {
        "FutureOS".to_string()
    } else {
        trimmed
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn random_token(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// The invitation token: a bearer credential the desktop mints and only ever
/// stores as a hash. See `room::hash_join`.
pub fn join_token() -> String {
    random_token("join_")
}

/// The long-lived half of a client credential, mirroring the platform's
/// `refresh_token`. Only its hash is stored.
pub fn refresh_token() -> String {
    random_token("rt_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unguessable_and_distinct() {
        let join = join_token();
        assert!(join.len() >= 40, "{join}");
        assert_ne!(join, join_token());
        let refresh = refresh_token();
        assert!(refresh.starts_with("rt_"));
        assert_ne!(refresh, refresh_token());
        // Both are subject-safe and header-safe.
        assert!(join.bytes().all(|byte| byte.is_ascii_graphic()));
        assert!(refresh.bytes().all(|byte| byte.is_ascii_graphic()));
    }

    #[test]
    fn a_display_name_cannot_smuggle_control_characters_into_a_claim() {
        assert_eq!(display_host("GUI"), "GUI");
        assert_eq!(display_host("  "), "FutureOS");
        assert_eq!(display_host("a\u{0}b"), "ab");
        assert!(display_host(&"x".repeat(200)).len() <= 64);
    }

    /// A clock that disagrees is refused for the right reason, and the answer says
    /// which direction and by how much — the reader's fix is to correct a clock, not
    /// to look for a forged request.
    #[test]
    fn a_stale_signature_blames_the_clocks_and_names_the_skew() {
        let behind = clock_skew_message(Some((now() - 749).to_string()));
        assert!(behind.contains("749s behind"), "{behind}");
        assert!(behind.contains("time service"), "{behind}");
        assert!(!behind.contains("not signed"), "{behind}");

        let ahead = clock_skew_message(Some((now() + 400).to_string()));
        assert!(ahead.contains("400s ahead"), "{ahead}");

        // An unparsable header still explains itself rather than claiming forgery.
        for header in [None, Some("not-a-number".to_string())] {
            let message = clock_skew_message(header);
            assert!(message.contains("Unix time"), "{message}");
            assert!(message.contains("time service"), "{message}");
        }
    }
}

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

    // ── The HTTP surface ─────────────────────────────────────────────────────
    //
    // The handlers are called directly rather than through `Router`, so each
    // test names the one thing that decides its answer: the signature headers,
    // the body, and the room registry. `Router` itself is a route table with no
    // logic to get wrong.

    const PAIR: &str = "PAIR1";
    const JOIN: &str = "join_token_for_the_handler_tests";

    fn config(account_seed: String) -> RelayConfig {
        RelayConfig {
            listen: "127.0.0.1:8099".into(),
            public_url: "http://127.0.0.1:8099".into(),
            nats_url: "nats://127.0.0.1:4222".into(),
            nats_ws_url: "ws://127.0.0.1:9090".into(),
            account_seed,
            jwt_ttl_seconds: 300,
            invitation_ttl_seconds: 300,
            room_ttl_seconds: 7_776_000,
            subs_limit: -1,
            payload_limit: jwt::DEFAULT_PAYLOAD_LIMIT,
            http_timeout_seconds: 15,
            tls_cert: None,
            tls_key: None,
        }
    }

    /// A relay with a freshly minted account key and an in-memory registry.
    fn relay() -> Arc<Relay> {
        relay_with_room_ttl(7_776_000)
    }

    /// The same relay, but rooms live `room_ttl_seconds` — the knob a test
    /// shortens so a room can expire inside a test instead of in 90 days.
    fn relay_with_room_ttl(room_ttl_seconds: i64) -> Arc<Relay> {
        let account = KeyPair::new_account();
        let seed = account.seed().expect("a new account key has a seed");
        let mut config = config(seed);
        config.room_ttl_seconds = room_ttl_seconds;
        Arc::new(Relay::new(config, None).expect("a fresh relay"))
    }

    /// The three headers a signed call carries, at `timestamp`.
    fn signed_at(
        key: &KeyPair,
        method: &str,
        path: &str,
        timestamp: i64,
        body: &[u8],
    ) -> HeaderMap {
        let (public_key, stamp, signature) =
            sign::sign(key, method, path, timestamp, body).expect("signing a request");
        let mut headers = HeaderMap::new();
        headers.insert("x-future-public-key", public_key.parse().unwrap());
        headers.insert("x-future-timestamp", stamp.parse().unwrap());
        headers.insert("x-future-signature", signature.parse().unwrap());
        headers
    }

    fn signed(key: &KeyPair, method: &str, path: &str, body: &[u8]) -> HeaderMap {
        signed_at(key, method, path, now(), body)
    }

    fn room_path() -> String {
        format!("/v1/rooms/{PAIR}")
    }

    /// The URI a handler receives from the route it is mounted on.
    fn token_uri() -> Uri {
        "/client/v1/remote/auth/token".parse().unwrap()
    }

    fn register_body(join_hash: Option<&str>) -> String {
        let hash = match join_hash {
            Some(hash) => format!(r#","joinHash":"{hash}""#),
            None => String::new(),
        };
        format!(
            r#"{{"pairId":"{PAIR}","hostDeviceId":"desktop1","hostName":"FutureOS Desktop"{hash}}}"#
        )
    }

    /// Register the room over HTTP, the way the desktop does.
    async fn register(relay: &Arc<Relay>, host: &KeyPair) -> (StatusCode, Json<Value>) {
        let body = register_body(Some(&hash_join(JOIN)));
        register_room(
            State(relay.clone()),
            signed(host, "POST", "/v1/rooms", body.as_bytes()),
            "/v1/rooms".parse().unwrap(),
            Bytes::from(body),
        )
        .await
    }

    async fn describe(
        relay: &Arc<Relay>,
        caller: &KeyPair,
        pair: &str,
    ) -> (StatusCode, Json<Value>) {
        let path = format!("/v1/rooms/{pair}");
        describe_room(
            State(relay.clone()),
            Path(pair.to_string()),
            signed(caller, "GET", &path, b""),
            path.parse().unwrap(),
        )
        .await
    }

    /// The answer a desktop needs to reach its own room, and the roster it reads
    /// back afterwards, in the field names the desktop parses.
    #[tokio::test]
    async fn a_desktop_registers_its_room_and_reads_the_roster_back() {
        let relay = relay();
        let host = KeyPair::new_user();

        let (status, answer) = register(&relay, &host).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["pairId"], PAIR);
        assert_eq!(answer["natsUrl"], "nats://127.0.0.1:4222");
        assert_eq!(answer["natsWsUrl"], "ws://127.0.0.1:9090");
        let jwt = answer["userJwt"].as_str().expect("a user JWT");
        assert_eq!(
            jwt.split('.').count(),
            3,
            "a NATS user JWT is three dot-separated parts, not an opaque token: {jwt}"
        );
        assert!(
            answer["jwtExpiresAt"].as_i64().unwrap() > now(),
            "the expiry is a future wall-clock second: {answer:?}"
        );

        let (status, roster) = describe(&relay, &host, PAIR).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(roster["pairId"], PAIR);
        assert_eq!(roster["hostName"], "FutureOS Desktop");
        assert_eq!(roster["hostPublicKey"], host.public_key());
        assert_eq!(roster["invitationOpen"], true);
        assert!(
            roster["devices"].as_array().unwrap().is_empty(),
            "nobody has claimed the invitation yet: {roster:?}"
        );
    }

    /// A request the relay cannot read is refused before the registry is
    /// touched, and each refusal names the field the caller has to fix.
    #[tokio::test]
    async fn an_unsigned_room_request_is_refused_before_anything_is_read() {
        let relay = relay();
        let host = KeyPair::new_user();
        let body = register_body(Some(&hash_join(JOIN)));

        // Registering and deleting both prove the caller with a signature; an
        // unsigned one never reaches the body.
        let (status, answer) = register_room(
            State(relay.clone()),
            HeaderMap::new(),
            "/v1/rooms".parse().unwrap(),
            Bytes::from(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(answer["error"], "signature_missing");
        assert_eq!(
            relay.rooms.len(),
            0,
            "an unsigned registration creates nothing"
        );

        let (status, _) = register(&relay, &host).await;
        assert_eq!(status, StatusCode::OK);
        let path = room_path();
        let (status, answer) = revoke_room(
            State(relay.clone()),
            Path(PAIR.to_string()),
            HeaderMap::new(),
            path.parse().unwrap(),
            Bytes::new(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(answer["error"], "signature_missing");
        assert_eq!(relay.rooms.len(), 1, "an unsigned delete keeps the room");
    }

    /// A room request that cannot be read is refused before the registry is
    /// touched, and each refusal names the field the caller has to fix.
    #[tokio::test]
    async fn a_room_request_that_cannot_be_read_names_the_field_to_fix() {
        let relay = relay();
        let host = KeyPair::new_user();

        let (status, answer) = register_room(
            State(relay.clone()),
            signed(&host, "POST", "/v1/rooms", b"{not json"),
            "/v1/rooms".parse().unwrap(),
            Bytes::from_static(b"{not json"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer["error"], "invalid_request");
        assert_eq!(answer["message"], "Malformed room request.");

        // A join hash is compared against a SHA-256 the desktop computed, so a
        // value that is not one can never match and is refused up front.
        let body = register_body(Some("not-a-hash"));
        let (status, answer) = register_room(
            State(relay.clone()),
            signed(&host, "POST", "/v1/rooms", body.as_bytes()),
            "/v1/rooms".parse().unwrap(),
            Bytes::from(body),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer["message"], "joinHash must be 64 hex characters.");

        // The pair id and device id become part of a NATS subject; anything
        // outside the subject-safe alphabet is refused rather than escaped.
        let body = format!(
            r#"{{"pairId":"pair 1","hostDeviceId":"desktop1","hostName":"GUI","joinHash":"{}"}}"#,
            hash_join(JOIN)
        );
        let (status, answer) = register_room(
            State(relay.clone()),
            signed(&host, "POST", "/v1/rooms", body.as_bytes()),
            "/v1/rooms".parse().unwrap(),
            Bytes::from(body),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(answer["error"], "identifier_invalid");
        assert_eq!(
            relay.rooms.len(),
            0,
            "a refused registration leaves the registry untouched"
        );
    }

    /// Re-registering is how a desktop survives a relay restart, so the same key
    /// keeps its room; a different key for the same pair id is refused instead of
    /// silently taking the namespace over.
    #[tokio::test]
    async fn a_second_desktop_cannot_take_over_an_existing_room() {
        let relay = relay();
        let host = KeyPair::new_user();
        let (status, _) = register(&relay, &host).await;
        assert_eq!(status, StatusCode::OK, "the room exists before the case");

        let intruder = KeyPair::new_user();
        let (status, answer) = register(&relay, &intruder).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(answer["error"], "room_owned_by_another_host");
        assert_eq!(answer["message"], "This room belongs to another desktop.");

        // The owner still reads its own room, with its own name on it.
        let (status, roster) = describe(&relay, &host, PAIR).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(roster["hostPublicKey"], host.public_key());
    }

    /// Two signature failures that are *not* "no signature": one the relay
    /// cannot even decode, and one it decodes and finds belongs to another key.
    /// Both leave through `VerifyError::code`, so the client is told which of the
    /// two it has to fix instead of the generic "not signed by its caller".
    #[tokio::test]
    async fn an_unreadable_or_foreign_signature_is_named_on_the_wire() {
        let relay = relay();
        let host = KeyPair::new_user();
        let (status, _) = register(&relay, &host).await;
        assert_eq!(status, StatusCode::OK, "the room exists before the case");
        let path = room_path();

        // A signature header that is not the base64 it is documented to be.
        let mut headers = signed(&host, "GET", &path, b"");
        headers.insert("x-future-signature", "not base64!".parse().unwrap());
        let (status, answer) = describe_room(
            State(relay.clone()),
            Path(PAIR.to_string()),
            headers,
            path.parse().unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(answer["error"], "signature_malformed");
        assert_eq!(
            answer["message"],
            "This request is not signed by its caller."
        );

        // A well-formed header naming the host's key, signed by a stranger's:
        // the signature is genuine and simply not the one the header promises.
        let stranger = KeyPair::new_user();
        let mut headers = signed(&stranger, "GET", &path, b"");
        headers.insert("x-future-public-key", host.public_key().parse().unwrap());
        let (status, answer) = describe_room(
            State(relay.clone()),
            Path(PAIR.to_string()),
            headers,
            path.parse().unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(answer["error"], "signature_invalid");
        assert_eq!(
            answer["message"],
            "This request is not signed by its caller."
        );

        // The host's own signature over the same path still opens the roster, so
        // the two refusals above are about the signature and not the request.
        let (status, _) = describe(&relay, &host, PAIR).await;
        assert_eq!(status, StatusCode::OK);
    }

    /// The roster is the desktop's business: an unsigned read, a read by another
    /// key, and a read of a room that is not there are three different answers.
    #[tokio::test]
    async fn the_roster_is_only_readable_by_the_desktop_that_owns_the_room() {
        let relay = relay();
        let host = KeyPair::new_user();
        let (status, _) = register(&relay, &host).await;
        assert_eq!(status, StatusCode::OK, "the room exists before the case");

        let path = room_path();
        let (status, answer) = describe_room(
            State(relay.clone()),
            Path(PAIR.to_string()),
            HeaderMap::new(),
            path.parse().unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(answer["error"], "signature_missing");
        assert_eq!(
            answer["message"],
            "This request is not signed by its caller."
        );

        // A clock that disagrees is a different problem, with different advice.
        let (status, answer) = describe_room(
            State(relay.clone()),
            Path(PAIR.to_string()),
            signed_at(&host, "GET", &path, now() - 10_000, b""),
            path.parse().unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(answer["error"], "signature_stale");
        let message = answer["message"].as_str().unwrap();
        assert!(message.contains("behind the relay's clock"), "{message}");
        assert!(
            !message.contains("not signed"),
            "the signature is the caller's; only its timestamp is wrong: {message}"
        );

        let stranger = KeyPair::new_user();
        let (status, answer) = describe(&relay, &stranger, PAIR).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(answer["error"], "room_owned_by_another_host");

        let (status, answer) = describe(&relay, &host, "PAIR_UNKNOWN").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(answer["error"], "room_unknown");
        assert_eq!(answer["message"], "No such room.");
    }

    /// Deleting a room is the host's own operation, and it takes the room with it.
    #[tokio::test]
    async fn deleting_a_room_needs_the_owning_key_and_removes_it() {
        let relay = relay();
        let host = KeyPair::new_user();
        let (status, _) = register(&relay, &host).await;
        assert_eq!(status, StatusCode::OK, "the room exists before the case");

        let path = room_path();
        let stranger = KeyPair::new_user();
        let (status, answer) = revoke_room(
            State(relay.clone()),
            Path(PAIR.to_string()),
            signed(&stranger, "DELETE", &path, b""),
            path.parse().unwrap(),
            Bytes::new(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(answer["error"], "room_owned_by_another_host");
        assert_eq!(relay.rooms.len(), 1, "a refused delete keeps the room");

        let (status, answer) = revoke_room(
            State(relay.clone()),
            Path(PAIR.to_string()),
            signed(&host, "DELETE", &path, b""),
            path.parse().unwrap(),
            Bytes::new(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["ok"], true);
        assert_eq!(relay.rooms.len(), 0);

        let (status, answer) = describe(&relay, &host, PAIR).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(answer["message"], "No such room.");
    }

    /// A phone claims the invitation and can then forget its own credential: a
    /// lost phone that revokes is not a support ticket.
    #[tokio::test]
    async fn a_phone_claims_the_invitation_and_can_forget_its_own_credential() {
        let relay = relay();
        let host = KeyPair::new_user();
        let (status, _) = register(&relay, &host).await;
        assert_eq!(status, StatusCode::OK);

        let phone = KeyPair::new_user();
        let body = format!(
            r#"{{"nonce":"{JOIN}","device_id":"phone1","device_public_key":"{}","device_name":""}}"#,
            phone.public_key()
        );
        let (status, answer) = claim(State(relay.clone()), Bytes::from(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["pair_id"], PAIR);
        assert_eq!(answer["nats_url"], "nats://127.0.0.1:4222");
        let refresh = answer["refresh_token"]
            .as_str()
            .expect("the long-lived half of the credential")
            .to_string();
        assert!(refresh.starts_with("rt_"), "{refresh}");

        // An unnamed device still gets a name a person can recognize in the
        // roster, instead of an empty row.
        let (_, roster) = describe(&relay, &host, PAIR).await;
        assert_eq!(roster["invitationOpen"], false, "the invitation was used");
        let devices = roster["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 1, "{roster:?}");
        assert_eq!(devices[0]["name"], "FutureOS device");

        // The same invitation cannot be spent by a second device.
        let other = KeyPair::new_user();
        let body = format!(
            r#"{{"nonce":"{JOIN}","device_id":"phone2","device_public_key":"{}","device_name":"Other"}}"#,
            other.public_key()
        );
        let (status, answer) = claim(State(relay.clone()), Bytes::from(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(answer["error"], "join_used");

        let revoke = format!(
            r#"{{"pair_id":"{PAIR}","device_id":"phone1","public_key":"{}","refresh_token":"{refresh}"}}"#,
            phone.public_key()
        );
        let (status, answer) =
            client_revoke(State(relay.clone()), Bytes::from(revoke.clone())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["ok"], true);
        let (_, roster) = describe(&relay, &host, PAIR).await;
        assert!(
            roster["devices"].as_array().unwrap().is_empty(),
            "the device is gone from the roster: {roster:?}"
        );

        // Revoking twice is the same outcome, not an error: the caller wanted the
        // credential gone, and it is.
        let (status, answer) = client_revoke(State(relay.clone()), Bytes::from(revoke)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["ok"], true);
    }

    /// Wait until the wall clock is at the very start of a whole second.
    ///
    /// `now()` is whole UNIX seconds and a room's lifetime is measured in them,
    /// so a test that must observe a room *before* it expires wants the whole
    /// second ahead of it rather than whatever tail of one it happened to land
    /// in. Returns immediately when it is already early enough in the second.
    async fn wait_for_a_fresh_second() {
        const SLACK_MS: u128 = 100;
        loop {
            let offset = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_millis() % 1_000)
                .unwrap_or_default();
            if offset < SLACK_MS {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis((1_000 - offset) as u64)).await;
        }
    }

    /// Long enough that `register` → `claim` cannot straddle it, short enough
    /// that the test still expires the room inside a test. See the doc comment
    /// on `a_revoke_against_an_expired_room_says_the_room_is_gone`.
    const ROOM_TTL_SECONDS: i64 = 3;

    /// A room that reaches its TTL is gone for the client too: the revoke is
    /// answered with the room error rather than with the "already gone" success,
    /// and the registry drops the credential with the room instead of keeping a
    /// device that can never authenticate again.
    ///
    /// **The two wall-clock windows in this test, and why neither can flip.**
    /// A room lives `expires_at = now + room_ttl` over whole UNIX seconds
    /// (`now()`), and every credential path refuses once `now >= expires_at`.
    /// That leaves two spans of real time the test depends on, and both are
    /// pinned instead of trusting an idle machine:
    ///
    /// * **`register` → `claim` has to finish before the room expires.** At
    ///   `room_ttl = 1` the room dies at the next second boundary, so the claim
    ///   gets only the tail of the second the registration landed in: a
    ///   scheduler hiccup on a loaded runner and `room_for_join` looks at a room
    ///   that is already gone, answering `403 pairing_code_invalid` before the
    ///   expiry path is ever reached. So the test starts on a fresh second
    ///   (`wait_for_a_fresh_second`) *and* gives the room
    ///   `ROOM_TTL_SECONDS = 3`: the whole second `register` runs in plus two
    ///   more, ~2.9 s of wall clock for two in-memory calls, which is not a
    ///   budget a stalled thread spends.
    /// * **`claim` → `revoke` has to outlast it.** `RoomRegistry::join` pushes
    ///   `expires_at` to `claim_time + ROOM_TTL_SECONDS`, so the sleep is
    ///   derived from the TTL rather than written down: waiting
    ///   `ROOM_TTL_SECONDS * 1000 + 100` ms from the claim gives
    ///   `floor(claim_ms + 3100) >= floor(claim_ms) + 3 = expires_at` for every
    ///   sub-second offset `claim_ms` can carry, i.e. the refusal condition
    ///   already holds when the sleep returns. The sleep is a lower bound
    ///   (tokio never returns early), so a loaded machine or a stalled scheduler
    ///   only adds slack to an assertion that is on the correct side of it.
    ///
    /// Nothing here retries or polls, so there is no window in which a slower
    /// run flips the result.
    #[tokio::test]
    async fn a_revoke_against_an_expired_room_says_the_room_is_gone() {
        // Start the clock at a fresh second so `register` and `claim` share the
        // full TTL ahead of them instead of racing the boundary they landed on.
        wait_for_a_fresh_second().await;
        let relay = relay_with_room_ttl(ROOM_TTL_SECONDS);
        let host = KeyPair::new_user();
        let (status, _) = register(&relay, &host).await;
        assert_eq!(status, StatusCode::OK);

        let phone = KeyPair::new_user();
        let body = format!(
            r#"{{"nonce":"{JOIN}","device_id":"phone1","device_public_key":"{}","device_name":"Phone"}}"#,
            phone.public_key()
        );
        let (status, answer) = claim(State(relay.clone()), Bytes::from(body)).await;
        assert_eq!(status, StatusCode::OK);
        let refresh = answer["refresh_token"]
            .as_str()
            .expect("the long-lived half of the credential")
            .to_string();
        assert_eq!(relay.rooms.len(), 1, "the room exists inside its TTL");

        tokio::time::sleep(std::time::Duration::from_millis(
            ROOM_TTL_SECONDS.unsigned_abs() * 1_000 + 100,
        ))
        .await;

        let revoke = format!(
            r#"{{"pair_id":"{PAIR}","device_id":"phone1","public_key":"{}","refresh_token":"{refresh}"}}"#,
            phone.public_key()
        );
        let (status, answer) = client_revoke(State(relay.clone()), Bytes::from(revoke)).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "an expired room outlives no credential, including its own revoke: {answer:?}"
        );
        assert_eq!(answer["error"], "room_expired");
        assert_eq!(answer["message"], "This room has expired.");
        assert_eq!(
            relay.rooms.len(),
            0,
            "the registry dropped the room, and the device credential went with it"
        );
    }

    /// A claim that cannot be read, cannot name the device key, or names no
    /// invitation is refused before the registry is touched — and the last two
    /// answers do not tell a stranger which rooms exist.
    #[tokio::test]
    async fn a_claim_that_cannot_identify_the_phone_is_refused_by_name() {
        let relay = relay();
        let host = KeyPair::new_user();
        let (status, _) = register(&relay, &host).await;
        assert_eq!(status, StatusCode::OK);

        let (status, answer) = claim(State(relay.clone()), Bytes::from_static(b"{not json")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer["message"], "Malformed claim request.");

        let (status, answer) = claim(
            State(relay.clone()),
            Bytes::from_static(
                br#"{"nonce":"whatever","device_id":"phone1","device_public_key":"NOPE","device_name":""}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            answer["message"],
            "device_public_key must be an NKey user key."
        );

        let phone = KeyPair::new_user();
        let body = format!(
            r#"{{"nonce":"join_guessed","device_id":"phone1","device_public_key":"{}","device_name":""}}"#,
            phone.public_key()
        );
        let (status, answer) = claim(State(relay.clone()), Bytes::from(body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(answer["error"], "pairing_code_invalid");
        assert_eq!(answer["message"], "This invitation is not valid.");
        assert_eq!(
            relay.rooms.len(),
            1,
            "a refused claim does not create a room"
        );
    }

    /// A credential is rotated two ways: the desktop proves itself with its
    /// pairing key, the phone with its refresh token. Each refusal names what the
    /// caller actually got wrong.
    #[tokio::test]
    async fn a_credential_request_is_refused_for_the_reason_it_is_wrong() {
        let relay = relay();
        let host = KeyPair::new_user();
        let (status, _) = register(&relay, &host).await;
        assert_eq!(status, StatusCode::OK);

        let path = "/client/v1/remote/auth/token";

        let (status, answer) = rotate_token(
            State(relay.clone()),
            HeaderMap::new(),
            token_uri(),
            Bytes::from_static(b"{not json"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer["message"], "Malformed token request.");

        let (status, answer) = rotate_token(
            State(relay.clone()),
            HeaderMap::new(),
            token_uri(),
            Bytes::from_static(br#"{"pair_id":"PAIR1","public_key":"Ux","role":"owner"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer["message"], "role must be `bridge` or `client`.");

        // A bridge rotation with no signature at all.
        let body = format!(
            r#"{{"pair_id":"{PAIR}","public_key":"{}","role":"bridge"}}"#,
            host.public_key()
        );
        let (status, answer) = rotate_token(
            State(relay.clone()),
            HeaderMap::new(),
            token_uri(),
            Bytes::from(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(answer["error"], "signature_missing");

        // Signed by one key, asking for a credential for another: the signature
        // proves the caller, and the caller is not the credential's owner.
        let other = KeyPair::new_user();
        let body = format!(
            r#"{{"pair_id":"{PAIR}","public_key":"{}","role":"bridge"}}"#,
            other.public_key()
        );
        let (status, answer) = rotate_token(
            State(relay.clone()),
            signed(&host, "POST", path, body.as_bytes()),
            token_uri(),
            Bytes::from(body),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(answer["error"], "signature_invalid");
        assert_eq!(
            answer["message"],
            "The signature does not match the credential."
        );

        // The host's own rotation, in the field names the desktop parses.
        let body = format!(
            r#"{{"pair_id":"{PAIR}","public_key":"{}","role":"bridge"}}"#,
            host.public_key()
        );
        let (status, answer) = rotate_token(
            State(relay.clone()),
            signed(&host, "POST", path, body.as_bytes()),
            token_uri(),
            Bytes::from(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["nats_url"], "nats://127.0.0.1:4222");
        assert_eq!(
            answer["user_jwt"].as_str().unwrap().split('.').count(),
            3,
            "a NATS user JWT, not an opaque token"
        );

        // A phone presenting a refresh token for a device that was never paired
        // with this room.
        let (status, answer) = rotate_token(
            State(relay.clone()),
            HeaderMap::new(),
            token_uri(),
            Bytes::from_static(
                br#"{"pair_id":"PAIR1","device_id":"phone1","public_key":"Uphone","role":"client","refresh_token":"rt_gone"}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(answer["error"], "device_unknown");
        assert_eq!(
            answer["message"],
            "This device is not paired with that room."
        );

        // A well-signed bridge rotation for a room that is not there: the
        // signature is the host's, so the answer is about the room.
        let body = format!(
            r#"{{"pair_id":"PAIR_UNKNOWN","public_key":"{}","role":"bridge"}}"#,
            host.public_key()
        );
        let (status, answer) = rotate_token(
            State(relay.clone()),
            signed(&host, "POST", path, body.as_bytes()),
            token_uri(),
            Bytes::from(body),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(answer["error"], "room_unknown");
    }

    /// A revoke the relay cannot read is refused as a malformed request: the
    /// caller has to send the credential, not a bare word, to end a pairing.
    #[tokio::test]
    async fn a_revoke_that_cannot_be_read_is_refused() {
        let relay = relay();
        let (status, answer) =
            client_revoke(State(relay.clone()), Bytes::from_static(b"{not json")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(answer["error"], "invalid_request");
        assert_eq!(answer["message"], "Malformed revoke request.");
    }

    /// A seed that is not an account key cannot sign the relay's user JWTs, and
    /// the refusal says which kind of key was handed over.
    #[test]
    fn a_user_seed_cannot_be_the_relay_account() {
        let user = KeyPair::new_user();
        let result = Relay::new(config(user.seed().unwrap()), None);
        let message = result.map(|_| ()).unwrap_err().to_string();
        assert!(message.contains("account seed"), "{message}");
    }

    /// A desktop that is already paired re-registers without an invitation —
    /// that is how it refreshes a credential after a relay restart. The roster
    /// it has already collected must survive, and no QR code may reopen.
    #[tokio::test]
    async fn re_registering_without_an_invitation_keeps_the_code_closed() {
        let relay = relay();
        let host = KeyPair::new_user();
        let (status, _) = register(&relay, &host).await;
        assert_eq!(status, StatusCode::OK);

        let phone = KeyPair::new_user();
        let body = format!(
            r#"{{"nonce":"{JOIN}","device_id":"phone1","device_public_key":"{}","device_name":"Pixel"}}"#,
            phone.public_key()
        );
        let (status, _) = claim(State(relay.clone()), Bytes::from(body)).await;
        assert_eq!(status, StatusCode::OK);

        let body = register_body(None);
        let (status, answer) = register_room(
            State(relay.clone()),
            signed(&host, "POST", "/v1/rooms", body.as_bytes()),
            "/v1/rooms".parse().unwrap(),
            Bytes::from(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["pairId"], PAIR);

        let (_, roster) = describe(&relay, &host, PAIR).await;
        assert_eq!(
            roster["invitationOpen"], false,
            "a credential refresh must not reopen the code: {roster:?}"
        );
        let devices = roster["devices"].as_array().unwrap();
        assert_eq!(
            devices.len(),
            1,
            "the paired phone keeps its row: {roster:?}"
        );
        assert_eq!(devices[0]["name"], "Pixel");
    }

    /// Every refusal a room route can produce, in the reader's terms: the status
    /// a script branches on, the code it logs, and the sentence it shows.
    #[test]
    fn every_room_refusal_carries_its_own_status_code_and_advice() {
        for (failure, status, code, message) in [
            (
                RoomError::Unknown,
                StatusCode::NOT_FOUND,
                "room_unknown",
                "No such room.",
            ),
            (
                RoomError::Expired,
                StatusCode::NOT_FOUND,
                "room_expired",
                "This room has expired.",
            ),
            (
                RoomError::InvitationExpired,
                StatusCode::CONFLICT,
                "invitation_expired",
                "This invitation has expired. Create a new one on the desktop.",
            ),
            (
                RoomError::HostMismatch,
                StatusCode::CONFLICT,
                "room_owned_by_another_host",
                "This room belongs to another desktop.",
            ),
            (
                RoomError::JoinInvalid,
                StatusCode::FORBIDDEN,
                "join_invalid",
                "This invitation is not valid for this room.",
            ),
            (
                RoomError::JoinUsed,
                StatusCode::CONFLICT,
                "join_used",
                "This invitation has already been used.",
            ),
            (
                RoomError::DeviceUnknown,
                StatusCode::FORBIDDEN,
                "device_unknown",
                "This device is not paired with that room.",
            ),
            (
                RoomError::RefreshInvalid,
                StatusCode::FORBIDDEN,
                "invalid_remote_credential",
                "This device credential is no longer valid.",
            ),
            (
                RoomError::BadIdentifier,
                StatusCode::FORBIDDEN,
                "identifier_invalid",
                "One of the identifiers is not usable.",
            ),
        ] {
            let (answer_status, answer) = room_error(failure);
            assert_eq!(answer_status, status, "{code}");
            assert_eq!(answer["error"], code);
            assert_eq!(answer["message"], message, "{code}");
        }
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

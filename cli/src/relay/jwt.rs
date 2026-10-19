//! NATS user JWTs for the open relay.
//!
//! The relay is the *only* thing here that resembles the official platform:
//! it mints a short-lived NATS user JWT whose `nats.pub`/`nats.sub` allow lists
//! are scoped to one pairing's subject namespace, exactly the way the platform
//! service does (`platform/service/src/routes/remote.rs::sign_user_jwt`). That
//! is not an account: the relay never sees a password, an email, or a billing
//! subject, and it keeps no database. It is the broker's admission rule.
//!
//! The claims are written out by hand rather than through a JWT library
//! because NATS has its own claim shape (`nats.type`, `nats.version`) and its
//! own signing convention (`ed25519-nkey` over the base64url header.payload).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use nkeys::KeyPair;
use serde::Serialize;
use sha2::{Digest, Sha512_256};

/// The largest application record the shipping clients can send.
///
/// Mirrors the platform's `payload: 1024 * 1024` claim: one sealed record is
/// exactly 1 MiB, and a larger one is dropped by the broker rather than
/// answered, which the sender sees as a timeout instead of an error.
pub const DEFAULT_PAYLOAD_LIMIT: i64 = 1024 * 1024;

/// Who a credential belongs to. The two roles are disjoint: a client may
/// publish commands but never receive them, and a bridge may receive commands
/// but never publish them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Bridge,
    Client,
}

impl Role {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "bridge" => Some(Self::Bridge),
            "client" => Some(Self::Client),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bridge => "bridge",
            Self::Client => "client",
        }
    }
}

/// The subject allow lists for one pairing.
///
/// Kept as a pure function of `(pair, device, role)` so the permission table is
/// testable without a broker, and so the relay has exactly one place that
/// decides what a connection can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permissions {
    pub publish: Vec<String>,
    pub subscribe: Vec<String>,
}

pub fn permissions(pair_id: &str, device_id: &str, role: Role) -> Permissions {
    match role {
        Role::Bridge => Permissions {
            publish: vec![
                format!("p.{pair_id}.presence"),
                format!("p.{pair_id}.state.>"),
                format!("p.{pair_id}.evt.>"),
                format!("p.{pair_id}.xfer.down.>"),
                format!("p.{pair_id}.rep.>"),
            ],
            subscribe: vec![
                format!("p.{pair_id}.cmd.>"),
                format!("p.{pair_id}.xfer.up.>"),
                format!("p.{pair_id}.rep.{device_id}.>"),
            ],
        },
        Role::Client => Permissions {
            publish: vec![
                format!("p.{pair_id}.cmd.>"),
                format!("p.{pair_id}.xfer.up.>"),
            ],
            subscribe: vec![
                format!("p.{pair_id}.presence"),
                format!("p.{pair_id}.state.>"),
                format!("p.{pair_id}.evt.>"),
                format!("p.{pair_id}.xfer.down.>"),
                format!("p.{pair_id}.rep.{device_id}.>"),
            ],
        },
    }
}

#[derive(Serialize)]
struct UserClaims<'a> {
    jti: String,
    iat: i64,
    exp: i64,
    iss: &'a str,
    name: &'a str,
    sub: &'a str,
    nats: UserNats,
}

#[derive(Serialize)]
struct UserNats {
    #[serde(rename = "pub")]
    publish: Permission,
    sub: Permission,
    subs: i64,
    data: i64,
    payload: i64,
    #[serde(rename = "type")]
    claim_type: &'static str,
    version: u8,
}

#[derive(Serialize)]
struct Permission {
    allow: Vec<String>,
}

#[derive(Serialize)]
struct Identity<'a> {
    iat: i64,
    exp: i64,
    iss: &'a str,
    name: &'a str,
    sub: &'a str,
}

/// Sign one NATS user JWT.
///
/// `account` must be the account key (or an account signing key) whose public
/// key appears in the broker's account JWT; the broker rejects a credential
/// signed by anything else.
///
/// The argument list is the claim set, not a service signature: every value
/// here lands in the JWT verbatim, and bundling them into a struct would only
/// rename the same fields at the one call site that has them.
#[allow(clippy::too_many_arguments)]
pub fn sign_user(
    account: &KeyPair,
    user_public_key: &str,
    name: &str,
    permissions: Permissions,
    iat: i64,
    exp: i64,
    subs_limit: i64,
    payload_limit: i64,
) -> Result<String, String> {
    let issuer = account.public_key();
    let identity = Identity {
        iat,
        exp,
        iss: &issuer,
        name,
        sub: user_public_key,
    };
    let identity_json = serde_json::to_vec(&identity).map_err(|error| error.to_string())?;
    // The platform derives `jti` the same way. NATS only requires it to be a
    // non-empty unique string (it is the revocation handle), so any stable
    // digest of the claims works; matching the platform keeps a broker-side
    // `nats server report jetstream` / `nats account info` view familiar.
    let jti = URL_SAFE_NO_PAD.encode(Sha512_256::digest(identity_json));

    let claims = UserClaims {
        jti,
        iat,
        exp,
        iss: &issuer,
        name,
        sub: user_public_key,
        nats: UserNats {
            publish: Permission {
                allow: permissions.publish,
            },
            sub: Permission {
                allow: permissions.subscribe,
            },
            subs: subs_limit,
            data: -1,
            payload: payload_limit,
            claim_type: "user",
            version: 2,
        },
    };
    sign_claims(account, &claims)
}

#[derive(Serialize)]
struct OperatorClaims<'a> {
    jti: String,
    iat: i64,
    iss: &'a str,
    name: &'a str,
    sub: &'a str,
    nats: OperatorNats,
}

#[derive(Serialize)]
struct OperatorNats {
    signing_keys: Vec<String>,
    #[serde(rename = "type")]
    claim_type: &'static str,
    version: u8,
}

#[derive(Serialize)]
struct AccountClaims<'a> {
    jti: String,
    iat: i64,
    iss: &'a str,
    name: &'a str,
    sub: &'a str,
    nats: AccountNats,
}

#[derive(Serialize)]
struct AccountNats {
    limits: AccountLimits,
    signing_keys: Vec<String>,
    #[serde(rename = "type")]
    claim_type: &'static str,
    version: u8,
}

#[derive(Serialize)]
struct AccountLimits {
    subs: i64,
    data: i64,
    payload: i64,
    imports: i64,
    exports: i64,
    wildcards: bool,
    conn: i64,
    leaf: i64,
    mem_storage: i64,
    disk_storage: i64,
    streams: i64,
    consumer: i64,
    max_ack_pending: i64,
    mem_max_stream_bytes: i64,
    disk_max_stream_bytes: i64,
    max_bytes_required: bool,
}

impl Default for AccountLimits {
    fn default() -> Self {
        // `-1` is NATS' "unlimited". Omitting a limit serializes it as `0`,
        // which the broker reads as "none allowed" — an account with no `subs`
        // budget rejects every subscription.
        Self {
            subs: -1,
            data: -1,
            payload: DEFAULT_PAYLOAD_LIMIT,
            imports: -1,
            exports: -1,
            wildcards: true,
            conn: -1,
            leaf: -1,
            mem_storage: -1,
            disk_storage: -1,
            streams: -1,
            consumer: -1,
            max_ack_pending: -1,
            mem_max_stream_bytes: -1,
            disk_max_stream_bytes: -1,
            max_bytes_required: false,
        }
    }
}

/// A self-signed operator JWT. The relay does not issue accounts separately:
/// one relay instance owns one operator and one account, which is the smallest
/// trust setup a self-hoster can run without the `nsc` toolchain.
pub fn operator_jwt(operator: &KeyPair, name: &str, iat: i64) -> Result<String, String> {
    let public = operator.public_key();
    let claims = OperatorClaims {
        jti: String::new(),
        iat,
        iss: &public,
        name,
        sub: &public,
        nats: OperatorNats {
            signing_keys: Vec::new(),
            claim_type: "operator",
            version: 2,
        },
    };
    sign_claims(operator, &claims)
}

/// An account JWT signed by the operator.
pub fn account_jwt(
    operator: &KeyPair,
    account: &KeyPair,
    name: &str,
    iat: i64,
) -> Result<String, String> {
    let claims = AccountClaims {
        jti: String::new(),
        iat,
        iss: &operator.public_key(),
        name,
        sub: &account.public_key(),
        nats: AccountNats {
            limits: AccountLimits::default(),
            signing_keys: Vec::new(),
            claim_type: "account",
            version: 2,
        },
    };
    let issuer = operator.public_key();
    let claims = AccountClaims {
        iss: &issuer,
        ..claims
    };
    sign_claims(operator, &claims)
}

fn sign_claims<T: Serialize>(key: &KeyPair, claims: &T) -> Result<String, String> {
    let mut value = serde_json::to_value(claims).map_err(|error| error.to_string())?;
    // `jti` is required by NATS. Derive it from the finished claims, minus the
    // field itself, so it is stable for identical content and unique across
    // different content (each JWT carries its own `iat`/`exp`).
    if value
        .get("jti")
        .and_then(serde_json::Value::as_str)
        .is_some_and(str::is_empty)
    {
        let without = without_jti(&value);
        let encoded = serde_json::to_vec(&without).map_err(|error| error.to_string())?;
        let jti = URL_SAFE_NO_PAD.encode(Sha512_256::digest(encoded));
        value["jti"] = serde_json::Value::String(jti);
    }
    let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ed25519-nkey"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).map_err(|e| e.to_string())?);
    let signing_input = format!("{header}.{payload}");
    let signature = key
        .sign(signing_input.as_bytes())
        .map_err(|error| error.to_string())?;
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn without_jti(value: &serde_json::Value) -> serde_json::Value {
    let mut clone = value.clone();
    if let Some(object) = clone.as_object_mut() {
        object.remove("jti");
    }
    clone
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> (KeyPair, KeyPair) {
        (KeyPair::new_operator(), KeyPair::new_account())
    }

    #[test]
    fn bridge_and_client_permissions_are_disjoint_where_it_matters() {
        let bridge = permissions("PAIR1", "desktop_a", Role::Bridge);
        let client = permissions("PAIR1", "dev_a", Role::Client);
        // Only the client may speak commands; only the bridge may answer them.
        assert!(client.publish.iter().any(|s| s == "p.PAIR1.cmd.>"));
        assert!(!bridge.publish.iter().any(|s| s.starts_with("p.PAIR1.cmd")));
        assert!(bridge.subscribe.iter().any(|s| s == "p.PAIR1.cmd.>"));
        assert!(!client
            .subscribe
            .iter()
            .any(|s| s.starts_with("p.PAIR1.cmd")));
        // Neither may reach outside its own pairing.
        for subject in bridge.publish.iter().chain(bridge.subscribe.iter()) {
            assert!(subject.starts_with("p.PAIR1."), "{subject}");
        }
        for subject in client.publish.iter().chain(client.subscribe.iter()) {
            assert!(subject.starts_with("p.PAIR1."), "{subject}");
        }
    }

    #[test]
    fn a_reply_inbox_is_scoped_to_the_device_that_owns_it() {
        let bridge = permissions("PAIR1", "desktop_a", Role::Bridge);
        let client = permissions("PAIR1", "dev_a", Role::Client);
        assert_eq!(
            bridge.subscribe.last().map(String::as_str),
            Some("p.PAIR1.rep.desktop_a.>")
        );
        assert_eq!(
            client.subscribe.last().map(String::as_str),
            Some("p.PAIR1.rep.dev_a.>")
        );
        // The bridge may answer any inbox in the pairing; the client may not
        // answer at all.
        assert!(bridge.publish.iter().any(|s| s == "p.PAIR1.rep.>"));
        assert!(!client.publish.iter().any(|s| s.starts_with("p.PAIR1.rep")));
    }

    #[test]
    fn an_operator_and_account_jwt_carry_the_nats_claim_shape() {
        let (operator, account) = keys();
        let operator_jwt = operator_jwt(&operator, "FUTURE-RELAY", 1_700_000_000).unwrap();
        let account_jwt = account_jwt(&operator, &account, "RELAY", 1_700_000_000).unwrap();

        let claims = decode(&operator_jwt);
        assert_eq!(claims["nats"]["type"], "operator");
        assert_eq!(claims["nats"]["version"], 2);
        assert_eq!(claims["sub"], operator.public_key());
        assert_eq!(claims["iss"], operator.public_key());
        assert!(!claims["jti"].as_str().unwrap().is_empty());

        let claims = decode(&account_jwt);
        assert_eq!(claims["nats"]["type"], "account");
        assert_eq!(claims["sub"], account.public_key());
        assert_eq!(claims["iss"], operator.public_key());
        // Unlimited, not zero: a missing limit reads as "none allowed".
        assert_eq!(claims["nats"]["limits"]["subs"], -1);
        assert_eq!(claims["nats"]["limits"]["wildcards"], true);
    }

    #[test]
    fn a_user_jwt_is_signed_by_the_account_and_scoped() {
        let (_operator, account) = keys();
        let user = KeyPair::new_user();
        let jwt = sign_user(
            &account,
            &user.public_key(),
            "bridge:test",
            permissions("PAIR1", "desktop_a", Role::Bridge),
            1_700_000_000,
            1_700_000_300,
            -1,
            DEFAULT_PAYLOAD_LIMIT,
        )
        .unwrap();
        let claims = decode(&jwt);
        assert_eq!(claims["iss"], account.public_key());
        assert_eq!(claims["sub"], user.public_key());
        assert_eq!(claims["nats"]["type"], "user");
        assert_eq!(claims["nats"]["payload"], 1_048_576);
        assert_eq!(claims["exp"], 1_700_000_300);
        assert_eq!(claims["nats"]["pub"]["allow"][0], "p.PAIR1.presence");
    }

    #[test]
    fn the_signature_is_a_real_nkey_signature_over_the_header_and_payload() {
        let (_operator, account) = keys();
        let jwt = sign_user(
            &account,
            &KeyPair::new_user().public_key(),
            "client:test",
            permissions("PAIR1", "dev_a", Role::Client),
            1,
            2,
            -1,
            DEFAULT_PAYLOAD_LIMIT,
        )
        .unwrap();
        let mut parts = jwt.split('.');
        let header = parts.next().unwrap();
        let payload = parts.next().unwrap();
        let signature = URL_SAFE_NO_PAD.decode(parts.next().unwrap()).unwrap();
        assert!(parts.next().is_none());
        KeyPair::from_public_key(&account.public_key())
            .unwrap()
            .verify(format!("{header}.{payload}").as_bytes(), &signature)
            .expect("the account key verifies its own JWT");
    }

    #[test]
    fn different_claim_content_gets_a_different_jti() {
        let (_operator, account) = keys();
        let user = KeyPair::new_user();
        let first = sign_user(
            &account,
            &user.public_key(),
            "client:test",
            permissions("PAIR1", "dev_a", Role::Client),
            1,
            2,
            -1,
            DEFAULT_PAYLOAD_LIMIT,
        )
        .unwrap();
        let second = sign_user(
            &account,
            &user.public_key(),
            "client:test",
            permissions("PAIR1", "dev_a", Role::Client),
            1,
            3,
            -1,
            DEFAULT_PAYLOAD_LIMIT,
        )
        .unwrap();
        assert_ne!(decode(&first)["jti"], decode(&second)["jti"]);
    }

    fn decode(jwt: &str) -> serde_json::Value {
        let payload = jwt.split('.').nth(1).expect("payload segment");
        let bytes = URL_SAFE_NO_PAD.decode(payload).expect("base64url payload");
        serde_json::from_slice(&bytes).expect("JSON claims")
    }
}

//! Client identity and grant DTOs. Persistence belongs to the host.
use serde::{Deserialize, Serialize};

/// The Noise identity this desktop uses with one remote host.
///
/// `peer_public_key` is the host's Noise static key. It is known from the
/// invitation itself (the QR carries it), not learned from the handshake — so
/// it is pinned at claim time and *verified* during every handshake, including
/// the first one. A key offered by the broker or the platform is refused
/// rather than adopted: there is no trust-on-first-use here.
///
/// `secret` is the invitation PSK. It exists only until the first handshake
/// succeeds; the host deletes its own copy at the same moment, so keeping ours
/// would leave a verbatim bearer credential on disk for no benefit.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PeerIdentity {
    pub private_key: String,
    pub public_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_public_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

/// Everything needed to reach one remote host again, minus the label the user
/// chose (that lives on the stored host record, so it survives a credential
/// refresh).
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PeerCreds {
    pub pair_id: String,
    /// The *host* desktop's identity — what the UI names and de-duplicates by.
    pub desktop_id: String,
    /// This installation's pairing device id (shared with the host role; the
    /// platform binds the pair to it).
    pub device_id: String,
    pub nkey_seed: String,
    pub user_jwt: String,
    pub refresh_token: String,
    pub nats_url: String,
    pub nats_ws_url: String,
    /// Unix seconds. Drives the same pre-expiry refresh the host performs.
    pub jwt_expires_at: i64,
    /// `…/client/v1/remote/auth/token` for the platform that minted the pair.
    pub token_url: String,
    /// Absent only for a v1-era import; a v2 pairing always has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure: Option<PeerIdentity>,
}

impl PeerCreds {
    /// Expired (or unreadable) JWTs are refreshed before a connect attempt.
    /// Mirrors the mobile client's 60 s margin: a token that expires *during*
    /// the handshake fails it in a way that looks like a protocol fault.
    pub fn needs_refresh(&self, now: i64) -> bool {
        self.jwt_expires_at > 0 && self.jwt_expires_at - 60 <= now
    }

    pub fn revoke_url(&self) -> String {
        self.token_url.replace("/auth/token", "/pair/revoke")
    }
}

impl std::fmt::Debug for PeerIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerIdentity")
            .field("paired", &self.peer_public_key.is_some())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PeerCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerCreds")
            .field("pair_id", &self.pair_id)
            .field("desktop_id", &self.desktop_id)
            .field("jwt_expires_at", &self.jwt_expires_at)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(desktop_id: &str, pair_id: &str) -> PeerCreds {
        PeerCreds {
            pair_id: pair_id.into(),
            desktop_id: desktop_id.into(),
            device_id: "dev_1".into(),
            nkey_seed: "SUAISECRET".into(),
            user_jwt: "jwt".into(),
            refresh_token: "refresh".into(),
            nats_url: "nats://127.0.0.1:4222".into(),
            nats_ws_url: "wss://example.invalid".into(),
            jwt_expires_at: 1_000,
            token_url: "https://future-os.cn/client/v1/remote/auth/token".into(),
            secure: Some(PeerIdentity {
                private_key: "priv".into(),
                public_key: "pub".into(),
                peer_public_key: Some("PEER".into()),
                secret: None,
            }),
        }
    }

    /// Refreshing is a clock decision, with a margin: a JWT that expires inside
    /// the margin is refreshed *before* the connect that would otherwise fail.
    #[test]
    fn refresh_is_required_inside_the_sixty_second_margin() {
        let mut creds = creds("desktop_a", "pair_a");
        creds.jwt_expires_at = 1_000;
        assert!(!creds.needs_refresh(900));
        assert!(creds.needs_refresh(940));
        assert!(creds.needs_refresh(1_000));
        // An unreadable expiry is not "expired": it is a broken credential the
        // handshake will reject, and looping a refresh on it would hammer the
        // token endpoint.
        creds.jwt_expires_at = 0;
        assert!(!creds.needs_refresh(9_999));
    }

    /// The revoke endpoint is derived, not stored, so a platform change (or a
    /// self-hosted one) needs no extra field — matching the host and mobile.
    #[test]
    fn the_revoke_url_is_derived_from_the_token_url() {
        assert_eq!(
            creds("desktop_a", "pair_a").revoke_url(),
            "https://future-os.cn/client/v1/remote/pair/revoke"
        );
    }

    #[test]
    fn debug_output_never_contains_identity_or_bearer_credentials() {
        let mut creds = creds("desktop_a", "pair_a");
        creds.user_jwt = "user-token-fixture".into();
        creds.refresh_token = "refresh-token-fixture".into();
        creds.secure.as_mut().unwrap().private_key = "private-key-fixture".into();
        let rendered = format!("{creds:?} {:?}", creds.secure);
        for sensitive in [
            "SUAISECRET",
            "user-token-fixture",
            "refresh-token-fixture",
            "private-key-fixture",
            "PEER",
        ] {
            assert!(!rendered.contains(sensitive), "debug exposed a credential");
        }
    }
}

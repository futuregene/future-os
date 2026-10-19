//! Host-owned credential value. Field names/defaults retain the existing file format.
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCreds {
    #[serde(default)]
    pub handshake_version: u32,
    #[serde(default)]
    pub secure: Option<crate::secure::PairingIdentity>,
    pub pair_id: String,
    pub desktop_id: String,
    pub nkey_seed: String,
    pub user_jwt: String,
    pub nats_url: String,
    pub nats_ws_url: String,
    pub jwt_expires_at: i64,
    /// The self-hosted relay that mints this pairing's credentials.
    ///
    /// `None` means the official platform, which is what every existing
    /// credential says by omission. It is per pairing on purpose: one machine
    /// can hold an official device and a self-hosted one at the same time, and
    /// each keeps asking the relay that issued it.
    #[serde(default)]
    pub control_url: Option<String>,
}

impl std::fmt::Debug for PairingCreds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingCreds")
            .field("handshake_version", &self.handshake_version)
            .field("pair_id", &self.pair_id)
            .field("desktop_id", &self.desktop_id)
            .field("secure", &self.secure)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_credentials_keep_the_original_defaults_and_field_names() {
        let input = json!({
            "pairId": "pair_legacy", "desktopId": "desktop_legacy",
            "nkeySeed": "seed_fixture", "userJwt": "jwt_fixture",
            "natsUrl": "tls://relay.invalid:4222", "natsWsUrl": "wss://relay.invalid",
            "jwtExpiresAt": 42
        });
        let creds: PairingCreds = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(creds.handshake_version, 0);
        assert!(creds.secure.is_none());
        assert!(creds.control_url.is_none());
        let mut expected = input;
        expected["handshakeVersion"] = json!(0);
        expected["secure"] = json!(null);
        expected["controlUrl"] = json!(null);
        assert_eq!(serde_json::to_value(creds).unwrap(), expected);
    }

    #[test]
    fn secure_credentials_roundtrip_but_debug_never_contains_secrets() {
        let input = json!({
            "handshakeVersion": 2,
            "secure": {
                "privateKey": "private_fixture", "publicKey": "public_fixture",
                "peerPublicKey": null, "secret": "invitation_fixture", "expiresAt": 40
            },
            "pairId": "pair_fixture", "desktopId": "desktop_fixture",
            "nkeySeed": "seed_fixture", "userJwt": "jwt_fixture",
            "natsUrl": "tls://relay.invalid:4222", "natsWsUrl": "wss://relay.invalid",
            "jwtExpiresAt": 42, "controlUrl": "https://relay.invalid"
        });
        let creds: PairingCreds = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(serde_json::to_value(&creds).unwrap(), input);
        let debug = format!("{creds:?}");
        for secret in [
            "private_fixture",
            "invitation_fixture",
            "seed_fixture",
            "jwt_fixture",
        ] {
            assert!(!debug.contains(secret));
        }
    }
}

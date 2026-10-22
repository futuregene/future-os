//! Server NATS connector. The host supplies trust material; platform pairings
//! require verified TLS, while a self-hosted relay may declare plaintext.
use crate::{
    health::{reconnect_delay, NatsHealth},
    PairingCreds,
};
use std::sync::{atomic::Ordering, Arc};

#[derive(Clone, Debug, Default)]
pub struct ConnectionOptions {
    pub root_bundle: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "test-support"))]
    loopback_fixture: bool,
}
impl ConnectionOptions {
    /// Sets the trust material the host supplies.
    ///
    /// Setters are the only way another crate can build this type under
    /// `test-support`: the feature adds a private field, so a struct literal
    /// (and therefore the `..Default::default()` update syntax) is not
    /// constructible from outside, while the single-field literal that works
    /// without the feature is `needless_update` with it on.
    pub fn with_root_bundle(mut self, root_bundle: Option<std::path::PathBuf>) -> Self {
        self.root_bundle = root_bundle;
        self
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_loopback_fixture(mut self) -> Self {
        self.loopback_fixture = true;
        self
    }
    fn require_tls(&self, _endpoint: &str) -> Result<bool, crate::Error> {
        #[cfg(any(test, feature = "test-support"))]
        if self.loopback_fixture {
            let endpoint = url::Url::parse(_endpoint)
                .map_err(|_| crate::Error::Message("remote_test_endpoint_invalid".into()))?;
            if !endpoint
                .host_str()
                .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1"))
            {
                return Err(crate::Error::Message(
                    "remote_test_endpoint_not_loopback".into(),
                ));
            }
            return Ok(false);
        }
        Ok(true)
    }
}

pub struct ConnectedNats {
    pub client: async_nats::Client,
    pub health: Arc<NatsHealth>,
}

/// A pairing issued by a self-hosted relay may use its declared plaintext
/// endpoint. Platform pairings and TLS relay endpoints retain verified TLS.
pub async fn connect_nats(
    creds: &PairingCreds,
    credential_was_refreshed: bool,
    environment: &ConnectionOptions,
) -> Result<ConnectedNats, crate::Error> {
    let key_pair = std::sync::Arc::new(
        nkeys::KeyPair::from_seed(&creds.nkey_seed)
            .map_err(|error| crate::Error::Message(format!("Invalid desktop NKey: {error}")))?,
    );
    let health = Arc::new(NatsHealth::default());
    health
        .credential_was_refreshed
        .store(credential_was_refreshed, Ordering::Release);
    health
        .credential_expires_at
        .store(creds.jwt_expires_at.max(0) as u64, Ordering::Release);
    let event_health = health.clone();
    let options = async_nats::ConnectOptions::with_jwt(creds.user_jwt.clone(), move |nonce| {
        let key_pair = key_pair.clone();
        async move { key_pair.sign(&nonce).map_err(async_nats::AuthError::new) }
    })
    .custom_inbox_prefix(format!("p.{}.rep.{}", creds.pair_id, creds.desktop_id))
    .reconnect_delay_callback(|attempt| reconnect_delay(attempt, rand::random::<f64>()))
    .event_callback(move |event| {
        let health = event_health.clone();
        async move { health.handle_event(&event) }
    });
    // The host supplies its platform-store snapshot: repeatedly loading it in
    // async-nats can fail intermittently on macOS during credential rotation.
    // None retains async-nats' ordinary platform load.
    let options = match environment.root_bundle.clone() {
        Some(bundle) => options.add_root_certificates(bundle),
        None => options,
    };
    let fixture_requires_tls = environment.require_tls(&creds.nats_url)?;
    let options = options.require_tls(fixture_requires_tls && !plaintext_relay(creds));
    let client = options.connect(&creds.nats_url).await.map_err(|error| {
        let message = format!("Failed to connect to NATS: {error}");
        classify_nats_connect_error(error.kind(), message)
    })?;
    Ok(ConnectedNats { client, health })
}

/// Preserve the relay policy from the application before extraction: only a
/// relay-issued pairing whose endpoint declares nats:// waives required TLS.
fn plaintext_relay(creds: &PairingCreds) -> bool {
    creds.control_url.is_some() && creds.nats_url.starts_with("nats://")
}

fn classify_nats_connect_error(
    kind: async_nats::ConnectErrorKind,
    message: String,
) -> crate::Error {
    match kind {
        async_nats::ConnectErrorKind::Authentication
        | async_nats::ConnectErrorKind::AuthorizationViolation => {
            crate::Error::RemoteAuthorization(message)
        }
        async_nats::ConnectErrorKind::Dns
        | async_nats::ConnectErrorKind::TimedOut
        | async_nats::ConnectErrorKind::Io
        | async_nats::ConnectErrorKind::Tls
        | async_nats::ConnectErrorKind::MaxReconnects => crate::Error::RemoteTransport(message),
        async_nats::ConnectErrorKind::ServerParse => crate::Error::Message(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Only a self-hosted relay that actually declares a plaintext endpoint may
    /// keep a plaintext NATS connection; the official platform and a relay that
    /// kept TLS both keep the unconditional requirement.
    #[test]
    fn only_a_plaintext_self_hosted_relay_waives_the_tls_requirement() {
        let mut creds = serde_json::from_value::<PairingCreds>(serde_json::json!({
            "pairId":"pair_plain", "desktopId":"desktop", "nkeySeed":"fixture",
            "userJwt":"fixture", "natsUrl":"nats://127.0.0.1:4222",
            "natsWsUrl":"ws://127.0.0.1:8080", "jwtExpiresAt":0
        }))
        .unwrap();
        // The fixture is a platform pairing (no relay) on a plaintext URL: that
        // combination is exactly the downgrade the rule must refuse.
        assert!(!plaintext_relay(&creds));

        creds.control_url = Some("http://192.168.1.10:8099".into());
        assert!(
            plaintext_relay(&creds),
            "a LAN relay serving nats:// is plaintext"
        );

        // Keeping TLS is the relay's own choice and stays enforced.
        creds.nats_url = "tls://192.168.1.10:4222".into();
        assert!(!plaintext_relay(&creds));

        // A platform pairing names no relay, so a plaintext URL is not a waiver.
        creds.nats_url = "nats://192.168.1.10:4222".into();
        creds.control_url = None;
        assert!(!plaintext_relay(&creds));
    }

    #[test]
    fn classify_nats_connect_error_covers_server_parse() {
        assert!(matches!(
            classify_nats_connect_error(
                async_nats::ConnectErrorKind::ServerParse,
                "bad url".to_string()
            ),
            crate::Error::Message(_)
        ));
    }
    #[test]
    fn initial_nats_connect_errors_distinguish_authorization_from_network() {
        for kind in [
            async_nats::ConnectErrorKind::Authentication,
            async_nats::ConnectErrorKind::AuthorizationViolation,
        ] {
            assert!(matches!(
                classify_nats_connect_error(kind, "rejected".to_string()),
                crate::Error::RemoteAuthorization(_)
            ));
        }
        for kind in [
            async_nats::ConnectErrorKind::Dns,
            async_nats::ConnectErrorKind::TimedOut,
            async_nats::ConnectErrorKind::Io,
        ] {
            assert!(matches!(
                classify_nats_connect_error(kind, "offline".to_string()),
                crate::Error::RemoteTransport(_)
            ));
        }
    }
    #[test]
    fn tls_is_required_even_for_loopback_and_nats_scheme_by_default() {
        let environment = ConnectionOptions::default();
        for endpoint in [
            "nats://127.0.0.1:4222",
            "nats://relay.example:4222",
            "tls://relay.example:4222",
        ] {
            assert!(environment.require_tls(endpoint).unwrap());
        }
    }
    #[test]
    fn plaintext_fixture_refuses_non_loopback_destinations() {
        let environment = ConnectionOptions::default().for_loopback_fixture();
        for endpoint in [
            "nats://127.0.0.1:4222",
            "nats://[::1]:4222",
            "nats://localhost:4222",
        ] {
            assert!(!environment.require_tls(endpoint).unwrap());
        }
        for endpoint in [
            "nats://relay.example:4222",
            "nats://127.0.0.1.evil.example:4222",
            "nats://localhost@relay.example:4222",
            "not a URL",
        ] {
            assert!(environment.require_tls(endpoint).is_err());
        }
    }
}

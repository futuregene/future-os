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

/// How many times `async-nats` may retry the bridge's socket before it gives up
/// and ends the generation.
///
/// The library's reconnect loop is the only thing that retries a *dropped*
/// socket, and it re-presents the JWT the socket was opened with. A relay that
/// refuses that JWT — expired, or rotated away by a refresh this client has not
/// seen — refuses it however often it is presented, so an unbounded budget is a
/// loop that cannot end: `remote: NATS authorization rejected [AU002]` once,
/// then the same `-ERR` every 30 s (the top rung of [`reconnect_delay`]) for as
/// long as the process lives. A generation whose supervisor has already moved
/// on has nobody left to stop it, which is what makes the bound a property of
/// the client rather than of the supervisor.
///
/// One attempt, matching `packages/remote-client`'s `NATS_RECONNECT_BUDGET`:
/// `async-nats` maps `max_reconnects(0)` to "no limit", so one is the least that
/// still gives up. It is also enough for the case that must stay transparent —
/// a socket that merely went away: the first rung of the backoff is ~2 s and a
/// successful reconnect resets the counter, so every ordinary blip gets its own
/// attempt, while anything longer ends the generation and the runtime
/// supervisor rebuilds it under its own bounded budget.
const NATS_RECONNECT_BUDGET: usize = 1;

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
    // A refused credential must end this generation, not be retried by the
    // library forever; see [`NATS_RECONNECT_BUDGET`].
    .max_reconnects(Some(NATS_RECONNECT_BUDGET))
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

    // ── The bridge's own reconnect loop ────────────────────────────────────
    //
    // A live bridge meets exactly two reconnects that must be told apart, and
    // only a socket the test controls can stage the second one: the library's
    // loop is what performs the retry, so the fixture counts the *connections*
    // it sees rather than reading anything the library says about itself.

    /// What the broker answers on the connections it accepts.
    #[derive(Clone, Copy)]
    enum Greeting {
        /// Complete the handshake and keep the socket open.
        Accept,
        /// Answer `CONNECT` with `-ERR 'Authorization Violation'`.
        Refuse,
    }

    /// A raw-socket broker that answers only the NATS handshake.
    ///
    /// `script[i]` decides what the `i`-th connection meets; once the script is
    /// exhausted it repeats its last entry, which is what makes "the credential
    /// is refused however often it is presented" a property of the fixture
    /// instead of a thing the test has to keep re-arming.
    struct ScriptedBroker {
        url: String,
        connections: Arc<std::sync::atomic::AtomicUsize>,
        sever: tokio::sync::broadcast::Sender<()>,
        task: tokio::task::JoinHandle<()>,
    }

    impl ScriptedBroker {
        async fn start(script: Vec<Greeting>) -> Self {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .expect("bind the scripted broker");
            let url = format!("nats://{}", listener.local_addr().expect("broker address"));
            let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let (sever, _) = tokio::sync::broadcast::channel::<()>(8);
            let counts = connections.clone();
            let severs = sever.clone();
            let task = tokio::spawn(async move {
                // A JoinSet aborts every live connection when this task is
                // dropped, so a finished test never leaves a socket behind.
                let mut live = tokio::task::JoinSet::new();
                let mut accepted = 0usize;
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    let greeting = script
                        .get(accepted)
                        .or(script.last())
                        .copied()
                        .expect("the script names at least one greeting");
                    accepted += 1;
                    counts.fetch_add(1, Ordering::AcqRel);
                    let sever = severs.subscribe();
                    live.spawn(async move {
                        let (read_half, mut write_half) = stream.into_split();
                        let info = concat!(
                            "INFO {\"server_id\":\"SCRIPTED\",\"server_name\":\"scripted\",",
                            "\"version\":\"2.10.2\",\"host\":\"127.0.0.1\",\"port\":0,",
                            "\"max_payload\":1048576,\"proto\":1,\"headers\":true,",
                            "\"auth_required\":true,\"nonce\":\"dGVzdA==\"}\r\n"
                        );
                        if write_half.write_all(info.as_bytes()).await.is_err() {
                            return;
                        }
                        let mut reader = BufReader::new(read_half);
                        let mut line = String::new();
                        // CONNECT and PING go out in one flush; the PONG that
                        // answers the PING is what completes the handshake.
                        loop {
                            line.clear();
                            match reader.read_line(&mut line).await {
                                Ok(0) | Err(_) => return,
                                Ok(_) => {}
                            }
                            if line.starts_with("CONNECT") {
                                break;
                            }
                        }
                        if matches!(greeting, Greeting::Refuse) {
                            let _ = write_half
                                .write_all(b"-ERR 'Authorization Violation'\r\n")
                                .await;
                            let _ = write_half.flush().await;
                            return;
                        }
                        let mut sever = sever;
                        loop {
                            line.clear();
                            tokio::select! {
                                // The test severs the live socket: a relay
                                // restart, with the client already up.
                                _ = sever.recv() => return,
                                read = reader.read_line(&mut line) => {
                                    match read {
                                        Ok(0) | Err(_) => return,
                                        Ok(_) => {}
                                    }
                                    if line.starts_with("PING") {
                                        let _ =
                                            write_half.write_all(b"PONG\r\n").await;
                                        let _ = write_half.flush().await;
                                    }
                                }
                            }
                        }
                    });
                }
            });
            ScriptedBroker {
                url,
                connections,
                sever,
                task,
            }
        }

        /// Connections handed to the script so far.
        fn connections(&self) -> usize {
            self.connections.load(Ordering::Acquire)
        }

        /// Drop every live socket without closing the listener: the client is
        /// disconnected, and its next attempt is accepted normally.
        fn hang_up(&self) {
            let _ = self.sever.send(());
        }
    }

    impl Drop for ScriptedBroker {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn loopback_creds(url: &str, pair_id: &str) -> PairingCreds {
        PairingCreds {
            handshake_version: 2,
            secure: None,
            pair_id: pair_id.to_string(),
            desktop_id: "host_budget".to_string(),
            nkey_seed: nkeys::KeyPair::new_user()
                .seed()
                .expect("a fresh user key pair has a seed"),
            user_jwt: "fixture.jwt".to_string(),
            nats_url: url.to_string(),
            nats_ws_url: "ws://127.0.0.1:0".to_string(),
            jwt_expires_at: 0,
            control_url: None,
        }
    }

    /// Poll `check` until it holds; panics with `what` after `budget`.
    async fn wait_until(what: &str, budget: std::time::Duration, check: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + budget;
        while !check() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    /// A credential the relay refuses once the socket is live must end that
    /// client's reconnect loop. `async-nats` otherwise keeps presenting the
    /// same refused JWT forever, and an orphaned generation (a replaced bridge,
    /// or a client whose supervisor has moved on) has nobody left to stop it —
    /// the production log this pins is `authorization violation` every 30 s for
    /// hours.
    #[tokio::test]
    async fn a_refused_grant_ends_the_reconnect_loop_instead_of_retrying_forever() {
        let broker = ScriptedBroker::start(vec![Greeting::Accept, Greeting::Refuse]).await;
        let creds = loopback_creds(&broker.url, "pair_refused");
        let connected = connect_nats(
            &creds,
            false,
            &ConnectionOptions::default().for_loopback_fixture(),
        )
        .await
        .expect("the first handshake is accepted");

        // The relay now refuses the credential this client holds — what a
        // rotated-away grant meets on its next reconnect. Forcing it is the
        // same path a dropped socket takes.
        connected
            .client
            .force_reconnect()
            .await
            .expect("a reconnect can be requested");

        // The reconnect waits out the first rung of the backoff (~2 s), and the
        // library reports the refusal through its event channel, so both the
        // second connection and the health view are waited for.
        wait_until(
            "the refused reconnect attempt to reach the health view",
            std::time::Duration::from_secs(6),
            || broker.connections() >= 2 && connected.health.needs_reconnect(),
        )
        .await;

        // The measured failure: with an unbounded budget the loop presents the
        // credential again on the 4 s rung of the backoff (3.2–4.8 s with the
        // ±20% jitter), then 8 s, then 16 s, then every 30 s. A window this
        // wide separates "gave up after one attempt" from "still retrying".
        tokio::time::sleep(std::time::Duration::from_secs(6)).await;
        assert_eq!(
            broker.connections(),
            2,
            "a refused credential must not be presented a second time"
        );
    }

    /// A socket that merely went away — a relay restart, a network blip of a
    /// second or two — must come back inside the client's one permitted
    /// attempt. Otherwise ordinary churn would reach the supervisor as a dead
    /// generation and spend its 10-minute `RT001` rebuild budget instead.
    #[tokio::test]
    async fn one_reconnect_attempt_carries_a_socket_that_merely_went_away() {
        let broker = ScriptedBroker::start(vec![Greeting::Accept]).await;
        let creds = loopback_creds(&broker.url, "pair_blip");
        let connected = connect_nats(
            &creds,
            false,
            &ConnectionOptions::default().for_loopback_fixture(),
        )
        .await
        .expect("the handshake is accepted");
        assert_eq!(
            connected.client.connection_state(),
            async_nats::connection::State::Connected
        );

        // Sever the live socket; the listener stays up, so the next attempt
        // meets an ordinary accepted handshake.
        broker.hang_up();

        wait_until("the reconnect", std::time::Duration::from_secs(8), || {
            broker.connections() >= 2
                && connected.client.connection_state() == async_nats::connection::State::Connected
        })
        .await;
        assert!(
            !connected.health.needs_reconnect(),
            "a socket that came back is not a dead generation: reporting one would \
             start the supervisor's rebuild path for mere churn"
        );
    }
}

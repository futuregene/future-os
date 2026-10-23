//! Broker-level proof that the relay's credentials are the real thing.
//!
//! The unit tests prove the claims *look* right; this one proves `nats-server`
//! accepts them and that the subject allow lists it enforces are the ones the
//! remote protocol needs. It starts a real `nats-server` in operator mode with
//! JWTs shaped exactly like `future-relay init` writes them, runs the relay's
//! HTTP API in-process, and talks to the broker the way the desktop and the
//! phone do.
//!
//! Ignored by default: CI has no `nats-server` binary. Run it with
//!
//! ```text
//! FUTURE_RELAY_NATS_SERVER=/path/to/nats-server \
//!   cargo test -p future-cli --test relay_nats_auth -- --ignored --nocapture
//! ```
//!
//! Every negative case is asserted as "the message never arrives". That is
//! stricter than watching for an error and it is what actually matters: a
//! permission violation that still delivered the payload would be worse than
//! one that closed the connection.

use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use future_cli::relay::room::hash_join;
use future_cli::relay::{jwt, sign, Relay, RelayConfig};
use futures_util::StreamExt;
use nkeys::KeyPair;

const PAIR_A: &str = "PAIR0123456789ABCDEF";
const PAIR_B: &str = "PAIRFEDCBA987654321";

struct NatsServer {
    child: Child,
    url: String,
    /// The account the broker preloads. The relay must sign with *this* seed.
    account_seed: String,
}

impl Drop for NatsServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn nats_binary() -> Option<String> {
    std::env::var("FUTURE_RELAY_NATS_SERVER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// `NatsServer::drop` kills and reaps the child; the lint cannot see the wait
/// across the struct boundary.
#[allow(clippy::zombie_processes)]
fn start_nats(dir: &Path) -> NatsServer {
    let binary = nats_binary().expect("FUTURE_RELAY_NATS_SERVER");
    let operator = KeyPair::new_operator();
    let account = KeyPair::new_account();
    let iat = future_cli::relay::now();
    let operator_jwt = jwt::operator_jwt(&operator, "TEST", iat).unwrap();
    let account_jwt = jwt::account_jwt(&operator, &account, "RELAY", iat).unwrap();

    let operator_path = dir.join("operator.jwt");
    std::fs::write(&operator_path, format!("{operator_jwt}\n")).unwrap();
    let port = free_port();
    let conf = dir.join("nats.conf");
    std::fs::write(
        &conf,
        format!(
            "server_name: relay-test\n\
             listen: 127.0.0.1:{port}\n\
             operator: \"{}\"\n\
             resolver: MEMORY\n\
             resolver_preload: {{\n  {}: \"{}\"\n}}\n",
            operator_path.to_string_lossy().replace('\\', "/"),
            account.public_key(),
            account_jwt
        ),
    )
    .unwrap();

    let child = Command::new(binary)
        .arg("-c")
        .arg(&conf)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start nats-server");

    let url = format!("nats://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return NatsServer {
                child,
                url,
                account_seed: account.seed().unwrap(),
            };
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("nats-server did not start on {url}");
}

struct RunningRelay {
    _server: tokio::task::JoinHandle<()>,
    url: String,
    relay: std::sync::Arc<Relay>,
}

async fn start_relay(nats_url: &str, account_seed: &str, rooms: Option<&Path>) -> RunningRelay {
    let config = RelayConfig {
        listen: "127.0.0.1:0".into(),
        public_url: String::new(),
        nats_url: nats_url.to_string(),
        nats_ws_url: nats_url.replace("nats://", "ws://"),
        account_seed: account_seed.to_string(),
        jwt_ttl_seconds: 300,
        invitation_ttl_seconds: 300,
        room_ttl_seconds: 90 * 24 * 60 * 60,
        subs_limit: -1,
        payload_limit: jwt::DEFAULT_PAYLOAD_LIMIT,
        http_timeout_seconds: 15,
        // This test drives the broker, not the API transport.
        tls_cert: None,
        tls_key: None,
    };
    let relay = std::sync::Arc::new(Relay::new(config, rooms).unwrap());
    let app = future_cli::relay::router(relay.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    RunningRelay {
        _server: server,
        url: format!("http://{addr}"),
        relay,
    }
}

struct Host {
    key: KeyPair,
    pair_id: String,
    device_id: String,
    join: String,
}

impl Host {
    fn new(pair_id: &str, device_id: &str, join_byte: u8) -> Self {
        Self {
            key: KeyPair::new_user(),
            pair_id: pair_id.to_string(),
            device_id: device_id.to_string(),
            join: URL_SAFE_NO_PAD.encode([join_byte; 32]),
        }
    }

    /// Register through the shipping shape: the desktop signs the body with its
    /// pairing NKey and sends the *hash* of the invitation token.
    async fn register(&self, relay: &RunningRelay) -> serde_json::Value {
        let body = serde_json::json!({
            "pairId": self.pair_id,
            "hostDeviceId": self.device_id,
            "hostName": "GUI",
            "joinHash": hash_join(&self.join),
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let now = future_cli::relay::now();
        let (public, timestamp, signature) =
            sign::sign(&self.key, "POST", "/v1/rooms", now, &bytes).unwrap();
        let response = future_agent::http::client()
            .post(format!("{}/v1/rooms", relay.url))
            .header("x-future-public-key", public)
            .header("x-future-timestamp", timestamp)
            .header("x-future-signature", signature)
            .header("content-type", "application/json")
            .body(bytes)
            .send()
            .await
            .expect("register request");
        let status = response.status();
        let value: serde_json::Value = response.json().await.expect("JSON body");
        assert!(status.is_success(), "register -> {status}: {value}");
        value
    }

    async fn rotate_bridge(&self, relay: &RunningRelay) -> serde_json::Value {
        let body = serde_json::json!({
            "pair_id": self.pair_id,
            "device_id": self.device_id,
            "public_key": self.key.public_key(),
            "role": "bridge",
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let now = future_cli::relay::now();
        let (public, timestamp, signature) = sign::sign(
            &self.key,
            "POST",
            "/client/v1/remote/auth/token",
            now,
            &bytes,
        )
        .unwrap();
        let response = future_agent::http::client()
            .post(format!("{}/client/v1/remote/auth/token", relay.url))
            .header("x-future-public-key", public)
            .header("x-future-timestamp", timestamp)
            .header("x-future-signature", signature)
            .header("content-type", "application/json")
            .body(bytes)
            .send()
            .await
            .expect("token request");
        let status = response.status();
        let value: serde_json::Value = response.json().await.expect("JSON body");
        assert!(status.is_success(), "bridge token -> {status}: {value}");
        value
    }
}

struct Device {
    key: KeyPair,
    device_id: String,
}

impl Device {
    fn new(id: &str) -> Self {
        Self {
            key: KeyPair::new_user(),
            device_id: id.to_string(),
        }
    }

    /// The exact body the phone sends: `nonce` is the invitation's join token.
    async fn claim(&self, relay: &RunningRelay, host: &Host) -> serde_json::Value {
        let response = future_agent::http::client()
            .post(format!("{}/client/v1/remote/pair/claim", relay.url))
            .json(&serde_json::json!({
                "nonce": host.join,
                "device_id": self.device_id,
                "device_public_key": self.key.public_key(),
                "device_name": "Test phone",
            }))
            .send()
            .await
            .expect("claim request");
        let status = response.status();
        let value: serde_json::Value = response.json().await.expect("JSON body");
        assert!(status.is_success(), "claim -> {status}: {value}");
        value
    }

    async fn rotate(
        &self,
        relay: &RunningRelay,
        pair_id: &str,
        refresh: &str,
    ) -> (u16, serde_json::Value) {
        let response = future_agent::http::client()
            .post(format!("{}/client/v1/remote/auth/token", relay.url))
            .json(&serde_json::json!({
                "pair_id": pair_id,
                "device_id": self.device_id,
                "public_key": self.key.public_key(),
                "role": "client",
                "refresh_token": refresh,
            }))
            .send()
            .await
            .expect("token request");
        let status = response.status().as_u16();
        let value: serde_json::Value = response.json().await.expect("JSON body");
        (status, value)
    }
}

async fn connect(url: &str, key: &KeyPair, user_jwt: &str, label: &str) -> async_nats::Client {
    let key = key.clone();
    async_nats::ConnectOptions::with_jwt(user_jwt.to_string(), move |nonce| {
        let key = key.clone();
        async move { key.sign(&nonce).map_err(async_nats::AuthError::new) }
    })
    .require_tls(false)
    .name(label.to_string())
    .connect(url)
    .await
    .expect("connect with the relay's credential")
}

fn text(value: &serde_json::Value) -> String {
    value.as_str().expect("string field").to_string()
}

/// Both tests start their own broker. `free_port` releases the port it probed
/// before `nats-server` claims it, so two of them running side by side can hand
/// the same number to both servers. A process-wide async mutex removes that
/// race (and the CPU contention that makes the silence assertions hover near
/// their timeout) without making the suite slower in practice: each test is a
/// few seconds and they would otherwise share the CPU anyway.
fn serial() -> &'static tokio::sync::Mutex<()> {
    static SERIAL: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    SERIAL.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Assert that `subject` receives nothing for a while. Used for the negative
/// cases: a permission that was *not* enforced would deliver here.
async fn expect_silence(subscriber: &mut async_nats::Subscriber, subject: &str, what: &str) {
    match tokio::time::timeout(Duration::from_millis(900), subscriber.next()).await {
        Err(_) => {}
        Ok(Some(message)) => panic!(
            "{what}: received {} bytes on {} — the broker did not enforce the permission",
            message.payload.len(),
            message.subject
        ),
        Ok(None) => panic!("{what}: subscription {subject} ended unexpectedly"),
    }
}

async fn expect_delivery(subscriber: &mut async_nats::Subscriber, expected: &str) -> Vec<u8> {
    let message = tokio::time::timeout(Duration::from_secs(5), subscriber.next())
        .await
        .expect("a message arrives")
        .expect("subscription alive");
    assert_eq!(message.subject.as_str(), expected);
    message.payload.to_vec()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs FUTURE_RELAY_NATS_SERVER pointing at a nats-server binary"]
async fn relay_credentials_are_accepted_and_scoped_by_the_broker() {
    let _serial = serial().lock().await;
    let dir = tempfile::tempdir().unwrap();
    let broker = start_nats(dir.path());
    let relay = start_relay(&broker.url, &broker.account_seed, None).await;

    let host = Host::new(PAIR_A, "desktop_aaaaaaaa", 7);
    let registered = host.register(&relay).await;
    let bridge_jwt = text(&registered["userJwt"]);
    assert_eq!(registered["natsUrl"], broker.url);

    let phone = Device::new("dev_aaaaaaaa");
    let claimed = phone.claim(&relay, &host).await;
    assert_eq!(claimed["pair_id"], PAIR_A);
    let client_jwt = text(&claimed["user_jwt"]);
    let refresh = text(&claimed["refresh_token"]);
    assert_eq!(claimed["nats_url"], broker.url);

    // ── Both roles connect and use their own half of the protocol ──
    let bridge = connect(&broker.url, &host.key, &bridge_jwt, "bridge-a").await;
    let client = connect(&broker.url, &phone.key, &client_jwt, "client-a").await;

    let mut presence = client
        .subscribe(format!("p.{PAIR_A}.presence"))
        .await
        .unwrap();
    client.flush().await.unwrap();
    bridge
        .publish(
            format!("p.{PAIR_A}.presence"),
            br#"{"online":true}"#.to_vec().into(),
        )
        .await
        .unwrap();
    bridge.flush().await.unwrap();
    assert_eq!(
        expect_delivery(&mut presence, &format!("p.{PAIR_A}.presence")).await,
        br#"{"online":true}"#
    );

    let mut commands = bridge
        .queue_subscribe(format!("p.{PAIR_A}.cmd.>"), format!("bridge.{PAIR_A}"))
        .await
        .unwrap();
    bridge.flush().await.unwrap();
    client
        .publish(format!("p.{PAIR_A}.cmd.sess1"), b"FRE2".to_vec().into())
        .await
        .unwrap();
    client.flush().await.unwrap();
    assert_eq!(
        expect_delivery(&mut commands, &format!("p.{PAIR_A}.cmd.sess1")).await,
        b"FRE2"
    );

    // ── The client may not publish as the bridge ──
    let observer = connect(&broker.url, &phone.key, &client_jwt, "client-a-observer").await;
    let mut presence_observer = observer
        .subscribe(format!("p.{PAIR_A}.presence"))
        .await
        .unwrap();
    observer.flush().await.unwrap();
    let _ = client
        .publish(
            format!("p.{PAIR_A}.presence"),
            br#"{"forged":true}"#.to_vec().into(),
        )
        .await;
    let _ = client.flush().await;
    expect_silence(
        &mut presence_observer,
        &format!("p.{PAIR_A}.presence"),
        "a client publishing on the bridge's subject",
    )
    .await;
    // ...while the bridge's own publish still lands, so the observer is real.
    bridge
        .publish(
            format!("p.{PAIR_A}.presence"),
            br#"{"online":true}"#.to_vec().into(),
        )
        .await
        .unwrap();
    bridge.flush().await.unwrap();
    expect_delivery(&mut presence_observer, &format!("p.{PAIR_A}.presence")).await;

    // ── A credential cannot reach another pairing ──
    let other_host = Host::new(PAIR_B, "desktop_bbbbbbbb", 9);
    let other_registered = other_host.register(&relay).await;
    let other_bridge_jwt = text(&other_registered["userJwt"]);
    let other_phone = Device::new("dev_bbbbbbbb");
    let other_claimed = other_phone.claim(&relay, &other_host).await;
    let other_client_jwt = text(&other_claimed["user_jwt"]);
    let other_bridge = connect(&broker.url, &other_host.key, &other_bridge_jwt, "bridge-b").await;
    let other_client = connect(&broker.url, &other_phone.key, &other_client_jwt, "client-b").await;
    let mut other_events = other_client
        .subscribe(format!("p.{PAIR_B}.evt.>"))
        .await
        .unwrap();
    other_client.flush().await.unwrap();

    let spy = connect(&broker.url, &host.key, &bridge_jwt, "bridge-a-spy").await;
    let mut spy_events = spy.subscribe(format!("p.{PAIR_B}.evt.>")).await.unwrap();
    let _ = spy.flush().await;
    other_bridge
        .publish(format!("p.{PAIR_B}.evt.sess9"), b"FRE2".to_vec().into())
        .await
        .unwrap();
    other_bridge.flush().await.unwrap();
    assert_eq!(
        expect_delivery(&mut other_events, &format!("p.{PAIR_B}.evt.sess9")).await,
        b"FRE2"
    );
    expect_silence(
        &mut spy_events,
        &format!("p.{PAIR_B}.evt.>"),
        "pairing A subscribing to pairing B",
    )
    .await;

    // ── A wildcard subscription is not a way around the namespace ──
    let wildcard = connect(&broker.url, &host.key, &bridge_jwt, "bridge-a-wildcard").await;
    let mut everything = wildcard.subscribe(">").await.unwrap();
    let _ = wildcard.flush().await;
    bridge
        .publish(
            format!("p.{PAIR_A}.presence"),
            br#"{"online":true}"#.to_vec().into(),
        )
        .await
        .unwrap();
    bridge.flush().await.unwrap();
    expect_silence(&mut everything, ">", "a bridge subscribing to `>`").await;

    // ── A reply inbox belongs to one device, not to the whole pairing ──
    let inbox_spy = connect(&broker.url, &host.key, &bridge_jwt, "bridge-a-inbox-spy").await;
    let mut spied = inbox_spy
        .subscribe(format!("p.{PAIR_A}.rep.dev_aaaaaaaa.>"))
        .await
        .unwrap();
    let _ = inbox_spy.flush().await;
    let mut inbox = client
        .subscribe(format!("p.{PAIR_A}.rep.dev_aaaaaaaa.r1"))
        .await
        .unwrap();
    client.flush().await.unwrap();
    bridge
        .publish(
            format!("p.{PAIR_A}.rep.dev_aaaaaaaa.r1"),
            b"reply".to_vec().into(),
        )
        .await
        .unwrap();
    bridge.flush().await.unwrap();
    assert_eq!(
        expect_delivery(&mut inbox, &format!("p.{PAIR_A}.rep.dev_aaaaaaaa.r1")).await,
        b"reply"
    );
    expect_silence(
        &mut spied,
        &format!("p.{PAIR_A}.rep.dev_aaaaaaaa.>"),
        "the bridge subscribing to the client's reply inbox",
    )
    .await;

    // ── Both roles rotate exactly the way the shipping clients do ──
    let rotated = host.rotate_bridge(&relay).await;
    let rotated_jwt = text(&rotated["user_jwt"]);
    assert_ne!(rotated_jwt, bridge_jwt);
    let rotated_bridge = connect(&broker.url, &host.key, &rotated_jwt, "bridge-a-rotated").await;
    rotated_bridge
        .publish(
            format!("p.{PAIR_A}.presence"),
            br#"{"rotated":true}"#.to_vec().into(),
        )
        .await
        .unwrap();
    rotated_bridge.flush().await.unwrap();

    let (status, renewed) = phone.rotate(&relay, PAIR_A, &refresh).await;
    assert_eq!(status, 200, "{renewed}");
    let renewed_jwt = text(&renewed["user_jwt"]);
    assert_ne!(renewed_jwt, client_jwt);
    let _ = connect(&broker.url, &phone.key, &renewed_jwt, "client-a-renewed").await;

    // ── A wrong refresh credential is refused ──
    let (status, refused) = phone.rotate(&relay, PAIR_A, "rt_not-the-token").await;
    assert_eq!(status, 403, "{refused}");
    assert_eq!(refused["error"], "invalid_remote_credential");

    // ── The invitation is single use ──
    let intruder = Device::new("dev_intruder");
    let response = future_agent::http::client()
        .post(format!("{}/client/v1/remote/pair/claim", relay.url))
        .json(&serde_json::json!({
            "nonce": host.join,
            "device_id": intruder.device_id,
            "device_public_key": intruder.key.public_key(),
            "device_name": "Intruder",
        }))
        .send()
        .await
        .expect("claim request");
    assert_eq!(response.status().as_u16(), 409);

    // ── The relay's own room bookkeeping matches what happened ──
    let room = relay
        .relay
        .rooms
        .describe(PAIR_A, future_cli::relay::now())
        .expect("room A exists");
    assert_eq!(room.host_device_id, "desktop_aaaaaaaa");
    assert_eq!(room.devices.len(), 1);
    assert_eq!(room.devices[0].device_id, "dev_aaaaaaaa");
    assert!(!room.invitation_open, "the invitation was consumed");
}

/// A restarted relay comes back with the same rooms: the pairing lives in the
/// credentials and in the room file, not in a process.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs FUTURE_RELAY_NATS_SERVER pointing at a nats-server binary"]
async fn a_restarted_relay_reissues_from_the_persisted_registry() {
    let _serial = serial().lock().await;
    let dir = tempfile::tempdir().unwrap();
    let broker = start_nats(dir.path());
    let rooms = dir.path().join("rooms.json");
    let host = Host::new(PAIR_A, "desktop_aaaaaaaa", 7);
    let phone = Device::new("dev_aaaaaaaa");

    let refresh = {
        let relay = start_relay(&broker.url, &broker.account_seed, Some(&rooms)).await;
        host.register(&relay).await;
        let claimed = phone.claim(&relay, &host).await;
        text(&claimed["refresh_token"])
    };

    let restarted = start_relay(&broker.url, &broker.account_seed, Some(&rooms)).await;
    let rotated = host.rotate_bridge(&restarted).await;
    let jwt = text(&rotated["user_jwt"]);
    let _ = connect(&broker.url, &host.key, &jwt, "bridge-after-restart").await;

    let (status, renewed) = phone.rotate(&restarted, PAIR_A, &refresh).await;
    assert_eq!(status, 200, "{renewed}");
    let room = restarted
        .relay
        .rooms
        .describe(PAIR_A, future_cli::relay::now())
        .expect("room A survived the restart");
    assert_eq!(room.devices.len(), 1);
}

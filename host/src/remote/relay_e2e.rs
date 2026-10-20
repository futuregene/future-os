//! End-to-end: a client pairs and works through a **self-hosted relay**, using
//! the shipping code paths on both ends.
//!
//! Nothing here is stubbed except the model vendor and the Future account:
//!
//! * the host is the real `remote::start()` bridge — `create_pairing` mints the
//!   v3 invitation through `remote::relay`, `connect_nats` authenticates with
//!   the NATS user JWT the relay issued, and the production command / event /
//!   transfer loops run;
//! * the broker is a real `nats-server` started from `future relay init`'s own
//!   configuration (operator mode, per-pair subject ACLs);
//! * the relay is a real `future relay serve` process;
//! * the client is the real client role (`remote_peer`): the same protocol the
//!   phone speaks — `Noise XXpsk0` on the invitation, the `FRE2` record layer,
//!   JSON commands.
//!
//! Run it against a live relay:
//!
//! ```text
//! future relay init --dir /tmp/relay --nats-url nats://127.0.0.1:14222
//! nats-server -c /tmp/relay/nats.conf
//! future relay serve --config /tmp/relay/relay.json
//! RELAY_E2E_RELAY_URL=http://127.0.0.1:8099 \
//!   cargo test -p futureos --lib remote::relay_e2e -- --ignored --nocapture
//! ```
//!
//! This is a `cfg(test)` build, so the broker may be plaintext — the same
//! carve-out `remote/real_network.rs` uses. A release build refuses a non-TLS
//! relay, which is what a deployment gets.

#![cfg(test)]

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::json;
use std::time::Duration;

use super::test_support::{ensure_mock_agent, init_store, thread_for_session, unique, HomeGuard};
use super::RemoteStartInput;

/// The relay the test talks to. Required: a relay started in-process would not
/// prove the process boundary this test exists for.
fn relay_url() -> String {
    std::env::var("RELAY_E2E_RELAY_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .expect("RELAY_E2E_RELAY_URL must point at a running `future relay serve`")
}

#[tokio::test]
#[ignore = "needs a running future relay serve (docs/guide/self-hosted-relay.md)"]
async fn a_client_pairs_and_works_through_a_self_hosted_relay() {
    let relay = relay_url();
    // The app installs the provider at startup; a test process has to do it
    // itself, or the first `reqwest::Client` panics.
    crate::install_rustls_provider();
    // The host reads the relay from the environment first, so the test never
    // has to write `relay.json` into the guest HOME.
    std::env::set_var("FUTURE_RELAY_URL", &relay);
    let _home = HomeGuard::new("relay-e2e-host");
    init_store();
    ensure_mock_agent();
    let rooms_before = rooms_at(&relay).await;

    // ── The host mints a v3 invitation without any account ──
    let status = super::start(RemoteStartInput {})
        .await
        .expect("relay start");
    let invitation = status
        .pairing_code
        .clone()
        .expect("a fresh invitation is printed");
    let fields = query_fields(&invitation);
    assert_eq!(
        fields.get("v").map(String::as_str),
        Some("3"),
        "{invitation}"
    );
    assert_eq!(
        fields.get("desktopId"),
        Some(&status.desktop_id),
        "the invitation names the bridge the status reports"
    );
    assert!(
        fields
            .get("desktopKey")
            .is_some_and(|key| key.starts_with('U')),
        "the invitation carries the bridge NKey: {invitation}"
    );
    assert!(
        fields.get("secureKey").is_some_and(|key| key.len() == 43),
        "the invitation carries the Noise static key: {invitation}"
    );
    assert!(
        fields.get("secret").is_some_and(|key| key.len() == 43),
        "the invitation carries the one-time secret: {invitation}"
    );
    let code = decode_payload(fields.get("code").expect("code"));
    assert_eq!(
        code["relay"],
        json!(relay),
        "the code names its relay: {code}"
    );
    assert_eq!(
        code["claim_url"],
        json!(format!("{relay}/client/v1/remote/pair/claim")),
        "{code}"
    );
    assert_eq!(
        code["nonce"].as_str().map(str::len),
        Some(43),
        "the claim nonce is the join token: {code}"
    );

    // ── The client role claims it, exactly as a phone would ──
    let desktop_id = fields.get("desktopId").expect("desktopId").clone();
    let paired = crate::remote_peer::runtime::pair_with_emitter(&invitation, None)
        .await
        .expect("the client claims the invitation through the relay");
    assert!(paired.connected, "the client reports a live connection");
    assert_eq!(paired.pair_id, status.pair_id);
    let bridge_instance = paired
        .bridge_instance_id
        .clone()
        .expect("the host's bridge identity");
    println!(
        "RELAY_E2E_PAIRED {}",
        json!({
            "relay": relay,
            "pairId": paired.pair_id,
            "desktopId": desktop_id,
            "bridgeInstanceId": bridge_instance,
            "features": paired.features,
        })
    );

    // ── A command crosses the encrypted channel and is answered ──
    let presence = crate::remote_peer::runtime::request(
        &desktop_id,
        json!({ "type": "get_presence" }),
        "presence",
    )
    .await
    .expect("get_presence over the relay");
    assert_eq!(
        presence["bridgeInstanceId"].as_str(),
        Some(bridge_instance.as_str()),
        "{presence}"
    );

    let session = format!("sess_{}", unique("relay"));
    thread_for_session(&session);
    let listing = crate::remote_peer::runtime::request(
        &desktop_id,
        json!({ "type": "list_sessions" }),
        "list",
    )
    .await
    .expect("list_sessions over the relay");
    assert!(
        listing.get("sessions").is_some() || listing.get("threads").is_some(),
        "the catalog came back: {listing}"
    );

    // ── The relay remembers the room and the device it accepted ──
    assert_eq!(
        rooms_at(&relay).await,
        rooms_before + 1,
        "the relay created exactly one room"
    );
    let room = describe_room(&relay, &status.pair_id).await;
    assert_eq!(
        room["hostDeviceId"].as_str(),
        Some(status.desktop_id.as_str())
    );
    assert_eq!(room["invitationOpen"], json!(false), "{room}");
    let devices = room["devices"].as_array().expect("device list");
    assert_eq!(devices.len(), 1, "{room}");
    // The client role reports the device id it claimed with; the relay must
    // have bound the same one.
    let book = crate::remote_peer::creds::load().expect("the client stored its credential");
    let claimed_device = book
        .find(&paired.desktop_id)
        .expect("the paired host is in the client's book")
        .creds
        .device_id
        .clone();
    assert_eq!(devices[0]["deviceId"], json!(claimed_device));

    // ── Unpair takes the room away, so the pair id stops existing ──
    crate::remote_peer::runtime::unpair(&desktop_id)
        .await
        .expect("client unpair");
    super::unpair().await.expect("host unpair");
    // Unpairing never blocks on the network (a user who unpairs offline still
    // stops *now*), so the room delete is detached and this is the only place
    // that can observe it. A room that outlived the wait is not a security
    // hole — the desktop dropped its key, so nobody serves that namespace — but
    // it is a leak worth failing on.
    let gone = wait_for_room_count(&relay, rooms_before, Duration::from_secs(10)).await;
    assert!(
        gone,
        "the room is deleted after an explicit unpair (rooms now {})",
        rooms_at(&relay).await
    );
}

fn query_fields(url: &str) -> std::collections::HashMap<String, String> {
    reqwest::Url::parse(url)
        .expect("invitation URL")
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

fn decode_payload(value: &String) -> serde_json::Value {
    let bytes = URL_SAFE_NO_PAD.decode(value).expect("base64url payload");
    serde_json::from_slice(&bytes).expect("JSON payload")
}

async fn wait_for_room_count(relay: &str, expected: u64, budget: Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    loop {
        if rooms_at(relay).await == expected {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn rooms_at(relay: &str) -> u64 {
    let health: serde_json::Value = reqwest::get(format!("{relay}/v1/health"))
        .await
        .expect("relay health")
        .json()
        .await
        .expect("health JSON");
    assert_eq!(health["ok"], json!(true), "{health}");
    health["rooms"].as_u64().expect("room count")
}

/// Read a room the way an operator would: signed with the host's own NKey.
async fn describe_room(relay: &str, pair_id: &str) -> serde_json::Value {
    let creds = crate::remote_host::pairing::load_creds_checked()
        .expect("read the pairing store")
        .expect("a pairing was just created");
    let key = nkeys::KeyPair::from_seed(&creds.nkey_seed).expect("host NKey");
    let path = format!("/v1/rooms/{pair_id}");
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    let signature = key
        .sign(super::relay::canonical_request("GET", &path, timestamp, &[]).as_bytes())
        .expect("sign the request");
    let response = reqwest::Client::new()
        .get(format!("{relay}{path}"))
        .header("x-future-public-key", key.public_key())
        .header("x-future-timestamp", timestamp.to_string())
        .header("x-future-signature", URL_SAFE_NO_PAD.encode(signature))
        .send()
        .await
        .expect("describe room");
    let status = response.status();
    let body: serde_json::Value = response.json().await.expect("JSON body");
    assert!(status.is_success(), "describe room -> {status}: {body}");
    body
}

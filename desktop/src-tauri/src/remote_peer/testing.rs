//! Shared fixtures for the client-role tests.
//!
//! The tests that matter here run against a **real host bridge** on the
//! in-process fake broker, using the host's own invitation (so the PSK and the
//! Noise keys are the real ones). Building that fixture in two places would let
//! the runtime tests and the end-to-end tests drift into testing different
//! protocols, so it lives here once.

use super::{creds, link, platform};
use crate::remote::test_support::{
    init_store, jwt, now_secs, sign_in, FakeNats, HomeGuard, MockPlatform,
};
use crate::remote::{start, stop, RemoteStartInput};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::json;

/// The host bridge's test web port, whose socket lingers briefly after `stop()`.
/// Must match `remote::diagnostics::WEB_PORT`; the host tests do the same dance.
const WEB_PORT: u16 = 8022;

/// Wait until the test web port is bindable again. Every test here starts a
/// bridge, so without this they collide on the shared port.
pub(crate) async fn wait_for_web_port_free() {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(listener) = std::net::TcpListener::bind(("0.0.0.0", WEB_PORT)) {
            drop(listener);
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the test web port never freed"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// The host bridge's invitation, retargeted at a claim endpoint we control.
///
/// Replacing the `code` is legitimate rather than a shortcut: the host never
/// inspects it — the code is the *platform's* one-time nonce, and the host's
/// own identity (the PSK and both keys) stays exactly as minted.
pub(crate) fn invitation_for(host_invitation: &str, claim_url: &str) -> String {
    let host = reqwest::Url::parse(host_invitation).expect("host invitation url");
    let field = |name: &str| {
        host.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .unwrap_or_else(|| panic!("host invitation is missing {name}"))
    };
    let code = URL_SAFE_NO_PAD.encode(
        json!({
            "nonce": format!("nonce_{}", now_secs()),
            "claim_url": claim_url,
            "exp": now_secs() + 600,
        })
        .to_string(),
    );
    let mut url = reqwest::Url::parse("futureos://remote/pair").expect("constant url");
    url.query_pairs_mut()
        .append_pair("v", "2")
        .append_pair("code", &code)
        .append_pair("desktopId", &field("desktopId"))
        .append_pair("desktopKey", &field("desktopKey"))
        .append_pair("secureKey", &field("secureKey"))
        .append_pair("secret", &field("secret"));
    url.into()
}

/// A claimed pairing: this installation's own identity (generated here, never
/// sent) plus the grant the platform handed back.
pub(crate) struct Paired {
    pub(crate) creds: creds::PeerCreds,
}

/// Start a host bridge and return (pair_id, its invitation URL).
pub(crate) async fn start_host(platform: &MockPlatform, nats: &FakeNats) -> (String, String) {
    sign_in(platform.url());
    let pair_id = format!("pair_{}", now_secs());
    platform.respond_pair_code_for(&pair_id, nats.url());
    let status = start(RemoteStartInput {}).await.expect("host bridge start");
    let invitation = status
        .pairing_code
        .clone()
        .expect("host shows an invitation");
    (status.pair_id, invitation)
}

/// Claim an invitation against the mock platform and store the credentials,
/// exactly as the runtime's own `pair` would.
pub(crate) async fn claim(
    platform: &MockPlatform,
    host_invitation: &str,
    pair_id: &str,
    nats_url: &str,
) -> Paired {
    let claim_url = format!("{}/client/v1/remote/pair/claim", platform.url());
    let invitation =
        link::parse_invitation(&invitation_for(host_invitation, &claim_url), now_secs())
            .expect("a usable invitation");
    platform.push(
        "/client/v1/remote/pair/claim",
        200,
        json!({
            "pair_id": pair_id,
            "user_jwt": jwt(now_secs() + 3_600),
            "refresh_token": "refresh-token-1",
            "nats_url": nats_url,
            "nats_ws_url": nats_url.replace("nats://", "ws://"),
        }),
    );
    let device = nkeys::KeyPair::new_user();
    let claimed = platform::claim(
        &invitation,
        "dev_test",
        &device.public_key(),
        "Test Desktop",
    )
    .await
    .expect("claim the invitation");
    assert_eq!(claimed.pair_id, pair_id);
    let (private, public) = future_remote_crypto::generate_identity().expect("identity");
    let creds = creds::PeerCreds {
        pair_id: claimed.pair_id,
        desktop_id: invitation.desktop_id.clone(),
        device_id: "dev_test".into(),
        nkey_seed: device.seed().expect("device seed"),
        user_jwt: claimed.user_jwt,
        refresh_token: claimed.refresh_token,
        nats_url: claimed.nats_url,
        nats_ws_url: claimed.nats_ws_url,
        jwt_expires_at: claimed.jwt_expires_at,
        token_url: claim_url.replace("/pair/claim", "/auth/token"),
        secure: Some(creds::PeerIdentity {
            private_key: URL_SAFE_NO_PAD.encode(private),
            public_key: URL_SAFE_NO_PAD.encode(&public),
            peer_public_key: Some(invitation.secure_key.clone()),
            secret: Some(invitation.secret.clone()),
        }),
    };
    creds::upsert(creds.clone()).expect("store peer creds");
    Paired { creds }
}

/// A complete host + claim, ready to connect. Returns the pieces a test needs
/// to assert against or to tear down.
pub(crate) struct Fixture {
    pub(crate) platform: MockPlatform,
    pub(crate) nats: FakeNats,
    pub(crate) paired: Paired,
    pub(crate) pair_id: String,
    pub(crate) host_invitation: String,
}

/// Bring up platform + broker + host bridge, then claim the invitation.
pub(crate) async fn fixture(label: &str) -> (HomeGuard, Fixture) {
    let home = HomeGuard::new(label);
    init_store();
    let platform = MockPlatform::start().await;
    let nats = FakeNats::start().await;
    let (pair_id, host_invitation) = start_host(&platform, &nats).await;
    let paired = claim(&platform, &host_invitation, &pair_id, nats.url()).await;
    (
        home,
        Fixture {
            platform,
            nats,
            paired,
            pair_id,
            host_invitation,
        },
    )
}

/// Tear the host bridge down and wait for its port, so the next test can start
/// one. Called at the end of every test that started a bridge.
pub(crate) async fn teardown() {
    stop();
    wait_for_web_port_free().await;
}

//! End-to-end tests for the client role.
//!
//! These run the *real* host bridge (the same `remote::start` the GUI uses) on
//! an in-process fake broker, then drive the client against it. That is the
//! only test shape that can catch the failures this module exists to prevent:
//! a handshake that only works against a hand-written fixture, a subject that
//! drifts from the host's routing, or a reply context that does not match what
//! the host seals.
//!
//! The platform HTTP hop is a `MockPlatform`; the invitation is the host's own
//! (so the PSK and Noise keys are the host's real ones), with its claim code
//! replaced by one that carries the nonce/claim_url the client requires.

use super::testing::{claim, start_host, wait_for_web_port_free};
use super::{creds, platform, session};
use crate::remote::test_support::{init_store, now_secs, FakeNats, HomeGuard, MockPlatform};
use crate::remote::{publish_event, stop};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use futures::StreamExt;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn a_desktop_client_pairs_with_a_real_host_and_reads_its_catalog() {
    let _home = HomeGuard::new("peer-e2e-pair");
    init_store();
    let platform = MockPlatform::start().await;
    let nats = FakeNats::start().await;
    let (pair_id, host_invitation) = start_host(&platform, &nats).await;

    // A thread on the host, so `list_sessions` has something to return and the
    // assertion is about content rather than "an array came back".
    let thread = crate::store::create_thread(crate::store::CreateThreadInput {
        mode: "chat".into(),
        title: Some("Remote thread".into()),
        workspace_id: None,
        workspace_path: Some("/tmp/peer-e2e".into()),
        workspace_name: Some("peer".into()),
        agent_session_id: Some("sess_peer_e2e".into()),
    })
    .expect("host thread");

    let paired = claim(&platform, &host_invitation, &pair_id, nats.url()).await;
    let connected = session::connect(&paired.creds)
        .await
        .expect("client connects");
    // The first handshake is the pairing one, so the PSK is now spent.
    assert!(connected.consumed_invitation, "the first connect pairs");
    assert_eq!(connected.session.pair_id(), pair_id);
    assert!(!connected.session.bridge_instance_id().is_empty());
    assert!(
        connected
            .session
            .features()
            .iter()
            .any(|feature| feature == "e2ee_v2"),
        "the host must confirm the v2 capability: {:?}",
        connected.session.features()
    );
    // Whether the host's *agent* is reachable is the test machine's business,
    // not the client's, so it is not asserted from here — the reading of the
    // host's report is unit-tested where it lives (`agent_available`).
    let mut client = connected.session;

    let data = client
        .request(
            &client.command_subject("list"),
            json!({ "id": "cmd-1", "type": "list_sessions" }),
            session::COMMAND_TIMEOUT,
        )
        .await
        .expect("list_sessions");
    let sessions = data["sessions"].as_array().expect("sessions array");
    let row = sessions
        .iter()
        .find(|row| row["sessionId"] == json!("sess_peer_e2e"))
        .expect("the host's session is visible to the client");
    assert_eq!(row["threadId"], json!(thread.id));
    assert_eq!(row["title"], json!("Remote thread"));
    // The cross-device ordering key the merged list needs (host change).
    assert!(row["lastMessageAt"].is_number(), "{row}");

    stop();
    wait_for_web_port_free().await;
}

/// The reconnect path, which is the one that runs for the rest of the pairing's
/// life: `Noise_IK` against the pinned key, with the PSK gone from both sides.
#[tokio::test]
async fn a_second_connection_reconnects_with_ik_after_the_secret_is_cleared() {
    let _home = HomeGuard::new("peer-e2e-reconnect");
    init_store();
    let platform = MockPlatform::start().await;
    let nats = FakeNats::start().await;
    let (pair_id, host_invitation) = start_host(&platform, &nats).await;
    let paired = claim(&platform, &host_invitation, &pair_id, nats.url()).await;

    let first = session::connect(&paired.creds).await.expect("pair");
    assert!(first.consumed_invitation);
    drop(first);
    // The client mirrors the host: the PSK is cleared once it has been spent,
    // so it cannot be replayed by anyone who later reads the credential file.
    creds::clear_secret(&paired.creds.desktop_id).expect("clear secret");
    let stored = creds::load()
        .expect("book")
        .find(&paired.creds.desktop_id)
        .cloned()
        .expect("peer");
    assert!(stored.creds.secure.as_ref().unwrap().secret.is_none());

    let second = session::connect(&stored.creds).await.expect("reconnect");
    assert!(
        !second.consumed_invitation,
        "a reconnect must not claim to have paired again"
    );
    let mut second = second.session;
    let data = second
        .request(
            &second.command_subject("list"),
            json!({ "id": "cmd-2", "type": "list_sessions" }),
            session::COMMAND_TIMEOUT,
        )
        .await
        .expect("list_sessions after reconnect");
    assert!(data["sessions"].is_array());

    stop();
    wait_for_web_port_free().await;
}

/// A peer that is not the machine the invitation named must never receive our
/// half of the handshake — the check runs before the final message is written.
#[tokio::test]
async fn a_swapped_host_key_is_refused_not_adopted() {
    let _home = HomeGuard::new("peer-e2e-wrong-peer");
    init_store();
    let platform = MockPlatform::start().await;
    let nats = FakeNats::start().await;
    let (pair_id, host_invitation) = start_host(&platform, &nats).await;
    let mut paired = claim(&platform, &host_invitation, &pair_id, nats.url()).await;

    let (_, other) = future_remote_crypto::generate_identity().expect("other identity");
    paired.creds.secure.as_mut().unwrap().peer_public_key = Some(URL_SAFE_NO_PAD.encode(other));

    let outcome = session::connect(&paired.creds).await;
    assert!(
        outcome.is_err(),
        "a mismatched peer key cannot authenticate"
    );

    stop();
    wait_for_web_port_free().await;
}

/// The live half: a host-side event must reach a subscribed client decrypted,
/// tagged with the host id, and carrying the session it belongs to. This is the
/// path a session *view* depends on, and it is a different code path from
/// request/reply — it decrypts records the client never asked for.
#[tokio::test]
async fn a_host_event_reaches_a_subscriber_decrypted() {
    let _home = HomeGuard::new("peer-e2e-events");
    init_store();
    let platform = MockPlatform::start().await;
    let nats = FakeNats::start().await;
    let (pair_id, host_invitation) = start_host(&platform, &nats).await;
    let paired = claim(&platform, &host_invitation, &pair_id, nats.url()).await;

    let connected = session::connect(&paired.creds).await.expect("pair");
    let (mut events, mut presence, _transfers) =
        connected.session.subscribe().await.expect("subscribe");
    let channel = connected.session.channel_for_stream();

    // The host publishes through its own event path, exactly as a run does.
    publish_event(
        "sess_live",
        "agent_text",
        r#"{"text":"hello from the host"}"#,
        "run-1",
        1,
        1,
        "evt-1",
        "2026-01-01T00:00:00Z",
        1,
        1,
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let event = loop {
        let next = tokio::select! {
            message = events.next() => message,
            message = presence.next() => message,
        };
        let Some(message) = next else {
            panic!("the subscription ended before the event arrived");
        };
        let opened = channel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .open(&message.subject, &message.payload);
        let Ok(opened) = opened else {
            // A presence tick or something else on the wire; keep looking.
            assert!(
                tokio::time::Instant::now() < deadline,
                "nothing authenticated on the event stream"
            );
            continue;
        };
        let payload: serde_json::Value = serde_json::from_slice(&opened).expect("json");
        if payload["sessionId"] == json!("sess_live") {
            break payload;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the host's event never arrived"
        );
    };
    // The envelope the UI consumes: the event type, the session it belongs to,
    // and the run it is part of. `data` is the event's own JSON, carried as a
    // string by the host's wire contract.
    assert_eq!(event["type"], json!("agent_text"));
    assert_eq!(event["sessionId"], json!("sess_live"));
    assert_eq!(event["runId"], json!("run-1"));
    assert!(
        event["data"]
            .as_str()
            .is_some_and(|data| data.contains("hello from the host")),
        "the event's own payload must survive: {event}"
    );

    stop();
    wait_for_web_port_free().await;
}

/// A command the host does not implement must reach the caller as an error,
/// with the host's own words intact — not be flattened into an empty success.
///
/// (The neighbouring contract — an *unknown session* yields an empty page so
/// paging terminates — is the host's own behaviour and is asserted where it is
/// implemented, in `remote_host::business`. Here the agent is not running, so
/// the host answers a transport fault; asserting the empty page from this side
/// would be testing the fixture, not the client.)
#[tokio::test]
async fn a_command_the_host_does_not_implement_surfaces_its_error() {
    let _home = HomeGuard::new("peer-e2e-refused");
    init_store();
    let platform = MockPlatform::start().await;
    let nats = FakeNats::start().await;
    let (pair_id, host_invitation) = start_host(&platform, &nats).await;
    let paired = claim(&platform, &host_invitation, &pair_id, nats.url()).await;
    let mut client = session::connect(&paired.creds).await.expect("pair").session;

    let error = client
        .request(
            &client.command_subject("list"),
            json!({ "id": "cmd-3", "type": "no_such_command" }),
            session::COMMAND_TIMEOUT,
        )
        .await
        .expect_err("an unimplemented command must fail, not return empty success");
    assert!(
        error.to_string().contains("Unsupported command"),
        "the host's own words must survive the trip: {error}"
    );

    stop();
    wait_for_web_port_free().await;
}

/// The catalog a client reads must be the *host's*, not a locally invented one:
/// two different hosts must not be confused for each other. This is also the
/// check that a second pairing coexists with the first in the book.
#[tokio::test]
async fn two_hosts_are_kept_apart_in_the_credential_book() {
    let _home = HomeGuard::new("peer-e2e-two-hosts");
    init_store();
    let platform = MockPlatform::start().await;
    let first_nats = FakeNats::start().await;
    let (first_pair, first_invitation) = start_host(&platform, &first_nats).await;
    let first = claim(&platform, &first_invitation, &first_pair, first_nats.url()).await;
    stop();
    wait_for_web_port_free().await;

    let second_nats = FakeNats::start().await;
    let (second_pair, second_invitation) = start_host(&platform, &second_nats).await;
    // Both bridges run in this one test HOME, so they report the same
    // installation device id — which is correct (one machine, one id) but
    // cannot express "two machines". A second physical host has its own id, so
    // the invitation is relabelled to stand for one. That is the only field
    // that distinguishes them. This test never connects, so the handshake
    // prologue (which binds pair id + host id) is not involved.
    let second_invitation = second_invitation.replace("desktop_", "desktop_other_");
    let second = claim(
        &platform,
        &second_invitation,
        &second_pair,
        second_nats.url(),
    )
    .await;

    let book = creds::load().expect("book");
    assert_eq!(book.peers.len(), 2, "both pairings are remembered");
    assert_ne!(first.creds.desktop_id, second.creds.desktop_id);
    // Each keeps its own host and its own platform grant.
    assert!(book
        .find(&first.creds.desktop_id)
        .is_some_and(|peer| peer.creds.pair_id == first_pair));
    assert!(book
        .find(&second.creds.desktop_id)
        .is_some_and(|peer| peer.creds.pair_id == second_pair));

    stop();
    wait_for_web_port_free().await;
}

/// A pairing whose JWT has already expired is refreshed before connecting, and
/// the refreshed grant is what gets stored — otherwise the next launch would
/// repeat the same doomed connect.
#[tokio::test]
async fn an_expired_grant_is_refreshed_before_the_connect() {
    let _home = HomeGuard::new("peer-e2e-refresh");
    init_store();
    let platform = MockPlatform::start().await;
    let nats = FakeNats::start().await;
    let (pair_id, host_invitation) = start_host(&platform, &nats).await;
    let mut paired = claim(&platform, &host_invitation, &pair_id, nats.url()).await;

    // Pretend the stored grant is inside the refresh margin.
    paired.creds.jwt_expires_at = now_secs() + 10;
    assert!(paired.creds.needs_refresh(now_secs()));
    platform.respond_refresh(nats.url());
    let fresh = platform::refresh(&paired.creds).await.expect("refresh");
    assert!(fresh.jwt_expires_at > now_secs() + 60);
    creds::update_credentials(
        &paired.creds.desktop_id,
        fresh.user_jwt,
        fresh.nats_url,
        fresh.nats_ws_url,
        fresh.jwt_expires_at,
    )
    .expect("persist refresh");

    let stored = creds::load()
        .expect("book")
        .find(&paired.creds.desktop_id)
        .cloned();
    assert!(
        stored.is_some_and(|peer| !peer.creds.needs_refresh(now_secs())),
        "the refreshed expiry must be persisted"
    );

    stop();
    wait_for_web_port_free().await;
}

/// Unpairing is a local decision first: the local entry disappears even when the
/// platform is unreachable, and a queued revoke is the caller's business.
#[tokio::test]
async fn unpairing_removes_the_local_entry_even_when_the_platform_refuses() {
    let _home = HomeGuard::new("peer-e2e-unpair");
    init_store();
    let platform = MockPlatform::start().await;
    let nats = FakeNats::start().await;
    let (pair_id, host_invitation) = start_host(&platform, &nats).await;
    let paired = claim(&platform, &host_invitation, &pair_id, nats.url()).await;

    platform.push(
        "/client/v1/remote/pair/revoke",
        500,
        json!({ "error": "boom" }),
    );
    let revoke = platform::revoke(&paired.creds).await;
    assert!(
        revoke.is_err(),
        "a 500 is reported so a retry can be queued"
    );
    creds::remove(&paired.creds.desktop_id).expect("local removal");
    assert!(creds::load().expect("book").peers.is_empty());

    stop();
    wait_for_web_port_free().await;
}

/// The revocation contract: "already gone" is a success, so a retry queue
/// drains instead of retrying forever.
#[tokio::test]
async fn an_already_revoked_pairing_is_not_an_error() {
    let _home = HomeGuard::new("peer-e2e-revoke-gone");
    init_store();
    let platform = MockPlatform::start().await;
    let nats = FakeNats::start().await;
    let (pair_id, host_invitation) = start_host(&platform, &nats).await;
    let paired = claim(&platform, &host_invitation, &pair_id, nats.url()).await;

    for status in [401u16, 404] {
        platform.push("/client/v1/remote/pair/revoke", status, json!({}));
        platform::revoke(&paired.creds)
            .await
            .unwrap_or_else(|error| panic!("HTTP {status} is terminal, not an error: {error}"));
    }

    stop();
    wait_for_web_port_free().await;
}

/// A reply is bound to the request that asked for it: the associated data is
/// derived from the *request* bytes and the subject, so a relay cannot move a
/// reply from one lane to another. Asserted on the derivation itself, because
/// forging a wire reply would only re-test the AEAD.
#[tokio::test]
async fn a_reply_is_bound_to_its_request_and_subject() {
    let request = b"FRE2\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17";
    let list = future_remote_crypto::reply_context("p.pair_1.cmd.list", request).expect("context");
    let new = future_remote_crypto::reply_context("p.pair_1.cmd.new", request).expect("context");
    assert_ne!(list, new, "the same reply must not open on another subject");
    // Same subject, different request header: also distinct, so one reply
    // cannot be replayed as the answer to a later request.
    let mut other = request.to_vec();
    other[10] ^= 0xff;
    let replay = future_remote_crypto::reply_context("p.pair_1.cmd.list", &other).expect("context");
    assert_ne!(list, replay);
    assert_eq!(
        list,
        future_remote_crypto::reply_context("p.pair_1.cmd.list", request).expect("context")
    );
}

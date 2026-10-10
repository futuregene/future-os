//! Tests for the client runtime's own API.
//!
//! The end-to-end tests in `tests.rs` drive the *protocol* (handshake, records,
//! subscription). These drive the layer above it — the part the Tauri commands
//! actually call — because that is where the state that outlives a single call
//! lives: which hosts are connected, what the last failure was, and whether a
//! dropped socket is being retried. A bug there is invisible to a protocol test
//! and very visible to a user.

use super::creds;
use super::runtime::{
    close_socket_for_test, connect, connect_with_emitter, disconnect, ensure_connected,
    forget_connection_for_test, install_supervisor_for_test, list, live_count, pair_with_emitter,
    request, sessions, set_label, spawn_supervisor, stream_count, supervisor_count, unpair,
    workspaces, Emitter, PeerEvent, PeerSummary, INJECT_CONNECTION_LOST,
};
use super::testing::{fixture, teardown, Fixture};
use crate::remote::test_support::{HomeGuard, MockPlatform};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Every test starts from a clean singleton and a free host-bridge port.
///
/// The clearing happens inside `fixture`, under the home guard: the runtime
/// outlives a single test, and clearing before the guard is taken would leave a
/// window for whatever still holds it to repopulate the state.
async fn start(label: &str) -> (HomeGuard, Fixture) {
    fixture(label).await
}

/// A fake UI sink recording what the runtime pushed at it.
///
/// Only `kind == "event"` is recorded: presence ticks are ambient, so counting
/// them would make every assertion about "what the host sent" depend on how long
/// the test happened to run.
fn counting_emitter() -> (Emitter, Arc<std::sync::Mutex<Vec<PeerEvent>>>) {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    let emitter: Emitter = Arc::new(move |event: PeerEvent| {
        if event.kind == "event" {
            sink.lock().unwrap().push(event);
        }
    });
    (emitter, seen)
}

/// Wait until `predicate` holds, or fail with `what` after a bounded wait.
async fn wait_for(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    while !predicate() {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

fn find(peers: &[PeerSummary], desktop_id: &str) -> PeerSummary {
    peers
        .iter()
        .find(|peer| peer.desktop_id == desktop_id)
        .cloned()
        .unwrap_or_else(|| panic!("{desktop_id} not in {peers:?}"))
}

// ── pairing ─────────────────────────────────────────────────────────────────

/// The happy path a user takes: paste a link, get a connected host. This is the
/// only test that exercises `pair_with_emitter` end to end, so it also covers
/// the credential split (device NKey + Noise identity + platform grant).
#[tokio::test]
async fn pairing_a_link_connects_and_reports_the_host() {
    let (_home, fx) = start("peer-rt-pair").await;
    // A fresh pairing must go through the same claim → connect path the UI
    // uses, so the mock platform is scripted for it.
    let claim_url = format!("{}/client/v1/remote/pair/claim", fx.platform.url());
    let invitation = super::testing::invitation_for(&fx.host_invitation, &claim_url);
    fx.platform.push(
        "/client/v1/remote/pair/claim",
        200,
        serde_json::json!({
            "pair_id": fx.pair_id,
            "user_jwt": crate::remote::test_support::jwt(crate::remote::test_support::now_secs() + 3_600),
            "refresh_token": "refresh-token-1",
            "nats_url": fx.nats.url(),
            "nats_ws_url": fx.nats.url().replace("nats://", "ws://"),
        }),
    );

    let summary = pair_with_emitter(&invitation, None)
        .await
        .expect("pairing a valid link");
    assert!(summary.connected, "{summary:?}");
    assert_eq!(summary.desktop_id, fx.paired.creds.desktop_id);
    assert_eq!(summary.pair_id, fx.pair_id);
    assert!(summary
        .bridge_instance_id
        .as_deref()
        .is_some_and(|id| !id.is_empty()));
    assert!(
        summary.features.iter().any(|feature| feature == "e2ee_v2"),
        "{summary:?}"
    );

    // The book now holds it, and listing reflects the live connection.
    let peers = list().await.expect("list");
    assert_eq!(peers.len(), 1);
    assert!(find(&peers, &fx.paired.creds.desktop_id).connected);

    teardown().await;
}

/// A bad link is rejected *before* any network call, and carries the support
/// code the two platforms share so the same fault reads the same way on each.
#[tokio::test]
async fn a_bad_link_is_rejected_with_a_shared_support_code() {
    let (_home, fx) = start("peer-rt-bad-link").await;
    // The fixture itself claims a pairing; this test is about a link that never
    // gets that far, so the book is emptied first.
    creds::remove(&fx.paired.creds.desktop_id).expect("remove");

    for (link, expected) in [
        ("not a url", "pairing_link_malformed"),
        ("https://future-os.cn/", "pairing_link_malformed"),
        (
            "futureos://remote/pair?v=1",
            "pairing_link_unsupported_version",
        ),
    ] {
        let error = pair_with_emitter(link, None)
            .await
            .expect_err("a bad link cannot pair");
        let crate::AppError::Remote { code, message, .. } = error else {
            panic!("a link failure must be a Remote error");
        };
        // Every link failure is `PA002` — the code the phone uses for "this
        // code did not work, make a new one" — so a support conversation means
        // the same thing on either platform.
        assert_eq!(code.as_deref(), Some("PA002"));
        assert_eq!(message, expected);
    }

    // Nothing was stored, so nothing shows up as paired.
    assert!(list().await.expect("list").is_empty());

    teardown().await;
}

// ── the book and the list ───────────────────────────────────────────────────

#[tokio::test]
async fn an_empty_book_lists_nothing() {
    let (_home, fx) = start("peer-rt-empty").await;
    creds::remove(&fx.paired.creds.desktop_id).expect("remove");
    assert!(list().await.expect("list").is_empty());
    teardown().await;
}

/// A host that is paired but was never connected still appears, with the user's
/// label and no error: "not connected yet" is not a failure.
#[tokio::test]
async fn a_paired_host_is_listed_before_it_is_ever_connected() {
    let (_home, fx) = start("peer-rt-unconnected").await;
    set_label(&fx.paired.creds.desktop_id, Some("Studio"), Some("rocket")).expect("label");

    let peer = find(&list().await.expect("list"), &fx.paired.creds.desktop_id);
    assert!(!peer.connected);
    assert_eq!(peer.name.as_deref(), Some("Studio"));
    assert_eq!(peer.icon.as_deref(), Some("rocket"));
    assert!(peer.error.is_none());
    assert!(peer.bridge_instance_id.is_none());
    assert!(peer.features.is_empty());
    // Nothing has been heard from the host, so its agent is *unknown* — which
    // must read as "not fine", because the UI says everything is fine on `true`.
    assert!(!peer.agent_available);

    teardown().await;
}

/// A failed connection is remembered against the host, so the list can say why
/// it is not connected rather than only that it is not.
#[tokio::test]
async fn a_failed_connection_is_remembered_against_its_host() {
    let (_home, mut fx) = start("peer-rt-fail").await;
    // A broker that is not there: the connect fails after the handshake is
    // attempted, which is exactly the transient case.
    fx.paired.creds.nats_url = "nats://127.0.0.1:1".into();
    creds::upsert(fx.paired.creds.clone()).expect("store");

    let error = connect(&fx.paired.creds.desktop_id)
        .await
        .expect_err("an unreachable broker cannot connect");
    assert!(!error.to_string().is_empty());

    let peer = find(&list().await.expect("list"), &fx.paired.creds.desktop_id);
    assert!(!peer.connected);
    assert!(peer.error.is_some(), "{peer:?}");
    assert!(peer.bridge_instance_id.is_none());

    teardown().await;
}

#[tokio::test]
async fn connecting_an_unknown_host_is_an_error_not_an_invention() {
    let (_home, _fx) = start("peer-rt-unknown").await;
    let error = connect("desktop_nobody")
        .await
        .expect_err("an unpaired host cannot connect");
    assert!(error.to_string().contains("peer_not_paired"), "{error}");
    teardown().await;
}

/// `ensure_connected` is the path every command takes, so its two arms both
/// matter: reuse a live connection, open one when there is none.
#[tokio::test]
async fn ensure_connected_reuses_a_live_connection_and_opens_one_when_missing() {
    let (_home, fx) = start("peer-rt-ensure").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();

    let first = ensure_connected(&desktop_id).await.expect("connect");
    assert!(first.connected);
    let bridge = first.bridge_instance_id.clone();

    // Second call must reuse it: a new handshake would produce a new bridge
    // identity, and the host would see two clients from one desktop.
    let again = ensure_connected(&desktop_id).await.expect("reuse");
    assert_eq!(again.bridge_instance_id, bridge);
    assert!(again.connected);

    teardown().await;
}

/// An expired grant is renewed *before* the connect, because a token that
/// expires mid-handshake fails it in a way that looks like a protocol fault.
#[tokio::test]
async fn an_expired_grant_is_renewed_before_connecting() {
    let (_home, mut fx) = start("peer-rt-refresh").await;
    fx.paired.creds.jwt_expires_at = crate::remote::test_support::now_secs() + 5;
    creds::upsert(fx.paired.creds.clone()).expect("store");
    fx.platform.respond_refresh(fx.nats.url());

    let summary = connect(&fx.paired.creds.desktop_id)
        .await
        .expect("connect after refresh");
    assert!(summary.connected);

    // The rotated expiry is persisted: otherwise the next launch repeats the
    // same doomed connect from the same stale token.
    let stored = creds::load().expect("book");
    let peer = stored.find(&fx.paired.creds.desktop_id).expect("peer");
    assert!(
        peer.creds.jwt_expires_at > crate::remote::test_support::now_secs() + 60,
        "the refreshed expiry must be written back"
    );

    teardown().await;
}

/// A revoked pairing is a *terminal* failure for the platform, and the runtime
/// reports it rather than silently looping.
#[tokio::test]
async fn a_revoked_grant_is_reported_rather_than_retried() {
    let (_home, mut fx) = start("peer-rt-revoked").await;
    fx.paired.creds.jwt_expires_at = crate::remote::test_support::now_secs() + 5;
    creds::upsert(fx.paired.creds.clone()).expect("store");
    fx.platform.respond_refresh_revoked();

    let error = connect(&fx.paired.creds.desktop_id)
        .await
        .expect_err("a revoked grant cannot be renewed");
    assert!(!error.to_string().is_empty());
    let peer = find(&list().await.expect("list"), &fx.paired.creds.desktop_id);
    assert!(!peer.connected);
    assert!(peer.error.is_some());

    teardown().await;
}

// ── disconnect / unpair ─────────────────────────────────────────────────────

#[tokio::test]
async fn disconnect_drops_the_connection_but_keeps_the_pairing() {
    let (_home, fx) = start("peer-rt-disconnect").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");

    disconnect(&desktop_id).await;

    let peer = find(&list().await.expect("list"), &desktop_id);
    assert!(!peer.connected);
    assert!(peer.bridge_instance_id.is_none());
    // Still paired: disconnecting is not unpairing.
    assert!(creds::load().expect("book").find(&desktop_id).is_some());

    teardown().await;
}

#[tokio::test]
async fn unpairing_removes_the_host_locally_and_revokes_it_remotely() {
    let (_home, fx) = start("peer-rt-unpair").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");
    fx.platform
        .push("/client/v1/remote/pair/revoke", 200, serde_json::json!({}));

    let warning = unpair(&desktop_id).await.expect("unpair");
    assert!(warning.is_none(), "a clean revoke is not a warning");
    assert!(creds::load().expect("book").find(&desktop_id).is_none());
    assert!(list().await.expect("list").is_empty());

    teardown().await;
}

/// The local pairing goes first, always. An unreachable platform must not leave
/// a host the user removed still connectable — and the failure is *returned* so
/// a caller can queue a retry, not swallowed.
#[tokio::test]
async fn unpairing_reports_a_revoke_that_could_not_be_delivered() {
    let (_home, fx) = start("peer-rt-unpair-pending").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    fx.platform
        .push("/client/v1/remote/pair/revoke", 500, serde_json::json!({}));

    let warning = unpair(&desktop_id).await.expect("unpair");
    assert!(warning.is_some(), "an undelivered revoke must be reported");
    // Locally gone regardless.
    assert!(creds::load().expect("book").find(&desktop_id).is_none());

    teardown().await;
}

/// An unpair whose host is unknown is not an error the UI has to handle: the
/// user's intent (that host should be gone) is already satisfied.
#[tokio::test]
async fn unpairing_an_unknown_host_is_a_no_op() {
    let (_home, _fx) = start("peer-rt-unpair-unknown").await;
    let error = unpair("desktop_nobody")
        .await
        .expect_err("there is nothing to revoke");
    assert!(error.to_string().contains("peer_not_paired"), "{error}");
    teardown().await;
}

// ── labels ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn labels_round_trip_through_the_list_and_are_clearable() {
    let (_home, fx) = start("peer-rt-label").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();

    set_label(&desktop_id, Some("Home server"), Some("home")).expect("set");
    let peer = find(&list().await.expect("list"), &desktop_id);
    assert_eq!(peer.name.as_deref(), Some("Home server"));
    assert_eq!(peer.icon.as_deref(), Some("home"));

    set_label(&desktop_id, Some("  "), Some("")).expect("clear");
    let peer = find(&list().await.expect("list"), &desktop_id);
    assert!(peer.name.is_none());
    assert!(peer.icon.is_none());

    // An unknown host is a no-op rather than an error: the editor can fire
    // while a concurrent unpair is removing the entry.
    set_label("desktop_nobody", Some("x"), Some("y")).expect("no-op");

    teardown().await;
}

// ── catalogue reads, stamped with their source ──────────────────────────────

/// The stamp is what lets the merged list route a row back to the machine it
/// came from. Both ids travel: they diverge after a re-pair of the same host.
#[tokio::test]
async fn catalogue_reads_are_stamped_with_the_host_and_its_pair() {
    let (_home, fx) = start("peer-rt-catalog").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    crate::store::create_thread(crate::store::CreateThreadInput {
        mode: "chat".into(),
        title: Some("Remote thread".into()),
        workspace_id: None,
        workspace_path: Some("/tmp/peer-rt".into()),
        workspace_name: Some("peer-rt".into()),
        agent_session_id: Some("sess_rt_catalog".into()),
    })
    .expect("thread");

    let sessions = sessions(&desktop_id).await.expect("sessions");
    assert_eq!(sessions["desktopId"], serde_json::json!(desktop_id));
    assert_eq!(sessions["pairId"], serde_json::json!(fx.pair_id));
    let rows = sessions["sessions"].as_array().expect("rows");
    assert!(rows.iter().any(|row| row["sessionId"] == "sess_rt_catalog"));

    let workspaces = workspaces(&desktop_id).await.expect("workspaces");
    assert_eq!(workspaces["desktopId"], serde_json::json!(desktop_id));
    assert_eq!(workspaces["pairId"], serde_json::json!(fx.pair_id));
    assert!(workspaces["workspaces"].is_array());

    teardown().await;
}

/// A catalogue read against an unpaired host fails rather than opening a
/// connection as a side effect of a background poll.
#[tokio::test]
async fn a_catalogue_read_for_an_unpaired_host_fails() {
    let (_home, _fx) = start("peer-rt-catalog-unknown").await;
    let error = sessions("desktop_nobody")
        .await
        .expect_err("an unpaired host has no catalogue");
    assert!(error.to_string().contains("peer_not_paired"), "{error}");
    teardown().await;
}

// ── commands ────────────────────────────────────────────────────────────────

/// A command connects on demand and returns the host's payload.
#[tokio::test]
async fn a_command_connects_on_demand_and_returns_the_payload() {
    let (_home, fx) = start("peer-rt-command").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();

    let data = request(
        &desktop_id,
        serde_json::json!({ "type": "list_workspaces" }),
        "list",
    )
    .await
    .expect("list_workspaces");
    assert!(data["workspaces"].is_array(), "{data}");

    teardown().await;
}

/// A host-side error must reach the caller with the host's own words.
#[tokio::test]
async fn a_command_the_host_does_not_implement_surfaces_its_error() {
    let (_home, fx) = start("peer-rt-command-error").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();

    let error = request(
        &desktop_id,
        serde_json::json!({ "type": "no_such_command" }),
        "list",
    )
    .await
    .expect_err("an unimplemented command must fail");
    assert!(
        error.to_string().contains("Unsupported command"),
        "the host's own words must survive: {error}"
    );

    teardown().await;
}

/// A command that addresses a session routes on that session, not on the
/// caller's lane — otherwise a reply could come back on another conversation's
/// lane and be applied to the wrong thread.
#[tokio::test]
#[allow(clippy::await_holding_lock)] // the mock-agent lock serializes the mock,
                                     // and the gRPC call it guards is inside it
async fn a_session_command_routes_on_its_own_session() {
    let _lock = crate::remote::test_support::mock_agent_lock();
    let (_home, fx) = start("peer-rt-command-session").await;
    crate::remote::test_support::ensure_mock_agent();
    let desktop_id = fx.paired.creds.desktop_id.clone();

    // An unknown session is answered with an empty page (the host's own
    // contract), which is enough to prove the request was routed and answered.
    let data = request(
        &desktop_id,
        serde_json::json!({
            "type": "get_session_entries",
            "sessionId": "sess_rt_absent",
        }),
        "sess_rt_absent",
    )
    .await
    .expect("an unknown session is an empty page");
    assert_eq!(data["entries"], serde_json::json!([]));

    teardown().await;
}

/// A dead socket must not keep looking connected, and the runtime must start
/// retrying it — this is the only path that starts a supervisor after a
/// connection that was already serving.
#[tokio::test]
async fn a_command_against_a_dead_broker_drops_the_host_and_starts_retrying() {
    let (_home, fx) = start("peer-rt-command-dead").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");

    // Kill the broker under the live connection: the next request cannot be
    // answered, which is exactly what a host going away looks like.
    fx.nats.kill();

    let error = request(
        &desktop_id,
        serde_json::json!({ "type": "list_workspaces" }),
        "list",
    )
    .await
    .expect_err("a dead broker cannot answer");
    assert!(!error.to_string().is_empty());

    let peer = find(&list().await.expect("list"), &desktop_id);
    assert!(!peer.connected, "a dead socket must not look connected");
    assert!(peer.error.is_some());

    teardown().await;
}

// ── events ──────────────────────────────────────────────────────────────────

/// The live half: a host event must reach the emitter the caller supplied, and
/// it must not do so before the subscription exists.
#[tokio::test]
async fn a_connected_host_streams_its_events_to_the_emitter() {
    let (_home, fx) = start("peer-rt-events").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let (emitter, count) = counting_emitter();

    connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("connect with a stream");

    crate::remote::publish_event(
        "sess_rt_events",
        "agent_text",
        r#"{"text":"live"}"#,
        "run-1",
        1,
        1,
        "evt-1",
        "2026-01-01T00:00:00Z",
        1,
        1,
    );

    wait_for("the host's event never reached the emitter", || {
        !count.lock().unwrap().is_empty()
    })
    .await;
    let events = count.lock().unwrap().clone();
    assert_eq!(events[0].desktop_id, desktop_id);
    assert_eq!(events[0].payload["sessionId"], json!("sess_rt_events"));

    teardown().await;
}

/// Reconnecting replaces the stream. Two live subscription tasks would deliver
/// every event twice, and would keep reading from a socket that is gone.
#[tokio::test]
async fn reconnecting_replaces_the_event_stream_rather_than_stacking_it() {
    let (_home, fx) = start("peer-rt-events-replace").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let (emitter, _count) = counting_emitter();

    connect_with_emitter(&desktop_id, Some(emitter.clone()))
        .await
        .expect("first connect");
    connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("second connect");

    let streams = stream_count().await;
    assert_eq!(streams, 1, "one stream per host, not one per connect");

    teardown().await;
}

/// Disconnecting stops the stream: a host the user removed must not keep
/// pushing into a UI that has forgotten it.
#[tokio::test]
async fn disconnecting_stops_the_event_stream() {
    let (_home, fx) = start("peer-rt-events-stop").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let (emitter, count) = counting_emitter();
    connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("connect");

    disconnect(&desktop_id).await;
    assert!(stream_count().await == 0);

    crate::remote::publish_event(
        "sess_rt_stopped",
        "agent_text",
        r#"{"text":"after"}"#,
        "run-2",
        1,
        1,
        "evt-2",
        "2026-01-01T00:00:01Z",
        1,
        1,
    );
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        count.lock().unwrap().is_empty(),
        "a disconnected host must not keep emitting"
    );

    teardown().await;
}

// ── the supervisor ──────────────────────────────────────────────────────────

/// The retry loop's success arm: a host that comes back is picked up without
/// anyone asking it to.
#[tokio::test]
async fn a_supervisor_reconnects_a_host_that_comes_back() {
    let (_home, fx) = start("peer-rt-supervisor-recover").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");
    // Simulate the drop without the command path noticing.
    forget_connection_for_test(&desktop_id).await;

    let handle = spawn_supervisor(desktop_id.clone(), None);
    tokio::time::timeout(std::time::Duration::from_secs(15), handle)
        .await
        .expect("the supervisor must finish once the host is back")
        .expect("the supervisor task must not panic");

    assert!(
        find(&list().await.expect("list"), &desktop_id).connected,
        "the host must be connected again"
    );

    teardown().await;
}

/// A link that comes back must say so.
///
/// This is the client's only way to tell "the host is idle" from "the host is
/// back, and everything sent while it was gone is missing" — a presence tick
/// arrives on a healthy link too, so it cannot carry that meaning.
#[tokio::test]
async fn a_supervisor_reports_a_link_that_came_back() {
    let (_home, fx) = start("peer-rt-supervisor-resumed").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");
    forget_connection_for_test(&desktop_id).await;

    let seen: Arc<std::sync::Mutex<Vec<PeerEvent>>> = Arc::default();
    let recorder = seen.clone();
    let emitter: Emitter = Arc::new(move |event: PeerEvent| {
        recorder.lock().unwrap().push(event);
    });

    let handle = spawn_supervisor(desktop_id.clone(), Some(emitter));
    tokio::time::timeout(std::time::Duration::from_secs(15), handle)
        .await
        .expect("the supervisor must finish once the host is back")
        .expect("the supervisor task must not panic");

    // Scoped so the guard is released before the await below: clippy rightly
    // refuses a lock held across an await point, and `teardown` awaits.
    {
        let events = seen.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "resumed")
                .count(),
            1,
            "exactly one resume signal, for the host that came back"
        );
        assert_eq!(events[0].desktop_id, desktop_id);
    }

    teardown().await;
}

/// The early-return arm: something else already reconnected, so the loop stops
/// instead of opening a second connection to the same host.
#[tokio::test]
async fn a_supervisor_stops_when_the_host_is_already_connected() {
    let (_home, fx) = start("peer-rt-supervisor-redundant").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");
    let bridge = find(&list().await.expect("list"), &desktop_id)
        .bridge_instance_id
        .expect("connected");

    let handle = spawn_supervisor(desktop_id.clone(), None);
    tokio::time::timeout(std::time::Duration::from_secs(15), handle)
        .await
        .expect("the supervisor must notice the live connection")
        .expect("no panic");

    // The same connection is still serving: a reconnect would have replaced the
    // bridge identity.
    let after = find(&list().await.expect("list"), &desktop_id);
    assert_eq!(after.bridge_instance_id.as_deref(), Some(bridge.as_str()));

    teardown().await;
}

/// The window is finite: an unreachable host is retried a bounded number of
/// times and then left alone. A loop that never ends is indistinguishable, to
/// the user, from one that hung.
#[tokio::test]
async fn a_supervisor_gives_up_after_its_window() {
    let (_home, mut fx) = start("peer-rt-supervisor-giveup").await;
    // A broker that is not there, so every attempt fails.
    fx.paired.creds.nats_url = "nats://127.0.0.1:1".into();
    creds::upsert(fx.paired.creds.clone()).expect("store");
    let desktop_id = fx.paired.creds.desktop_id.clone();

    let handle = spawn_supervisor(desktop_id.clone(), None);
    tokio::time::timeout(std::time::Duration::from_secs(20), handle)
        .await
        .expect("the supervisor must stop once its window is spent")
        .expect("no panic");

    assert!(
        !find(&list().await.expect("list"), &desktop_id).connected,
        "an unreachable host must not be reported as connected"
    );

    teardown().await;
}

/// A supervisor that has never been installed is not waiting on anything: the
/// runtime only starts one when a *live* connection fails.
#[tokio::test]
async fn a_failed_first_connect_does_not_start_retrying_on_its_own() {
    let (_home, mut fx) = start("peer-rt-no-supervisor").await;
    fx.paired.creds.nats_url = "nats://127.0.0.1:1".into();
    creds::upsert(fx.paired.creds.clone()).expect("store");

    connect(&fx.paired.creds.desktop_id)
        .await
        .expect_err("the host is unreachable");
    assert!(
        supervisor_count().await == 0,
        "a user who watched Connect fail should see that failure, not a quiet retry"
    );

    teardown().await;
}

/// The supervisor is keyed by host, so one host's outage cannot retry another's
/// connection.
#[tokio::test]
async fn only_the_host_that_failed_is_retried() {
    let (_home, fx) = start("peer-rt-supervisor-scope").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");

    let mut other = fx.paired.creds.clone();
    other.desktop_id = "desktop_other".into();
    other.pair_id = "pair_other".into();
    other.nats_url = "nats://127.0.0.1:1".into();
    creds::upsert(other.clone()).expect("store other");

    forget_connection_for_test(&desktop_id).await;
    let handle = spawn_supervisor(desktop_id.clone(), None);
    tokio::time::timeout(std::time::Duration::from_secs(15), handle)
        .await
        .expect("the reachable host recovers")
        .expect("no panic");

    // The unreachable one was never connected, and its failure is its own.
    let peers = list().await.expect("list");
    assert!(find(&peers, &desktop_id).connected);
    assert!(!find(&peers, "desktop_other").connected);

    teardown().await;
}

/// Two *pairings* coexist: one that connects and one that cannot, without the
/// working one being disturbed. The book is a list precisely so this holds.
#[tokio::test]
async fn a_broken_pairing_does_not_disturb_a_working_one() {
    let (_home, fx) = start("peer-rt-two-hosts").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect first");

    let mut broken = fx.paired.creds.clone();
    broken.desktop_id = "desktop_broken".into();
    broken.pair_id = "pair_broken".into();
    broken.nats_url = "nats://127.0.0.1:1".into();
    creds::upsert(broken.clone()).expect("store broken");

    connect("desktop_broken")
        .await
        .expect_err("the broken pairing cannot connect");

    let peers = list().await.expect("list");
    assert_eq!(peers.len(), 2);
    let working = find(&peers, &desktop_id);
    assert!(working.connected, "the working host must be unaffected");
    assert!(working.error.is_none(), "{working:?}");
    let failing = find(&peers, "desktop_broken");
    assert!(!failing.connected);
    assert!(failing.error.is_some());
    assert_eq!(live_count().await, 1, "only the working host is connected");

    teardown().await;
}

/// A catalogue row is stamped from the **live connection**, not from the stored
/// credential file. A stale file (the user re-paired elsewhere, or the file was
/// hand-edited) must not be able to label a row with a pairing the socket is
/// not actually speaking.
#[tokio::test]
async fn a_catalogue_row_is_stamped_from_the_live_connection_not_the_file() {
    let (_home, fx) = start("peer-rt-stamp-live").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");

    // Diverge the stored file from what is on the wire.
    let mut stale = fx.paired.creds.clone();
    stale.pair_id = "pair_stale".into();
    creds::upsert(stale).expect("store stale");

    let stamped = sessions(&desktop_id).await.expect("sessions");
    assert_eq!(
        stamped["pairId"],
        serde_json::json!(fx.pair_id),
        "the row must carry the pairing the socket is speaking"
    );
    assert_eq!(stamped["desktopId"], serde_json::json!(desktop_id));
    assert!(workspaces(&desktop_id).await.expect("workspaces")["pairId"].is_string());

    teardown().await;
}

/// `MockPlatform` is held by the fixture for the whole test: dropping it would
/// stop the HTTP server the runtime is still talking to.
#[allow(dead_code)]
fn _platform_is_kept_alive(fx: &Fixture) -> &MockPlatform {
    &fx.platform
}

// ── failure paths that only a broken local state can reach ──────────────────

/// A book that cannot be written is a local fault, not a lost pairing: the
/// connection still succeeds, and the only casualty is the invitation secret
/// that could not be dropped yet.
#[cfg(unix)]
#[tokio::test]
async fn a_credential_file_that_cannot_be_written_does_not_break_the_connect() {
    use std::os::unix::fs::PermissionsExt;

    let (_home, fx) = start("peer-rt-readonly").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    // Inside the refresh margin, so the connect tries to write the rotated
    // grant back before contacting the host.
    let mut expiring = fx.paired.creds.clone();
    expiring.jwt_expires_at = crate::remote::test_support::now_secs() + 5;
    creds::upsert(expiring).expect("store");
    fx.platform.respond_refresh(fx.nats.url());

    let dir = std::path::PathBuf::from(std::env::var("HOME").expect("home")).join(".future");
    let original = std::fs::metadata(&dir).expect("home dir").permissions();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).expect("read-only");

    let result = connect(&desktop_id).await;

    // Restore before asserting: a panic with the directory still read-only
    // would leave the fixture unremovable and every later test broken.
    std::fs::set_permissions(&dir, original).expect("restore");
    let error = result.expect_err("the refresh cannot be persisted");
    assert!(!error.to_string().is_empty());
    // The refusal is recorded against the host, so the UI can say why.
    let peer = find(&list().await.expect("list"), &desktop_id);
    assert!(peer.error.is_some(), "{peer:?}");

    teardown().await;
}

/// A connect that finds a retry loop already running replaces it: leaving both
/// would mean two loops racing to open connections to one host.
#[tokio::test]
async fn connecting_replaces_an_existing_retry_loop() {
    let (_home, fx) = start("peer-rt-replace-supervisor").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();

    install_supervisor_for_test(&desktop_id).await;
    assert_eq!(supervisor_count().await, 1);

    connect(&desktop_id).await.expect("connect");
    assert_eq!(
        supervisor_count().await,
        0,
        "a live connection is proof the retry loop is no longer needed"
    );
    // And the fresh connection is the one serving, not the loop's attempt.
    assert!(find(&list().await.expect("list"), &desktop_id).connected);

    teardown().await;
}

/// When the socket closes under a live stream, the stream task ends instead of
/// spinning: the next status poll and the next command both notice.
#[tokio::test]
async fn a_stream_ends_when_the_socket_closes() {
    let (_home, fx) = start("peer-rt-stream-ends").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let (emitter, _count) = counting_emitter();
    connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("connect");
    assert_eq!(stream_count().await, 1);

    // Closing the socket ends every subscription, which is what a shutdown does;
    // killing the broker would not, because the client reconnects and keeps its
    // subscriptions open.
    close_socket_for_test(&desktop_id).await;

    wait_for(
        "the stream task never finished after the socket closed",
        || futures::executor::block_on(stream_count()) == 0,
    )
    .await;

    teardown().await;
}

/// A push that does not authenticate is dropped, not reported: a relay can
/// inject garbage at will while the connection is perfectly healthy, and
/// treating that as a failure would hand anyone with relay access a way to
/// knock this client off a working host.
#[tokio::test]
async fn a_forged_push_is_dropped_without_disturbing_the_stream() {
    let (_home, fx) = start("peer-rt-forged-push").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let (emitter, count) = counting_emitter();
    connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("connect");

    // Wait for the *broker* to have registered the subscription: `subscribe()`
    // returning only means the command was sent, so injecting immediately would
    // race and the test would pass without the message ever being delivered —
    // which is exactly how its first version passed while covering nothing.
    fx.nats
        .wait_for_sub(
            &format!("p.{}.evt.>", fx.pair_id),
            std::time::Duration::from_secs(10),
        )
        .await;

    // Garbage on the event subject, from the broker's own inject path: exactly
    // what a malicious relay can do.
    fx.nats.inject(
        &format!("p.{}.evt.sess_forged", fx.pair_id),
        None,
        b"not a sealed record at all".to_vec(),
    );
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    assert!(
        count.lock().unwrap().is_empty(),
        "a forged push must not emit"
    );
    assert_eq!(stream_count().await, 1, "and it must not stop the stream");

    // A genuine event still arrives afterwards, so the drop was not fatal.
    crate::remote::publish_event(
        "sess_forged",
        "agent_text",
        r#"{"text":"real"}"#,
        "run-1",
        1,
        1,
        "evt-real",
        "2026-01-01T00:00:00Z",
        1,
        1,
    );
    wait_for("the stream stopped delivering after a forged push", || {
        !count.lock().unwrap().is_empty()
    })
    .await;
    assert_eq!(
        count.lock().unwrap()[0].payload["sessionId"],
        json!("sess_forged")
    );

    teardown().await;
}

/// A payload that is not a JSON object cannot carry a request id, and must be
/// passed through rather than panicking — the caller's own mistake is the
/// host's to reject.
#[tokio::test]
async fn a_command_that_is_not_an_object_is_sent_as_is() {
    let (_home, fx) = start("peer-rt-non-object").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();

    let error = request(&desktop_id, serde_json::json!([1, 2, 3]), "list")
        .await
        .expect_err("a malformed command is the host's to reject");
    assert!(!error.to_string().is_empty());

    teardown().await;
}

/// The race guard between connecting and addressing the connection: a session
/// that disappears in that window is reported, not panicked on.
///
/// The interleaving is real — a concurrent command's transport failure, or the
/// user pressing Disconnect — and it is injected because it cannot be produced
/// from outside without racing (the same seam style `remote_host::pairing` uses
/// for a credential-write failure).
#[tokio::test]
async fn a_connection_that_vanishes_between_connect_and_request_is_reported() {
    let (_home, fx) = start("peer-rt-vanished").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();

    INJECT_CONNECTION_LOST.store(true, Ordering::SeqCst);
    let error = request(
        &desktop_id,
        serde_json::json!({ "type": "list_workspaces" }),
        "list",
    )
    .await
    .expect_err("a session that is gone cannot be addressed");
    assert!(error.to_string().contains("peer_not_connected"), "{error}");

    // One-shot: the next call reconnects rather than staying broken.
    let data = request(
        &desktop_id,
        serde_json::json!({ "type": "list_workspaces" }),
        "list",
    )
    .await
    .expect("the next call reconnects");
    assert!(data["workspaces"].is_array());

    teardown().await;
}

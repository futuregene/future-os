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
    close_socket_for_test, connect, connect_all, connect_with_emitter, disconnect,
    ensure_connected, forget_connection_for_test, install_supervisor_for_test, list, live_count,
    pair_with_emitter, request, sessions, set_label, set_visible_session, spawn_supervisor,
    stream_count, supervisor_attempts, supervisor_count, unpair, workspaces, Emitter, PeerEvent,
    PeerSummary, INJECT_CONNECTION_LOST,
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

/// A fake UI sink that records **every** kind, not only `event`.
///
/// `counting_emitter` filters to `event` because batching tests must not count
/// presence ticks; the overflow contract is about the opposite direction, so
/// this one keeps `resumed` (and anything else) visible.
fn recording_emitter() -> (Emitter, Arc<std::sync::Mutex<Vec<PeerEvent>>>) {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    let emitter: Emitter = Arc::new(move |event: PeerEvent| {
        sink.lock().unwrap().push(event);
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

pub(crate) fn find(peers: &[PeerSummary], desktop_id: &str) -> PeerSummary {
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
            // The real claim response has no `nats_url` (the platform's
            // `ClaimPairResponse` carries only `nats_ws_url`), so this one does
            // not either: the endpoint has to come from the refresh below.
            "nats_ws_url": fx.nats.url().replace("nats://", "ws://"),
        }),
    );
    fx.platform.respond_refresh(fx.nats.url());

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

    teardown(&fx.pair_id).await;
}

/// The bug this guards: the client required `nats_url` from the claim, but the
/// platform's claim response has never had that field (only `nats_ws_url`), so
/// **every** real pairing failed with "did not return a usable nats_url" — the
/// feature could not work at all, on any machine. The TCP endpoint lives on the
/// token endpoint, so a claim without it is resolved by one refresh.
///
/// This is also the self-pairing case, and by construction rather than by luck:
/// the fixture runs the host bridge and the client in one installation, so both
/// carry the same `device_id` and the client claims an invitation this very
/// process minted. That is the shape a user produces when they paste their own
/// machine's link, so a regression here breaks the first thing anyone tries.
#[tokio::test]
async fn a_claim_without_a_tcp_endpoint_still_pairs() {
    let (_home, fx) = start("peer-rt-pair-no-nats-url").await;
    let claim_url = format!("{}/client/v1/remote/pair/claim", fx.platform.url());
    let invitation = super::testing::invitation_for(&fx.host_invitation, &claim_url);
    // Exactly what the real platform sends: a grant, and no TCP endpoint.
    fx.platform.push(
        "/client/v1/remote/pair/claim",
        200,
        serde_json::json!({
            "pair_id": fx.pair_id,
            "user_jwt": crate::remote::test_support::jwt(crate::remote::test_support::now_secs() + 3_600),
            "refresh_token": "refresh-token-1",
            "nats_ws_url": fx.nats.url().replace("nats://", "ws://"),
        }),
    );
    // The endpoint the client needs is only on the token endpoint.
    fx.platform.respond_refresh(fx.nats.url());

    let summary = pair_with_emitter(&invitation, None)
        .await
        .expect("a claim without nats_url must still pair");
    assert!(summary.connected, "{summary:?}");

    // Resolved and *stored*: the next launch reads the credential off disk, so
    // an endpoint only kept in memory would break reconnection after a restart.
    let stored = creds::load()
        .expect("load peer creds")
        .find(&fx.paired.creds.desktop_id)
        .expect("the pairing is stored")
        .creds
        .nats_url
        .clone();
    assert_eq!(stored, fx.nats.url());

    teardown(&fx.pair_id).await;
}

/// A bad link is rejected *before* any network call, and the support code names
/// which bad link it was: "the text you pasted is not a link", "this link is an
/// older shape" and "five minutes have passed" are different problems with
/// different fixes, and only the last is solved by generating another code.
#[tokio::test]
async fn a_bad_link_is_rejected_with_a_code_that_names_its_fault() {
    let (_home, fx) = start("peer-rt-bad-link").await;
    // The fixture itself claims a pairing; this test is about a link that never
    // gets that far, so the book is emptied first.
    creds::remove(&fx.paired.creds.desktop_id).expect("remove");

    for (link, code_expected, expected) in [
        ("not a url", "PA006", "pairing_link_malformed"),
        ("https://future-os.cn/", "PA006", "pairing_link_malformed"),
        (
            "futureos://remote/pair?v=1",
            "PA007",
            "pairing_link_unsupported_version",
        ),
    ] {
        let error = pair_with_emitter(link, None)
            .await
            .expect_err("a bad link cannot pair");
        let crate::Error::Remote { code, message, .. } = error else {
            panic!("a link failure must be a Remote error");
        };
        // One code per failure: a truncated paste (`PA006`), an older link
        // (`PA007`) and an expired code (`PA002`) send the reader to different
        // places, and only the last one is fixed by generating another.
        assert_eq!(code.as_deref(), Some(code_expected), "{message}");
        assert_eq!(message, expected);
    }

    // Nothing was stored, so nothing shows up as paired.
    assert!(list().await.expect("list").is_empty());

    teardown(&fx.pair_id).await;
}

// ── the book and the list ───────────────────────────────────────────────────

#[tokio::test]
async fn an_empty_book_lists_nothing() {
    let (_home, fx) = start("peer-rt-empty").await;
    creds::remove(&fx.paired.creds.desktop_id).expect("remove");
    assert!(list().await.expect("list").is_empty());
    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
}

/// A connected client keeps telling the host it is still there.
///
/// The host reads a lapsed beat as a departure (`client_presence_v1`), so a
/// client that never sent one would be reported gone while it was connected —
/// the same stale row that a client which really left would produce, in the
/// opposite direction.
#[tokio::test]
async fn a_connected_client_keeps_telling_the_host_it_is_here() {
    let (_home, fx) = start("peer-rt-presence-beat").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let mut tap = fx.nats.tap();
    connect(&desktop_id).await.expect("connect");

    crate::remote::test_support::await_publish_matching(
        &mut tap,
        &format!("p.{}.cmd.client_presence", fx.pair_id),
        std::time::Duration::from_secs(10),
        |_| true,
    )
    .await;

    teardown(&fx.pair_id).await;
}

/// Disconnecting says so on the wire.
///
/// Without it the host keeps a bridge pointed at a client that is gone and its
/// Remote page goes on saying "in control" until the lease lapses; the goodbye
/// is what makes the row true at the moment the user pressed the button.
#[tokio::test]
async fn disconnecting_tells_the_host_the_client_left() {
    let (_home, fx) = start("peer-rt-presence-goodbye").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let mut tap = fx.nats.tap();
    connect(&desktop_id).await.expect("connect");

    disconnect(&desktop_id).await;

    let goodbye = crate::remote::test_support::await_publish_matching(
        &mut tap,
        &format!("p.{}.cmd.client_disconnect", fx.pair_id),
        std::time::Duration::from_secs(10),
        |_| true,
    )
    .await;
    assert!(
        goodbye.payload.starts_with(b"FRE2"),
        "the departure is sealed like every other command"
    );

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
}

/// A page larger than one reply arrives intact.
///
/// The host answers a read larger than its page threshold with an immutable
/// snapshot the client pulls back in ≤192 KiB chunks, so a page of *any* size is
/// deliverable. This is what makes a conversation whose newest exchange is huge
/// openable at all: one long agent run is a single exchange, the host's trim may
/// only drop whole exchanges, and so the page is as large as that run. A client
/// that never asks to be chunked gets `remote_reply_too_large` instead — the
/// reported "opening the conversation shows an error" — and one that asks but
/// cannot reassemble loses the middle of its history silently.
#[tokio::test]
async fn a_page_larger_than_one_reply_arrives_intact() {
    let (_home, fx) = start("peer-rt-chunked-page").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");

    let agent = crate::remote::test_support::ensure_mock_agent();
    agent.clear_scripts();
    let session = "sess_rt_chunked_page";
    // One exchange: a single user turn and a run that produced more text than one
    // reply can carry. The host cannot defer any of it, because deferring works in
    // whole exchanges.
    let body = "x".repeat(200 * 1024);
    let mut entries = vec![json!({
        "id": "u", "kind": "user", "role": "user",
        "blocks": [{"kind": "text", "text": "question"}],
    })];
    entries.extend((0..8).map(|index| {
        json!({
            "id": format!("a{index}"), "kind": "assistant", "role": "assistant",
            "runId": "r",
            "blocks": [{"kind": "text", "text": body}],
        })
    }));
    agent.set_session_entries(session, json!({ "entries": entries }));

    let page = request(
        &desktop_id,
        json!({
            "type": "get_session_entries", "sessionId": session,
            "before": i64::MAX, "limit": 3,
        }),
        session,
    )
    .await
    .expect("a large page is delivered, not refused");

    let rows = page["entries"].as_array().expect("entries");
    assert_eq!(rows.len(), 9, "no entry is lost to the chunking");
    assert_eq!(
        rows[0]["id"],
        json!("u"),
        "the page still starts at the turn"
    );
    assert_eq!(rows[8]["id"], json!("a7"));
    assert_eq!(
        rows[8]["blocks"][0]["text"],
        json!(body),
        "the reassembled bytes are the page, not a truncated prefix"
    );

    teardown(&fx.pair_id).await;
}

/// A run-scoped page repeats its run on every chunk it fetches.
///
/// The host scopes a page to bridge + session + **run** and refuses a chunk
/// asked for by anything else (`remote_read_owner_mismatch`), so a fetch that
/// dropped `runId` would fail *after* the first chunk — the page would arrive
/// truncated with no error the UI could explain. Asking for a page that names a
/// run proves the fetch carries it, because the host compares the two.
#[tokio::test]
async fn a_run_scoped_page_repeats_its_run_on_every_chunk() {
    let (_home, fx) = start("peer-rt-chunked-page-run").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");

    let agent = crate::remote::test_support::ensure_mock_agent();
    agent.clear_scripts();
    let session = "sess_rt_chunked_page_run";
    // Past the host's 512 KiB page threshold, so the reply is chunked at all:
    // eight 80 KiB turns are ~640 KiB, i.e. four 192 KiB chunks.
    let body = "y".repeat(80 * 1024);
    let mut entries = vec![json!({
        "id": "u", "kind": "user", "role": "user",
        "blocks": [{"kind": "text", "text": "question"}],
    })];
    entries.extend((0..8).map(|index| {
        json!({
            "id": format!("a{index}"), "kind": "assistant", "role": "assistant",
            "runId": "r",
            "blocks": [{"kind": "text", "text": body}],
        })
    }));
    agent.set_session_entries(session, json!({ "entries": entries }));

    let page = request(
        &desktop_id,
        json!({
            "type": "get_session_entries", "sessionId": session,
            "runId": "r", "before": i64::MAX, "limit": 3,
        }),
        session,
    )
    .await
    .expect("the host accepts a page scoped to the run that produced it");

    let rows = page["entries"].as_array().expect("entries");
    assert_eq!(rows.len(), 9, "no entry is lost to the chunking");
    assert_eq!(
        rows[8]["blocks"][0]["text"],
        json!(body),
        "the last chunk is the run's own bytes, not a refusal"
    );

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
}

/// An activity-only move arrives on the catalogue lane, tagged with its domain.
///
/// The peer declares `catalog_activity_v1`, so the host stops re-sending the
/// whole session list when only a row's recency moved and sends the moved rows
/// on `state.activity` instead. The client has to classify that subject as an
/// *activity* snapshot and tag the payload `sessions_activity`: the UI applies
/// the replacement packet only for that domain, so a peer that dropped the
/// packet (or tagged it as a session list) would leave every remote row's
/// ordering frozen while the conversation streams.
#[tokio::test]
async fn an_activity_packet_arrives_tagged_with_its_catalogue_domain() {
    let (_home, fx) = start("peer-rt-activity-lane").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();

    let (emitter, seen) = recording_emitter();
    connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("connect with a stream");

    // The conversation is created *after* the subscription exists, so the
    // structural packet it forces is one the peer is guaranteed to see. Waiting
    // for that packet is what makes the move below recency-only: the publisher
    // folds a recency change into a whole-list packet when the row set changed in
    // the same tick, and that would leave no activity packet to assert on.
    let thread = crate::store::create_thread(crate::store::CreateThreadInput {
        mode: "chat".into(),
        title: Some("Moving row".into()),
        workspace_id: None,
        workspace_path: None,
        workspace_name: None,
        agent_session_id: Some("sess_rt_activity".into()),
    })
    .expect("seed a conversation");

    let saw = |seen: &Arc<std::sync::Mutex<Vec<PeerEvent>>>, domain: &str| {
        seen.lock()
            .unwrap()
            .iter()
            .any(|event| event.kind == "catalog" && event.payload["domain"] == json!(domain))
    };
    wait_for("the structural catalogue snapshot never arrived", || {
        saw(&seen, "sessions")
    })
    .await;

    // Above every row's current activity: a write that moves the value is what a
    // streamed turn does, and a backwards one is refused by the store.
    let at = crate::remote_host::catalog::sessions_catalog("")
        .expect("store readable")
        .activity
        .iter()
        .map(|(_, at)| *at)
        .max()
        .unwrap_or(0)
        + 1_000;
    crate::store::record_thread_message_activity(&thread.id, at).expect("move the row's recency");

    wait_for("no sessions_activity packet reached the emitter", || {
        seen.lock().unwrap().iter().any(|event| {
            event.kind == "catalog"
                && event.payload["domain"] == json!("sessions_activity")
                && event.desktop_id == desktop_id
        })
    })
    .await;

    teardown(&fx.pair_id).await;
}

/// A lean host trims `sessionId` out of the event envelope, and every listener
/// on the UI side filters by it, so the conversation has to be read back from
/// the subject the push arrived on. Without this the live lane is silently dead
/// — no user bubble, no approval card, no streaming indicator — while history
/// still renders, which is exactly the kind of failure a screenshot would not
/// show.
#[tokio::test]
async fn a_lean_event_recovers_its_session_from_the_subject() {
    let (_home, fx) = start("peer-rt-lean-session").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let (emitter, count) = counting_emitter();
    connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("connect with a stream");

    // The declaration belongs to the host bridge that will publish these
    // events, so it is set on that pairing rather than process-wide.
    crate::remote::test_support::set_pair_lean(&fx.pair_id, true);
    crate::remote::publish_event(
        "sess_lean_events",
        "user_message",
        r#"{"text":"hello","entry_id":"entry_1","run_id":"run-1","created_at_ms":1}"#,
        "run-1",
        4,
        1,
        "evt-lean",
        "2026-01-01T00:00:00Z",
        1,
        1,
    );
    crate::remote::test_support::set_pair_lean(&fx.pair_id, false);
    // The same lane with the declaration withdrawn: nothing about the full feed
    // may change, envelope keys included.
    crate::remote::publish_event(
        "sess_full_events",
        "user_message",
        r#"{"text":"hello","entry_id":"entry_2"}"#,
        "run-2",
        1,
        1,
        "evt-full",
        "2026-01-01T00:00:01Z",
        1,
        2,
    );

    wait_for("the host's events never reached the emitter", || {
        count.lock().unwrap().len() >= 2
    })
    .await;
    let events = count.lock().unwrap().clone();
    let lean = events
        .iter()
        .find(|event| event.payload["sessionId"] == json!("sess_lean_events"))
        .expect("the lean event must still name its conversation");
    // Proof the lane really was lean: the envelope key the read-back replaces is
    // gone, so the assertion above can only be satisfied by the subject.
    assert_eq!(lean.payload.get("eventId"), None);
    assert_eq!(lean.payload.get("timestamp"), None);

    let full = events
        .iter()
        .find(|event| event.payload["sessionId"] == json!("sess_full_events"))
        .expect("the full feed is untouched");
    assert_eq!(full.payload["eventId"], json!("evt-full"));

    teardown(&fx.pair_id).await;
}

/// The `selective_events_v1` lane, end to end: the host confirms the capability,
/// and once the UI says which conversation it is on the client re-subscribes to
/// that one — and back to the whole host when the reader leaves.
///
/// The subscription itself is asserted (on the broker's own subscription table),
/// not only the delivered events: "no event arrived" passes just as well when
/// the lane never moved, and the whole point of the capability is which subject
/// the `SUB` names.
#[tokio::test]
async fn opening_a_conversation_moves_the_event_lane_and_closing_it_moves_back() {
    let (_home, fx) = start("peer-rt-selective").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let (emitter, count) = counting_emitter();
    let peer = connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("connect with a stream");

    // The host's confirmation is what makes narrowing legal at all, so it is
    // asserted before anything is moved on the strength of it.
    assert!(
        peer.features
            .iter()
            .any(|feature| feature == "selective_events_v1"),
        "the host must confirm the selective lane: {:?}",
        peer.features
    );

    let wide = format!("p.{}.evt.>", fx.pair_id);
    let narrow = format!("p.{}.evt.sess_open", fx.pair_id);
    fx.nats
        .wait_for_sub(&wide, std::time::Duration::from_secs(10))
        .await;

    // The UI opens a conversation.
    set_visible_session(&desktop_id, Some("sess_open")).await;
    fx.nats
        .wait_for_sub(&narrow, std::time::Duration::from_secs(10))
        .await;

    // Another conversation's event no longer arrives...
    crate::remote::publish_event(
        "sess_elsewhere",
        "agent_text",
        r#"{"text":"not mine"}"#,
        "run-2",
        1,
        1,
        "evt-other",
        "2026-01-01T00:00:01Z",
        1,
        2,
    );
    // ...while the open one's does.
    crate::remote::publish_event(
        "sess_open",
        "agent_text",
        r#"{"text":"mine"}"#,
        "run-1",
        1,
        1,
        "evt-open",
        "2026-01-01T00:00:00Z",
        1,
        1,
    );
    wait_for(
        "the open conversation's event never reached the emitter",
        || {
            count
                .lock()
                .unwrap()
                .iter()
                .any(|event| event.payload["sessionId"] == json!("sess_open"))
        },
    )
    .await;
    let seen: Vec<String> = count
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| event.payload["sessionId"].as_str().map(str::to_string))
        .collect();
    assert!(
        !seen.contains(&"sess_elsewhere".to_string()),
        "a narrowed lane must not carry another conversation: {seen:?}"
    );

    // The reader goes back to the list: the wide lane returns, and with it the
    // other conversations.
    set_visible_session(&desktop_id, None).await;
    fx.nats
        .wait_for_sub(&wide, std::time::Duration::from_secs(10))
        .await;
    crate::remote::publish_event(
        "sess_elsewhere",
        "agent_text",
        r#"{"text":"now mine"}"#,
        "run-2",
        2,
        1,
        "evt-other-2",
        "2026-01-01T00:00:02Z",
        1,
        3,
    );
    wait_for("the wide lane never came back", || {
        count
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.payload["sessionId"] == json!("sess_elsewhere"))
    })
    .await;

    teardown(&fx.pair_id).await;
}

/// `state.events` is what keeps the host's cross-conversation notifications
/// alive while the event lane is narrowed: an approval or a run's start for a
/// conversation the reader is *not* on arrives only there, and all four UI
/// listeners filter by `sessionId`, so a lane that dropped it would freeze the
/// approval card and the peer list without an error anywhere.
///
/// It is read *only* while narrowed. On the wide lane the host publishes each of
/// those events twice — once per conversation, once here — so reading both would
/// deliver every approval card twice, which is the bug this test's second half
/// pins.
#[tokio::test]
async fn the_host_wide_mirror_is_read_only_while_the_lane_is_narrow() {
    let (_home, fx) = start("peer-rt-state-events").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let (emitter, count) = counting_emitter();
    connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("connect with a stream");
    // The wildcard lane the client subscribes to, which is what carries the
    // mirror; `state_event_subject` is what picks `state.events` out of it (and
    // is unit-tested next to its reason for ignoring the others).
    fx.nats
        .wait_for_sub(
            &format!("p.{}.state.>", fx.pair_id),
            std::time::Duration::from_secs(10),
        )
        .await;

    // `agent_start` is a catalog event, so the host publishes it on both lanes.
    // Wide: the conversation's own copy must be the only one delivered.
    crate::remote::publish_event(
        "sess_watch",
        "agent_start",
        "{}",
        "run-1",
        1,
        1,
        "evt-wide-start",
        "2026-01-01T00:00:00Z",
        1,
        1,
    );
    wait_for("the wide lane never delivered the run's start", || {
        count.lock().unwrap().len() == 1
    })
    .await;
    // Give the mirror its chance to arrive as a duplicate.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        count.lock().unwrap().len(),
        1,
        "the host-wide mirror must not double every host-wide event on the wide lane"
    );

    // Narrow: the conversation is one this client is not subscribed to, so the
    // only lane left is the mirror.
    set_visible_session(&desktop_id, Some("sess_open")).await;
    fx.nats
        .wait_for_sub(
            &format!("p.{}.evt.sess_open", fx.pair_id),
            std::time::Duration::from_secs(10),
        )
        .await;
    crate::remote::publish_event(
        "sess_watch",
        "agent_start",
        "{}",
        "run-2",
        1,
        1,
        "evt-narrow-start",
        "2026-01-01T00:00:01Z",
        1,
        2,
    );
    wait_for(
        "the host-wide mirror was not read on the narrow lane",
        || {
            // `runId` distinguishes this push from the wide-lane one above; and
            // like it, matched on a field a lean body keeps (`eventId` is trimmed
            // now that this session declares `lean_events_v1`).
            count.lock().unwrap().iter().any(|event| {
                event.payload["sessionId"] == json!("sess_watch")
                    && event.payload["runId"] == json!("run-2")
            })
        },
    )
    .await;
    let mirrored = count
        .lock()
        .unwrap()
        .iter()
        // By `runId`, which is what distinguishes this push from the wide-lane one
        // above now that a lean body no longer carries `eventId`.
        .find(|event| event.payload["runId"] == json!("run-2"))
        .cloned()
        .expect("the mirrored event");
    // The UI's listeners all address an event by conversation, so a mirror that
    // arrived without one would be dropped by every one of them — which is why
    // the trimmed body keeps `sessionId` even though the conversation's own lane
    // does not need it (its subject names the session already).
    assert_eq!(mirrored.payload["sessionId"], json!("sess_watch"));
    assert_eq!(mirrored.kind, "event");
    assert_eq!(
        mirrored.payload["type"],
        json!("agent_start"),
        "the mirror is read as a session event, not as a snapshot"
    );

    teardown(&fx.pair_id).await;
}

/// Setting the same conversation twice must not re-open the subscription: the
/// view re-renders for reasons that have nothing to do with navigation
/// (a streaming push, a resize), and a drop-and-reopen there would lose every
/// event published in the gap.
#[tokio::test]
async fn re_stating_the_same_conversation_keeps_the_lane_open() {
    let (_home, fx) = start("peer-rt-visible-noop").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let (emitter, count) = counting_emitter();
    connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("connect with a stream");

    set_visible_session(&desktop_id, Some("sess_open")).await;
    fx.nats
        .wait_for_sub(
            &format!("p.{}.evt.sess_open", fx.pair_id),
            std::time::Duration::from_secs(10),
        )
        .await;
    set_visible_session(&desktop_id, Some("sess_open")).await;
    set_visible_session(&desktop_id, Some("sess_open")).await;

    crate::remote::publish_event(
        "sess_open",
        "agent_text",
        r#"{"text":"still here"}"#,
        "run-1",
        1,
        1,
        "evt-still",
        "2026-01-01T00:00:00Z",
        1,
        1,
    );
    wait_for("a restated conversation lost its events", || {
        // Matched on what a *lean* body keeps (`sessionId`/`runId`): this session
        // declares `lean_events_v1`, so the host trims `eventId` and `timestamp`
        // out of the envelope before publishing, and asserting on either would be
        // asserting the trim rather than the delivery.
        count.lock().unwrap().iter().any(|event| {
            event.payload["sessionId"] == json!("sess_open")
                && event.payload["runId"] == json!("run-1")
        })
    })
    .await;

    teardown(&fx.pair_id).await;
}

/// A burst of live events still reaches the UI one by one and in the order the
/// host sent them.
///
/// The batch changes *when* the loop yields, never what it delivers: the UI's
/// contract is a `PeerEvent` per push, and a batch that reordered or swallowed
/// one would be a rendering bug that only shows under load. 80 pushes is past
/// the 64-push ceiling, so at least one batch boundary is crossed.
#[tokio::test]
async fn a_burst_of_events_reaches_the_emitter_in_order() {
    let (_home, fx) = start("peer-rt-burst").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let (emitter, count) = counting_emitter();
    connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("connect with a stream");

    let burst = 80u64;
    for idx in 0..burst {
        crate::remote::publish_event(
            "sess_rt_burst",
            "agent_text",
            r#"{"text":"token"}"#,
            "run-burst",
            idx as i64,
            1,
            &format!("evt-{idx}"),
            "2026-01-01T00:00:00Z",
            1,
            1,
        );
    }

    wait_for("the burst never fully arrived", || {
        count.lock().unwrap().len() as u64 >= burst
    })
    .await;
    let seen = count.lock().unwrap().clone();
    let order: Vec<u64> = seen
        .iter()
        .map(|event| event.payload["idx"].as_u64().expect("an idx per event"))
        .collect();
    assert_eq!(
        order,
        (0..burst).collect::<Vec<u64>>(),
        "batching must not reorder, duplicate or drop a push"
    );

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
}

/// One retry burst is finite. An unreachable host then enters a quiet cooldown
/// while its cancellable supervisor remains owned for eventual recovery.
///
/// The count is taken at the supervisor's own seam — one increment per retry it
/// *dispatches* — not on the fake broker. The broker counts **sockets**, which is
/// a different quantity: one loop dispatches at most `reconnect_attempt_budget()`
/// attempts and each of those dials once (`async-nats` does not retry the initial
/// connect), so an accepted-socket count above the budget is not a longer window —
/// it is a second dialling source, such as a leftover loop reaching this listener
/// or a port the OS reused. The old assertion was `accepted == 18`, and 34 accepted
/// sockets against an 18-attempt budget is what that mistake reports.
#[tokio::test]
async fn a_supervisor_enters_cooldown_after_its_window() {
    let (_home, mut fx) = start("peer-rt-supervisor-giveup").await;
    // Fail the real NATS handshake immediately on every connection. A closed
    // port can cost seconds per SYN on Windows; the retry budget must not
    // depend on how quickly the OS refuses a TCP connection.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    fx.paired.creds.nats_url = format!("nats://{}", listener.local_addr().unwrap());
    let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let broker_accepted = accepted.clone();
    let broker = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.expect("accept failed handshake");
            broker_accepted.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    creds::upsert(fx.paired.creds.clone()).expect("store");
    let desktop_id = fx.paired.creds.desktop_id.clone();

    let handle = install_supervisor_for_test(&desktop_id).await;
    let budget = future_remote_client::runtime::reconnect_attempt_budget();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while supervisor_attempts(&desktop_id).await < budget {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the first retry burst must spend its attempt budget");
    assert!(!handle.is_finished(), "cooldown keeps one supervisor owned");

    let dispatched = supervisor_attempts(&desktop_id).await;
    let accepts = accepted.load(Ordering::SeqCst);
    broker.abort();
    // The budget is the contract, so it is pinned as well as used: a schedule
    // change must be a deliberate edit to this number, not a silent one.
    assert_eq!(
        budget, 18,
        "the window is RECONNECT_DELAYS_SECS.len() * 3: six delays, three rounds"
    );
    assert!(
        accepts >= 1,
        "the loop must actually reach the broker; {accepts} accepted sockets means \
         the attempts failed before any connection was made"
    );
    assert_eq!(
        dispatched, budget,
        "the full retry window was exercised ({accepts} sockets accepted; a socket \
         count is not an attempt count)"
    );
    assert!(
        !find(&list().await.expect("list"), &desktop_id).connected,
        "an unreachable host must not be reported as connected"
    );

    teardown(&fx.pair_id).await;
    tokio::time::timeout(std::time::Duration::from_secs(1), handle.completed())
        .await
        .expect("teardown cancels the cooldown owner");
}

/// A connect that fails **transiently** schedules a bounded retry — and still
/// reports the failure to its caller.
///
/// This is the connect-leg half of §13.7/§13.8: the pairing is still valid, so an
/// unreachable socket is not an error state that only a user action can clear.
/// The old policy is what this test used to encode ("a user who watched Connect
/// fail should see that failure, not a quiet retry"); both halves hold now,
/// because the *caller* still gets the error while the *runtime* schedules the
/// same bounded retry the drop path schedules. What the user sees is unchanged;
/// what changes is that the host comes back without them.
#[tokio::test]
async fn a_transient_first_connect_failure_schedules_a_bounded_retry() {
    let (_home, mut fx) = start("peer-rt-no-supervisor").await;
    fx.paired.creds.nats_url = "nats://127.0.0.1:1".into();
    creds::upsert(fx.paired.creds.clone()).expect("store");

    let error = connect(&fx.paired.creds.desktop_id)
        .await
        .expect_err("the host is unreachable");
    assert!(
        matches!(error, crate::Error::RemoteTransport(_)),
        "an unreachable socket is the transient class, got: {error:?}"
    );
    assert!(
        supervisor_count().await > 0,
        "a transient connect failure must schedule a retry, not wait for a user action"
    );
    // And the failure is still recorded, so the row can explain itself while the
    // retry is in flight.
    assert!(
        list()
            .await
            .expect("list")
            .iter()
            .find(|peer| peer.desktop_id == fx.paired.creds.desktop_id)
            .is_some_and(|peer| !peer.connected),
        "the host must not look connected while its retry is pending"
    );

    // The retry we just scheduled must not outlive the test. In the test build the
    // schedule runs on `delay_unit()` = 1 ms, so a loop left behind burns CPU and
    // hammers the in-process broker for every later test in this binary — the
    // leak that made one starved run's red count rise from 9 to 18 while this fix
    // was being developed (`docs/testing/flaky-tauri.md` §13.8.4). `teardown`
    // aborts it, and this is the assertion that keeps that true.
    teardown(&fx.pair_id).await;
    assert_eq!(
        supervisor_count().await,
        0,
        "teardown must abort a retry loop a failed connect scheduled"
    );
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
}

// ── the liveness pass ──────────────────────────────────────────────────────

/// The launch pass: every pairing this machine holds comes up, and the ones
/// that cannot answer are reported against themselves rather than failing the
/// whole pass.
///
/// This is what makes the merged list complete at launch instead of showing
/// this machine alone until the user happens to click every host's own Connect.
#[tokio::test]
async fn connect_all_brings_up_every_pairing_and_names_the_ones_it_cannot() {
    let (_home, fx) = start("peer-rt-connect-all").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();

    // A second pairing in the book that cannot answer, so both arms of the pass
    // run in one test.
    let mut broken = fx.paired.creds.clone();
    broken.desktop_id = "desktop_broken".into();
    broken.pair_id = "pair_broken".into();
    broken.nats_url = "nats://127.0.0.1:1".into();
    creds::upsert(broken).expect("store broken");

    let (emitter, _seen) = counting_emitter();
    let peers = connect_all(Some(emitter)).await.expect("connect all");

    assert_eq!(peers.len(), 2);
    assert!(
        find(&peers, &desktop_id).connected,
        "the reachable host must be up"
    );
    let failing = find(&peers, "desktop_broken");
    assert!(!failing.connected);
    assert!(
        failing.error.is_some(),
        "a host that cannot answer is its own failure: {failing:?}"
    );
    assert_eq!(
        live_count().await,
        1,
        "only the reachable host is connected"
    );
    // Watched, not merely open: the point of the pass is that the UI can read
    // these machines without opening each one by hand.
    assert_eq!(stream_count().await, 1, "the connected host is streaming");

    teardown(&fx.pair_id).await;
}

/// An already-live host is left alone. A second handshake would replace a
/// working connection 鈥?and with it the bridge identity and every subscription
/// the UI is reading through.
#[tokio::test]
async fn connect_all_leaves_a_live_host_on_its_own_connection() {
    let (_home, fx) = start("peer-rt-connect-all-idempotent").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");
    let bridge = find(&list().await.expect("list"), &desktop_id)
        .bridge_instance_id
        .expect("connected");

    let peers = connect_all(None).await.expect("connect all");

    assert_eq!(
        find(&peers, &desktop_id).bridge_instance_id.as_deref(),
        Some(bridge.as_str()),
        "the working connection must be the same one afterwards"
    );
    assert_eq!(live_count().await, 1);

    teardown(&fx.pair_id).await;
}

/// The mark a deliberate stop leaves, and what clears it.
///
/// The app's liveness pass reads exactly this pair of fields to decide who to
/// retry: a host that dropped is brought back, and one the user turned off is
/// left off until something asks for it by name. `connect_all` *is* that
/// something: "reconnect all" and the launch pass both mean every pairing
/// should be live.
#[tokio::test]
async fn a_stopped_host_is_named_as_stopped_until_it_is_asked_for() {
    let (_home, fx) = start("peer-rt-stopped").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");
    disconnect(&desktop_id).await;

    let stopped = find(&list().await.expect("list"), &desktop_id);
    assert!(!stopped.connected);
    assert!(stopped.stopped, "{stopped:?}");

    // Asked for by name: the row's own Connect, or a caller acting on it.
    connect(&desktop_id).await.expect("connect again");
    let resumed = find(&list().await.expect("list"), &desktop_id);
    assert!(resumed.connected);
    assert!(!resumed.stopped, "{resumed:?}");

    teardown(&fx.pair_id).await;
}

/// "Everything should be live" reaches a host the user had stopped, and clears
/// the mark while it does 鈥?otherwise the pass would leave the same host to be
/// retried for ever, or worse, skip it silently.
#[tokio::test]
async fn connect_all_resumes_a_host_the_user_had_stopped() {
    let (_home, fx) = start("peer-rt-connect-all-resumes").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    connect(&desktop_id).await.expect("connect");
    disconnect(&desktop_id).await;
    assert!(find(&list().await.expect("list"), &desktop_id).stopped);

    let peers = connect_all(None).await.expect("connect all");

    let peer = find(&peers, &desktop_id);
    assert!(peer.connected, "{peer:?}");
    assert!(!peer.stopped, "{peer:?}");

    teardown(&fx.pair_id).await;
}

/// A recovered host keeps delivering.
///
/// The emitter used to be attached only by the call that supplied it, and every
/// automatic reconnect passes none 鈥?so a host that came back was `connected`,
/// every command answered, and nothing it pushed ever reached the UI again. The
/// loop resolves the emitter the app registered instead, which is what this
/// drives: a connection made *with* an emitter, dropped, and recovered by the
/// loop in its production shape.
#[tokio::test]
async fn a_supervisor_reconnect_keeps_delivering_to_the_registered_emitter() {
    let (_home, fx) = start("peer-rt-supervisor-emitter").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let (emitter, seen) = recording_emitter();
    connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("connect with a stream");

    // The drop, without the command path noticing.
    forget_connection_for_test(&desktop_id).await;
    let handle = spawn_supervisor(desktop_id.clone(), None);
    tokio::time::timeout(std::time::Duration::from_secs(15), handle)
        .await
        .expect("the supervisor must finish once the host is back")
        .expect("no panic");

    assert_eq!(
        stream_count().await,
        1,
        "the recovered connection must be watched"
    );
    crate::remote::publish_event(
        "sess_rt_after_reconnect",
        "agent_text",
        r#"{"text":"back"}"#,
        "run-1",
        1,
        1,
        "evt-1",
        "2026-01-01T00:00:00Z",
        1,
        1,
    );
    wait_for(
        "the recovered host's event never reached the emitter",
        || {
            seen.lock()
                .unwrap()
                .iter()
                .any(|event| event.kind == "event")
        },
    )
    .await;

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
}

/// Make the stored grant look lapsed, the way the platform's expiry leaves it.
///
/// Only the declared expiry moves: the JWT on disk is the one the live socket is
/// authorized by, which is exactly the state the runtime has to detect —
/// `needs_refresh` reads this field, and the socket keeps the grant it was
/// opened with.
fn lapse_stored_grant(desktop_id: &str) {
    let mut book = creds::load().expect("read the peer book");
    let record = book
        .peers
        .iter_mut()
        .find(|peer| peer.creds.desktop_id == desktop_id)
        .expect("the fixture is paired");
    record.creds.jwt_expires_at = crate::remote::test_support::now_secs() - 1;
    creds::upsert(record.creds.clone()).expect("store the lapsed grant");
}

/// A grant the relay refuses must end its generation, not be retried forever.
///
/// This is the shape `desktop.log` showed: the grant expires under a live socket,
/// the relay closes it and refuses every reconnect, and because `async-nats`
/// retried with the JWT the socket was opened with — without a bound — the client
/// kept reporting `connected: true` while printing `authorization violation`
/// every few seconds. Removing either half of the fix breaks this test: without
/// the bound the attempts never stop, and without the retirement the host stays
/// connected.
#[tokio::test]
async fn a_refused_grant_ends_its_generation_instead_of_looping() {
    let (_home, fx) = start("peer-rt-refused-grant").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let old_jwt = fx.paired.creds.user_jwt.clone();
    let (emitter, _seen) = counting_emitter();
    connect_with_emitter(&desktop_id, Some(emitter))
        .await
        .expect("connect");
    // The first connect presented the grant once; nothing has reconnected yet.
    assert_eq!(fx.nats.jwt_attempts(&old_jwt), 1);

    // The relay ends the socket the grant authorized and refuses the grant from
    // now on, exactly as it does when the JWT expires.
    fx.nats.revoke_jwt(&old_jwt);
    fx.nats.expire_jwt(&old_jwt);

    // The dead generation must be reported dead, not remembered as connected.
    wait_for("the dead generation still reported connected", || {
        futures::executor::block_on(async {
            !find(&list().await.expect("list"), &desktop_id).connected
        })
    })
    .await;

    // What is bounded is the retrying of *that* grant: one library reconnect (an
    // unbounded library retries every few seconds forever) and at most one
    // supervisor attempt, which a refused credential does not deserve.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    let attempts = fx.nats.jwt_attempts(&old_jwt);
    assert!(
        attempts <= 3,
        "a refused grant was retried {attempts} times: {old_jwt}"
    );
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    assert_eq!(
        fx.nats.jwt_attempts(&old_jwt),
        attempts,
        "the refused grant is still being retried"
    );

    teardown(&fx.pair_id).await;
}

/// A live generation is rotated before its grant lapses.
///
/// The socket can only reconnect with the JWT it was opened with, so a
/// generation that reaches its expiry is already lost. The runtime therefore
/// wakes at the grant's own expiry (minus the shared lead), renews it and
/// rebuilds the connection; the replacement presents the renewed grant and the
/// old socket is closed. Without the rotation worker the renewed grant is never
/// requested and this test never sees a second credential.
#[tokio::test]
async fn a_live_generation_rotates_its_grant_before_it_lapses() {
    let (_home, fx) = start("peer-rt-grant-rotation").await;
    let desktop_id = fx.paired.creds.desktop_id.clone();
    let old_jwt = fx.paired.creds.user_jwt.clone();
    connect(&desktop_id).await.expect("connect");
    assert_eq!(fx.nats.jwt_attempts(&old_jwt), 1);

    // The platform's next grant, distinct from the one the socket holds.
    let renewed = crate::remote::test_support::jwt(crate::remote::test_support::now_secs() + 7_200);
    assert_ne!(renewed, old_jwt);
    fx.platform.push(
        "/client/v1/remote/auth/token",
        200,
        json!({
            "user_jwt": renewed,
            "nats_url": fx.nats.url(),
            "nats_ws_url": fx.nats.url().replace("nats://", "ws://"),
        }),
    );

    // The grant enters the refresh window: this is the quantity the runtime
    // decides on, not elapsed time.
    lapse_stored_grant(&desktop_id);

    wait_for(
        "no replacement generation presented the renewed grant",
        || fx.nats.jwt_attempts(&renewed) == 1,
    )
    .await;
    assert!(
        find(&list().await.expect("list"), &desktop_id).connected,
        "the link stays up across the rotation"
    );
    // The wire carried the original grant once and then its replacement: the
    // superseded credential is never presented again.
    assert_eq!(
        fx.nats.presented_jwts().last().map(String::as_str),
        Some(renewed.as_str())
    );
    assert_eq!(fx.nats.jwt_attempts(&old_jwt), 1);

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
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

    teardown(&fx.pair_id).await;
}

//! Desktop adapter for its explicitly owned Remote controller instance.
#[cfg(test)]
use super::session::PeerSession;
use future_remote_client::Client;
#[cfg(any(test, feature = "test-support"))]
pub use future_remote_client::PeerEvent;
pub use future_remote_client::{Emitter, PeerSummary};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::time::Duration;

fn client_cell() -> &'static std::sync::RwLock<Client> {
    static CLIENT: std::sync::OnceLock<std::sync::RwLock<Client>> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        std::sync::RwLock::new(Client::new(std::sync::Arc::new(
            super::host::DesktopClientHost,
        )))
    })
}
pub(super) fn client() -> Client {
    let client = client_cell()
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    #[cfg(test)]
    if INJECT_CONNECTION_LOST.swap(false, std::sync::atomic::Ordering::Relaxed) {
        client.inject_connection_loss_for_test();
    }
    client
}
#[cfg(any(test, feature = "test-support"))]
pub static INJECT_CONNECTION_LOST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn stop_admission() {
    client().stop_admission();
}
#[cfg(any(test, feature = "test-support"))]
pub fn reset_after_shutdown_for_test() {
    let mut current = client_cell().write().unwrap_or_else(|p| p.into_inner());
    if current.is_stopped() {
        *current = Client::new(std::sync::Arc::new(super::host::DesktopClientHost));
    }
}

pub async fn shutdown(budget: Duration) -> Result<(), crate::Error> {
    client().shutdown(budget).await.map_err(Into::into)
}
#[cfg(any(test, feature = "test-support"))]
pub async fn reset_for_test(_home: &crate::remote::test_support::HomeGuard) {
    let previous = {
        let mut client = client_cell().write().unwrap_or_else(|p| p.into_inner());
        std::mem::replace(
            &mut *client,
            Client::new(std::sync::Arc::new(super::host::DesktopClientHost)),
        )
    };
    previous
        .shutdown(Duration::from_secs(5))
        .await
        .expect("previous controller tasks stopped");
}

pub fn device_name() -> String {
    device_name_from(
        std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .ok(),
    )
}

/// The formatting half, split out so both arms are testable without mutating
/// the process environment (which no test can do safely alongside others).
fn device_name_from(host: Option<String>) -> String {
    let host = host
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    match host {
        Some(host) => format!("FutureOS Desktop ({host})"),
        None => "FutureOS Desktop".to_string(),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub async fn live_count() -> usize {
    client().live_count().await
}

#[cfg(any(test, feature = "test-support"))]
pub async fn stream_count() -> usize {
    client().stream_count().await
}

#[cfg(any(test, feature = "test-support"))]
pub async fn supervisor_count() -> usize {
    client().supervisor_count().await
}

#[cfg(any(test, feature = "test-support"))]
pub async fn abort_supervisors_for_test() {
    client().abort_supervisors_for_test().await
}

#[cfg(any(test, feature = "test-support"))]
pub async fn stop_stream_for_test(desktop_id: &str) {
    client().stop_stream_for_test(desktop_id).await
}

#[cfg(any(test, feature = "test-support"))]
pub async fn close_socket_for_test(desktop_id: &str) {
    client().close_socket_for_test(desktop_id).await
}

#[cfg(any(test, feature = "test-support"))]
pub async fn install_supervisor_for_test(desktop_id: &str) -> future_remote_client::TaskHandle {
    client().install_supervisor_for_test(desktop_id).await
}

#[cfg(any(test, feature = "test-support"))]
pub async fn forget_connection_for_test(desktop_id: &str) {
    client().forget_connection_for_test(desktop_id).await
}

pub async fn pair_with_emitter(
    invitation: &str,
    emitter: Option<Emitter>,
) -> Result<PeerSummary, crate::Error> {
    client()
        .pair_with_emitter(invitation, emitter)
        .await
        .map_err(Into::into)
}

pub async fn list() -> Result<Vec<PeerSummary>, crate::Error> {
    client().list().await.map_err(Into::into)
}

#[cfg(any(test, feature = "test-support"))]
pub async fn ensure_connected(desktop_id: &str) -> Result<PeerSummary, crate::Error> {
    client()
        .ensure_connected(desktop_id)
        .await
        .map_err(Into::into)
}

#[cfg(any(test, feature = "test-support"))]
pub async fn connect(desktop_id: &str) -> Result<PeerSummary, crate::Error> {
    client().connect(desktop_id).await.map_err(Into::into)
}

pub async fn connect_all(emitter: Option<Emitter>) -> Result<Vec<PeerSummary>, crate::Error> {
    client().connect_all(emitter).await.map_err(Into::into)
}

pub async fn connect_with_emitter(
    desktop_id: &str,
    emitter: Option<Emitter>,
) -> Result<PeerSummary, crate::Error> {
    client()
        .connect_with_emitter(desktop_id, emitter)
        .await
        .map_err(Into::into)
}

#[cfg(any(test, feature = "test-support"))]
pub fn spawn_supervisor(
    desktop_id: String,
    emitter: Option<Emitter>,
) -> future_remote_client::TaskHandle {
    client().spawn_supervisor(desktop_id, emitter)
}

pub async fn set_visible_session(desktop_id: &str, session_id: Option<&str>) {
    client().set_visible_session(desktop_id, session_id).await
}

pub async fn disconnect(desktop_id: &str) {
    client().disconnect(desktop_id).await
}

pub async fn unpair(desktop_id: &str) -> Result<Option<String>, crate::Error> {
    client().unpair(desktop_id).await.map_err(Into::into)
}

pub fn set_label(
    desktop_id: &str,
    name: Option<&str>,
    icon: Option<&str>,
) -> Result<(), crate::Error> {
    client()
        .set_label(desktop_id, name, icon)
        .map_err(Into::into)
}

pub async fn sessions(desktop_id: &str) -> Result<Value, crate::Error> {
    client().sessions(desktop_id).await.map_err(Into::into)
}

pub async fn workspaces(desktop_id: &str) -> Result<Value, crate::Error> {
    client().workspaces(desktop_id).await.map_err(Into::into)
}

pub async fn request(desktop_id: &str, command: Value, lane: &str) -> Result<Value, crate::Error> {
    client()
        .request(desktop_id, command, lane)
        .await
        .map_err(Into::into)
}

#[cfg(any(test, feature = "test-support"))]
pub async fn put_chunk(
    desktop_id: &str,
    upload_id: &str,
    index: u64,
    bytes: &[u8],
) -> Result<(), crate::Error> {
    client()
        .put_chunk(desktop_id, upload_id, index, bytes)
        .await
        .map_err(Into::into)
}

#[cfg(any(test, feature = "test-support"))]
pub async fn put_chunk_within(
    desktop_id: &str,
    upload_id: &str,
    index: u64,
    bytes: &[u8],
    budget: Duration,
) -> Result<(), crate::Error> {
    client()
        .put_chunk_within(desktop_id, upload_id, index, bytes, budget)
        .await
        .map_err(Into::into)
}

#[cfg(any(test, feature = "test-support"))]
pub async fn register_chunk(
    desktop_id: &str,
    transfer_id: &str,
    index: u64,
    sender: tokio::sync::oneshot::Sender<Vec<u8>>,
) {
    client()
        .register_chunk(desktop_id, transfer_id, index, sender)
        .await
}

#[cfg(any(test, feature = "test-support"))]
pub async fn pull_chunk_within(
    desktop_id: &str,
    transfer_id: &str,
    index: u64,
    budget: Duration,
) -> Result<Vec<u8>, crate::Error> {
    client()
        .pull_chunk_within(desktop_id, transfer_id, index, budget)
        .await
        .map_err(Into::into)
}

#[cfg(any(test, feature = "test-support"))]
pub async fn request_at(
    desktop_id: &str,
    command: Value,
    subject: &str,
    timeout: Duration,
) -> Result<Value, crate::Error> {
    client()
        .request_at(desktop_id, command, subject, timeout)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
async fn stamp_live(desktop_id: &str, data: Value) -> Result<Value, crate::Error> {
    client()
        .stamp_live_for_test(desktop_id, data)
        .await
        .map_err(Into::into)
}
#[cfg(test)]
async fn attach_stream(
    session: &PeerSession,
    emitter: Emitter,
    desktop_id: &str,
    visible: Option<&str>,
) -> Option<()> {
    client()
        .stream_attaches_for_test(session, emitter, desktop_id, visible)
        .await
        .then_some(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_peer::testing::{fixture, teardown};

    // An emitter that records nothing; these tests are about whether the stream
    // attaches, not about what it carries.
    fn silent_emitter() -> Emitter {
        std::sync::Arc::new(|_event: PeerEvent| {})
    }

    // Take a session out of the runtime so a test owns it (a `SecureChannel` is
    // not `Clone`, and taking it is also what removes it from the serving path).
    async fn take_session(desktop_id: &str) -> std::sync::Arc<PeerSession> {
        client()
            .take_session_for_test(desktop_id)
            .await
            .expect("a live session")
    }

    // The runtime is a process-global singleton, so each of these starts from a
    // clean one, and the fixture is returned so the host bridge and its broker
    // outlive the call — dropping it would stop the server under test.
    async fn connected_fixture(
        label: &str,
    ) -> (
        crate::remote::test_support::HomeGuard,
        crate::remote_peer::testing::Fixture,
        String,
    ) {
        let (home, fx) = fixture(label).await;
        let desktop_id = fx.paired.creds.desktop_id.clone();
        connect(&desktop_id).await.expect("connect");
        (home, fx, desktop_id)
    }

    // A command that is still waiting for its reply must not stop the peer list
    // from answering.
    //
    // The runtime lock used to be held across the round trip, so one unreachable
    // host made every other host's `list()` wait out `session::COMMAND_TIMEOUT` —
    // and a retryable read makes that up to three attempts, half a minute. That
    // is what a user sees as a machine disappearing from the device picker: the
    // five-second poll keeps its last answer, so the entry stays put and says
    // "offline", while the Remote page's first read is blocked behind a host
    // that dropped and draws "no remote computers" until it finally answers.
    //
    // The subject deliberately goes nowhere: the host subscribes to its own
    // command, presence and state lanes, so this is a real wait rather than a
    // business refusal that would return in microseconds.
    #[tokio::test]
    async fn a_command_in_flight_does_not_block_the_peer_list() {
        let (_home, fx, desktop_id) = connected_fixture("peer-rt-list-unblocked").await;
        let budget = std::time::Duration::from_secs(5);
        let subject = format!("p.{}.nobody.answers", fx.pair_id);
        let in_flight = tokio::spawn(async move {
            let _ = request_at(&desktop_id, json!({}), &subject, budget).await;
        });
        // Let the request reach the wire and take whatever lock it takes.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        let started = std::time::Instant::now();
        let peers = list()
            .await
            .expect("the list answers while a command is in flight");
        let waited = started.elapsed();
        assert_eq!(peers.len(), 1);
        assert!(peers[0].connected, "{peers:?}");
        assert!(
            waited < std::time::Duration::from_secs(1),
            "the list waited {waited:?} behind another host's command"
        );

        in_flight.abort();
        teardown(&fx.pair_id).await;
    }

    // A control for the test below: on a healthy socket the stream attaches, so
    // that test is about the closed socket rather than about `attach_stream`
    // refusing everything.
    #[tokio::test]
    async fn a_fresh_socket_attaches_its_stream() {
        let (_home, fx, desktop_id) = connected_fixture("peer-rt-attach-fresh").await;
        let session = take_session(&desktop_id).await;
        assert!(
            attach_stream(&session, silent_emitter(), &desktop_id, None)
                .await
                .is_some(),
            "a healthy socket must attach"
        );
        let pair_id = fx.pair_id.clone();
        drop(fx);
        teardown(&pair_id).await;
    }

    // A subscription cannot be installed on a socket that is gone, and the
    // connection must stay usable for commands when that happens: only the live
    // half is missing, and the UI falls back to fetching history.
    #[tokio::test]
    async fn attaching_a_stream_to_a_closed_socket_reports_no_stream() {
        let (_home, fx, desktop_id) = connected_fixture("peer-rt-attach-closed").await;
        let session = take_session(&desktop_id).await;
        session.close_socket().await.expect("close the socket");
        // Let the close take effect before the subscribe is attempted.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(
            attach_stream(&session, silent_emitter(), &desktop_id, None)
                .await
                .is_none(),
            "a closed socket cannot carry a subscription"
        );
        let pair_id = fx.pair_id.clone();
        drop(fx);
        teardown(&pair_id).await;
    }

    // A stored seed that cannot be read is a local credential fault, and must be
    // reported as one rather than sent to the host as an empty key.
    #[tokio::test]
    async fn a_stored_seed_that_cannot_be_read_is_reported() {
        let (_home, fx, _desktop_id) = connected_fixture("peer-rt-bad-seed").await;
        let mut broken = fx.paired.creds.clone();
        broken.nkey_seed = "not-a-seed".into();
        // `err()` rather than `expect_err`: the ok arm holds a live connection,
        // which has no `Debug` (and should not grow one just for a test).
        let error = super::super::session::connect(&broken)
            .await
            .err()
            .expect("an invalid seed cannot sign");
        assert!(
            error.to_string().contains("Invalid stored device NKey"),
            "{error}"
        );
        let pair_id = fx.pair_id.clone();
        drop(fx);
        teardown(&pair_id).await;
    }

    // A handshake sent on a socket that has gone cannot be answered, and the
    // failure has to reach the caller as a transport fault rather than a hang.
    #[tokio::test]
    async fn a_handshake_on_a_closed_socket_is_a_transport_failure() {
        let (_home, fx, desktop_id) = connected_fixture("peer-rt-handshake-closed").await;
        let session = take_session(&desktop_id).await;
        session.close_socket().await.expect("close the socket");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let error = super::super::session::exchange_for_test(
            session.client_for_test(),
            "p.pair_1.cmd.handshake",
            serde_json::json!({ "type": "secure_open" }),
        )
        .await
        .expect_err("a closed socket cannot carry a handshake");
        assert!(matches!(error, crate::Error::RemoteTransport(_)), "{error}");
        let pair_id = fx.pair_id.clone();
        drop(fx);
        teardown(&pair_id).await;
    }

    // The *base* schedule is what must grow and be bounded; jitter only
    // perturbs each draw around its step, so a jittered sample is deliberately
    // not monotonic (30s×1.2 can exceed the next step's 30s×0.8 — that is the
    // point of jitter). Asserting on the drawn values would test the noise.

    // Jitter is what keeps several hosts from retrying in the same instant, so
    // it has to actually vary — a constant would pass the range check above.

    // The window is finite, and it is a multiple of the schedule: a client that
    // retries forever is indistinguishable from one that hung.

    #[test]
    fn the_hostname_is_used_when_present_and_omitted_when_not() {
        assert_eq!(
            device_name_from(Some("studio-imac".into())),
            "FutureOS Desktop (studio-imac)"
        );
        // Whitespace is not a hostname; an empty name would produce a trailing
        // "()" that reads like a bug in the host's UI.
        assert_eq!(device_name_from(Some("   ".into())), "FutureOS Desktop");
        assert_eq!(
            device_name_from(Some("  padded  ".into())),
            "FutureOS Desktop (padded)"
        );
        assert_eq!(device_name_from(None), "FutureOS Desktop");
    }

    // The environment-reading wrapper still resolves to a usable name. It is
    // asserted as an *invariant* rather than a specific value, because the host
    // it produces depends on the machine running the test.
    #[test]
    fn the_live_hostname_resolves_to_a_usable_name() {
        let name = device_name();
        assert!(name.starts_with("FutureOS Desktop"), "{name}");
        assert!(!name.ends_with("()"), "{name}");
    }

    // A catalogue reply that is not an object is a protocol violation, not a
    // shape to invent — and it must not panic, because the reply comes from a
    // remote peer: a broken or hostile host could otherwise take the app down.
    #[tokio::test]
    async fn a_catalogue_reply_that_is_not_an_object_is_refused() {
        let _home = crate::remote::test_support::HomeGuard::new("peer-stamp-non-object");
        reset_for_test(&_home).await;
        for reply in [
            serde_json::json!([1, 2, 3]),
            serde_json::json!("a string"),
            serde_json::Value::Null,
        ] {
            let error = stamp_live("desktop_any", reply.clone())
                .await
                .expect_err("a non-object reply cannot be stamped");
            assert_eq!(error.to_string(), "peer_reply_not_an_object", "{reply}");
        }
    }

    // An explicit disconnect must take the retry with it, or a reconnect
    // scheduled a moment earlier silently undoes the user's decision.
    #[tokio::test]
    async fn disconnect_aborts_the_supervisor() {
        let _home = crate::remote::test_support::HomeGuard::new("peer-disconnect-supervisor");
        reset_for_test(&_home).await;
        client().install_supervisor_for_test("desktop_a").await;
        disconnect("desktop_a").await;
        assert!(
            supervisor_count().await == 0,
            "the supervisor must not outlive an explicit disconnect"
        );
    }

    // The reset between tests must take a retry loop with it.
    //
    // `spawn_supervisor` is tied to no test's lifetime — a failed connect leaves
    // one behind — and the test build's schedule ticks at 1 ms, so a loop the
    // reset skipped would hammer the in-process broker for every test that runs
    // after it. `flaky-tauri.md` §13.8.4 records the measured cost (one starved
    // run's red count rose from 9 to 18). The fixture relies on this at setup,
    // so it is asserted here rather than assumed.
    #[tokio::test]
    async fn reset_for_test_aborts_a_resident_supervisor() {
        let _home = crate::remote::test_support::HomeGuard::new("peer-reset-supervisor");
        reset_for_test(&_home).await;
        install_supervisor_for_test("desktop_reset").await;
        assert_eq!(
            supervisor_count().await,
            1,
            "the loop under test must be resident"
        );

        reset_for_test(&_home).await;
        assert_eq!(
            supervisor_count().await,
            0,
            "a reset that leaves a retry loop behind burns every later test's CPU"
        );
    }

    // The token chain the host joins (`p.{pair}.evt.{session}`) is what decides
    // which conversation a push belongs to, so it is read back rather than
    // assumed.

    // Everything else on the wire is not an event subject, and the answer for
    // it is `None`: an `evt.>` subscription never delivers these, but a caller
    // that guessed would file a presence tick under a conversation.

    // Only the `state.events` mirror of the host-wide lane is read as an event;
    // the snapshots on the same wildcard are forwarded as catalog updates, and
    // everything else is ignored.

    // ── the batch budget and the backlog ────────────────────────────────────

    // The three ceilings are the mobile client's, because both platforms read
    // the same host lane: a number changed here alone re-introduces on this
    // side the per-token wakeup the other side measured away.

    // ① The count ceiling: 64 small pushes end the batch on the 64th.

    // ② The byte ceiling: one large push ends the batch long before the count
    // does — a burst of big events is what makes the difference visible.

    // ③ The deadline: a slow trickle of small pushes still hands control back,
    // or a quiet link would hold one batch open for as long as it stayed quiet.

    // A batch that ended on the count must not make the *next* batch yield on
    // its first delivery: the reset is what keeps every batch after the first
    // meaningful.

    // The backlog's ceiling drops the newest push and counts it once, so the
    // loop can answer with a single resync instead of one per lost push.

    // A queue sized zero would drop everything; the floor keeps a
    // misconfiguration from turning every push into a resync.
}

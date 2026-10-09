//! End-to-end: one desktop app connected to several headless hosts at once.
//!
//! The hosts run as **separate processes** of this same test binary, and that is
//! not a convenience. The host bridge is a process-global singleton (one
//! `SUPERVISOR`), and `HOME` — which is where the installation's device identity
//! lives — is process-global too, so one process can only ever be one host with
//! one identity. A child process gets its own HOME, its own device id and its
//! own bridge: the same arrangement as running the headless binary on several
//! machines.
//!
//! The parent plays the desktop app: it claims every host's invitation, connects
//! to all of them at once, and checks the properties that can only break with
//! more than one host —
//!
//!   * each host's catalogue is its own, and is stamped with its own identities;
//!   * each host's events arrive under that host's identity (and the record
//!     layer would drop them if the channels were crossed, since the traffic
//!     keys differ per pairing);
//!   * a command that addresses one host cannot reach another;
//!   * losing one host leaves the others connected and answering.
//!
//! Relay and platform are the in-process fakes the rest of the suite uses. They
//! are real TCP listeners, so the child processes reach them the same way they
//! would reach a deployed relay; only the transport's TLS is out of scope here
//! (a `cfg(test)` client speaks plaintext to a local fake).
//!
//! One artefact of running several hosts on *one* machine, which real hosts on
//! separate machines do not have: the test-only web client binds a fixed port,
//! so only the first child gets it and the others log `web_bind`. That path is
//! already a non-critical warning by design (`RemoteStatus::warning_code`), and
//! nothing asserted here depends on it — but a reader seeing the log should know
//! it is expected rather than a second host failing to start.

use super::creds;
use super::runtime::{
    connect, connect_with_emitter, disconnect, list, live_count, reset_for_test, sessions, unpair,
    Emitter, PeerEvent, PeerSummary,
};
use super::testing::claim;
use crate::remote::test_support::{init_store, sign_in, FakeNats, HomeGuard, MockPlatform};
use crate::remote::{start, stop, RemoteStartInput};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The child entry point, by its full test name.
const CHILD_TEST: &str = "remote_peer::multi_host_tests::host_process_child";
/// Set only in a child, so the child entry point is a no-op in a normal run.
const HOST_ENV: &str = "PEER_E2E_HOST";
const LABEL_ENV: &str = "PEER_E2E_LABEL";
const PLATFORM_ENV: &str = "PEER_E2E_PLATFORM_URL";
const SIGNAL_ENV: &str = "PEER_E2E_SIGNAL_DIR";

/// How long a host is given to come up. Starting a bridge mints an invitation,
/// opens a socket and completes a handshake; on a loaded CI box that is not
/// instant, and a flaky timeout here would be worse than a slow test.
const HOST_TIMEOUT: Duration = Duration::from_secs(60);

// ── the child: one headless host ────────────────────────────────────────────

/// Runs a real host bridge until told to stop. Does nothing at all unless the
/// parent asked for it, so an ordinary `cargo test` run is unaffected.
#[test]
fn host_process_child() {
    if std::env::var(HOST_ENV).is_err() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("child runtime");
    runtime.block_on(run_host_process());
}

async fn run_host_process() {
    let label = env(LABEL_ENV);
    let platform_url = env(PLATFORM_ENV);
    let signals = PathBuf::from(env(SIGNAL_ENV));

    // Its own HOME: a distinct installation device id is what makes this a
    // *different* machine as far as the pairing protocol is concerned.
    let _home = HomeGuard::new(&label);
    init_store();
    sign_in(&platform_url);

    let session_id = format!("sess_{label}");
    crate::store::create_thread(crate::store::CreateThreadInput {
        mode: "chat".into(),
        title: Some(format!("host-{label}")),
        workspace_id: None,
        workspace_path: Some(format!("/tmp/host-{label}")),
        workspace_name: Some(label.clone()),
        agent_session_id: Some(session_id.clone()),
    })
    .expect("host thread");

    let status = start(RemoteStartInput {}).await.expect("host bridge start");
    let invitation = status
        .pairing_code
        .clone()
        .expect("the host shows an invitation");
    // Readiness goes through a file rather than stdout: a test binary's stdout
    // belongs to libtest, and parsing it would be a second thing to keep in sync.
    write_marker(
        &signals,
        &format!("{label}.ready"),
        &json!({
            "label": label,
            "pairId": status.pair_id,
            "desktopId": desktop_id_of(&invitation),
            "invitation": invitation,
            "sessionId": session_id,
        })
        .to_string(),
    );

    let publish_request = signals.join(format!("{label}.publish"));
    loop {
        // Consume the request: acting on it every tick would publish the same
        // event repeatedly and the parent would see duplicates.
        if publish_request.exists() {
            std::fs::remove_file(&publish_request).expect("consume the publish request");
            crate::remote::publish_event(
                &session_id,
                "agent_text",
                &json!({ "text": format!("from-{label}") }).to_string(),
                &format!("run-{label}"),
                1,
                1,
                &format!("evt_{label}"),
                "2026-01-01T00:00:00Z",
                1,
                1,
            );
            write_marker(&signals, &format!("{label}.published"), "1");
        }
        if signals.join(format!("{label}.stop")).exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    stop();
}

/// The host identity carried by the invitation the host just minted. The parent
/// checks it against what the platform's claim reports, so the two halves agree
/// about which machine this is.
fn desktop_id_of(invitation: &str) -> String {
    reqwest::Url::parse(invitation)
        .expect("invitation url")
        .query_pairs()
        .find(|(key, _)| key == "desktopId")
        .map(|(_, value)| value.into_owned())
        .expect("an invitation names its desktop")
}

fn write_marker(dir: &Path, name: &str, contents: &str) {
    std::fs::write(dir.join(name), contents).expect("write signal marker");
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required in a host child"))
}

// ── the parent: a desktop app with several hosts ────────────────────────────

/// A host child, plus the directory it signals through.
struct HostProcess {
    child: Child,
    label: String,
    signals: PathBuf,
}

impl HostProcess {
    fn spawn(label: &str, platform_url: &str, signals: &Path) -> Self {
        let exe = std::env::current_exe().expect("the running test binary");
        let child = Command::new(exe)
            .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
            .env(HOST_ENV, "1")
            .env(LABEL_ENV, label)
            .env(PLATFORM_ENV, platform_url)
            .env(SIGNAL_ENV, signals)
            // The child resolves its own HOME through `HomeGuard`; inheriting
            // this one would let it read the parent's auth and identity.
            .env_remove("HOME")
            .env_remove("USERPROFILE")
            // A sibling test may have pointed this process at its own mock
            // agent; the child must resolve its own, or none.
            .env_remove("FUTURE_AGENT_GRPC_ADDR")
            .env_remove("FUTURE_AGENT_SOCKET")
            .env_remove("FUTURE_HOME")
            // The child's stdout is libtest's; keep it for a debugging session
            // rather than trying to parse it.
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn host process");
        Self {
            child,
            label: label.to_string(),
            signals: signals.to_path_buf(),
        }
    }

    /// Wait for the host to announce itself and return what it said.
    fn wait_ready(&mut self) -> Value {
        let path = self.signals.join(format!("{}.ready", self.label));
        let deadline = std::time::Instant::now() + HOST_TIMEOUT;
        while !path.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "host {} never became ready (exit: {:?})",
                self.label,
                self.child.try_wait()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let raw = std::fs::read_to_string(&path).expect("readiness payload");
        serde_json::from_str(&raw).expect("the readiness payload is JSON")
    }

    /// Ask the host to publish one event on its own session, and wait for it to
    /// say it did — the event has to be published *after* the client subscribed,
    /// because the relay does not replay.
    fn publish(&self) {
        write_marker(&self.signals, &format!("{}.publish", self.label), "1");
        let published = self.signals.join(format!("{}.published", self.label));
        let deadline = std::time::Instant::now() + HOST_TIMEOUT;
        while !published.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "host {} never published",
                self.label
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for HostProcess {
    fn drop(&mut self) {
        // Leave no orphan bridges behind: they hold the fake broker's port and
        // would confuse the next test in this process.
        write_marker(&self.signals, &format!("{}.stop", self.label), "1");
        let deadline = std::time::Instant::now() + HOST_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A recording sink: what the app would show, condensed to the two fields the
/// assertions are about.
type Seen = Arc<Mutex<Vec<(String, String)>>>;

fn recording_emitter(seen: &Seen) -> Emitter {
    let sink = seen.clone();
    Arc::new(move |event: PeerEvent| {
        if event.kind != "event" {
            return;
        }
        let session = event
            .payload
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        sink.lock()
            .unwrap()
            .push((event.desktop_id.clone(), session));
    })
}

/// Wait for `predicate`, or fail with `what`.
async fn wait_until(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + HOST_TIMEOUT;
    while !predicate() {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn peer<'a>(peers: &'a [PeerSummary], desktop_id: &str) -> &'a PeerSummary {
    peers
        .iter()
        .find(|peer| peer.desktop_id == desktop_id)
        .unwrap_or_else(|| panic!("{desktop_id} is not in the list"))
}

const HOST_COUNT: usize = 3;

/// The headline case: three headless hosts on one app at the same time.
///
/// Multi-threaded on purpose: the parent blocks while a child starts up (and
/// while it publishes), and the child's pairing request has to be served by the
/// parent's own mock platform meanwhile. On the single-threaded flavor the two
/// would deadlock, which is a property of the test, not of the code under test.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_desktop_app_connects_to_several_headless_hosts_at_once() {
    let _home = HomeGuard::new("peer-e2e-multi-host");
    // Under the guard: the guard is what serializes these tests, so clearing
    // before it is taken leaves a window for the previous test's work to
    // repopulate the runtime.
    reset_for_test().await;
    init_store();
    let platform = MockPlatform::start().await;
    let nats = FakeNats::start().await;
    let signals = std::env::temp_dir().join(format!("futureos-peer-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&signals).expect("signal dir");

    // Each host is started (and given its scripted invitation) one at a time:
    // the mock platform answers pairing requests in order, so starting them
    // concurrently would hand one host another's invitation.
    let mut hosts = Vec::new();
    for index in 0..HOST_COUNT {
        let label = format!("host{index}");
        let pair_id = format!("pair_multi_{index}");
        platform.respond_pair_code_for(&pair_id, nats.url());
        let mut host = HostProcess::spawn(&label, platform.url(), &signals);
        let ready = host.wait_ready();
        assert_eq!(ready["label"], json!(label));
        assert_eq!(ready["pairId"], json!(pair_id));
        hosts.push((host, pair_id, ready));
    }

    // Pair and connect to every host, exactly as the UI does.
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let mut paired: Vec<(String, String, String)> = Vec::new();
    for (host, pair_id, ready) in &hosts {
        let desktop_id = {
            let invitation = ready["invitation"].as_str().expect("invitation");
            let paired = claim(&platform, invitation, pair_id, nats.url()).await;
            assert_eq!(paired.creds.desktop_id, ready["desktopId"].clone());
            let desktop_id = paired.creds.desktop_id.clone();
            connect_with_emitter(&desktop_id, Some(recording_emitter(&seen)))
                .await
                .expect("connect to this host");
            desktop_id
        };
        paired.push((
            host.label.clone(),
            desktop_id,
            ready["sessionId"].as_str().expect("session").to_string(),
        ));
    }

    // All three are connected, and each is a distinct machine.
    let peers = list().await.expect("list");
    assert_eq!(
        peers.len(),
        HOST_COUNT,
        "the book holds {} peers: {:?} (HOME={:?}, mine={:?})",
        peers.len(),
        peers
            .iter()
            .map(|peer| (
                peer.desktop_id.clone(),
                peer.pair_id.clone(),
                peer.name.clone()
            ))
            .collect::<Vec<_>>(),
        std::env::var("HOME"),
        paired
            .iter()
            .map(|(label, desktop_id, _)| (label.clone(), desktop_id.clone()))
            .collect::<Vec<_>>(),
    );
    assert_eq!(live_count().await, HOST_COUNT);
    let desktop_ids: Vec<&str> = peers.iter().map(|peer| peer.desktop_id.as_str()).collect();
    assert_eq!(
        desktop_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        HOST_COUNT,
        "every host is its own machine: {desktop_ids:?}"
    );

    // Each host's catalogue is its own: one thread, named for that host, and
    // stamped with that host's identities rather than a neighbour's.
    for (label, desktop_id, session_id) in &paired {
        let catalogue = sessions(desktop_id).await.expect("sessions");
        assert_eq!(catalogue["desktopId"], json!(desktop_id), "{label}");
        let rows = catalogue["sessions"].as_array().expect("rows");
        assert_eq!(
            rows.len(),
            1,
            "{label} serves only its own session: {rows:?}"
        );
        assert_eq!(rows[0]["sessionId"], json!(session_id), "{label}");
        assert_eq!(rows[0]["title"], json!(format!("host-{label}")), "{label}");

        // A session that belongs to another host is simply not here: the
        // isolation is in the content, not only in the label. Asserted on the
        // catalogue rather than by asking for the stranger's history, because
        // that path reaches the host's *agent* — and what is under test is the
        // client's routing, not whether each child happens to run an agent.
        for (other, _, stranger) in &paired {
            if other == label {
                continue;
            }
            assert!(
                !rows.iter().any(|row| row["sessionId"] == json!(stranger)),
                "{label} must not list {stranger}, which belongs to {other}"
            );
        }
    }

    // Every host's event reaches the app under that host's identity. A crossed
    // traffic-key channel would fail to authenticate and be dropped, so three
    // correctly attributed events is also evidence the per-host channels are
    // wired to the right sockets.
    for (host, _, _) in &hosts {
        host.publish();
    }
    // Exactly one event per host: not fewer (a host whose events were dropped,
    // which is what crossed traffic keys would look like) and not more (one
    // host's event delivered under another's identity, or a stream read twice).
    let expected: std::collections::HashSet<(String, String)> = paired
        .iter()
        .map(|(_, desktop_id, session)| (desktop_id.clone(), session.clone()))
        .collect();
    wait_until("not every host's event arrived", || {
        let seen = seen.lock().unwrap();
        expected.iter().all(|wanted| seen.contains(wanted))
    })
    .await;
    // Give a mis-delivery a chance to show up before declaring the set exact.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let received: std::collections::HashSet<(String, String)> =
        seen.lock().unwrap().iter().cloned().collect();
    assert_eq!(
        received.len(),
        HOST_COUNT,
        "one event per host, no duplicates or cross-delivery: {:?}",
        seen.lock().unwrap()
    );
    assert_eq!(
        received, expected,
        "each event must arrive tagged with the host that published it"
    );

    // Losing one host leaves the others alone — the case a single-host test
    // cannot see, because it has nothing to leave intact.
    let (lost_label, lost_id, _) = paired[0].clone();
    disconnect(&lost_id).await;
    let peers = list().await.expect("list");
    assert!(!peer(&peers, &lost_id).connected, "{lost_label}");
    for (label, desktop_id, _) in &paired[1..] {
        assert!(peer(&peers, desktop_id).connected, "{label} must stay up");
        // And still *working*, not merely reported as connected.
        let catalogue = sessions(desktop_id)
            .await
            .expect("sessions after a peer left");
        assert_eq!(catalogue["sessions"].as_array().expect("rows").len(), 1);
    }

    // Unpairing one removes only that one.
    let warning = unpair(&lost_id).await.expect("unpair");
    assert!(warning.is_none(), "the mock platform accepted the revoke");
    let peers = list().await.expect("list");
    assert_eq!(peers.len(), HOST_COUNT - 1);
    assert!(
        !peers.iter().any(|peer| peer.desktop_id == lost_id),
        "the unpaired host must be gone"
    );
    for (label, desktop_id, _) in &paired[1..] {
        assert!(peer(&peers, desktop_id).connected, "{label} must survive");
    }

    drop(hosts);
    let _ = std::fs::remove_dir_all(&signals);
}

/// A credential directory that cannot be written is housekeeping trouble, not a
/// connection failure: pairing still succeeds, and only the write that drops the
/// spent invitation secret is skipped.
///
/// It has to be driven with a host in **another process**: in one process the
/// host and the client share `HOME`, and the host writes its own pairing file at
/// exactly the same moment, so a read-only directory would break the handshake
/// for a reason this test is not about.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_spent_secret_that_cannot_be_dropped_is_only_housekeeping() {
    use std::os::unix::fs::PermissionsExt;

    let _home = HomeGuard::new("peer-e2e-readonly-home");
    reset_for_test().await;
    init_store();
    let platform = MockPlatform::start().await;
    let nats = FakeNats::start().await;
    let signals = std::env::temp_dir().join(format!("futureos-peer-ro-{}", std::process::id()));
    std::fs::create_dir_all(&signals).expect("signal dir");

    let pair_id = "pair_readonly";
    platform.respond_pair_code_for(pair_id, nats.url());
    let mut host = HostProcess::spawn("readonly", platform.url(), &signals);
    let ready = host.wait_ready();
    let invitation = ready["invitation"].as_str().expect("invitation");
    // Claiming legitimately writes the credentials; the directory becomes
    // unwritable only afterwards, so the pairing itself is not in question.
    let paired = claim(&platform, invitation, pair_id, nats.url()).await;
    let desktop_id = paired.creds.desktop_id.clone();

    let dir = PathBuf::from(std::env::var("HOME").expect("home")).join(".future");
    let original = std::fs::metadata(&dir).expect("home dir").permissions();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).expect("read-only");

    let result = connect(&desktop_id).await;

    // Restore before asserting: a panic with the directory still read-only
    // would leave the fixture unremovable and every later test broken.
    std::fs::set_permissions(&dir, original).expect("restore");

    let summary = result.expect("the connect itself succeeds");
    assert!(summary.connected, "{summary:?}");
    // The pairing still works, and the housekeeping that was skipped is visible
    // for what it is: the invitation secret is still on disk.
    let catalogue = sessions(&desktop_id).await.expect("sessions");
    assert_eq!(catalogue["desktopId"], json!(desktop_id));
    assert!(
        creds::load()
            .expect("book")
            .find(&desktop_id)
            .expect("peer")
            .creds
            .secure
            .as_ref()
            .and_then(|identity| identity.secret.clone())
            .is_some(),
        "the write that would have dropped the secret was the one skipped"
    );

    drop(host);
    let _ = std::fs::remove_dir_all(&signals);
}

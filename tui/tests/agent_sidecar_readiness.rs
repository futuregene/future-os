//! The **successful** half of the sidecar launch: a stub that spawns *and*
//! then answers the readiness handshake, which is the arm that returns
//! `Ok(Some(OwnedAgent { child }))` — and the hard-error arm beside it, where
//! the spawned stub's agent is reachable but incompatible, so the child is
//! killed and the incompatibility reported instead of an owned guard returned.
//!
//! The unit-test build cannot reach either (`ensure_agent_running` refuses to
//! launch under `cfg!(test)`), so this integration target drives the real loop.
//! Nothing real is started, and no host binary can be picked up:
//!
//! * the sidecar program is a fixture-owned stub, handed to
//!   `ensure_agent_running_within` as its `program` argument — nothing is
//!   looked up on `PATH`. The stub uses `/bin/sleep` by absolute path and a
//!   shell builtin for its sentinel, so it needs no `PATH` at all;
//! * an in-process mock Agent serves the instance's own local endpoint
//!   (`$FUTURE_HOME/run/agent.sock`) — but only **after** the stub has touched
//!   its sentinel. `ensure_agent_running` probes every 125 ms, so answering
//!   earlier would let the fixture win the race against the shell the loop just
//!   spawned and the sentinel assertion would be about timing instead of about
//!   which program ran (observed: green without instrumentation, red under
//!   `cargo llvm-cov` on a loaded host).
//!
//! Both tests mutate the process-global `FUTURE_HOME` and
//! `FUTURE_AGENT_SOCKET`, and a test target's tests run in parallel threads, so
//! they serialize on [`ENV_LOCK`].

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use future_rpc::proto::future_agent_server::{FutureAgent, FutureAgentServer};
use future_rpc::proto::{RpcCommand, RpcResponse, StreamEvent, StreamRequest};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// The probe window this fixture gives the launch loop.
///
/// The loop's *default* window is a product budget (how long a user waits for a
/// cold sidecar), and this fixture must not assert the host's speed against it:
/// the stub here is injected directly as the sidecar program, but it is still a
/// `/bin/sh` script, so what the loop is really waiting for is a fork/exec plus
/// a filesystem write, and on a loaded host that is unbounded. The window is
/// therefore supplied by the fixture, and its only job is to outlast
/// [`SENTINEL_WAIT`] so the loop is still probing while the fixture waits for
/// the child to become observable.
const LAUNCH_FIXTURE_WINDOW: Duration = Duration::from_secs(60);

/// Bound on waiting for the launched stub's sentinel **before this fixture
/// binds its endpoint**, and it is what makes the assertions below deterministic:
///
/// * the fixture must not be listening while the launch loop is still *deciding* -
///   `select_agent` treats a reachable endpoint as an agent that was already
///   there, `ensure_agent_running` then returns `Ok(None)`, and this target is
///   about exactly that;
/// * so the bind is gated on the sentinel, i.e. on the stub having *run*, and
///   because this bound is well inside [`LAUNCH_FIXTURE_WINDOW`] the loop is still
///   probing when the sentinel appears - which is why the guard is still alive
///   when the loop returns, and why "the sentinel exists" needs no wait.
///
/// Measured: the sentinel appears in 0.38-1.07 s (20 runs at load ~80). A host
/// that misses this bound by two orders of magnitude is then reported by the
/// sentinel assertion, not by the loop's window - which is the point of the pair.
const SENTINEL_WAIT: Duration = Duration::from_secs(30);

/// The probe window of the give-up fixture, and it has one hard requirement
/// beyond outlasting the stub's startup: the stub's stderr echo must be
/// *written before the loop kills the child at window end*, because a child
/// killed before its first write leaves the pipes empty and the report's
/// output tail legitimately empty - the assertion on `stuck before answering`
/// would then flake on host speed, not on the loop. Measured on an idle host,
/// the stub's fork/exec plus shell startup alone runs **~130-165 ms** (30-run
/// standalone repro; 2 of 30 runs exceeded a 250 ms window, and the same race
/// showed as intermittent empty tails under `cargo test`), and
/// [`SENTINEL_WAIT`]'s notes put stub startup at 0.38-1.07 s under load. 5 s
/// clears both with margin while keeping the give-up (the loop burns the whole
/// window probing a candidate that never answers) still fast.
const GIVE_UP_FIXTURE_WINDOW: Duration = Duration::from_secs(5);

/// The readiness handshake a real Agent answers, with the fixture's own data
/// root so `validate_readiness`'s home check passes, and a configurable version
/// so the incompatible-agent arm is reachable too.
struct ReadinessMock {
    data_root_id: String,
    version: String,
    handshakes: Arc<AtomicUsize>,
}

#[tonic::async_trait]
impl FutureAgent for ReadinessMock {
    async fn execute_command(
        &self,
        request: tonic::Request<RpcCommand>,
    ) -> Result<tonic::Response<RpcResponse>, tonic::Status> {
        let cmd = request.into_inner();
        let data = if cmd.r#type == "get_agent_readiness" {
            self.handshakes.fetch_add(1, Ordering::SeqCst);
            serde_json::json!({
                "version": self.version,
                "agentInstanceId": "sidecar-launch-fixture",
                "protocolVersion": future_rpc::connection::AGENT_PROTOCOL_VERSION,
                "dataRootId": self.data_root_id,
                "storageSchemaVersion": future_rpc::connection::AGENT_STORAGE_SCHEMA_VERSION,
            })
            .to_string()
        } else {
            "{}".to_string()
        };
        Ok(tonic::Response::new(RpcResponse {
            id: cmd.id,
            r#type: "response".into(),
            command: cmd.r#type,
            success: true,
            data,
            ..Default::default()
        }))
    }

    type StreamEventsStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<StreamEvent, tonic::Status>> + Send>>;

    async fn stream_events(
        &self,
        _request: tonic::Request<StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        Ok(tonic::Response::new(Box::pin(
            futures_util::stream::pending(),
        )))
    }
}

fn restore(key: &str, previous: Option<std::ffi::OsString>) {
    match previous {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}

/// What one fixture run leaves behind for the assertions. The temp dirs are held
/// so the sentinel the stub wrote still exists when the test looks for it.
struct FixtureRun {
    /// `Ok(true)` = the launch loop returned an owned child, `Ok(false)` = an
    /// existing agent was found, `Err` = the reported failure. The guard itself
    /// is dropped inside the helper, so the stub child is reaped either way.
    outcome: Result<bool, String>,
    /// The stub handed to the launch loop as the sidecar program, so a failure
    /// message can be checked against the exact program it names.
    program: PathBuf,
    sentinel: PathBuf,
    handshakes: Arc<AtomicUsize>,
    _home: tempfile::TempDir,
    _tools: tempfile::TempDir,
}

/// Wait (bounded) for `path` to exist, from an async task, without a runtime
/// timer dependency on the launch budget.
async fn wait_for_path(path: &Path, budget: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + budget;
    while tokio::time::Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    path.exists()
}

/// Run the launch loop against a stub program the loop is handed directly and,
/// when `answers` is set, a readiness fixture that reports `version`. Everything
/// the tests below need, so the environment mutation exists once.
///
/// `answers: false` is the loop's give-up arm: the stub spawns and stays alive
/// but never serves anything, so the attempt cannot be accepted. `probe_window`
/// is the loop's launch budget, supplied by the caller because it is a product
/// decision and a stand-in child's fork/exec latency is the host's (see
/// [`LAUNCH_FIXTURE_WINDOW`]).
fn run_launch_fixture(version: &str, answers: bool, probe_window: Duration) -> FixtureRun {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    let home = tempfile::tempdir().expect("tempdir");
    let tools = tempfile::tempdir().expect("tempdir");
    let sentinel = tools.path().join("spawned");
    let stub = tools.path().join("future");
    let mut file = std::fs::File::create(&stub).expect("create the stub");
    // The sentinel is the only proof that *this* stub was launched, since the
    // launch loop reports failures only. `: >` is a shell builtin and the sleeper
    // is absolute, so the stub needs no `PATH` of its own - and nothing on
    // `PATH` is ever consulted, because the program is handed over directly.
    // The stderr echoes are the fixture's words for the failure reports: the
    // incompatible and give-up arms below assert they surface, so a launch
    // that drops the sidecar's own output fails here too.
    std::writeln!(
        file,
        "#!/bin/sh\n{}\nexec /bin/sleep 30\n",
        if answers {
            format!(": > '{}'\necho refusing >&2", sentinel.display())
        } else {
            "echo stuck before answering >&2\n: true".to_string()
        }
    )
    .expect("write the stub");
    file.flush().expect("flush the stub");
    drop(file);
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let old_home = std::env::var_os("FUTURE_HOME");
    std::env::set_var("FUTURE_HOME", home.path());
    // The local endpoint is `$FUTURE_HOME/run/agent.sock` unless this overrides
    // it; an ambient value would move the fixture's socket elsewhere.
    let old_socket = std::env::var_os("FUTURE_AGENT_SOCKET");
    std::env::remove_var("FUTURE_AGENT_SOCKET");

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let handshakes = Arc::new(AtomicUsize::new(0));
    let data_root_id =
        future_rpc::connection::data_root_id(home.path()).expect("the scratch home is canonical");
    let socket = home.path().join("run").join("agent.sock");
    std::fs::create_dir_all(socket.parent().expect("run dir")).expect("create the run dir");
    let mock = ReadinessMock {
        data_root_id,
        version: version.to_string(),
        handshakes: handshakes.clone(),
    };
    let spawned = sentinel.clone();
    let server = answers.then(|| {
        runtime.spawn(async move {
            // Serve only once the launched candidate has run: an agent that answers
            // before the shell starts would satisfy the probe while the sentinel is
            // still unwritten, and the "which candidate" assertion would become a
            // race. See SENTINEL_WAIT: this bound sits inside the window this fixture
            // hands the loop (LAUNCH_FIXTURE_WINDOW), so expiring here can only mean
            // the child never ran, which the sentinel assertion reports directly.
            if !wait_for_path(&spawned, SENTINEL_WAIT).await {
                eprintln!(
                    "the launched candidate did not appear within {SENTINEL_WAIT:?}; \
                 the sentinel assertion reports it"
                );
            }
            let listener =
                tokio::net::UnixListener::bind(&socket).expect("bind the local endpoint");
            Server::builder()
                .add_service(FutureAgentServer::new(mock))
                .serve_with_incoming(UnixListenerStream::new(listener))
                .await
                .expect("serve the readiness fixture");
        })
    });

    let outcome = runtime
        .block_on(future_tui::agent_supervisor::ensure_agent_running_within(
            "auto",
            &stub,
            probe_window,
        ))
        .map(|guard| guard.is_some());
    if let Some(server) = server {
        server.abort();
    }
    drop(runtime);

    restore("FUTURE_HOME", old_home);
    restore("FUTURE_AGENT_SOCKET", old_socket);

    FixtureRun {
        outcome,
        program: stub,
        sentinel,
        handshakes,
        _home: home,
        _tools: tools,
    }
}

fn launch_with_a_stub_candidate(version: &str) -> FixtureRun {
    run_launch_fixture(version, true, LAUNCH_FIXTURE_WINDOW)
}

/// The launch loop's success arm: the stub spawned, the agent answered a
/// matching readiness handshake, and the guard the TUI keeps alive is returned.
#[test]
fn a_candidate_that_answers_readiness_is_returned_as_an_owned_child() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let run = launch_with_a_stub_candidate(future_rpc::connection::EXPECTED_AGENT_VERSION);

    match run.outcome {
        Ok(owned) => assert!(
            owned,
            "the agent must come from the launch loop, not from an endpoint that was already there"
        ),
        Err(error) => panic!("a candidate that answers readiness must be accepted: {error}"),
    }
    assert!(
        run.sentinel.exists(),
        "the injected stub must be the program that was launched"
    );
    assert!(
        run.handshakes.load(Ordering::SeqCst) >= 1,
        "the guard must have been accepted on a real readiness handshake"
    );
}

/// The launch loop's hard-error arm: the stub spawned and its agent
/// answered, but with a version this client refuses. The loop must kill that
/// child and report the incompatibility itself — not bury it in a generic
/// give-up, and not return an agent it just failed to verify.
#[test]
fn an_incompatible_agent_stops_the_launch_loop_and_is_reported() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let run = launch_with_a_stub_candidate("9.9.9");

    let error = match run.outcome {
        Err(error) => error,
        Ok(owned) => panic!("an incompatible agent must not be accepted (owned={owned})"),
    };
    assert!(
        run.sentinel.exists(),
        "the candidate must have spawned before its agent was rejected"
    );
    assert!(
        run.handshakes.load(Ordering::SeqCst) >= 1,
        "the rejection must come from the agent's own answer"
    );
    assert!(
        error.contains("Incompatible Future Agent") && error.contains("9.9.9"),
        "the failure must name the incompatibility it found: {error}"
    );
    assert!(
        error.contains("refusing"),
        "the report must carry the sidecar's own output beside the incompatibility: {error}"
    );
}

/// The loop's give-up arm, deterministically: a stub that spawns and stays
/// alive but never answers.
///
/// Until now this arm was reachable only when a loaded host happened to exceed
/// the loop's own probe window, which made it both rare and indistinguishable
/// from the fixture's own timing (see `docs/testing/flaky-tui.md` §7). With the
/// window supplied by the caller it is an assertion about the *loop*: it must
/// kill the stub and report the give-up with the program it waited for - and,
/// since the launch now captures sidecar output, with the stub's own words
/// (see [`GIVE_UP_FIXTURE_WINDOW`] for why the window cannot be minimal).
#[test]
fn a_candidate_that_never_answers_is_reported_after_the_probe_window() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let run = run_launch_fixture(
        future_rpc::connection::EXPECTED_AGENT_VERSION,
        false,
        GIVE_UP_FIXTURE_WINDOW,
    );

    let error = match run.outcome {
        Err(error) => error,
        Ok(owned) => panic!("a candidate that never answers must not be accepted (owned={owned})"),
    };
    assert!(
        error.contains("no Future Agent was reachable"),
        "the failure must report the give-up it performed: {error}"
    );
    let program = run.program.display().to_string();
    assert!(
        error.contains(&program),
        "the failure must name the program it waited on: {error}"
    );
    assert!(
        error.contains("did not become ready"),
        "the give-up must say the probe window ran out: {error}"
    );
    assert!(
        error.contains("stuck before answering"),
        "a live but unready candidate must still show what it printed: {error}"
    );
    assert!(
        !run.sentinel.exists(),
        "the silent candidate must never have answered"
    );
    assert_eq!(
        run.handshakes.load(Ordering::SeqCst),
        0,
        "no readiness handshake can happen without an endpoint"
    );
}

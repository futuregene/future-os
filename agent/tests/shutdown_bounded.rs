//! A stop is bounded, and the instance it released can be taken over at once.
//!
//! Measured 2026-10-28: the Agent was stopped while clients were connected, the
//! drain reached its last wait ("RPC listener and streams"), the 5 s budget ran
//! out there, and the failure retained instance ownership for the life of the
//! process. In the Desktop that process stays alive, so no Agent could start for
//! ~40 minutes, every scheduled task failed with "unable to connect", and a
//! 48-minute run was cut off. These cases drive a real service in an isolated
//! home: a client holds a connection open across the stop, and the assertions are
//! (1) the stop's own deadline ends the drain — not the client — and (2) the
//! released home is immediately startable again.

use anyhow::{Context, Result};
use future_agent::service::{
    AgentConfig, AgentEndpoint, AgentService, ShutdownReport, ShutdownStatus, StartupCancellation,
};
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CASE_ENV: &str = "FUTURE_AGENT_SHUTDOWN_BOUNDED_CASE";

/// The budget a stop is given here: the same five seconds the Agent's own drain
/// has always been handed. Nothing in this file makes it longer.
const STOP_DEADLINE: Duration = Duration::from_secs(5);

fn run_case(case: &str) -> String {
    let parent = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("shutdown-fixtures");
    std::fs::create_dir_all(&parent).unwrap();
    let home = tempfile::tempdir_in(parent).unwrap();
    // A unix `sun_path` is 104 bytes on macOS; keep any endpoint short.
    let socket = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "shutdown_fixture",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CASE_ENV, case)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("FUTURE_HOME", home.path().join(".future"))
        .env("FUTURE_AGENT_SOCKET", socket.path().join("agent.sock"))
        .env_remove("FUTURE_AGENT_GRPC_ADDR")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "{case} timed out:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{case} failed:\n{rendered}");
    rendered
}

/// A client that stays connected across a stop must not be able to extend it:
/// the drain has one deadline, and the listener stage is inside it.
#[test]
fn a_connected_client_cannot_outlast_the_stop_deadline() {
    let output = run_case("stuck-connection");
    assert!(
        output.contains("SHUTDOWN-BOUNDED-CASE-RAN stuck-connection"),
        "the case must have run its assertions: {output}"
    );
}

/// The same stop must hand the data root back: a fresh service starts on it at
/// once, without a process restart.
#[test]
fn the_released_data_root_is_startable_without_a_process_restart() {
    let output = run_case("restart-after-stop");
    assert!(
        output.contains("SHUTDOWN-BOUNDED-CASE-RAN restart-after-stop"),
        "the case must have run its assertions: {output}"
    );
}

#[test]
fn shutdown_fixture() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let config = tcp_config();
    match case.as_str() {
        "stuck-connection" => {
            stop_with_a_connected_client(config).unwrap();
        }
        "restart-after-stop" => restart_after_stop(config).unwrap(),
        other => panic!("unknown case {other}"),
    }
    println!("SHUTDOWN-BOUNDED-CASE-RAN {case}");
}

fn tcp_config() -> AgentConfig {
    let mut config = AgentConfig::default();
    config.grpc_addr = Some("127.0.0.1:0".to_owned());
    config
}

fn endpoint_of(running: &future_agent::service::RunningAgent<'_, '_>) -> Result<SocketAddr> {
    match &running.identity().endpoint {
        AgentEndpoint::Tcp(address) => Ok(*address),
        other => anyhow::bail!("this case needs a TCP endpoint: {other:?}"),
    }
}

/// What one stop observed: the report, how long it took, and whether the serving
/// future — which is what the drain's last stage waits for — finished with it.
struct StopOutcome {
    report: ShutdownReport,
    elapsed: Duration,
    server_finished: bool,
}

/// One stop with a client connected throughout: the drain must end on its own
/// deadline, report a stop, and leave nothing of the listener running.
fn stop_with_a_connected_client(config: AgentConfig) -> Result<StopOutcome> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let outcome = AgentService::with_instance(config, |service| -> Result<StopOutcome> {
        let prepared = service.prepare(StartupCancellation::default())?;
        runtime.block_on(drive_one_stop(prepared))
    })?;
    drop(runtime);
    println!(
        "SHUTDOWN-BOUNDED-STOP status={:?} elapsed_ms={} server_finished={} errors={:?}",
        outcome.report.status,
        outcome.elapsed.as_millis(),
        outcome.server_finished,
        outcome.report.errors
    );
    assert_eq!(
        outcome.report.status,
        ShutdownStatus::Stopped,
        "a connected client must not turn a stop into a failed one: {:?}",
        outcome.report.errors
    );
    assert!(
        outcome.elapsed < STOP_DEADLINE,
        "the stop must finish inside its own deadline, took {:?}",
        outcome.elapsed
    );
    assert!(
        outcome.server_finished,
        "the listener must be finished when the stop reports"
    );
    Ok(outcome)
}

async fn drive_one_stop(
    prepared: future_agent::service::PreparedAgent<'_, '_>,
) -> Result<StopOutcome> {
    let running = prepared.initialize().await?;
    let handle = running.handle();
    let address = endpoint_of(&running)?;
    // Every host polls `serve`, so the case does too: an unpolled future would
    // make the listener stage unsettled for a reason that has nothing to do with
    // the client below.
    let mut server = Box::pin(running.serve(None));
    // A connection that never speaks: what a paused Desktop, a half-closed
    // socket or a client holding the endpoint looks like to the accepting side,
    // and the state the listener stage has to be able to abandon. Holding it
    // across the stop is the whole point of the case.
    let client = {
        let connect = tokio::net::TcpStream::connect(address);
        tokio::pin!(connect);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            tokio::select! {
                result = server.as_mut() => {
                    let result = result.context("the serving future must not fail before the stop");
                    return Err(anyhow::anyhow!("the service stopped before a client connected: {result:?}"));
                }
                result = &mut connect => match result {
                    Ok(stream) => break stream,
                    Err(_) if Instant::now() < deadline => {
                        connect.as_mut().set(tokio::net::TcpStream::connect(address));
                    }
                    Err(error) => return Err(error.into()),
                },
            }
        }
    };
    let (report_tx, mut report_rx) = tokio::sync::oneshot::channel();
    let stopper = tokio::spawn(async move {
        let report = handle.shutdown(STOP_DEADLINE).await;
        let _ = report_tx.send(report);
    });
    let started = Instant::now();
    // Two ways the stop can end: the drain reports (on its own deadline at the
    // latest) and the serving future then has to finish, or the serving future
    // finishes first and the report follows it.
    let (report, server_finished) = tokio::select! {
        result = server.as_mut() => {
            result.context("the serving future failed")?;
            (report_rx.await.context("the stop must report")?, true)
        }
        report = &mut report_rx => {
            let report = report.context("the stop must report")?;
            let finished = tokio::time::timeout(Duration::from_secs(2), server.as_mut())
                .await
                .is_ok();
            (report, finished)
        }
    };
    let elapsed = started.elapsed();
    drop(client);
    stopper.await.context("the stop task must finish")?;
    Ok(StopOutcome {
        report,
        elapsed,
        server_finished,
    })
}

/// A stop that a client held open, then a second service on the same home: the
/// lock, the socket and the journal must all be free.
fn restart_after_stop(config: AgentConfig) -> Result<()> {
    stop_with_a_connected_client(config)?;
    let started = Instant::now();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    AgentService::with_instance(tcp_config(), |service| -> Result<()> {
        let prepared = service.prepare(StartupCancellation::default())?;
        runtime.block_on(async move {
            let running = prepared.initialize().await?;
            let address = endpoint_of(&running)?;
            wait_until_serving(address).await?;
            let report = running.handle().shutdown(STOP_DEADLINE).await;
            assert_eq!(
                report.status,
                ShutdownStatus::Stopped,
                "the replacement's own stop must be clean too: {:?}",
                report.errors
            );
            Ok(())
        })
    })?;
    drop(runtime);
    let elapsed = started.elapsed();
    println!(
        "SHUTDOWN-BOUNDED-RESTART elapsed_ms={}",
        elapsed.as_millis()
    );
    assert!(
        elapsed < Duration::from_secs(30),
        "a replacement Agent must start promptly, took {elapsed:?}"
    );
    Ok(())
}

async fn wait_until_serving(address: SocketAddr) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if tokio::net::TcpStream::connect(address).await.is_ok() {
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "the service never accepted a connection on {address}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

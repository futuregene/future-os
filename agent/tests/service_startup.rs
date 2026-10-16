//! Real library startup in isolated processes, without logging or CLI signals.

use anyhow::{Context, Result};
use future_agent::service::{
    AgentConfig, AgentEndpoint, AgentService, StartupCancellation, StartupCancelled,
};
use future_rpc::proto::RpcCommand;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CASE_ENV: &str = "FUTURE_AGENT_SERVICE_FIXTURE_CASE";

fn run_case(case: &str) -> String {
    let parent = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("service-fixtures");
    std::fs::create_dir_all(&parent).unwrap();
    let home = tempfile::tempdir_in(parent).unwrap();
    // Keep Unix socket names short even when the worktree path is long.
    let socket = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "service_fixture",
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
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "{case} startup/cleanup timed out:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{case} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // Both streams: the logging check below reads what a host's layer wrote,
    // and the test runner's own chatter shares this pipe.
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn library_startup_serves_real_tcp_without_cli_initialization() {
    run_case("tcp");
}

#[test]
fn library_startup_serves_native_local_ipc() {
    run_case("local");
}

#[test]
fn occupied_instance_never_initializes_a_second_store() {
    run_case("locked");
}

#[test]
fn one_instance_scope_cannot_initialize_two_services() {
    run_case("duplicate");
}

#[test]
fn startup_cancellation_releases_resources_without_creating_storage() {
    run_case("cancel-before");
    run_case("cancel-after");
    run_case("cancel-bound");
}

#[test]
fn startup_errors_release_the_scope_and_preserve_existing_data() {
    run_case("corrupt");
    run_case("bind-failure");
}

#[test]
fn dropped_server_retains_instance_ownership_until_process_exit() {
    run_case("dropped-server");
}

#[test]
fn readiness_follows_legacy_import_without_changing_storage_format() {
    run_case("legacy");
}

/// A cold or network-mounted data root can keep the Agent busy for minutes
/// before it serves anything, and a host that kills the process for exceeding a
/// memory or per-user budget leaves no error behind. The phases therefore have
/// to be visible through the host's own logging layer, which is what makes the
/// next such report actionable. Captured from a real startup in its own process:
/// a thread-local subscriber inside the shared test binary cannot observe these
/// callsites deterministically.
#[test]
fn startup_progress_reaches_the_hosts_logging_layer() {
    // ANSI-stripped: this is the text an operator reads in the log file, which
    // is where a killed process's last phase has to be recognizable.
    let output = future_agent::logfile::strip_ansi(&run_case("logging"));
    for expected in [
        "Agent startup: loading the model catalogue",
        "session storage: opening",
        "Agent startup: scanning for legacy sessions to import",
        "Agent startup: importing legacy session",
        "Agent startup: legacy session scan complete",
        "session storage: ready",
        "Agent startup: binding the gRPC endpoint",
        "gRPC server listening",
    ] {
        assert!(
            output.contains(expected),
            "missing {expected:?} in:\n{output}"
        );
    }
    // The database path is what an operator checks after a kill, and the
    // per-session line names the session that a slow import is spending its
    // time on.
    assert!(output.contains("agent.db"), "{output}");
    assert!(output.contains("session=legacy"), "{output}");
}

#[test]
fn connection_gate_blocks_held_locks_and_startup_races_before_storage() {
    run_case("gated-lock");
    run_case("gated-race");
    run_case("gated-fresh");
}

#[test]
fn service_fixture() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    // A host installs its own logging layer before starting the service; the
    // service never installs one itself, which is why this fixture must. Only
    // the case that asserts on rendered progress does so.
    if case == "logging" {
        future_agent::logfile::init_embedded_tracing();
    }
    let mut config = AgentConfig::default();
    let root = config.future_home().to_owned();
    if case.starts_with("gated-") {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let select = || {
            runtime.block_on(future_rpc::connection::select_agent(
                "auto",
                future_agent::utils::VERSION,
                Duration::from_secs(2),
                Duration::from_secs(2),
            ))
        };
        if case == "gated-lock" {
            AgentService::with_instance(config, |_| {
                let metadata = root.join("agent").join("agent-instance.json");
                let original = std::fs::read(&metadata)?;
                assert!(matches!(
                    select(),
                    Err(future_rpc::connection::ConnectionError::StartupBlocked(_))
                ));
                assert_eq!(std::fs::read(metadata)?, original);
                assert!(!root.join("agent").join("agent.db").exists());
                Ok(())
            })
            .unwrap();
        } else {
            let selection = select().unwrap_or_else(|error| {
                panic!("fresh local endpoint must authorize startup: {error}")
            });
            let future_rpc::connection::AgentSelection::LocalStart(permit) = selection else {
                panic!("unexpected external service")
            };
            if case == "gated-race" {
                AgentService::with_instance(config.clone(), |_| {
                    let mut entered = false;
                    let result = AgentService::with_local_start(config, permit, |_| {
                        entered = true;
                        Ok(())
                    });
                    assert!(result.is_err());
                    assert!(!entered);
                    assert!(!root.join("agent").join("agent.db").exists());
                    Ok(())
                })
                .unwrap();
            } else {
                AgentService::with_local_start(config, permit, |_| Ok(())).unwrap();
            }
        }
        assert_released(&root);
        return;
    }
    if case == "locked" {
        AgentService::with_instance(config.clone(), |_| {
            let metadata = root.join("agent").join("agent-instance.json");
            let original = std::fs::read(&metadata)?;
            let mut called = false;
            let result = AgentService::with_instance(config, |_| {
                called = true;
                Ok(())
            });
            assert!(result.unwrap_err().to_string().contains("already running"));
            assert!(!called);
            assert!(!root.join("agent").join("agent.db").exists());
            assert_eq!(std::fs::read(metadata)?, original);
            Ok(())
        })
        .unwrap();
        assert_released(&root);
        return;
    }

    let occupied = if case == "bind-failure" {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        config.grpc_addr = Some(listener.local_addr().unwrap().to_string());
        Some(listener)
    } else {
        if case != "local" && case != "cancel-bound" {
            config.grpc_addr = Some("127.0.0.1:0".to_owned());
        }
        None
    };
    if case == "corrupt" {
        std::fs::create_dir_all(root.join("agent")).unwrap();
        std::fs::write(root.join("agent").join("agent.db"), "not sqlite").unwrap();
    }
    let legacy = if matches!(case.as_str(), "legacy" | "logging") {
        let sessions = root.join("agent").join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let original = serde_json::json!({
            "id": "entry-1", "type": "user", "role": "user",
            "content": "synthetic text", "timestamp": "2026-01-01T00:00:00Z"
        })
        .to_string();
        std::fs::write(sessions.join("legacy.jsonl"), &original).unwrap();
        Some(original)
    } else {
        None
    };

    let result = AgentService::with_instance(config, |service| {
        let cancellation = StartupCancellation::default();
        if case == "cancel-before" {
            cancellation.cancel();
            let error = service.prepare(cancellation).err().unwrap();
            assert!(error.downcast_ref::<StartupCancelled>().is_some());
            assert!(!root.join("agent").join("agent.db").exists());
            return Ok(());
        }
        let prepared = service.prepare(cancellation.clone())?;
        if case == "duplicate" {
            let error = service
                .prepare(StartupCancellation::default())
                .err()
                .unwrap();
            assert!(error.to_string().contains("already been started"));
            assert!(!root.join("agent").join("agent.db").exists());
        }
        if case == "cancel-after" {
            cancellation.cancel();
        }
        // Build outside async; dropping this runtime precedes owner unlock.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let result = prepared.initialize().await;
            if matches!(case.as_str(), "corrupt" | "bind-failure" | "cancel-after") {
                let error = result.err().expect("startup must fail");
                if case == "cancel-after" {
                    assert!(error.downcast_ref::<StartupCancelled>().is_some());
                    assert!(!root.join("agent").join("agent.db").exists());
                } else if case == "corrupt" {
                    assert!(error
                        .to_string()
                        .contains("initialize Agent SQLite storage"));
                    assert_eq!(
                        std::fs::read(root.join("agent").join("agent.db"))?,
                        b"not sqlite"
                    );
                } else {
                    assert!(error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|error| { error.kind() == std::io::ErrorKind::AddrInUse }));
                }
                return Ok(());
            }

            let running = result?;
            if case == "cancel-bound" {
                cancellation.cancel();
                let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
                let error = running.serve(Some(ready_tx)).await.unwrap_err();
                assert!(error.downcast_ref::<StartupCancelled>().is_some());
                assert!(ready_rx.await.is_err(), "cancelled startup is never Ready");
                #[cfg(unix)]
                assert!(!future_rpc::transport::local_socket_path().exists());
                return Ok(());
            }
            let handle = running.handle();
            let expected = running.identity().clone();
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let server = running.serve(Some(ready_tx));
            let client = async {
                let ready = ready_rx.await.context("service never announced ready")?;
                assert_eq!(ready, expected);
                verify_rpc(
                    &ready.endpoint,
                    &ready.agent_instance_id,
                    matches!(case.as_str(), "legacy" | "logging"),
                )
                .await
            };
            tokio::pin!(server);
            let mut streams = tokio::select! {
                result = server.as_mut() => {
                    result?;
                    anyhow::bail!("service stopped before the RPC checks");
                }
                result = tokio::time::timeout(Duration::from_secs(10), client) => result??,
            };
            if case == "dropped-server" {
                return Ok(());
            }
            let (report, server) = tokio::join!(handle.shutdown(Duration::from_secs(5)), server);
            assert_eq!(
                report.status,
                future_agent::service::ShutdownStatus::Stopped,
                "{:?}",
                report.errors
            );
            server?;
            assert_eq!(report, handle.shutdown(Duration::ZERO).await);
            for stream in &mut streams {
                tokio::time::timeout(Duration::from_secs(2), async {
                    while stream.message().await?.is_some() {}
                    Ok::<_, tonic::Status>(())
                })
                .await??;
            }
            Ok(())
        })
    });
    if case == "dropped-server" {
        assert!(result.unwrap_err().to_string().contains("failed to stop"));
        let metadata = root.join("agent").join("agent-instance.json");
        let original = std::fs::read(&metadata).unwrap();
        let mut reached_store = false;
        let retry = AgentService::with_instance(AgentConfig::default(), |_| {
            reached_store = true;
            Ok(())
        });
        assert!(retry.unwrap_err().to_string().contains("already running"));
        assert!(!reached_store);
        assert_eq!(std::fs::read(&metadata).unwrap(), original);
        std::env::set_var("FUTURE_HOME", root.join("another-home"));
        let retry = AgentService::with_instance(AgentConfig::default(), |_| {
            reached_store = true;
            Ok(())
        });
        assert!(retry
            .unwrap_err()
            .to_string()
            .contains("another Agent service"));
        assert!(!reached_store);
        assert!(!root
            .join("another-home")
            .join("agent")
            .join("agent.db")
            .exists());
        return;
    }
    result.unwrap();
    drop(occupied);
    assert_released(&root);

    if let Some(original) = legacy {
        assert_eq!(
            std::fs::read_to_string(root.join("agent").join("sessions").join("legacy.jsonl"))
                .unwrap(),
            original
        );
    }
    if !matches!(case.as_str(), "corrupt" | "cancel-before" | "cancel-after") {
        let connection = rusqlite::Connection::open_with_flags(
            root.join("agent").join("agent.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 6, "the extraction must not migrate the schema");
    }
}

fn assert_released(root: &Path) {
    assert!(!root.join("agent").join("agent-instance.json").exists());
    AgentService::with_instance(AgentConfig::default(), |_| Ok(())).unwrap();
    assert!(root.join("agent").join("agent-instance.lock").exists());
}

async fn verify_rpc(
    endpoint: &AgentEndpoint,
    instance_id: &str,
    imported: bool,
) -> Result<Vec<tonic::Streaming<future_rpc::proto::StreamEvent>>> {
    let address = match endpoint {
        AgentEndpoint::Local(_) => "auto".to_owned(),
        AgentEndpoint::Tcp(address) => address.to_string(),
    };
    // A legacy/raw transport may discover readiness, but cannot mutate or
    // hydrate data without the protocol-2 business contract.
    let connected =
        future_rpc::transport::connect_pinned_channel(Some(&address), Duration::from_secs(3), None)
            .await?;
    let mut raw = future_rpc::transport::agent_client(connected.channel);
    let response = raw
        .execute_command(RpcCommand {
            r#type: "get_agent_readiness".into(),
            ..Default::default()
        })
        .await?
        .into_inner();
    assert!(response.success);
    let error = raw
        .execute_command(RpcCommand {
            r#type: "new_session".into(),
            session_id: "unverified-must-not-exist".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    let error = raw
        .stream_events(future_rpc::proto::StreamRequest {
            session_id: "unverified-must-not-exist".into(),
            ..Default::default()
        })
        .await
        .err()
        .unwrap();
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    drop(raw);
    let mut client = future_rpc::connection::connect_verified(
        &address,
        future_rpc::connection::EXPECTED_AGENT_VERSION,
        Duration::from_secs(3),
        Duration::from_secs(3),
    )
    .await?;
    assert_eq!(client.readiness().agent_instance_id, instance_id);
    assert!(matches!(
        future_rpc::connection::select_agent(
            &address,
            future_rpc::connection::EXPECTED_AGENT_VERSION,
            Duration::from_secs(3),
            Duration::from_secs(3),
        )
        .await?,
        future_rpc::connection::AgentSelection::External(_)
    ));
    assert!(matches!(
        future_rpc::connection::select_agent(
            &address,
            "999.999.999",
            Duration::from_secs(3),
            Duration::from_secs(3),
        )
        .await,
        Err(future_rpc::connection::ConnectionError::Incompatible { .. })
    ));
    let readiness = client
        .execute_command(RpcCommand {
            id: "readiness".to_owned(),
            r#type: "get_agent_readiness".to_owned(),
            ..Default::default()
        })
        .await?
        .into_inner();
    assert!(readiness.success, "{}", readiness.error);
    let data = future_rpc::decode::response_data(&readiness);
    assert_eq!(data["agentInstanceId"], instance_id);
    assert_eq!(data["version"], future_agent::utils::VERSION);
    let sessions = client
        .execute_command(RpcCommand {
            id: "sessions".to_owned(),
            r#type: "list_sessions".to_owned(),
            ..Default::default()
        })
        .await?
        .into_inner();
    assert!(sessions.success, "{}", sessions.error);
    let data = future_rpc::decode::response_data(&sessions);
    assert_eq!(
        data["sessions"].as_array().unwrap().len(),
        usize::from(imported)
    );
    let mut global = client
        .stream_events(future_rpc::proto::StreamRequest {
            global_events: true,
            ..Default::default()
        })
        .await?
        .into_inner();
    assert!(global.message().await?.is_some());
    let created = client
        .execute_command(RpcCommand {
            id: "create-session".into(),
            r#type: "new_session".into(),
            session_id: "stream-session".into(),
            created_by: "cli".into(),
            ..Default::default()
        })
        .await?
        .into_inner();
    assert!(created.success, "{}", created.error);
    let mut session = client
        .stream_events(future_rpc::proto::StreamRequest {
            session_id: "stream-session".into(),
            ..Default::default()
        })
        .await?
        .into_inner();
    assert!(session.message().await?.is_some());
    // Keep both clients subscribed while the host shuts down.
    Ok(vec![global, session])
}

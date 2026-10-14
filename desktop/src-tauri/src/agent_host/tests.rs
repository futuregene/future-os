//! Fresh processes isolate Agent's process-wide caches, environment and locks.
use super::*;
use std::{
    path::Path,
    process::{Command, Stdio},
};
const CASE: &str = "FUTURE_DESKTOP_AGENT_HOST_TEST";

fn run_case(case: &str) {
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/host-fixtures");
    std::fs::create_dir_all(&parent).unwrap();
    let home = tempfile::tempdir_in(parent).unwrap();
    let socket = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "agent_host::tests::host_fixture",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CASE, case)
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("FUTURE_HOME", home.path().join(".future"))
        .env("FUTURE_AGENT_SOCKET", socket.path().join("agent.sock"))
        .env_remove("FUTURE_AGENT_GRPC_ADDR")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "{case} timed out: {} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{case}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn embedded_startup_and_restart_keep_the_agent_in_the_desktop_process() {
    run_case("restart");
}
#[test]
fn an_attached_host_does_not_stop_the_external_owner() {
    run_case("external");
}
#[test]
fn held_lock_rejects_startup_before_database_initialization() {
    run_case("locked");
}

#[test]
fn a_stalled_client_fails_stop_without_releasing_the_lock_or_restarting() {
    run_case("stalled");
}

#[test]
fn a_cli_shell_is_cancelled_by_the_embedded_owner() {
    run_case("shell");
}
#[test]
fn concurrent_starts_share_one_service_and_stop_is_idempotent() {
    run_case("concurrent");
}
#[test]
fn stopping_during_startup_never_publishes_a_ready_service() {
    run_case("cancel-start");
}

#[test]
fn cli_prompt_and_queued_follow_up_are_persisted_as_cancelled_on_exit() {
    run_case("prompt");
}

#[test]
fn host_fixture() {
    let Ok(case) = std::env::var(CASE) else {
        return;
    };
    let root = future_rpc::home::future_home();
    let host = AgentHost::default();
    if case == "locked" {
        std::fs::create_dir_all(root.join("agent")).unwrap();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("agent/agent-instance.lock"))
            .unwrap();
        fs2::FileExt::try_lock_exclusive(&file).unwrap();
        assert!(host.start("auto".into()).is_err());
        assert!(!host.owns_agent());
        assert!(!root.join("agent/agent.db").exists());
        assert!(!root.join("agent/auth.json").exists());
        return;
    }
    let provider = if case == "prompt" {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        std::fs::create_dir_all(root.join("agent")).unwrap();
        std::fs::write(
            root.join("agent/models.json"),
            serde_json::json!({
                "providers": { "fixture": { "api": "openai-completions", "baseUrl": url,
                    "apiKey": "fixture-key", "models": [{ "id": "stall",
                        "limit": { "context": 64000, "output": 4096 } }] } }
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            root.join("agent/settings.json"),
            r#"{"defaultModel":"fixture/stall"}"#,
        )
        .unwrap();
        let (started, received) = std::sync::mpsc::channel();
        let (stop, stopped) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0u8; 16384];
            assert!(stream.read(&mut bytes).unwrap() > 0);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
            let event = "data: {\"choices\":[{\"delta\":{\"content\":\"working\"},\"index\":0,\"finish_reason\":null}]}\n\n";
            write!(stream, "{:x}\r\n{}\r\n", event.len(), event).unwrap();
            stream.flush().unwrap();
            started.send(()).unwrap();
            let _ = stopped.recv_timeout(Duration::from_secs(30));
        });
        Some((received, stop, thread))
    } else {
        None
    };
    if case == "cancel-start" {
        let starter = host.clone();
        let thread = std::thread::spawn(move || starter.start("auto".into()));
        while host.phase() == Phase::Idle {
            std::thread::yield_now();
        }
        host.request_stop();
        let _ = host.stop();
        assert!(thread.join().unwrap().is_err());
        assert_ne!(host.phase(), Phase::Ready);
        assert!(!root.join("agent/agent-instance.json").exists());
        return;
    }
    if case == "concurrent" {
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let host = host.clone();
                std::thread::spawn(move || host.start("auto".into()))
            })
            .collect();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
    } else {
        host.start("auto".into()).unwrap();
    }
    assert!(host.owns_agent());
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("agent/agent-instance.json")).unwrap())
            .unwrap();
    assert_eq!(metadata["pid"], std::process::id());
    let rpc = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let first = rpc.block_on(async {
        let mut client = future_rpc::connection::connect_verified(
            "auto",
            crate::build_info::VERSION,
            Duration::from_secs(2),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        let identity = client.readiness().agent_instance_id.clone();
        let response = client
            .execute_command(future_rpc::proto::RpcCommand {
                r#type: "new_session".into(),
                session_id: "cli-session".into(),
                created_by: "cli".into(),
                cwd: root.display().to_string(),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        assert!(response.success, "{}", response.error);
        identity
    });
    if let Some((received, stop, provider_thread)) = provider {
        drop(rpc);
        let rpc = tokio::runtime::Runtime::new().unwrap();
        let mut client = rpc
            .block_on(future_rpc::connection::connect_verified(
                "auto",
                crate::build_info::VERSION,
                Duration::from_secs(2),
                Duration::from_secs(2),
            ))
            .unwrap();
        for run in ["cli-first", "cli-follow-up"] {
            let ack = rpc
                .block_on(client.execute_command(future_rpc::proto::RpcCommand {
                    r#type: "prompt".into(),
                    session_id: "cli-session".into(),
                    requested_run_id: run.into(),
                    client_request_id: run.into(),
                    message: "continue work".into(),
                    ..Default::default()
                }))
                .unwrap()
                .into_inner();
            assert!(ack.success, "{}", ack.error);
            if run == "cli-first" {
                received.recv_timeout(Duration::from_secs(3)).unwrap();
            }
        }
        assert_eq!(host.activity().unwrap().unwrap().queued, 1);
        host.stop().unwrap();
        let _ = stop.send(());
        provider_thread.join().unwrap();
        let manager = future_agent::session::Manager::new(root.join("agent/sessions"));
        let stored = manager.load("cli-session").unwrap();
        for run in ["cli-first", "cli-follow-up"] {
            assert_eq!(
                future_agent::session::find_run_terminal(&stored.entries, run).unwrap()["state"],
                "cancelled"
            );
        }
        return;
    }
    if case == "shell" {
        // Keep the CLI transport actively driven while Desktop initiates stop.
        drop(rpc);
        let rpc = tokio::runtime::Runtime::new().unwrap();
        let mut client = rpc
            .block_on(future_rpc::connection::connect_verified(
                "auto",
                crate::build_info::VERSION,
                Duration::from_secs(2),
                Duration::from_secs(2),
            ))
            .unwrap();
        let shell = rpc.spawn(async move {
            client
                .execute_command(future_rpc::proto::RpcCommand {
                    r#type: "shell".into(),
                    session_id: "cli-session".into(),
                    command: if cfg!(windows) {
                        "ping -n 30 127.0.0.1 > NUL"
                    } else {
                        "sleep 30"
                    }
                    .into(),
                    ..Default::default()
                })
                .await
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while host.activity().unwrap().unwrap().commands == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let started = Instant::now();
        host.stop().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        let result = rpc.block_on(shell).unwrap();
        assert!(result.is_err() || !result.unwrap().into_inner().success);
        assert!(!root.join("agent/agent-instance.json").exists());
        return;
    }
    if case == "external" {
        let attached = AgentHost::default();
        attached.start("auto".into()).unwrap();
        assert!(!attached.owns_agent());
        attached.stop().unwrap();
        rpc.block_on(future_rpc::connection::connect_verified(
            "auto",
            crate::build_info::VERSION,
            Duration::from_secs(2),
            Duration::from_secs(2),
        ))
        .unwrap();
    }
    if case == "stalled" {
        let client = rpc
            .block_on(future_rpc::connection::connect_verified(
                "auto",
                crate::build_info::VERSION,
                Duration::from_secs(2),
                Duration::from_secs(2),
            ))
            .unwrap();
        let started = Instant::now();
        let error = host.stop().unwrap_err().to_string();
        assert!(error.contains("RPC listener"), "{error}");
        assert!(started.elapsed() < STOP_TIMEOUT);
        assert!(root.join("agent/agent-instance.json").exists());
        assert!(host.start("auto".into()).is_err());
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(root.join("agent/agent-instance.lock"))
            .unwrap();
        assert!(fs2::FileExt::try_lock_exclusive(&file).is_err());
        drop(client);
        return;
    }
    // Let the client transport close before testing an ordinary complete drain.
    drop(rpc);
    host.stop().unwrap();
    assert_eq!(host.phase(), Phase::Stopped);
    host.stop().unwrap();
    assert!(!root.join("agent/agent-instance.json").exists());
    if case == "restart" {
        host.start("auto".into()).unwrap();
        let rpc = tokio::runtime::Runtime::new().unwrap();
        let next = rpc
            .block_on(future_rpc::connection::connect_verified(
                "auto",
                crate::build_info::VERSION,
                Duration::from_secs(2),
                Duration::from_secs(2),
            ))
            .unwrap();
        assert_ne!(first, next.readiness().agent_instance_id);
        drop(next);
        drop(rpc);
        host.stop().unwrap();
    }
}

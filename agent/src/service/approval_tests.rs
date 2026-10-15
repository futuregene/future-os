use super::*;
use std::time::{Duration, Instant};

/// A real synchronous tool approval must release an admitted blocking caller
/// before shutdown waits for its command count. Merely polling run streaming
/// would miss this source of work.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_releases_a_blocking_tool_approval() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::rpc::service_test_state();
    let handle = AgentHandle {
        state,
        shutdown: Arc::new(shutdown::ShutdownShared::default()),
    };
    let gate = handle.state.approval_gate.clone();
    let broadcaster = handle.state.sessions.read()["default"]
        .read()
        .broadcaster
        .clone();
    let admitted = handle.state.service_work.enter().unwrap();
    let cwd = root.path().to_string_lossy().to_string();
    let sandbox = crate::sandbox::ResolvedSandbox::resolve(
        &crate::sandbox::SandboxPolicy {
            tier: crate::sandbox::SandboxTier::Manual,
            model_reviewer: false,
        },
        &cwd,
    );
    let asking = std::thread::spawn(move || {
        let _admitted = admitted;
        gate.request(
            broadcaster.as_ref(),
            "default",
            &cwd,
            "shell",
            "tool-approval",
            &serde_json::json!({"command":"rm -rf /tmp/futureos-approval-shape"}),
            &sandbox,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while handle
        .state
        .approval_gate
        .pending_for_session("default")
        .is_empty()
    {
        assert!(Instant::now() < deadline, "tool approval never registered");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let report = handle.shutdown(Duration::from_secs(3)).await;
    assert_eq!(
        report.status,
        ShutdownStatus::Stopped,
        "{:?}",
        report.errors
    );
    assert!(handle
        .state
        .approval_gate
        .pending_for_session("default")
        .is_empty());
    assert!(asking.join().is_ok());
}

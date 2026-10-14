use super::*;
use crate::rpc::{handle_command_internal, RpcCommand};
use std::sync::atomic::Ordering;
use std::time::Duration;

fn fixture() -> AgentHandle {
    let state = crate::rpc::service_test_state();
    for session in state.sessions.read().values() {
        let mut session = session.write();
        session.service_work = state.service_work.clone();
        // Synthetic runs/queues still require the durable session row that
        // real prompt admission creates before accepting work.
        let mut stored = crate::session::Session::new(&session.cwd, &session.model);
        stored.id = session.session_id.clone();
        stored
            .entries
            .push(crate::session::SessionEntry::session_info(
                serde_json::json!({"cwd": session.cwd, "model": session.model}),
                session.model.clone(),
                String::new(),
            ));
        state.session_manager.save(&stored).unwrap();
    }
    AgentHandle {
        state,
        shutdown: Arc::new(shutdown::ShutdownShared::default()),
    }
}

fn command(kind: &str) -> RpcCommand {
    serde_json::from_value(serde_json::json!({"id":"client","type":kind,"sessionId":"default"}))
        .unwrap()
}

struct StallingProvider(Arc<std::sync::atomic::AtomicUsize>);

#[async_trait::async_trait]
impl crate::types::LLMProvider for StallingProvider {
    async fn stream_model(
        &self,
        _: crate::llm::schema::ModelRequest,
    ) -> anyhow::Result<tokio_stream::wrappers::ReceiverStream<crate::llm::schema::ModelStreamEvent>>
    {
        self.0.fetch_add(1, Ordering::AcqRel);
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        tokio::spawn(async move {
            sender.closed().await;
        });
        Ok(tokio_stream::wrappers::ReceiverStream::new(receiver))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_prompt_and_follow_up_use_real_run_finalization_without_dequeuing_on_shutdown() {
    let workspace = tempfile::tempdir().unwrap();
    let handle = fixture();
    let session = handle.state.sessions.read()["default"].clone();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    {
        let mut session = session.write();
        session.cwd = workspace.path().to_string_lossy().to_string();
        session.model = "mock".into();
        session.auto_compaction = false;
        session.agent_loop = Arc::new(tokio::sync::RwLock::new(crate::agent::Loop::new(
            Arc::new(StallingProvider(calls.clone())),
            "mock",
        )));
    }
    for (run, request) in [
        ("cli-first", "first-request"),
        ("cli-follow-up", "second-request"),
    ] {
        let mut prompt = command("prompt");
        prompt.requested_run_id = run.into();
        prompt.client_request_id = request.into();
        prompt.message = "continue work".into();
        let response: serde_json::Value =
            serde_json::from_str(&handle_command_internal(&handle.state, prompt)).unwrap();
        assert_eq!(response["success"], true, "{response}");
        if run == "cli-first" {
            tokio::time::timeout(Duration::from_secs(3), async {
                while calls.load(Ordering::Acquire) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
    }
    assert_eq!(handle.activity_snapshot().unwrap().queued, 1);
    let report = handle.shutdown(Duration::from_secs(3)).await;
    assert_eq!(
        report.status,
        ShutdownStatus::Stopped,
        "{:?}",
        report.errors
    );
    assert_eq!(
        calls.load(Ordering::Acquire),
        1,
        "queued follow-up must never contact the provider"
    );
    let manager = crate::session::Manager::new(handle.state.session_manager.dir.clone());
    let loaded = manager.load("default").unwrap();
    for run in ["cli-first", "cli-follow-up"] {
        assert_eq!(
            crate::session::find_run_terminal(&loaded.entries, run).unwrap()["state"],
            "cancelled"
        );
    }
    assert!(session.read().scheduler.queued().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_activity_includes_queue_approval_compaction_and_blocking_work() {
    let handle = fixture();
    let session = handle.state.sessions.read()["default"].clone();
    session
        .read()
        .scheduler
        .accept(
            "cli-follow-up",
            Some("queued"),
            crate::runtime::BusyPolicy::EnqueueIfBusy,
            serde_json::json!({"message":"later"}),
        )
        .unwrap();
    let approval = handle
        .state
        .approval_gate
        .insert_pending_for_test("approval", "default");
    let native = handle.state.service_work.background().unwrap();
    let blocking = handle.state.service_work.enter().unwrap();
    session
        .read()
        .compaction_in_progress
        .store(true, Ordering::Release);
    let activity = handle.activity_snapshot().unwrap();
    assert_eq!(
        (
            activity.queued,
            activity.approvals,
            activity.compactions,
            activity.commands,
            activity.background
        ),
        (1, 1, 1, 1, 1)
    );
    assert!(!activity.is_idle());
    let locked = session.write();
    assert!(
        handle.activity_snapshot().is_err(),
        "unknown activity must not be reported as idle"
    );
    drop(locked);
    session
        .read()
        .compaction_in_progress
        .store(false, Ordering::Release);
    drop(blocking);
    drop(native);
    let report = handle.shutdown(Duration::from_secs(3)).await;
    assert_eq!(
        report.status,
        ShutdownStatus::Stopped,
        "{:?}",
        report.errors
    );
    assert_eq!(
        approval.recv().unwrap().status,
        crate::rpc::ApprovalDecisionStatus::Cancelled
    );
    assert!(session.read().scheduler.queued().is_empty());
    let manager = crate::session::Manager::new(handle.state.session_manager.dir.clone());
    let loaded = manager.load("default").unwrap();
    let terminal = crate::session::find_run_terminal(&loaded.entries, "queued").unwrap();
    assert_eq!(terminal["state"], "cancelled");
    assert!(
        handle.state.session_manager.contains("default").is_err(),
        "the original SQLite connection must be closed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_run_cancels_persists_and_releases_its_monitor_before_stop() {
    let handle = fixture();
    let session = handle.state.sessions.read()["default"].clone();
    crate::rpc::ServerSession::ensure_scheduler_worker(&session);
    let runtime = session.read().runtime.clone();
    let lease = runtime.begin(Some("cli-run"), Some("cli-request")).unwrap();
    let (interrupt, mut cancelled) = tokio::sync::mpsc::channel(1);
    assert!(runtime.install_cancellation(&lease, interrupt, Arc::new(AtomicBool::new(false))));
    let persistence = session.read().persistence.clone();
    persistence
        .append(vec![crate::session::SessionEntry::run_started(
            &lease.run_id,
            lease.epoch,
        )])
        .unwrap();
    let task_runtime = runtime.clone();
    let task_lease = lease.clone();
    runtime
        .spawn(lease, async move {
            cancelled.recv().await.unwrap();
            persistence
                .append(vec![crate::session::SessionEntry::run_terminal(
                    &task_lease.run_id,
                    crate::session::RUN_STATE_CANCELLED,
                    0,
                    0,
                    None,
                )])
                .unwrap();
            assert!(task_runtime.begin_finalizing(&task_lease));
        })
        .unwrap();
    assert_eq!(handle.activity_snapshot().unwrap().running, 1);
    let report = handle.shutdown(Duration::from_secs(3)).await;
    assert_eq!(
        report.status,
        ShutdownStatus::Stopped,
        "{:?}",
        report.errors
    );
    assert!(!runtime.has_owned_task());
    assert!(runtime.auxiliary_settled());
    assert!(session.read().scheduler_settled());
    let manager = crate::session::Manager::new(handle.state.session_manager.dir.clone());
    let loaded = manager.load("default").unwrap();
    assert_eq!(
        crate::session::find_run_terminal(&loaded.entries, "cli-run").unwrap()["state"],
        "cancelled"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rpc_shutdown_remains_admission_only_and_host_shutdown_is_idempotent() {
    let handle = fixture();
    let response: serde_json::Value =
        serde_json::from_str(&handle_command_internal(&handle.state, command("shutdown"))).unwrap();
    assert_eq!(response["success"], true);
    assert!(!handle.state.service_work.is_closed());
    let response: serde_json::Value = serde_json::from_str(&handle_command_internal(
        &handle.state,
        command("list_sessions"),
    ))
    .unwrap();
    assert_eq!(response["success"], true);
    handle.request_shutdown(Duration::from_secs(3));
    let response: serde_json::Value = serde_json::from_str(&handle_command_internal(
        &handle.state,
        command("new_session"),
    ))
    .unwrap();
    assert_eq!(response["error_code"], "agent_stopping");
    let (first, second) = tokio::join!(
        handle.shutdown(Duration::from_secs(3)),
        handle.shutdown(Duration::ZERO)
    );
    assert_eq!(first.status, ShutdownStatus::Stopped, "{:?}", first.errors);
    assert_eq!(first, second);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_work_timeout_is_terminal_even_after_work_eventually_finishes() {
    let handle = fixture();
    let native = handle.state.service_work.background().unwrap();
    let first = handle.shutdown(Duration::from_millis(50)).await;
    assert_eq!(first.status, ShutdownStatus::FailedStopping);
    assert!(first.errors[0].contains("deadline exceeded"));
    drop(native);
    assert_eq!(first, handle.shutdown(Duration::from_secs(3)).await);
    assert!(handle.state.service_work.enter().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn writer_failure_is_a_failed_stop_and_keeps_the_store_unavailable_for_reuse() {
    let handle = fixture();
    handle.state.sessions.read()["default"]
        .read()
        .persistence
        .fail_next_close();
    let report = handle.shutdown(Duration::from_secs(3)).await;
    assert_eq!(report.status, ShutdownStatus::FailedStopping);
    assert!(report
        .errors
        .iter()
        .any(|error| error.contains("session writer")));
    assert!(handle.state.service_work.enter().is_err());
}

#[test]
fn cancellation_during_starting_is_delivered_when_targets_are_installed() {
    for already_stuck in [false, true] {
        let runtime = Arc::new(crate::runtime::SessionRuntime::new(Arc::new(
            AtomicBool::new(false),
        )));
        let lease = runtime.begin(Some("starting"), None).unwrap();
        runtime.request_abort(None).unwrap();
        if already_stuck {
            assert!(runtime.mark_stuck(&lease, "delayed initialization"));
        }
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let flag = Arc::new(AtomicBool::new(false));
        assert!(runtime.install_cancellation(&lease, sender, flag.clone()));
        assert!(flag.load(Ordering::Acquire));
        assert!(receiver.try_recv().is_ok());
        assert_eq!(
            runtime.snapshot().unwrap().phase,
            if already_stuck {
                crate::runtime::RunPhase::CancellationStuck
            } else {
                crate::runtime::RunPhase::Cancelling
            }
        );
    }
}

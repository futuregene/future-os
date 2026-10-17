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
    session
        .read()
        .runtime
        .request_abort(None)
        .expect("the run is cancellable");
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

/// The optimistic-concurrency check at the end of `activity_snapshot`
/// (`activity.rs` 70-71): the work revision is read once, the session list is
/// walked, and the revision is re-read before the numbers are published. If work
/// started or finished **during** the walk, the counts describe neither state, so
/// the snapshot is refused rather than reported as authoritative.
///
/// The window is real but microseconds wide, and nothing in the function can be
/// blocked from outside to widen it: every lock it takes (`sessions.try_read`,
/// each `session.try_read`, `try_pending_count`, `journal_compaction_activity`)
/// is a `try_` lock, so holding one produces a *different* error instead of a
/// delay. That leaves one honest fixture: keep the revision moving on another
/// thread and take the snapshot repeatedly, biased towards a hit. This is the
/// F-43 pattern — the assertion is "at least one hit in N attempts", never an
/// exact count, because a pass is a matter of timing and pinning a number would
/// make the test flake under load.
///
/// The fixture is deliberately *narrow*: line 70 is the only place it can produce
/// an error, because nothing else is contended (no shutdown was requested, no
/// session lock is held, no approval is pending). When this test was written, the
/// single line's count was checked by probe to confirm which arm ran.
#[test]
fn activity_is_unavailable_when_work_changes_during_the_snapshot() {
    const ATTEMPTS: usize = 20_000;
    let handle = fixture();
    let work = handle.state.service_work.clone();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mover = {
        let work = work.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            // Each guard taken and dropped bumps the revision, which is exactly
            // the change the snapshot re-checks for.
            while !stop.load(Ordering::Relaxed) {
                if let Ok(guard) = work.background() {
                    drop(guard);
                }
            }
        })
    };

    let mut refused = 0usize;
    for _ in 0..ATTEMPTS {
        if handle.activity_snapshot().is_err() {
            refused += 1;
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    mover.join().unwrap();

    println!("activity revision race: attempts={ATTEMPTS} refused={refused}");
    assert!(
        refused > 0,
        "the revision check was never reached in {ATTEMPTS} attempts while another thread moved \
         the revision continuously; either the fixture lost its bias or the check is gone"
    );
    // With the mover stopped, a snapshot is available again: the refusal is about
    // concurrent change, not about a broken handle.
    assert!(
        handle.activity_snapshot().is_ok(),
        "a quiet instance must report activity again"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ephemeral_sessions_cancelled_queue_is_not_written_as_a_terminal_row() {
    let handle = fixture();
    // An ephemeral session has no persisted transcript, so the drain must not
    // try to append a terminal marker for its cancelled queue: that write would
    // fail and turn a clean stop into a failed one.
    let mut ephemeral = crate::rpc::ServerSession::new_with_queue_budget(
        "ephemeral".to_string(),
        Arc::new(tokio::sync::RwLock::new(crate::agent::Loop::new(
            Arc::new(crate::test_support::EmptyProvider),
            "mock",
        ))),
        handle.state.session_manager.clone(),
        "/tmp",
        Arc::new(crate::rpc::SseBroadcaster::new()),
        handle.state.approval_gate.clone(),
        handle.state.model_registry.clone(),
        handle.state.queue_budget.clone(),
    );
    ephemeral.ephemeral = true;
    // A queued run that the drain will cancel: the ephemeral skip is only
    // observable when there is something to write.
    let queue = crate::runtime::InMemoryRunQueue::new("ephemeral", 0);
    queue
        .accept(
            "req-ephemeral",
            Some("run-ephemeral"),
            crate::runtime::BusyPolicy::EnqueueIfBusy,
            serde_json::json!({"text": "x"}),
        )
        .unwrap();
    ephemeral.scheduler = Arc::new(queue);
    handle.state.create_session(ephemeral);
    let ephemeral = handle.state.sessions.read()["ephemeral"].clone();
    assert_eq!(ephemeral.read().scheduler.queued().len(), 1);
    assert_eq!(handle.state.sessions.read().len(), 2);

    let report = handle.shutdown(Duration::from_secs(3)).await;
    assert_eq!(
        report.status,
        ShutdownStatus::Stopped,
        "an ephemeral session's queue must not be persisted: {:?}",
        report.errors
    );
    assert!(
        !report
            .errors
            .iter()
            .any(|error| error.contains("terminal persistence")),
        "{:?}",
        report.errors
    );
}

/// A storage failure that appears between startup and the drain must still be
/// reported by the drain at every stage it touches: the event journal cannot
/// flush its queued delta, and the storage layer cannot close its writer. Both
/// are `FailedStopping` reasons, not silent cleanups.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dead_storage_worker_is_reported_by_the_journal_and_storage_stages() {
    let handle = fixture();
    let session = handle.state.sessions.read()["default"].clone();
    // A buffered delta is appended without a reply, so it stays queued: the
    // journal's own shutdown flush is then the stage that must fail.
    session
        .write()
        .broadcaster
        .broadcast(crate::rpc::SseEvent::new(
            "tool_delta",
            serde_json::json!({"tool_id": "t", "text": "partial"}),
        ));

    // Kill the SQLite worker from a job of its own: the panic happens on the
    // worker thread, so this thread only observes a dead connection.
    let store = handle.state.session_manager.storage().unwrap().clone();
    let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = store
            .db
            .call(|_| -> anyhow::Result<()> { panic!("injected worker panic") });
    }));
    assert!(first.is_ok(), "the panic happens on the worker thread");

    // Bounded wait for the refusal, so the assertion below is about the drain
    // rather than about a race with the unwinding worker.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match store.db.call(|connection| {
            Ok(connection.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?)
        }) {
            Err(error) if error.to_string().contains("SQLite worker is unavailable") => break,
            _ if std::time::Instant::now() >= deadline => {
                panic!("the dead worker was never reported as unavailable")
            }
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    }

    let report = handle.shutdown(Duration::from_secs(5)).await;
    assert_eq!(
        report.status,
        ShutdownStatus::FailedStopping,
        "a dead storage worker is not a clean stop: {:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("event journal:")),
        "the journal's failed shutdown flush must be reported: {:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("storage shutdown:")),
        "the storage writer's failed close must be reported: {:?}",
        report.errors
    );
}

/// The drain's per-stage `return errors` exits are the *deadline* exits: work
/// that stays pending in one specific stage is what makes a short deadline end
/// the shutdown there, and the stage name in the report is what tells an
/// operator which piece never settled. The run/native-work stage is the one
/// this reaches, because a background registration that is never released keeps
/// `counts()` non-zero while leaving the earlier command stage clear.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_deadline_reports_the_stage_that_never_settled() {
    let handle = fixture();
    // Never dropped: the drain's run/native-work wait can never settle, so the
    // deadline — not a clean drain — is what ends this shutdown.
    let _background = handle.state.service_work.background().unwrap();

    let report = handle.shutdown(Duration::from_millis(50)).await;

    // The deadline watcher publishes the report *before* the drain observes it:
    // `shutdown()` returns as soon as a report exists, while the supervisor
    // thread is still in its own wait loop. Give that thread its bounded turn so
    // the drain's stage exit is part of the test rather than a race with it.
    std::thread::sleep(Duration::from_millis(200));

    assert_eq!(
        report.status,
        ShutdownStatus::FailedStopping,
        "a drain that never settles is not a clean stop: {:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("deadline exceeded during runs and native background work")),
        "the report must name the stage that never settled: {:?}",
        report.errors
    );
    // The report is terminal: a later clean completion must not re-open it.
    assert_eq!(report, handle.shutdown(Duration::from_secs(5)).await);
}

/// The scheduler/monitor stage is the last one the drain can time out in, and it
/// is reachable when a session's scheduler worker is registered but can never be
/// observed to finish — which is what a host whose runtime has stopped being
/// driven looks like from here. The worker is spawned on a second runtime that
/// this test deliberately never blocks on, so its `Done` guard cannot drop and
/// `scheduler_settled()` stays false while `service_work.counts()` is already
/// `(0, 0)` (no run is owned), which is what distinguishes this stage from the
/// earlier exits.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_undriven_scheduler_worker_times_out_the_monitor_stage() {
    let handle = fixture();
    let session = handle.state.sessions.read()["default"].clone();
    // A second, current-thread runtime: entering it is enough for
    // `Handle::try_current()` to resolve inside `ensure_scheduler_worker`, and
    // never driving it is what keeps the spawned worker alive.
    let stalled = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    {
        let _entered = stalled.enter();
        crate::rpc::ServerSession::ensure_scheduler_worker(&session);
    }
    assert!(
        !session.read().scheduler_settled(),
        "the worker must be registered and unfinished"
    );

    let report = handle.shutdown(Duration::from_millis(150)).await;
    // Bounded grace for the supervisor's own turn (F-23).
    std::thread::sleep(Duration::from_millis(200));

    assert_eq!(
        report.status,
        ShutdownStatus::FailedStopping,
        "an unfinished scheduler worker is not a clean stop: {:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error
                .contains("deadline exceeded during scheduler and cancellation monitors")),
        "the report must name the monitor stage: {:?}",
        report.errors
    );
    // `shutdown_background` rather than a drop: a runtime cannot be dropped from
    // inside an async context, and only after the assertions is it safe for the
    // worker (and its `Done` guard) to go away.
    stalled.shutdown_background();
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

/// A deadline that cannot be represented as an instant is refused up front: the
/// instance keeps ownership instead of pretending a shutdown was scheduled.
#[test]
fn an_unrepresentable_shutdown_deadline_fails_without_starting_a_supervisor() {
    let handle = fixture();
    handle.request_shutdown(Duration::MAX);
    let report = handle
        .shutdown
        .report()
        .expect("the refusal is a terminal report");
    assert_eq!(
        report.status,
        crate::service::ShutdownStatus::FailedStopping
    );
    assert_eq!(
        report.errors,
        vec!["invalid Agent shutdown deadline".to_string()]
    );
    // Admission is still closed: nothing may start behind a failed shutdown.
    assert!(handle.state.shutting_down.load(Ordering::Acquire));
    assert!(handle.state.service_work.enter().is_err());
}

/// A drain that is still waiting when the deadline expires reports
/// FailedStopping and stops waiting, so the host is never left without an
/// answer (the instance is retained, not released).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_shutdown_deadline_reports_failed_stopping_and_stops_waiting() {
    let handle = fixture();
    let admitted = handle.state.service_work.enter().unwrap();
    let report = handle.shutdown(Duration::from_millis(60)).await;
    assert_eq!(
        report.status,
        crate::service::ShutdownStatus::FailedStopping
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("deadline exceeded")),
        "{:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("admitted commands")),
        "the stage that was still waiting is named: {:?}",
        report.errors
    );
    // The admission boundary is closed even though the drain did not finish.
    assert!(handle.state.service_work.enter().is_err());
    // Hold the admitted work past the report and past one poll interval of the
    // drain's wait loop: with the report already set, its next poll takes the
    // "deadline already reported" branch and the drain returns without waiting
    // for the work (that branch is the reason the host is never blocked).
    std::thread::sleep(Duration::from_millis(150));
    drop(admitted);
    // A late completion must not overwrite the terminal report.
    let after = handle.shutdown.report().expect("terminal report");
    assert_eq!(after.status, crate::service::ShutdownStatus::FailedStopping);
    assert_eq!(after.errors, report.errors);
}

/// A storage failure while persisting the terminal markers of cancelled queued
/// runs is reported *and* recorded: the host must not be told "stopped" when a
/// run's outcome never reached the journal, and the session's own recorded
/// append error must appear alongside it rather than being dropped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_storage_failure_while_persisting_cancelled_runs_is_reported() {
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
    for (run, request) in [("fail-first", "first-request"), ("fail-queued", "second")] {
        let mut prompt = command("prompt");
        prompt.requested_run_id = run.into();
        prompt.client_request_id = request.into();
        prompt.message = "continue work".into();
        let response: serde_json::Value =
            serde_json::from_str(&handle_command_internal(&handle.state, prompt)).unwrap();
        assert_eq!(response["success"], true, "{response}");
        if run == "fail-first" {
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

    // The journal refuses exactly the cancelled run's terminal marker (a full
    // disk at that moment), and nothing else: the ongoing run's own terminal
    // write still lands, so the drain reaches the queued-run pass instead of
    // stalling before it.
    handle.state.session_manager.test_execute(
        "CREATE TRIGGER refuse_cancelled_terminal BEFORE INSERT ON entries \
         WHEN NEW.entry_type='run_terminal' \
          AND NEW.content_json LIKE '%Agent shut down before queued execution%' \
         BEGIN SELECT RAISE(ABORT, 'injected terminal failure'); END;",
    );
    let report = handle.shutdown(Duration::from_secs(5)).await;
    assert_eq!(
        report.status,
        ShutdownStatus::FailedStopping,
        "a lost run outcome must not report a clean stop: {:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("terminal persistence")),
        "the refused terminal marker is named: {:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("fail-queued")),
        "the failure names the run it belongs to: {:?}",
        report.errors
    );
}

/// A session left holding a run snapshot with no owned task is the state a
/// crashed worker leaves behind. Shutdown must say so instead of releasing
/// instance ownership as if the session were idle.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_holding_an_unresolved_run_reports_a_failed_stop() {
    let handle = fixture();
    let session = handle.state.sessions.read()["default"].clone();
    let lease = session
        .read()
        .runtime
        .begin(Some("orphan-run"), Some("orphan-request"))
        .expect("no run is active in the fixture");
    drop(lease);

    let report = handle.shutdown(Duration::from_secs(5)).await;
    assert_eq!(
        report.status,
        ShutdownStatus::FailedStopping,
        "an unresolved run must not report a clean stop: {:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("orphan-run") && error.contains("unresolved")),
        "the unresolved run is named with its phase: {:?}",
        report.errors
    );
}

/// A recorded persistence error must reach the shutdown report. The session's
/// own writer knows an append failed; if the drain released instance ownership
/// without saying so, a host would restart onto a transcript with a hole in it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_persistence_error_is_reported_by_the_drain() {
    let handle = fixture();
    let session = handle.state.sessions.read()["default"].clone();
    let runtime = session.read().runtime.clone();
    let lease = runtime
        .begin(Some("warn-run"), Some("warn-request"))
        .unwrap();
    let (interrupt, mut cancelled) = tokio::sync::mpsc::channel(1);
    assert!(runtime.install_cancellation(&lease, interrupt, Arc::new(AtomicBool::new(false))));
    session
        .read()
        .runtime
        .request_abort(None)
        .expect("the run is cancellable");
    let persistence = session.read().persistence.clone();
    persistence
        .append(vec![crate::session::SessionEntry::run_started(
            &lease.run_id,
            lease.epoch,
        )])
        .unwrap();

    // The next durable write cannot be delivered at all: both spawn attempts
    // produce a worker whose receiver is already gone, so the writer records
    // why. That is exactly what the drain must surface.
    persistence.kill_worker_for_test();
    persistence.fail_spawns_for_test(2);
    let failed = persistence
        .append(vec![crate::session::SessionEntry::new_user(
            "assistant",
            serde_json::json!("half-written"),
        )])
        .unwrap_err();
    assert!(
        !failed.to_string().is_empty(),
        "the failing append reports its cause"
    );
    let recorded = persistence
        .last_error()
        .expect("a failed append is recorded on the session");
    assert!(
        recorded.contains("unavailable"),
        "the recorded error is the delivery failure: {recorded}"
    );

    // Finish the run so the drain is not blocked on it. The cancellation the
    // runtime installed is already queued on the channel.
    assert!(cancelled.try_recv().is_ok());
    assert!(runtime.begin_finalizing(&lease));

    let report = handle.shutdown(Duration::from_secs(5)).await;
    assert_eq!(
        report.status,
        ShutdownStatus::FailedStopping,
        "a recorded write failure must not report a clean stop: {:?}",
        report.errors
    );
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("unavailable")),
        "the recorded error is carried into the report: {:?}",
        report.errors
    );
}

/// A service that reported a failed stop is no longer a trustworthy source of
/// activity numbers: the host must be told "unavailable" rather than given
/// counters from a half-drained instance.
#[test]
fn activity_is_unavailable_once_a_stop_has_failed() {
    let handle = fixture();
    assert!(
        handle.activity_snapshot().is_ok(),
        "a healthy instance reports activity"
    );
    // A deadline that cannot be represented fails the shutdown immediately.
    handle.request_shutdown(Duration::MAX);
    assert_eq!(
        handle.shutdown.report().map(|report| report.status),
        Some(ShutdownStatus::FailedStopping)
    );
    assert!(
        handle.activity_snapshot().is_err(),
        "activity after a failed stop must be reported as unavailable"
    );
}

/// `finish_scope`'s **no-handle** branch: a host that never got as far as
/// `initialize()` (failed or cancelled startup) still has to wait for work that
/// a startup helper left owned. The loop it runs is the only thing standing
/// between a failed start and releasing the instance, so it must actually wait
/// rather than assume the work is already gone.
///
/// The fixture drives `finish_scope` directly on a service whose `handle` slot is
/// empty, which is exactly the state a failed start leaves: no `RunningAgent` was
/// built, so there is nothing to `shutdown_blocking`, but a guard is still held
/// by another thread. Before this test, the loop's body had never executed —
/// `finish_scope`'s else-branch ran 51 times in the measured report and
/// `counts()` was `(0, 0)` every one of them, so `while … {}` did no iterations.
///
/// `work::install` is deliberately **not** called: it registers the process-global
/// `ACTIVE` slot, and a lib test that installs one would poison the data-root
/// invariant for every other test in this binary.
/// A cancellation that lands **after** `serve`'s entry check and **before** the
/// server's first poll: `service/mod.rs` 343, and with it the "Agent serving
/// failed" report at 355-358.
///
/// F-21 reasoned that nothing could schedule this window — every statement
/// between the entry check (301) and the first poll (338) is synchronous, so
/// "only another OS thread can hit it" — and left the line explicitly *flagged
/// for empirical falsification* rather than classified. Measured, F-21 is wrong,
/// and the reason is worth keeping: the gap is not empty. Between setting
/// `server_running` (324) and polling the server (338), `serve` subscribes to two
/// `watch` channels and builds the guard and the pinned future, which is hundreds
/// of nanoseconds of real work — while a thread already spinning on
/// `server_running` sees the store in tens of nanoseconds. The spinner wins the
/// window in **every** attempt of this fixture (40/40 when first measured, and it
/// is the only thing that can: the entry check runs before `server_running` is
/// set, so a `StartupCancelled` observed here cannot have come from 301).
///
/// The arrangement is deliberate, not lucky: the canceller is spun up and given a
/// head start before the runtime polls `serve`, so it is hot when serving begins.
/// A stopper then closes the listener so `serve` cannot hang, the whole call is
/// bounded by a timeout, and the attempt count is a floor: one hit is what the
/// classification needs, and asserting "at least one" keeps the fixture from
/// becoming a coin flip on a loaded machine.
#[test]
fn a_cancellation_before_the_first_poll_is_reported_as_a_failed_start() {
    const ATTEMPTS: usize = 20;
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join(".future");
    let config = AgentConfig {
        future_home: home.clone(),
        ..AgentConfig::default()
    };
    let mut file =
        instance::InstanceFile::open(home.join("agent").join("agent-instance.lock")).unwrap();
    let guard = file.acquire(&home).unwrap();
    let service = AgentService {
        config,
        preparation_started: AtomicBool::new(false),
        startup_cleanup_started: AtomicBool::new(false),
        work: Arc::new(work::ServiceWork::default()),
        handle: parking_lot::Mutex::new(None),
        _instance: guard,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let mut window_hits = 0usize;
    let mut stopped_cleanly = 0usize;
    for _ in 0..ATTEMPTS {
        let state = crate::rpc::service_test_state();
        let listener = runtime
            .block_on(crate::grpc::AgentListener::bind_tcp("127.0.0.1", 0))
            .unwrap();
        let endpoint = listener.endpoint();
        let handle = AgentHandle {
            state: state.clone(),
            shutdown: Arc::new(shutdown::ShutdownShared::default()),
        };
        let cancellation = StartupCancellation::default();
        let running = RunningAgent {
            _owner: &service,
            handle: handle.clone(),
            state: state.clone(),
            listener,
            ready: AgentReady {
                agent_instance_id: state.agent_instance_id.clone(),
                version: state.welcome_version.clone(),
                endpoint,
            },
            cancellation: cancellation.clone(),
        };
        let (ready_tx, _ready_rx) = tokio::sync::oneshot::channel();

        // Hot before serving begins: this is the arrangement that decides the
        // window, and the head start biases the experiment *towards* a hit.
        let flag = handle.shutdown.clone();
        let cancel = cancellation.clone();
        let spinner = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !flag.server_running.load(Ordering::Acquire) {
                if std::time::Instant::now() >= deadline {
                    return false;
                }
                std::hint::spin_loop();
            }
            cancel.cancel();
            true
        });
        std::thread::sleep(Duration::from_millis(3));
        let stopper_work = state.service_work.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            stopper_work.stop_listener();
        });

        let outcome = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), running.serve(Some(ready_tx))).await
        });
        let saw_serving = spinner.join().unwrap();
        stopper.join().unwrap();
        assert!(
            saw_serving,
            "every attempt must reach the serving state before it can be stopped"
        );
        match outcome {
            Ok(Ok(())) => stopped_cleanly += 1,
            Ok(Err(ref error)) => {
                assert!(
                    error.downcast_ref::<StartupCancelled>().is_some(),
                    "only a cancellation may fail serving here: {error:#}"
                );
                window_hits += 1;
            }
            Err(_) => panic!("serving did not stop within the timeout"),
        }
    }

    println!(
        "before-first-poll cancellation: attempts={ATTEMPTS} hits={window_hits} \
         stopped_cleanly={stopped_cleanly}"
    );
    assert!(
        window_hits > 0,
        "the window was not hit in {ATTEMPTS} attempts: either the fixture lost its \
         head start or `service/mod.rs` 343 has become unreachable, and this row \
         must be re-measured"
    );
    assert_eq!(
        window_hits + stopped_cleanly,
        ATTEMPTS,
        "every attempt must resolve one way or the other"
    );
}

#[test]
fn a_failed_start_waits_for_background_work_before_releasing_the_instance() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join(".future");
    let config = AgentConfig {
        future_home: home.clone(),
        ..AgentConfig::default()
    };
    let mut file =
        instance::InstanceFile::open(home.join("agent").join("agent-instance.lock")).unwrap();
    let guard = file.acquire(&home).unwrap();
    let work = Arc::new(work::ServiceWork::default());
    let service = AgentService {
        config,
        preparation_started: AtomicBool::new(false),
        startup_cleanup_started: AtomicBool::new(false),
        work: work.clone(),
        handle: parking_lot::Mutex::new(None),
        _instance: guard,
    };

    let held = work.background().unwrap();
    assert_eq!(work.counts(), (0, 1), "the helper's work is still owned");
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(80));
        drop(held);
    });

    let started = std::time::Instant::now();
    service.finish_scope().unwrap();
    let waited = started.elapsed();
    releaser.join().unwrap();

    assert!(
        waited >= Duration::from_millis(60),
        "finish_scope returned after {waited:?} while background work was still owned"
    );
    assert_eq!(
        work.counts(),
        (0, 0),
        "the wait must end once work is released"
    );
    assert!(
        work.is_closed(),
        "a failed start closes admission before waiting"
    );
}

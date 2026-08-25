//! Tests for the run-lifecycle command handlers.

use std::sync::Arc;

use crate::rpc::ApprovalDecisionStatus;

use crate::rpc::commands::test_support::*;
use crate::rpc::handle_command_internal;

#[test]
fn prompt_rejects_unknown_busy_policy_with_stable_code() {
    let state = make_app_state();
    let mut cmd = make_cmd("prompt");
    cmd.message = "hello".to_string();
    cmd.busy_policy = "frobnicate".to_string();

    let resp = parse_response(&handle_command_internal(&state, cmd));

    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "invalid_busy_policy");
    assert_eq!(resp["error_data"]["provided"], "frobnicate");
}

#[test]
fn prompt_enqueue_if_busy_returns_canonical_queued_ack() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    let active = session
        .read()
        .runtime
        .begin(Some("run-active"), Some("request-active"))
        .unwrap();
    assert_eq!(active.run_id, "run-active");

    let mut cmd = make_cmd("prompt");
    cmd.message = "queued later".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    cmd.client_request_id = "request-next".to_string();

    let resp = parse_response(&handle_command_internal(&state, cmd));

    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["accepted_state"], "queued");
    assert_eq!(resp["data"]["queue_position"], 1);
    assert_eq!(
        session.read().scheduler.queued()[0].client_request_id,
        "request-next"
    );
}

#[test]
fn cancel_queued_run_removes_only_the_requested_run() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .runtime
        .begin(Some("run-active"), Some("request-active"))
        .unwrap();
    for number in 1..=2 {
        let mut prompt = make_cmd("prompt");
        prompt.message = format!("queued {number}");
        prompt.busy_policy = "enqueue_if_busy".to_string();
        prompt.client_request_id = format!("request-{number}");
        prompt.requested_run_id = format!("run-{number}");
        assert_eq!(
            parse_response(&handle_command_internal(&state, prompt))["success"],
            true
        );
    }

    let mut cancel = make_cmd("cancel_queued_run");
    cancel.run_id = "run-1".to_string();
    let response = parse_response(&handle_command_internal(&state, cancel));
    assert_eq!(response["success"], true);
    assert_eq!(response["data"]["state"], "cancelled");
    assert!(session.read().scheduled_setting_summary("run-1").is_none());
    assert_eq!(
        session
            .read()
            .scheduler
            .queued()
            .iter()
            .map(|run| run.run_id.as_str())
            .collect::<Vec<_>>(),
        vec!["run-2"]
    );
}

#[test]
fn prompt_rejected_after_shutdown() {
    let state = make_app_state();
    let cmd = make_cmd("shutdown");
    handle_command_internal(&state, cmd);
    let mut cmd = make_cmd("prompt");
    cmd.message = "hello".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("shutting down"));
}

#[test]
fn abort_works() {
    let state = make_app_state();
    let cmd = make_cmd("abort");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn stale_run_scoped_commands_are_rejected_without_touching_current_run() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    let lease = session
        .read()
        .runtime
        .begin(Some("run-current"), None)
        .unwrap();

    let mut abort = make_cmd("abort");
    abort.run_id = "run-old".to_string();
    let response = parse_response(&handle_command_internal(&state, abort));
    assert_eq!(response["success"], false);
    assert!(response["error"]
        .as_str()
        .is_some_and(|error| error.contains("run-old")));
    assert_eq!(
        session.read().runtime.snapshot().unwrap().phase,
        crate::runtime::RunPhase::Starting
    );
    let mut abort = make_cmd("abort");
    abort.run_id = "run-current".to_string();
    let response = parse_response(&handle_command_internal(&state, abort));
    assert_eq!(response["success"], true);
    assert_eq!(
        session.read().runtime.snapshot().unwrap().phase,
        crate::runtime::RunPhase::Cancelling
    );
    assert!(session.read().runtime.begin_finalizing(&lease));
    assert!(session.read().runtime.finish(&lease));
}

#[test]
fn approval_decision_invalid_mode() {
    let state = make_app_state();
    let mut cmd = make_cmd("approval_decision");
    cmd.mode = "invalid".to_string();
    cmd.entry_id = "req_1".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("approved, rejected, or cancelled"));
}

#[test]
fn abort_retry_works() {
    let state = make_app_state();
    let cmd = make_cmd("abort_retry");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn prompt_generates_client_request_id_when_omitted() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .runtime
        .begin(Some("run-active"), Some("request-active"))
        .unwrap();

    let mut cmd = make_cmd("prompt");
    cmd.message = "hello".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let session = state.get_session("default").unwrap();
    let request_id = session.read().scheduler.queued()[0]
        .client_request_id
        .clone();
    assert!(
        request_id.starts_with("request_"),
        "generated client_request_id, got {request_id:?}"
    );
}

#[test]
fn prompt_reports_duplicate_request_conflict() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .runtime
        .begin(Some("run-active"), Some("request-active"))
        .unwrap();

    let mut first = make_cmd("prompt");
    first.message = "one".to_string();
    first.busy_policy = "enqueue_if_busy".to_string();
    first.client_request_id = "dup-req".to_string();
    let resp = parse_response(&handle_command_internal(&state, first));
    assert_eq!(resp["success"], true);

    let mut second = make_cmd("prompt");
    second.message = "two — different body, same request id".to_string();
    second.busy_policy = "enqueue_if_busy".to_string();
    second.client_request_id = "dup-req".to_string();
    let resp = parse_response(&handle_command_internal(&state, second));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "duplicate_request_conflict");
}

#[test]
fn prompt_rejects_unsafe_requested_run_id() {
    let state = make_app_state();
    let mut cmd = make_cmd("prompt");
    cmd.message = "hello".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    cmd.requested_run_id = "bad run id!".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "invalid_run_id");
}

#[test]
fn prune_run_events_validates_run_id() {
    let state = make_app_state();

    let mut cmd = make_cmd("prune_run_events");
    cmd.run_id = String::new();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "invalid_run_id");

    let mut cmd = make_cmd("prune_run_events");
    cmd.run_id = "../escape".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "invalid_run_id");
}

#[test]
fn prune_run_events_removes_journal_and_tolerates_missing_file() {
    let state = make_app_state();
    let run_data = state.session_manager.run_data_path("default");
    std::fs::create_dir_all(&run_data).unwrap();
    std::fs::write(run_data.join("run-prune.jsonl"), "{}").unwrap();

    let mut cmd = make_cmd("prune_run_events");
    cmd.run_id = "run-prune".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["pruned"], true);
    assert!(!run_data.join("run-prune.jsonl").exists());

    // Already gone → still pruned (NotFound is success).
    let mut cmd = make_cmd("prune_run_events");
    cmd.run_id = "run-prune".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["pruned"], true);
}

#[test]
fn abort_session_on_idle_session_cancels_nothing() {
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("abort_session")));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["active_run_id"].is_null());
    assert_eq!(resp["data"]["queued_cancelled"], 0);
    assert_eq!(resp["data"]["state"], "cancelling");
}

#[test]
fn abort_session_cancels_queued_runs() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .runtime
        .begin(Some("run-active"), Some("request-active"))
        .unwrap();

    let mut cmd = make_cmd("prompt");
    cmd.message = "queued".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);

    let resp = parse_response(&handle_command_internal(&state, make_cmd("abort_session")));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["queued_cancelled"], 1);
    assert_eq!(resp["data"]["active_run_id"], "run-active");
}

#[test]
fn retry_persistence_on_healthy_session_fails() {
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("retry_persistence"),
    ));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "persistence_recovery_failed");
}

// ── coverage batch 1: approval_decision ─────────────────────────────────

#[test]
fn approval_decision_unknown_request_fails() {
    let state = make_app_state();
    let mut cmd = make_cmd("approval_decision");
    cmd.mode = "approved".to_string();
    cmd.entry_id = "no-such-request".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("not pending"));
}

#[test]
fn approval_decision_rejects_wrong_session_and_approves_owning_session() {
    let state = make_app_state();
    let rx = state
        .approval_gate
        .insert_pending_for_test("ap-own", "default");

    // Ownership is keyed on cmd.session_id: a decision naming a pending
    // entry owned by a *different* session is rejected.
    let _rx_other = state
        .approval_gate
        .insert_pending_for_test("ap-other", "other-session");
    let mut cmd = make_cmd_for("approval_decision", "default");
    cmd.entry_id = "ap-other".to_string();
    cmd.mode = "approved".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("does not belong"));

    // …and the owning session's decision lands on the waiting channel.
    let mut cmd = make_cmd_for("approval_decision", "default");
    cmd.entry_id = "ap-own".to_string();
    cmd.mode = "approved".to_string();
    cmd.message = "looks fine".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["approvalRequestId"], "ap-own");
    assert_eq!(resp["data"]["status"], "approved");
    let decision = rx.try_recv().expect("decision delivered");
    assert!(decision.approved);
    assert_eq!(decision.note, "looks fine");
}

#[test]
fn approval_decision_rejected_and_cancelled_modes() {
    let state = make_app_state();
    for (mode, expected) in [
        ("rejected", ApprovalDecisionStatus::Rejected),
        ("cancelled", ApprovalDecisionStatus::Cancelled),
    ] {
        let request_id = format!("ap-{mode}");
        let _rx = state
            .approval_gate
            .insert_pending_for_test(&request_id, "default");
        let mut cmd = make_cmd("approval_decision");
        cmd.entry_id = request_id;
        cmd.mode = mode.to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true, "{mode}");
        let decision = _rx.try_recv().expect("decision delivered");
        assert!(!decision.approved);
        assert_eq!(decision.status, expected);
    }
}

// ── coverage batch 1: simple session-scoped setters ─────────────────────

#[test]
fn prompt_default_enqueues_when_busy() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .runtime
        .begin(Some("run-active"), Some("request-active"))
        .unwrap();

    // Default (empty) busy policy now enqueues instead of rejecting.
    let mut cmd = make_cmd("prompt");
    cmd.message = "queued behind the active run".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["accepted_state"], "queued");
}

#[test]
fn prompt_supersede_replaces_queued_run() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .runtime
        .begin(Some("run-active"), Some("request-active"))
        .unwrap();
    // A queued entry makes the scheduler busy.
    let mut cmd = make_cmd("prompt");
    cmd.message = "queued".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);

    let mut cmd = make_cmd("prompt");
    cmd.message = "supersede".to_string();
    cmd.busy_policy = "supersede_session".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    // The superseded queued run is gone; the new request is queued behind
    // the still-active run.
    let queued = session.read().scheduler.queued().clone();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].payload["message"], "supersede");
}

#[test]
fn prompt_duplicate_run_id_maps_to_scheduler_error() {
    let state = make_app_state();
    // Plant a journal for run-dupe so the id is rejected as reused.
    let run_data = state.session_manager.run_data_path("default");
    std::fs::create_dir_all(&run_data).unwrap();
    std::fs::write(run_data.join("run-dupe.jsonl"), "").unwrap();

    let mut cmd = make_cmd("prompt");
    cmd.message = "dupe".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    cmd.requested_run_id = "run-dupe".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "scheduler_error");
}

#[test]
fn prompt_accepts_attachment_as_a_live_path_reference() {
    let state = make_app_state();
    state
        .get_session("default")
        .unwrap()
        .read()
        .runtime
        .begin(Some("run-active"), Some("request-active"))
        .unwrap();
    let mut cmd = make_cmd("prompt");
    cmd.message = "with attachment".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    cmd.attachments = vec![crate::types::Attachment {
        path: "/definitely/not/a/real/file.pdf".to_string(),
        kind: "file".to_string(),
        ..Default::default()
    }];
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true, "response: {resp}");
}

#[test]
fn prompt_reports_persistence_unavailable() {
    // A regular file where the run-events dir should be makes journal
    // configuration fail, which enqueue reports as persistence_unavailable.
    let dir = test_session_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(".run-events"), "not a dir").unwrap();
    let state = make_app_state_with(dir, Arc::new(crate::runtime::GlobalQueueBudget::defaults()));

    let mut cmd = make_cmd("prompt");
    cmd.message = "hello".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "persistence_unavailable");
}

#[test]
fn prompt_reports_session_queue_full() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .runtime
        .begin(Some("run-active"), Some("request-active"))
        .unwrap();

    // Fill the session queue to capacity (128).
    for i in 0..crate::runtime::DEFAULT_SESSION_QUEUE_CAPACITY {
        let mut cmd = make_cmd("prompt");
        cmd.message = format!("queued {i}");
        cmd.busy_policy = "enqueue_if_busy".to_string();
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true, "enqueue {i}");
    }
    let mut cmd = make_cmd("prompt");
    cmd.message = "one too many".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "queue_full");
    assert_eq!(
        resp["error_data"]["limit"],
        crate::runtime::DEFAULT_SESSION_QUEUE_CAPACITY as u64
    );
}

#[test]
fn prompt_reports_request_too_large() {
    let state = make_app_state();
    let mut cmd = make_cmd("prompt");
    cmd.message = "x".repeat(crate::runtime::DEFAULT_REQUEST_BYTES + 1);
    cmd.busy_policy = "enqueue_if_busy".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "request_too_large");
}

#[test]
fn prompt_reports_global_queue_full() {
    let state = make_app_state_with(
        test_session_dir(),
        Arc::new(crate::runtime::GlobalQueueBudget::new(0, usize::MAX)),
    );
    let mut cmd = make_cmd("prompt");
    cmd.message = "hello".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "queue_full");
}

#[test]
fn prompt_reports_global_queue_bytes_exceeded() {
    let state = make_app_state_with(
        test_session_dir(),
        Arc::new(crate::runtime::GlobalQueueBudget::new(usize::MAX, 1)),
    );
    let mut cmd = make_cmd("prompt");
    cmd.message = "more than one byte".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "queue_memory_limit");
}

#[test]
fn cancel_queued_run_requires_run_id() {
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("cancel_queued_run"),
    ));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "run_not_queued");
}

#[test]
fn cancel_queued_run_unknown_run_errors() {
    let state = make_app_state();
    let mut cmd = make_cmd("cancel_queued_run");
    cmd.run_id = "run-ghost".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "run_not_queued");
}

#[test]
fn prune_run_events_rejects_active_run() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .runtime
        .begin(Some("run-blocker"), Some("request-blocker"))
        .unwrap();
    let mut cmd = make_cmd("prompt");
    cmd.message = "queued".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    cmd.requested_run_id = "run-scheduled".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    session.read().scheduler.start_next(1).unwrap();

    let mut cmd = make_cmd("prune_run_events");
    cmd.run_id = "run-scheduled".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "run_active");
}

#[test]
fn prune_run_events_reports_io_error() {
    let state = make_app_state();
    // A directory where the journal file should be makes remove_file fail.
    let run_data = state.session_manager.run_data_path("default");
    std::fs::create_dir_all(run_data.join("run-dir.jsonl")).unwrap();

    let mut cmd = make_cmd("prune_run_events");
    cmd.run_id = "run-dir".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "prune_failed");
    let _ = std::fs::remove_dir_all(run_data.join("run-dir.jsonl"));
}

#[test]
fn retry_persistence_recovers_degraded_run() {
    let state = make_app_state();
    // The transcript must exist for the recovery append to land.
    save_via(
        &state,
        "default",
        "mock",
        vec![crate::session::SessionEntry::session_info(
            serde_json::json!({"cwd": state.welcome_cwd, "model": "mock"}),
            "mock".to_string(),
            "low".to_string(),
        )],
    );
    let session = state.get_session("default").unwrap();
    let lease = session
        .read()
        .runtime
        .begin(Some("run-degraded"), Some("request-degraded"))
        .unwrap();
    assert!(session
        .read()
        .runtime
        .mark_persistence_degraded(&lease, "disk full"));

    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("retry_persistence"),
    ));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["run_id"], "run-degraded");
    assert_eq!(resp["data"]["state"], "interrupted");
    assert_eq!(resp["data"]["recovered"], true);
}

#[test]
fn prompt_reports_session_queue_bytes_exceeded() {
    let state = make_app_state();
    // A tiny per-session queue-byte limit (smaller than the request limit,
    // so the payload passes RequestTooLarge and trips QueueBytesExceeded).
    let small = Arc::new(crate::runtime::InMemoryRunQueue::with_limits_and_global(
        "default",
        1,
        crate::runtime::DEFAULT_SESSION_QUEUE_CAPACITY,
        8,
        1024,
        256,
        Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
    ));
    state.get_session("default").unwrap().write().scheduler = small;

    let mut cmd = make_cmd("prompt");
    cmd.message = "hello there".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "queue_memory_limit");
}

#[test]
fn prompt_reports_busy_configuration_error() {
    let state = make_app_state();
    let agent_loop = state
        .get_session("default")
        .unwrap()
        .read()
        .agent_loop
        .clone();
    let _guard = agent_loop.try_write().unwrap();

    let mut cmd = make_cmd("prompt");
    cmd.message = "hi".to_string();
    cmd.busy_policy = "enqueue_if_busy".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("run configuration is busy"));
}

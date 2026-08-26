//! Tests for the read-only observability / export command handlers.

use std::sync::Arc;

use crate::rpc::{SseBroadcaster, SseEvent};

use crate::rpc::commands::observability::{
    page_events_tail, ExportDirGuard, EVENTS_PAGE_BYTE_BUDGET, EVENT_WIRE_OVERHEAD,
    EXPORT_TEST_LOCK,
};
use crate::rpc::commands::test_support::*;
use crate::rpc::handle_command_internal;

#[test]
fn get_state_returns_session_info() {
    let state = make_app_state();
    state
        .get_session("default")
        .unwrap()
        .read()
        .scheduler
        .accept(
            "queued-request",
            Some("queued-run"),
            crate::runtime::BusyPolicy::EnqueueIfBusy,
            serde_json::json!({"message":"later"}),
        )
        .unwrap();
    let cmd = make_cmd("get_state");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["sessionId"].is_string());
    assert_eq!(resp["data"]["agentInstanceId"], "agent-test-instance");
    assert_eq!(resp["data"]["queuedCount"], 1);
    assert_eq!(resp["data"]["queuedRuns"][0]["runId"], "queued-run");
    assert_eq!(resp["data"]["queuedRuns"][0]["displayText"], "later");
}

#[test]
fn get_state_reports_pending_approvals_for_owning_session() {
    let state = make_app_state();
    state
        .approval_gate
        .insert_pending_for_test("approval_req1", "default");
    state
        .approval_gate
        .insert_pending_for_test("approval_req2", "other-session");

    let resp = parse_response(&handle_command_internal(&state, make_cmd("get_state")));
    assert_eq!(resp["success"], true);
    // Only the session's own pending requests surface — never another
    // session's (ownership rule, same as approval decisions).
    let pending = resp["data"]["pendingApprovals"].as_array().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["approval_request_id"], "approval_req1");
    assert_eq!(pending[0]["session_id"], "default");
}

#[test]
fn get_state_pending_approvals_empty_when_none() {
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("get_state")));
    assert_eq!(resp["success"], true);
    assert_eq!(
        resp["data"]["pendingApprovals"].as_array().unwrap().len(),
        0
    );
}

#[test]
fn get_state_reports_interrupted_run_when_journal_unterminated() {
    let state = make_app_state();
    // Each make_app_state() now gets an isolated temp session dir (see
    // test_session_dir), so this test no longer shares a file with other
    // tests; the explicit id just names the session under test. get_state
    // hydrates the session from disk on demand.
    let session_id = "gi-interrupted";
    let info = crate::session::SessionEntry::session_info(
        serde_json::json!({"cwd": state.welcome_cwd, "model": "mock", "session_name": "n"}),
        "mock".to_string(),
        "low".to_string(),
    );
    let session = crate::session::Session::snapshot(
        session_id.to_string(),
        state.welcome_cwd.clone(),
        "mock".to_string(),
        "n".to_string(),
        String::new(),
        vec![
            info,
            crate::session::SessionEntry::new_user("user", serde_json::json!("hi")),
            crate::session::SessionEntry::run_started("run-interrupted", 3),
        ],
    );
    state.session_manager.save(&session).unwrap();

    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd_for("get_state", session_id),
    ));
    assert_eq!(resp["success"], true);
    // No live run, so activeRun is null and the unterminated run is surfaced
    // as interrupted_by_restart for the GUI's startup reconcile to consume.
    assert!(resp["data"]["activeRun"].is_null());
    assert_eq!(resp["data"]["interruptedRun"]["runId"], "run-interrupted");
    assert_eq!(
        resp["data"]["interruptedRun"]["state"],
        crate::session::RUN_STATE_INTERRUPTED_BY_RESTART
    );
    let _ = state.session_manager.delete(session_id);
}

#[test]
fn get_state_omits_interrupted_run_once_terminal_present() {
    let state = make_app_state();
    let session_id = "gi-terminal";
    let info = crate::session::SessionEntry::session_info(
        serde_json::json!({"cwd": state.welcome_cwd, "model": "mock", "session_name": "n"}),
        "mock".to_string(),
        "low".to_string(),
    );
    let session = crate::session::Session::snapshot(
        session_id.to_string(),
        state.welcome_cwd.clone(),
        "mock".to_string(),
        "n".to_string(),
        String::new(),
        vec![
            info,
            crate::session::SessionEntry::new_user("user", serde_json::json!("hi")),
            crate::session::SessionEntry::run_started("run-done", 1),
            crate::session::SessionEntry::run_terminal(
                "run-done",
                crate::session::RUN_STATE_COMPLETED,
                5,
                50,
                None,
            ),
        ],
    );
    state.session_manager.save(&session).unwrap();

    let mut command = make_cmd_for("get_state", session_id);
    command.run_id = "run-done".to_string();
    let resp = parse_response(&handle_command_internal(&state, command));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["activeRun"].is_null());
    assert!(resp["data"]["interruptedRun"].is_null());
    assert_eq!(resp["data"]["requestedRun"]["run_id"], "run-done");
    assert_eq!(
        resp["data"]["requestedRun"]["state"],
        crate::session::RUN_STATE_COMPLETED
    );
    let _ = state.session_manager.delete(session_id);
}

#[test]
fn get_state_preserves_markerless_legacy_history_without_reporting_interruption() {
    // Backward compatibility: sessions written before run lifecycle markers
    // (run_started/run_terminal) existed carry no run identity in their
    // JSONL. They must never be misclassified as an interrupted run, and
    // the compatibility read must not rewrite or discard their history
    // (no run_id backfill is performed on legacy data).
    let state = make_app_state();
    let session_id = "gi-legacy";
    let info = crate::session::SessionEntry::session_info(
        serde_json::json!({"cwd": state.welcome_cwd, "model": "mock", "session_name": "n"}),
        "mock".to_string(),
        "low".to_string(),
    );
    let session = crate::session::Session::snapshot(
        session_id.to_string(),
        state.welcome_cwd.clone(),
        "mock".to_string(),
        "n".to_string(),
        String::new(),
        vec![
            info,
            crate::session::SessionEntry::new_user("user", serde_json::json!("legacy message")),
            crate::session::SessionEntry::new_assistant(serde_json::json!("legacy reply"), vec![]),
        ],
    );
    state.session_manager.save(&session).unwrap();

    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd_for("get_state", session_id),
    ));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["activeRun"].is_null());
    // No run_started marker → nothing unterminated → not interrupted.
    assert!(resp["data"]["interruptedRun"].is_null());
    let loaded = state.session_manager.load(session_id).unwrap();
    assert!(loaded.entries.iter().any(|entry| entry
        .content
        .as_ref()
        .and_then(|content| content.as_array())
        .and_then(|blocks| blocks.first())
        .and_then(|block| block.get("text"))
        .and_then(|text| text.as_str())
        == Some("legacy message")));
    assert!(loaded.entries.iter().any(|entry| entry
        .content
        .as_ref()
        .and_then(|content| content.as_array())
        .and_then(|blocks| blocks.first())
        .and_then(|block| block.get("text"))
        .and_then(|text| text.as_str())
        == Some("legacy reply")));
    assert!(loaded
        .entries
        .iter()
        .all(|entry| { !is_lifecycle_marker(entry.entry_type.as_str()) }));
    let _ = state.session_manager.delete(session_id);
}

#[test]
fn get_messages_returns_empty() {
    let state = make_app_state();
    let cmd = make_cmd("get_messages");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["messages"].is_array());
}

#[test]
fn get_session_stats_works() {
    let state = make_app_state();
    let cmd = make_cmd("get_session_stats");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["sessionId"].is_string());
}

#[test]
fn get_runtime_metrics_exposes_five_observability_values() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    let (runtime, broadcaster) = {
        let session = session.read();
        (session.runtime.clone(), session.broadcaster.clone())
    };
    let lease = runtime.begin(Some("run-metrics"), None).unwrap();
    broadcaster.record_lag();

    let cmd = make_cmd("get_runtime_metrics");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["sessionId"].is_string());
    assert_eq!(resp["data"]["activeRunGauge"], 1);
    assert_eq!(resp["data"]["activeRunId"], "run-metrics");
    assert_eq!(resp["data"]["broadcastLag"], 1);
    for field in ["staleEpochDrops", "persistenceDegraded", "ringTruncations"] {
        assert_eq!(resp["data"][field], 0, "unexpected {field}");
    }

    assert!(runtime.begin_finalizing(&lease));
    assert!(runtime.finish(&lease));
}

#[test]
fn get_state_emits_canonical_session_name() {
    let state = make_app_state();
    state.sessions.read()["default"]
        .write()
        .set_session_name("My session");
    let resp = parse_response(&handle_command_internal(&state, make_cmd("get_state")));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["sessionName"], "My session");
    // Canonical only — no legacy snake_case alias.
    assert!(resp["data"].get("session_name").is_none());
    // Spot-check canonical camelCase keys around it.
    assert!(resp["data"].get("agentInstanceId").is_some());
    assert!(resp["data"].get("autoCompactionEnabled").is_some());
    assert!(resp["data"].get("queuedCount").is_some());
}

#[test]
fn get_events_since_rejects_unknown_run() {
    let state = make_app_state();
    let mut cmd = make_cmd("get_events_since");
    cmd.run_id = "run_1".to_string();
    cmd.since_idx = -1;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .is_some_and(|error| error.contains("not configured") || error.contains("not known")));
}

#[test]
fn page_events_tail_unlimited_without_max_events() {
    let events = vec![chunk_event(10); 3];
    for max_events in [0, -1] {
        let (page, has_more) = page_events_tail(events.clone(), max_events);
        assert_eq!(page.len(), 3);
        assert!(!has_more);
    }
}

#[test]
fn page_events_tail_count_cap_sets_has_more() {
    let events = vec![chunk_event(10); 5];
    let (page, has_more) = page_events_tail(events, 2);
    assert_eq!(page.len(), 2);
    assert!(has_more);

    // Exact fit: no tail remains, has_more stays false.
    let events = vec![chunk_event(10); 2];
    let (page, has_more) = page_events_tail(events, 2);
    assert_eq!(page.len(), 2);
    assert!(!has_more);
}

#[test]
fn page_events_tail_byte_budget_cuts_before_count_cap() {
    // Events sized to exactly a quarter of the budget (data = text plus
    // the 11-byte `{"text":""}` JSON envelope): four fit, the fifth is
    // cut even though the count cap allows more.
    let quarter = EVENTS_PAGE_BYTE_BUDGET / 4 - EVENT_WIRE_OVERHEAD - 11;
    let events = vec![chunk_event(quarter); 5];
    let (page, has_more) = page_events_tail(events, 10);
    assert_eq!(page.len(), 4);
    assert!(has_more);
}

#[test]
fn page_events_tail_oversized_first_event_still_progresses() {
    // A single event larger than the budget must still go out alone —
    // otherwise the caller's cursor never advances and paging deadlocks.
    let events = vec![chunk_event(EVENTS_PAGE_BYTE_BUDGET + 1), chunk_event(10)];
    let (page, has_more) = page_events_tail(events, 10);
    assert_eq!(page.len(), 1);
    assert!(has_more);
}

#[test]
fn get_events_since_pages_a_live_run_with_max_events() {
    let state = make_app_state();
    let session = state.get_session("default").expect("default session");
    let broadcaster = {
        let sess = session.read();
        sess.broadcaster.start_run("run_page".to_string(), 1);
        sess.broadcaster.clone()
    };
    for idx in 0..5 {
        broadcaster.broadcast(SseEvent::new(
            "text_chunk",
            serde_json::json!({"text": format!("chunk-{idx}")}),
        ));
    }

    // Page 1: since the beginning, two events per page.
    let mut cmd = make_cmd("get_events_since");
    cmd.run_id = "run_page".to_string();
    cmd.since_idx = -1;
    cmd.max_events = 2;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let data = &resp["data"];
    // The paged envelope must still encode its typed payload.
    assert!(future_rpc::encode::response_payload("get_events_since", data).is_some());
    let events = data["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(data["hasMore"], true);
    assert_eq!(events[0]["idx"], 0);
    assert_eq!(events[1]["idx"], 1);

    // Page 2 follows from the last idx; the final page reports no tail.
    let mut cursor = events.last().unwrap()["idx"].as_i64().unwrap();
    let mut seen = events.len();
    loop {
        let mut cmd = make_cmd("get_events_since");
        cmd.run_id = "run_page".to_string();
        cmd.since_idx = cursor;
        cmd.max_events = 2;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        let data = &resp["data"];
        let events = data["events"].as_array().unwrap();
        seen += events.len();
        let has_more = data["hasMore"].as_bool().unwrap_or(false);
        if let Some(last) = events.last() {
            cursor = last["idx"].as_i64().unwrap();
        }
        if !has_more {
            break;
        }
        assert!(!events.is_empty(), "has_more page must not be empty");
    }
    assert_eq!(seen, 5);
    assert_eq!(cursor, 4);

    // Legacy unpaged read: the whole tail, no hasMore key on the wire.
    let mut cmd = make_cmd("get_events_since");
    cmd.run_id = "run_page".to_string();
    cmd.since_idx = -1;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    let data = &resp["data"];
    assert_eq!(data["events"].as_array().unwrap().len(), 5);
    assert!(data.get("hasMore").is_none());
}

#[test]
#[cfg(not(windows))]
fn export_html_writes_file() {
    let _gate = EXPORT_TEST_LOCK.lock();
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("export_html")));
    assert_eq!(resp["success"], true);
    let path = resp["data"]["path"].as_str().unwrap();
    assert!(path.contains("future_agent_export_"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn get_session_events_since_returns_empty_tail() {
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("get_session_events_since"),
    ));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["events"], serde_json::json!([]));
}

// ── coverage batch 1: switch/delete session ─────────────────────────────

#[test]
fn get_session_events_since_returns_events_then_journal_error() {
    let state = make_app_state();
    // Broadcast a session-level event so the journal has content.
    let mut cmd = make_cmd("set_model");
    cmd.model_id = "mock".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);

    let mut cmd = make_cmd("get_session_events_since");
    cmd.since_idx = -1;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let events = resp["data"]["events"].as_array().unwrap();
    assert!(!events.is_empty());
    assert_eq!(events[0]["type"], "model_changed");

    // A directory where the journal file should be breaks reads.
    let journal = state
        .session_manager
        .run_data_path("default")
        .join("_session.jsonl");
    std::fs::remove_file(&journal).unwrap();
    std::fs::create_dir_all(&journal).unwrap();
    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("get_session_events_since"),
    ));
    assert_eq!(resp["success"], false);
    let _ = std::fs::remove_dir_all(&journal);
}

#[test]
fn get_events_since_returns_projection_over_truncated_ring() {
    let state = make_app_state();
    {
        let session = state.get_session("default").unwrap();
        let mut sess = session.write();
        // No journal configured → a cursor older than the in-memory ring
        // returns a compressed projection instead of a partial tail.
        sess.broadcaster = Arc::new(SseBroadcaster::new());
        sess.broadcaster.start_run("run-ring".to_string(), 1);
        for i in 0..2100 {
            sess.broadcaster
                .broadcast(SseEvent::new("text_chunk", serde_json::json!({"i": i})));
        }
    }
    let mut cmd = make_cmd("get_events_since");
    cmd.run_id = "run-ring".to_string();
    cmd.since_idx = 0;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert!(!resp["data"]["projection"].is_null());
}

#[test]
fn export_html_reports_write_failure() {
    let _gate = EXPORT_TEST_LOCK.lock();
    let _guard = ExportDirGuard::new(std::path::PathBuf::from(
        "/definitely/not/a/real/export/dir",
    ));
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("export_html")));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("failed to write file"));
}

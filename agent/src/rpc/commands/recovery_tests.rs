//! Tests for `acknowledge_recovery` — the explicit release of a recovery fence.
//!
//! Driven through the public dispatcher, so the registry entry, the session
//! resolution and the journal write are all exercised together.

use super::test_support::*;
use super::*;
use crate::session::{
    SessionEntry, ENTRY_NAME_RECOVERY_ACKNOWLEDGED, ENTRY_TYPE_CUSTOM,
    RUN_STATE_INTERRUPTED_BY_RESTART,
};
use crate::types::{ToolCall, ToolCallFn};
use serde_json::json;

fn shell_call(id: &str, command: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        call_type: "function".into(),
        function: ToolCallFn {
            name: "shell".into(),
            arguments: json!({ "command": command }),
        },
    }
}

/// A session whose restart cut two shell calls off: exactly the shape the
/// Agent left on disk in the incident this command exists for.
fn fenced_session(state: &AppState) {
    let entries = vec![
        SessionEntry::new_user("user", json!("finish my work")),
        SessionEntry::run_started("run", 1),
        SessionEntry::new_assistant(
            json!("working"),
            vec![
                shell_call("call-a", "make install"),
                shell_call("call-b", "rm -rf build"),
            ],
        ),
        SessionEntry::run_terminal("run", RUN_STATE_INTERRUPTED_BY_RESTART, 0, 0, None),
    ];
    save_via(state, "default", "mock", entries);
}

fn acknowledge(state: &AppState) -> serde_json::Value {
    let mut cmd = make_cmd("acknowledge_recovery");
    cmd.message = "I checked the workspace".into();
    cmd.created_by = "cli".into();
    parse_response(&handle_command_internal(state, cmd))
}

fn ack_entries(state: &AppState) -> Vec<SessionEntry> {
    state
        .session_manager
        .load_unrepaired_entries("default")
        .unwrap()
        .into_iter()
        .filter(|entry| {
            entry.entry_type == ENTRY_TYPE_CUSTOM && entry.name == ENTRY_NAME_RECOVERY_ACKNOWLEDGED
        })
        .collect()
}

#[test]
fn releases_exactly_the_fenced_calls_and_records_who_and_why() {
    let state = make_app_state();
    fenced_session(&state);
    let response = acknowledge(&state);
    assert_eq!(response["success"], true, "{response}");
    let released = response["data"]["released"].as_array().expect("released");
    assert_eq!(released.len(), 2, "{response}");
    let ids: Vec<&str> = released
        .iter()
        .map(|call| call["call_id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&"call-a") && ids.contains(&"call-b"),
        "{response}"
    );
    assert_eq!(released[0]["run_id"], "run");
    assert_eq!(response["data"]["replayed"], false);

    let recorded = ack_entries(&state);
    assert_eq!(recorded.len(), 1, "one release, one entry");
    let content = recorded[0].content.as_ref().unwrap();
    assert_eq!(content["by"], "cli");
    assert_eq!(content["reason"], "I checked the workspace");
    assert!(content["acknowledged_at"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));
    assert_eq!(content["calls"].as_array().unwrap().len(), 2);
}

#[test]
fn a_second_release_is_a_no_op() {
    // Idempotency is a property of the command, not of the caller: a retried
    // request must not append a second entry or claim a second release.
    let state = make_app_state();
    fenced_session(&state);
    let first = acknowledge(&state);
    assert_eq!(first["data"]["released"].as_array().unwrap().len(), 2);

    let second = acknowledge(&state);
    assert_eq!(second["success"], true, "{second}");
    assert!(
        second["data"]["released"].as_array().unwrap().is_empty(),
        "{second}"
    );
    assert_eq!(second["data"]["already_released"], true);
    assert_eq!(ack_entries(&state).len(), 1, "the retry appended nothing");
}

#[test]
fn an_unfenced_session_reports_nothing_to_release() {
    let state = make_app_state();
    let entries = vec![
        SessionEntry::new_user("user", json!("hello")),
        SessionEntry::run_started("run", 1),
        SessionEntry::new_assistant(json!("hi"), vec![]),
        SessionEntry::run_terminal("run", crate::session::RUN_STATE_COMPLETED, 0, 0, None),
    ];
    save_via(&state, "default", "mock", entries);
    let response = acknowledge(&state);
    assert_eq!(response["success"], true, "{response}");
    assert!(response["data"]["released"].as_array().unwrap().is_empty());
    assert_eq!(response["data"]["already_released"], true);
    assert!(ack_entries(&state).is_empty(), "no entry for nothing");
}

#[test]
fn releasing_never_rewrites_the_history_it_was_asked_about() {
    // The journal is append-only here: the interrupted call is still there to
    // be read and reconciled afterwards.
    let state = make_app_state();
    fenced_session(&state);
    let before = state
        .session_manager
        .load_unrepaired_entries("default")
        .unwrap();
    let mut cmd = make_cmd("acknowledge_recovery");
    cmd.message = String::new();
    handle_command_internal(&state, cmd);
    let after = state
        .session_manager
        .load_unrepaired_entries("default")
        .unwrap();
    assert_eq!(after.len(), before.len() + 1, "exactly one appended entry");
    let unchanged: Vec<&SessionEntry> = after
        .iter()
        .filter(|entry| {
            !(entry.entry_type == ENTRY_TYPE_CUSTOM
                && entry.name == ENTRY_NAME_RECOVERY_ACKNOWLEDGED)
        })
        .collect();
    assert_eq!(unchanged.len(), before.len());
    for (old, new) in before.iter().zip(unchanged) {
        assert_eq!(old.id, new.id);
        assert_eq!(old.content, new.content);
    }
}

//! Task/store and real-RPC seeds shared by core and GUI entry tests.
use future_tasks::{new_task_id, SessionPolicy, Task, TriggerKind};

pub fn task(name: &str) -> Task {
    Task {
        id: new_task_id(),
        name: name.into(),
        enabled: true,
        prompt: "do the thing".into(),
        prompt_version: 1,
        cwd: std::env::temp_dir().display().to_string(),
        model_id: None,
        thinking_level: None,
        timezone: None,
        session_policy: SessionPolicy::New,
        conversation_mode: future_tasks::ConversationMode::Workspace,
        thread_id: None,
        trigger_kind: TriggerKind::Manual,
        trigger_json: serde_json::json!({}),
        next_due_at: None,
        pending_request_at: None,
        pending_origin: None,
        pending_actor: None,
        session_retention: future_tasks::SessionRetention::Keep,
        created_at: 1,
        updated_at: 1,
        deleted_at: None,
    }
}

/// Script the commands a run sends before its stream: session creation,
/// the permission/sandbox push, and the idle probe.
pub fn script_agent_setup(mock: &crate::test_support::agent::MockAgentGuard, session: &str) {
    script_agent_setup_with_answer(
        mock,
        session,
        serde_json::json!({ "text": "the finished answer" }),
    );
}

/// [`script_agent_setup`], with the answer the run reads back from
/// `get_last_assistant_text`. `{"text": null}` is what an empty answer looks
/// like, and `lastTool` is the only clue a `session_retention = delete` task
/// keeps after deleting the conversation.
pub fn script_agent_setup_with_answer(
    mock: &crate::test_support::agent::MockAgentGuard,
    session: &str,
    answer: serde_json::Value,
) {
    script_agent_setup_with_history(mock, session, answer, recorded_turn());
}

/// [`script_agent_setup`] for a run whose conversation recorded **nothing**: the
/// session exists (the run opened it) and holds only the `session_info` snapshot
/// every session is born with. This is the empty shell a run leaves when its
/// prompt never reached the Agent, and the one the settle path removes.
pub fn script_agent_setup_without_records(
    mock: &crate::test_support::agent::MockAgentGuard,
    session: &str,
) {
    script_agent_setup_with_history(
        mock,
        session,
        serde_json::json!({ "text": "the finished answer" }),
        serde_json::json!({ "entries": [
            {"id":"s1", "kind":"session_info", "role":"", "createdAtMs":1000, "blocks":[]}
        ] }),
    );
}

/// The history page a run that was prompted leaves behind: the snapshot the
/// session was born with, then one user exchange and its answer.
fn recorded_turn() -> serde_json::Value {
    serde_json::json!({"entries": [
        {"id":"s1", "kind":"session_info", "role":"", "createdAtMs":999, "blocks":[]},
        {"id":"u1", "kind":"user", "role":"user", "createdAtMs":1000, "blocks":[]},
        {"id":"a1", "kind":"assistant", "role":"assistant", "createdAtMs":1001, "blocks":[]}
    ]})
}

/// The setup every task-run test scripts, with the session's history spelled
/// out. Settling reads that history to tell a conversation with something in it
/// from an empty shell it must remove, so the default (`{}`, which reads as "no
/// records") is never what a prompted run should leave.
fn script_agent_setup_with_history(
    mock: &crate::test_support::agent::MockAgentGuard,
    session: &str,
    answer: serde_json::Value,
    history: serde_json::Value,
) {
    mock.push_data("new_session", serde_json::json!({ "sessionId": session }));
    mock.push_data("set_permission_level", serde_json::json!({}));
    mock.push_data(
        "set_sandbox_policy",
        serde_json::json!({ "sandboxAvailable": true }),
    );
    // `get_state` must be typed: the idle probe decodes the typed payload.
    // Repeated idle probes may be needed when reusing a conversation.
    for _ in 0..4 {
        mock.push_typed_data(
            "get_state",
            crate::test_support::agent::get_state_payload(session, false),
        );
    }
    mock.push_typed_data("get_session_entries", history);
    mock.push_data("get_last_assistant_text", answer);
}

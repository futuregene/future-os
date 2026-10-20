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
    mock.push_data(
        "get_last_assistant_text",
        serde_json::json!({ "text": "the finished answer" }),
    );
}

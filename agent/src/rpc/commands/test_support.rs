//! Shared test helpers for the command-handler test modules.
//!
//! The command tests are integration tests: they build an `AppState` and drive
//! it through the public `handle_command_internal` entry point, so the helpers
//! to construct that state and its commands live here, shared by every
//! `*_tests.rs` module.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    agent::Loop,
    llm::schema::{ModelRequest, ModelStreamEvent},
    rpc::{AppState, ApprovalGate, RpcCommand, ServerSession, SseBroadcaster, SseEvent},
    types::LLMProvider,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub(crate) struct EmptyProvider;

#[async_trait::async_trait]
impl LLMProvider for EmptyProvider {
    async fn stream_model(
        &self,
        _request: ModelRequest,
    ) -> anyhow::Result<ReceiverStream<ModelStreamEvent>> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(ReceiverStream::new(rx))
    }
}

pub(crate) fn test_workspace() -> String {
    crate::test_support::unique_temp_path("cmd-test")
        .to_string_lossy()
        .to_string()
}

/// Unique, isolated session directory for a test's AppState. Each call
/// gets its own temp dir (timestamp + random hex) so parallel tests never
/// share a `default.jsonl`, and nothing is ever written to the real
/// `~/.future/agent/sessions` store (which `Manager::default_for` would
/// target, since `default_session_dir` ignores its cwd argument).
pub(crate) fn test_session_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("futureos-cmd-sess-{}", crate::utils::generate_id()))
}

pub(crate) fn make_app_state() -> AppState {
    make_app_state_with(
        test_session_dir(),
        Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
    )
}

pub(crate) fn make_app_state_with(
    session_dir: std::path::PathBuf,
    queue_budget: Arc<crate::runtime::GlobalQueueBudget>,
) -> AppState {
    make_app_state_with_provider(Arc::new(EmptyProvider), session_dir, queue_budget)
}

/// The same state, with a provider the test controls.
///
/// Restart recovery is the one path that decides *and* calls the model without
/// a client, so the test has to see what the model was asked.
pub(crate) fn make_app_state_with_provider(
    provider: Arc<dyn LLMProvider>,
    session_dir: std::path::PathBuf,
    queue_budget: Arc<crate::runtime::GlobalQueueBudget>,
) -> AppState {
    make_app_state_with_format(provider, session_dir, queue_budget, false)
}

/// Released raw-journal fixtures select their format before reserving the id.
pub(crate) fn make_legacy_app_state() -> AppState {
    make_app_state_with_format(
        Arc::new(EmptyProvider),
        test_session_dir(),
        Arc::new(crate::runtime::GlobalQueueBudget::defaults()),
        true,
    )
}

fn make_app_state_with_format(
    provider: Arc<dyn LLMProvider>,
    session_dir: std::path::PathBuf,
    queue_budget: Arc<crate::runtime::GlobalQueueBudget>,
    legacy: bool,
) -> AppState {
    let cwd = test_workspace();
    let model_registry = Arc::new(parking_lot::RwLock::new(crate::models::Registry::new()));
    let session_manager = Arc::new(crate::session::Manager::new(session_dir));
    if legacy {
        session_manager
            .storage()
            .unwrap()
            .bind_legacy_events("default")
            .unwrap();
    }
    let approval_gate = ApprovalGate::default();
    // One live session named "default" — sessions are equal peers now,
    // so tests address it explicitly by id.
    let session = ServerSession::new_with_queue_budget(
        "default".to_string(),
        Arc::new(tokio::sync::RwLock::new(Loop::new(
            provider.clone(),
            "mock",
        ))),
        session_manager.clone(),
        &cwd,
        Arc::new(SseBroadcaster::new()),
        approval_gate.clone(),
        model_registry.clone(),
        queue_budget.clone(),
    );
    let sessions: HashMap<String, Arc<parking_lot::RwLock<ServerSession>>> = [(
        "default".to_string(),
        Arc::new(parking_lot::RwLock::new(session)),
    )]
    .into_iter()
    .collect();
    AppState {
        agent_instance_id: "agent-test-instance".to_string(),
        control_plane: std::sync::Arc::new(crate::rpc::ControlPlane::default()),
        data_root_id: "0".repeat(64),
        sessions: Arc::new(parking_lot::RwLock::new(sessions)),
        queue_budget,
        session_manager,
        welcome_version: "0.0.0".to_string(),
        welcome_cwd: cwd.clone(),
        welcome_skills: Arc::new(parking_lot::RwLock::new(vec![])),
        welcome_exts: vec![],
        explicit_session: false,
        approval_gate,
        verbose: false,
        shutting_down: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        service_work: Arc::new(crate::service::work::ServiceWork::default()),
        model_registry: model_registry.clone(),
        loop_template: Arc::new(Loop::new(provider, "mock")),
    }
}

pub(crate) fn make_cmd(cmd_type: &str) -> RpcCommand {
    serde_json::from_str(&format!(
        r#"{{"id":"test_cmd","type":"{}","sessionId":"default"}}"#,
        cmd_type
    ))
    .unwrap()
}

pub(crate) fn make_cmd_for(cmd_type: &str, session_id: &str) -> RpcCommand {
    serde_json::from_str(&format!(
        r#"{{"id":"test_cmd","type":"{}","sessionId":"{}"}}"#,
        cmd_type, session_id
    ))
    .unwrap()
}

pub(crate) fn parse_response(json: &str) -> serde_json::Value {
    serde_json::from_str(json).unwrap()
}

pub(crate) fn is_lifecycle_marker(entry_type: &str) -> bool {
    matches!(
        entry_type,
        crate::session::ENTRY_TYPE_RUN_STARTED | crate::session::ENTRY_TYPE_RUN_TERMINAL
    )
}

pub(crate) fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

pub(crate) fn chunk_event(data_size: usize) -> SseEvent {
    SseEvent::new(
        "text_chunk",
        serde_json::json!({"text": "x".repeat(data_size)}),
    )
}

pub(crate) fn save_via(
    state: &AppState,
    session_id: &str,
    model: &str,
    entries: Vec<crate::session::SessionEntry>,
) {
    let snapshot = crate::session::Session::snapshot(
        session_id.to_string(),
        state.welcome_cwd.clone(),
        model.to_string(),
        String::new(),
        String::new(),
        entries,
    );
    state.session_manager.save(&snapshot).unwrap();
}

/// Exact equality with a compact field path instead of megabytes of body text.
pub(crate) fn assert_json_eq(left: &serde_json::Value, right: &serde_json::Value) {
    fn difference(left: &serde_json::Value, right: &serde_json::Value, path: &str) -> String {
        match (left, right) {
            (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
                for key in a.keys().chain(b.keys()) {
                    if a.get(key) != b.get(key) {
                        return difference(&a[key], &b[key], &format!("{path}/{key}"));
                    }
                }
            }
            (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
                for (index, (x, y)) in a.iter().zip(b).enumerate() {
                    if x != y {
                        return difference(x, y, &format!("{path}/{index}"));
                    }
                }
            }
            (serde_json::Value::String(a), serde_json::Value::String(b)) => {
                if let (Ok(x), Ok(y)) = (
                    serde_json::from_str::<serde_json::Value>(a),
                    serde_json::from_str::<serde_json::Value>(b),
                ) {
                    if x == y {
                        return format!("{path} (JSON encoding differs)");
                    }
                    return difference(&x, &y, &format!("{path}:json"));
                }
            }
            _ => {}
        }
        path.to_owned()
    }
    assert!(
        left == right,
        "JSON differs at {}",
        difference(left, right, "")
    );
}

//! Tests for the session settings command handlers.

use crate::test_support::TestHome;

use crate::rpc::commands::test_support::*;
use crate::rpc::handle_command_internal;
use crate::types::{AgentMessage, ContentBlock, LLMProvider};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

struct SlowSummaryProvider;

#[async_trait::async_trait]
impl LLMProvider for SlowSummaryProvider {
    async fn stream_model(
        &self,
        _request: crate::llm::schema::ModelRequest,
    ) -> anyhow::Result<ReceiverStream<crate::llm::schema::ModelStreamEvent>> {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let (tx, rx) = mpsc::channel(2);
        tx.try_send(crate::llm::schema::ModelStreamEvent::TextDelta {
            text: "## Objective\n- test\n\n## Important Details\n- none\n\n## Work State\n### Completed\n- none\n\n### Active\n- test\n\n### Blocked\n- none\n\n## Next Move\n1. test\n\n## Relevant Files\n- none".to_string(),
            id: String::new(),
        })
        .unwrap();
        tx.try_send(crate::llm::schema::ModelStreamEvent::Finish {
            reason: crate::llm::schema::FinishReason::Stop,
            usage: None,
        })
        .unwrap();
        Ok(ReceiverStream::new(rx))
    }
}

#[test]
fn set_permission_level_valid() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_permission_level");
    cmd.level = "workspace".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["permissionLevel"], "workspace");
}

#[test]
fn set_permission_level_invalid() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_permission_level");
    cmd.level = "invalid_level".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("invalid level"));
}

#[test]
fn set_thinking_level_works() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_thinking_level");
    cmd.level = "high".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn set_auto_compaction_works() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_auto_compaction");
    cmd.enabled = false;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn set_auto_retry_works() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_auto_retry");
    cmd.enabled = true;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn set_ephemeral_works() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_ephemeral");
    cmd.ephemeral = true;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["ephemeral"], true);
}

#[test]
fn cycle_thinking_level_advances() {
    let state = make_app_state();
    let cmd = make_cmd("cycle_thinking_level");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["level"].is_string());
}

#[test]
fn disable_tools_works() {
    let state = make_app_state();
    let cmd = make_cmd("disable_tools");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn disable_builtin_tools_works() {
    let state = make_app_state();
    let cmd = make_cmd("disable_builtin_tools");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn set_system_prompt_works() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_system_prompt");
    cmd.system_prompt = "You are helpful".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn append_system_prompt_works() {
    let state = make_app_state();
    let mut cmd = make_cmd("append_system_prompt");
    cmd.system_prompt = "Extra instructions".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn set_cwd_trims_trailing_slash() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_cwd");
    cmd.cwd = "/tmp/project/ ".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["cwd"], "/tmp/project");
}

/// `create_session` swaps in a fresh private broadcaster (fork/clone pass
/// the parent's); the event journal must be rebound to that broadcaster or
/// events silently stay memory-only and the durable journal is never
/// written. Regression guard for the Runs-panel blanking bug.
#[test]
fn set_sandbox_policy_missing_payload() {
    let state = make_app_state();
    let cmd = make_cmd("set_sandbox_policy");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"]
        .as_str()
        .unwrap()
        .contains("missing sandbox_policy"));
}

#[test]
fn compact_empty_session_returns_async_ack() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    let mut events = session.read().broadcaster.subscribe();
    let cmd = make_cmd("compact");
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["accepted"], true);
    let operation_id = resp["data"]["operationId"].as_str().unwrap();
    let terminal = events.blocking_recv().unwrap();
    assert_eq!(terminal.event_type, "compaction_unchanged");
    assert!(!session
        .read()
        .compaction_in_progress
        .load(std::sync::atomic::Ordering::Acquire));
    let data: serde_json::Value = serde_json::from_str(&terminal.data).unwrap();
    assert_eq!(data["operation_id"], operation_id);
}

#[test]
fn compact_reports_busy_loop_error_as_terminal_event() {
    let state = make_app_state();
    // Hold the run-configuration lock so the compaction snapshot's try_read
    // fails — the manual /compact path must report that as a clean error.
    let session = state.get_session("default").unwrap();
    let mut events = session.read().broadcaster.subscribe();
    let agent_loop = session.read().agent_loop.clone();
    let _guard = agent_loop.try_write().unwrap();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("compact")));
    assert_eq!(resp["success"], true);
    let operation_id = resp["data"]["operationId"].as_str().unwrap();
    let terminal = events.blocking_recv().unwrap();
    assert_eq!(terminal.event_type, "compaction_failed");
    assert!(!session
        .read()
        .compaction_in_progress
        .load(std::sync::atomic::Ordering::Acquire));
    let data: serde_json::Value = serde_json::from_str(&terminal.data).unwrap();
    assert_eq!(data["operation_id"], operation_id);
    assert!(data["error"].as_str().unwrap().contains("busy"));
}

#[test]
fn compact_rejects_a_duplicate_async_request() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .compaction_in_progress
        .store(true, std::sync::atomic::Ordering::Release);
    let resp = parse_response(&handle_command_internal(&state, make_cmd("compact")));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("already running"));
    session
        .read()
        .compaction_in_progress
        .store(false, std::sync::atomic::Ordering::Release);
}

#[test]
fn compact_retry_with_the_same_request_id_reuses_the_operation() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .compaction_in_progress
        .store(true, std::sync::atomic::Ordering::Release);
    *session.read().compaction_request.lock() =
        Some(("test_cmd".to_string(), "cmp-existing".to_string()));

    let resp = parse_response(&handle_command_internal(&state, make_cmd("compact")));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["operationId"], "cmp-existing");
}

#[test]
fn completed_compact_retry_with_the_same_request_id_reuses_the_operation() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    *session.read().compaction_request.lock() =
        Some(("test_cmd".to_string(), "cmp-complete".to_string()));

    let resp = parse_response(&handle_command_internal(&state, make_cmd("compact")));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["operationId"], "cmp-complete");
    assert!(!session
        .read()
        .compaction_in_progress
        .load(std::sync::atomic::Ordering::Acquire));
}

#[test]
fn session_mutations_fail_before_waiting_for_compaction() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session
        .read()
        .compaction_in_progress
        .store(true, std::sync::atomic::Ordering::Release);

    let resp = parse_response(&handle_command_internal(&state, make_cmd("prompt")));
    assert_eq!(resp["success"], false);
    assert_eq!(resp["error_code"], "session_busy");
    assert_eq!(resp["error_data"]["busy_reason"], "compaction");
    assert_eq!(resp["error_data"]["retryable"], true);
}

#[test]
fn compact_ack_does_not_wait_for_the_summary_provider() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    {
        let session = session.write();
        session.agent_loop.try_write().unwrap().provider = Arc::new(SlowSummaryProvider);
        let mut messages = session.messages.write();
        for (role, text) in [("user", "investigate"), ("assistant", "working")] {
            let mut message = AgentMessage {
                role: role.to_string(),
                content: vec![ContentBlock::text(text)],
                ..Default::default()
            };
            message.ensure_journal_entry_id();
            messages.push(message);
        }
    }
    let mut events = session.read().broadcaster.subscribe();
    let started_at = std::time::Instant::now();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("compact")));
    assert_eq!(resp["success"], true);
    assert!(started_at.elapsed() < std::time::Duration::from_millis(100));
    assert_eq!(
        events.blocking_recv().unwrap().event_type,
        "compaction_started"
    );
    let terminal = events.blocking_recv().unwrap();
    assert!(matches!(
        terminal.event_type.as_str(),
        "compaction_committed" | "compaction_failed"
    ));
}

#[test]
fn shell_echo() {
    let state = make_app_state();
    std::fs::create_dir_all(&state.welcome_cwd).unwrap();
    let mut cmd = make_cmd("shell");
    cmd.command = "echo test_output".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["output"]
        .as_str()
        .unwrap()
        .contains("test_output"));
    assert_eq!(resp["data"]["exitCode"], 0);
}

#[test]
fn reload_config_reports_busy_loop_and_skips_locked_update() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    let agent_loop = session.read().agent_loop.clone();
    {
        // A held WRITE guard makes the first try_read fail.
        let _write_guard = agent_loop.try_write().unwrap();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("agent is busy"));
    }
    // A held READ guard passes the try_read but blocks the final try_write
    // — the command still succeeds, just without updating the prompt.
    let _read_guard = agent_loop.try_read().unwrap();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
    assert_eq!(resp["success"], true);
}

#[test]
fn reload_config_tolerates_unreadable_context_file() {
    let state = make_app_state();
    // A CLAUDE.md that is a DIRECTORY exists but cannot be read.
    let cwd = state.welcome_cwd.clone();
    std::fs::create_dir_all(std::path::Path::new(&cwd).join("CLAUDE.md")).unwrap();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["contextFiles"], serde_json::json!([]));
    let _ = std::fs::remove_dir_all(&cwd);
}

#[test]
fn add_session_rule_works() {
    let state = make_app_state();
    let mut cmd = make_cmd("add_session_rule");
    cmd.message = "/tmp/**".to_string();
    cmd.mode = "read".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

/// The gRPC boundary dual-writes a typed payload for the Tier-1 read
/// commands; this pins that the agent's REAL envelopes always encode
/// (a None here would silently degrade typed clients to the JSON
/// fallback). get_events_since is covered by the future-rpc parity
/// fixtures — it needs a live run this fixture does not have.
#[test]
fn set_model_updates_session_and_broadcasts() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_model");
    cmd.model_id = "mock".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["model"], "mock");
}

#[test]
fn set_tools_broadcasts_new_tool_list() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_tools");
    cmd.tools = vec!["read".to_string(), "write".to_string()];
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["tools"], serde_json::json!(["read", "write"]));
}

#[test]
fn set_ephemeral_and_last_assistant_text() {
    let state = make_app_state();

    let resp = parse_response(&handle_command_internal(
        &state,
        make_cmd("get_last_assistant_text"),
    ));
    assert_eq!(resp["success"], true);
    assert!(resp["data"]["text"].is_null());
}

#[test]
fn set_session_name_on_unpersisted_session_broadcasts() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_session_name");
    cmd.name = "my session".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let session = state.get_session("default").unwrap();
    assert_eq!(session.read().session_name, "my session");
}

#[test]
fn set_session_name_persists_to_disk_session_info() {
    let state = make_app_state();
    // Persist the session (with a session_info entry) so the update_info
    // branch fires and the name lands on disk.
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

    let mut cmd = make_cmd("set_session_name");
    cmd.name = "persisted name".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let loaded = state.session_manager.load("default").unwrap();
    assert_eq!(loaded.name, "persisted name");
}

#[test]
fn cycle_model_with_no_credentialled_models_returns_empty() {
    let _home = TestHome::new();
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("cycle_model")));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["model"], "");
    assert_eq!(resp["data"]["thinkingLevel"], "");
}

#[test]
fn cycle_model_advances_to_next_credentialled_model() {
    let home = TestHome::new();
    let state = make_app_state();
    // Credential the provider of the first two catalog models so cycling
    // has somewhere to go.
    let providers: Vec<String> = {
        let registry = state.model_registry.read();
        let models = registry.all_models();
        let mut providers: Vec<String> = models.iter().map(|m| m.provider.clone()).collect();
        providers.sort();
        providers.dedup();
        providers.truncate(2);
        providers
    };
    assert!(!providers.is_empty(), "builtin catalog is never empty");
    let mut auth = serde_json::json!({});
    for provider in &providers {
        auth[provider] = serde_json::json!({"type": "api_key", "key": "k"});
    }
    let auth_path = home.auth_path();
    std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
    std::fs::write(&auth_path, serde_json::to_string_pretty(&auth).unwrap()).unwrap();
    *state.model_registry.write() = crate::models::Registry::new();

    let resp = parse_response(&handle_command_internal(&state, make_cmd("cycle_model")));
    assert_eq!(resp["success"], true);
    let next = resp["data"]["model"].as_str().unwrap();
    assert!(!next.is_empty());
    assert_eq!(resp["data"]["isScoped"], false);
}

#[test]
fn reload_config_without_context_file_returns_empty_list() {
    let state = make_app_state();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["contextFiles"], serde_json::json!([]));
}

#[test]
fn reload_config_picks_up_context_file() {
    let state = make_app_state();
    let cwd = state.welcome_cwd.clone();
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(std::path::Path::new(&cwd).join("CLAUDE.md"), "# context").unwrap();

    let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
    assert_eq!(resp["success"], true);
    assert_eq!(
        resp["data"]["contextFiles"],
        serde_json::json!(["CLAUDE.md"])
    );
    assert_eq!(
        state.welcome_context.read().as_slice(),
        &["# context".to_string()]
    );
    let _ = std::fs::remove_dir_all(&cwd);
}

// ── coverage batch 2: error-path arms ───────────────────────────────────

#[test]
fn set_model_fails_while_loop_is_locked() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    let agent_loop = session.read().agent_loop.clone();
    let _guard = agent_loop.try_write().unwrap();

    let mut cmd = make_cmd("set_model");
    cmd.model_id = "mock".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("busy"));
}

#[test]
fn shell_fails_with_missing_cwd() {
    let state = make_app_state(); // test_workspace() is never created
    let mut cmd = make_cmd("shell");
    cmd.command = "echo hi".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
}

#[test]
fn set_session_name_survives_persist_error() {
    let state = make_app_state();
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
    // Break the on-disk file so update_info fails (logged, still ok).
    let path = state.session_manager.find("default").unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir_all(&path).unwrap();

    let mut cmd = make_cmd("set_session_name");
    cmd.name = "still works".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn set_cwd_survives_persist_error() {
    let state = make_app_state();
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
    let path = state.session_manager.find("default").unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir_all(&path).unwrap();

    let mut cmd = make_cmd("set_cwd");
    cmd.cwd = "/tmp/new-cwd".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["cwd"], "/tmp/new-cwd");
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn cycle_model_fails_while_loop_is_locked() {
    let home = TestHome::new();
    let state = make_app_state();
    let providers: Vec<String> = {
        let registry = state.model_registry.read();
        let mut providers: Vec<String> = registry
            .all_models()
            .iter()
            .map(|m| m.provider.clone())
            .collect();
        providers.sort();
        providers.dedup();
        providers.truncate(2);
        providers
    };
    let mut auth = serde_json::json!({});
    for provider in &providers {
        auth[provider] = serde_json::json!({"type": "api_key", "key": "k"});
    }
    let auth_path = home.auth_path();
    std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
    std::fs::write(&auth_path, serde_json::to_string_pretty(&auth).unwrap()).unwrap();
    *state.model_registry.write() = crate::models::Registry::new();

    let session = state.get_session("default").unwrap();
    let agent_loop = session.read().agent_loop.clone();
    let _guard = agent_loop.try_write().unwrap();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("cycle_model")));
    assert_eq!(resp["success"], false);
}

#[test]
fn set_sandbox_policy_applies_tier() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_sandbox_policy");
    cmd.sandbox_policy = Some(crate::sandbox::SandboxPolicy {
        tier: crate::sandbox::SandboxTier::Off,
    });
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["tier"], "off");
    assert!(resp["data"]["sandboxAvailable"].is_boolean());
}

#[test]
fn set_cwd_persists_successfully_on_disk_session() {
    let state = make_app_state();
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
    let mut cmd = make_cmd("set_cwd");
    cmd.cwd = "/tmp/persisted-cwd".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["cwd"], "/tmp/persisted-cwd");
}

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

/// A catalogue of custom models, loaded through the real `models.json` path so a
/// provider-level default and a model-level override are exercised as filed.
fn state_from_models(providers: serde_json::Value) -> (TestHome, crate::rpc::AppState) {
    let home = TestHome::new();
    let path = home.models_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        serde_json::json!({"providers": providers}).to_string(),
    )
    .unwrap();
    let state = make_app_state();
    // Behind the command boundary the registry is swapped atomically on a
    // provider commit; mimic that swap for a file written by the test.
    *state.model_registry.write() = crate::models::Registry::new();
    (home, state)
}

fn select_model(state: &crate::rpc::AppState, model: &str) -> serde_json::Value {
    let mut cmd = make_cmd("set_model");
    cmd.model_id = model.to_string();
    let resp = parse_response(&handle_command_internal(state, cmd));
    assert_eq!(resp["success"], true, "{resp}");
    resp
}

/// The agent's clamp is the one place a level is snapped, so the RPC echo must
/// report the level the session actually took rather than the requested one.
#[test]
fn set_thinking_level_snaps_onto_the_models_declared_levels() {
    let (_home, state) = state_from_models(serde_json::json!({
        "narrow": {
            "api": "openai-completions",
            "baseUrl": "https://example.test/v1",
            "apiKey": "k",
            "models": [{"id": "m", "thinkingLevels": ["low", "high"]}]
        }
    }));
    select_model(&state, "narrow/m");

    let mut cmd = make_cmd("set_thinking_level");
    cmd.level = "medium".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true, "{resp}");
    // A level the model does not offer falls back to the provider default.
    assert_eq!(resp["data"]["level"], "default");
    assert_eq!(
        state.get_session("default").unwrap().read().thinking_level,
        "default"
    );
}

/// Cycling walks the model's own list, not the full six: stepping onto a level
/// that is immediately clamped away would read as a stuck key.
#[test]
fn cycle_thinking_level_walks_only_the_declared_levels() {
    let (_home, state) = state_from_models(serde_json::json!({
        "narrow": {
            "api": "openai-completions",
            "baseUrl": "https://example.test/v1",
            "apiKey": "k",
            "models": [{"id": "m", "thinkingLevels": ["low", "high"]}]
        }
    }));
    select_model(&state, "narrow/m");

    let seen: Vec<String> = (0..4)
        .map(|_| {
            let resp = parse_response(&handle_command_internal(
                &state,
                make_cmd("cycle_thinking_level"),
            ));
            assert_eq!(resp["success"], true, "{resp}");
            resp["data"]["level"].as_str().unwrap().to_string()
        })
        .collect();
    // The ring is the effective list: default -> low -> high -> default.
    assert_eq!(seen, ["low", "high", "default", "low"]);
}

/// A model switch can narrow the vocabulary, and the level travels with every
/// request — so the switch itself has to re-clamp, persist, and announce it.
#[test]
fn switching_to_a_narrower_model_reclamps_the_running_level() {
    let (_home, state) = state_from_models(serde_json::json!({
        "wide": {
            "api": "openai-completions",
            "baseUrl": "https://example.test/v1",
            "apiKey": "k",
            "models": [{"id": "m", "thinkingLevels": ["off", "minimal", "low", "medium", "high", "xhigh"]}]
        },
        "narrow": {
            "api": "openai-completions",
            "baseUrl": "https://example.test/v1",
            "apiKey": "k",
            "models": [{"id": "m", "thinkingLevels": ["off", "low", "high"]}]
        }
    }));
    // The wide model declares every level, so xhigh is accepted here.
    select_model(&state, "wide/m");
    let mut cmd = make_cmd("set_thinking_level");
    cmd.level = "xhigh".to_string();
    assert_eq!(
        parse_response(&handle_command_internal(&state, cmd))["data"]["level"],
        "xhigh"
    );

    let session = state.get_session("default").unwrap();
    let mut events = session.read().broadcaster.subscribe();
    let resp = select_model(&state, "narrow/m");
    // xhigh is not offered by the narrow model -> provider default.
    assert_eq!(resp["data"]["thinkingLevel"], "default");
    assert_eq!(session.read().thinking_level, "default");

    let mut announced = Vec::new();
    while let Ok(event) = events.try_recv() {
        if event.event_type == "thinking_level_changed" {
            let data: serde_json::Value = serde_json::from_str(&event.data).unwrap();
            announced.push(data["level"].as_str().unwrap().to_string());
        }
    }
    assert_eq!(
        announced,
        ["default"],
        "clients must hear the clamped level"
    );
}

/// Switching to a model that offers the running level changes nothing: no
/// announce, no rewrite — the default path, untouched.
#[test]
fn switching_to_a_model_that_allows_the_level_keeps_it_silently() {
    let (_home, state) = state_from_models(serde_json::json!({
        "wide": {
            "api": "openai-completions",
            "baseUrl": "https://example.test/v1",
            "apiKey": "k",
            "models": [{"id": "m", "thinkingLevels": ["off", "minimal", "low", "medium", "high", "xhigh"]}]
        },
        "also-wide": {
            "api": "openai-completions",
            "baseUrl": "https://example.test/v1",
            "apiKey": "k",
            "models": [{"id": "m", "thinkingLevels": ["off", "minimal", "low", "medium", "high", "xhigh"]}]
        }
    }));
    select_model(&state, "wide/m");
    let mut cmd = make_cmd("set_thinking_level");
    cmd.level = "medium".to_string();
    handle_command_internal(&state, cmd);

    let session = state.get_session("default").unwrap();
    let mut events = session.read().broadcaster.subscribe();
    let resp = select_model(&state, "also-wide/m");
    assert_eq!(resp["data"]["thinkingLevel"], "medium");
    assert_eq!(session.read().thinking_level, "medium");
    let changed: Vec<String> = std::iter::from_fn(|| events.try_recv().ok())
        .map(|event| event.event_type)
        .filter(|kind| kind == "thinking_level_changed")
        .collect();
    assert!(changed.is_empty(), "{changed:?}");
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
    let resp = parse_response(&handle_command_internal(&state, cmd.clone()));
    assert_eq!(resp["success"], true);
    // The stored value and the echo agree — this is what the session's next
    // run re-appends, so a caller can confirm the write without re-reading.
    assert_eq!(resp["data"]["appendSystemPrompt"], "Extra instructions");
    let session = state.get_session(&cmd.session_id).expect("session");
    assert_eq!(session.read().append_system_prompt, "Extra instructions");

    // An empty value is the documented way to clear it, and reports as such
    // rather than as a no-op.
    let mut clear = make_cmd("append_system_prompt");
    clear.session_id = cmd.session_id.clone();
    let resp = parse_response(&handle_command_internal(&state, clear));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["appendSystemPrompt"], "");
    assert_eq!(session.read().append_system_prompt, "");
}

/// `set_system_prompt` is the whole-prompt replacement, and it has to survive to
/// the next run — it used to write the live loop's prompt, which the run
/// boundary rebuilt, so it did nothing at all.
#[test]
fn set_system_prompt_stores_the_override_and_it_reaches_the_built_prompt() {
    let state = make_app_state();
    let mut cmd = make_cmd("set_system_prompt");
    cmd.system_prompt = "You are terse.".to_string();
    let session_id = cmd.session_id.clone();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    let session = state.get_session(&session_id).expect("session");
    assert_eq!(session.read().custom_system_prompt, "You are terse.");

    let mut read = make_cmd("get_system_prompt");
    read.session_id = session_id.clone();
    let first = parse_response(&handle_command_internal(&state, read));
    assert_eq!(first["data"]["customSystemPrompt"], "You are terse.");
    assert!(
        first["data"]["systemPrompt"]
            .as_str()
            .unwrap()
            .contains("You are terse."),
        "the built prompt must reflect the replacement"
    );
    assert!(
        !first["data"]["systemPrompt"]
            .as_str()
            .unwrap()
            .contains("You are FutureAgent"),
        "a replacement must not keep the built-in identity text"
    );

    // `get_state` carries it too, so a client sees the override without asking
    // for the whole prompt body.
    let state_value = crate::rpc::get_state_internal(&state, &session_id, None).expect("state");
    assert_eq!(state_value["customSystemPrompt"], "You are terse.");

    // An empty value restores the built-in prompt.
    let mut clear = make_cmd("set_system_prompt");
    clear.session_id = session_id;
    parse_response(&handle_command_internal(&state, clear));
    assert_eq!(session.read().custom_system_prompt, "");
}

/// `get_system_prompt` must return the prompt a run would send *and* be a pure
/// read: asking twice cannot change the answer.
#[test]
fn get_system_prompt_returns_the_built_prompt_including_the_append() {
    let state = make_app_state();
    let mut cmd = make_cmd("append_system_prompt");
    cmd.system_prompt = "MARKER_RULE".to_string();
    let session_id = cmd.session_id.clone();
    handle_command_internal(&state, cmd);

    let mut read = make_cmd("get_system_prompt");
    read.session_id = session_id.clone();
    let first = parse_response(&handle_command_internal(&state, read.clone()));
    assert_eq!(first["success"], true);
    let prompt = first["data"]["systemPrompt"].as_str().unwrap();
    assert!(prompt.contains("MARKER_RULE"));
    assert!(prompt.contains("You are FutureAgent"));
    assert_eq!(first["data"]["appendSystemPrompt"], "MARKER_RULE");
    assert_eq!(first["data"]["contextFilesEnabled"], true);

    let second = parse_response(&handle_command_internal(&state, read));
    assert_eq!(first["data"], second["data"]);
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
fn compact_rejects_an_active_run_without_starting_a_worker() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    let lease = session
        .read()
        .runtime
        .begin(Some("active-run"), Some("active-request"))
        .unwrap();
    let response = parse_response(&handle_command_internal(&state, make_cmd("compact")));
    assert_eq!(response["success"], false);
    assert_eq!(response["error_code"], "session_busy");
    assert!(!session
        .read()
        .compaction_in_progress
        .load(std::sync::atomic::Ordering::Relaxed));
    assert!(session.read().runtime.begin_finalizing(&lease));
    assert!(session.read().runtime.finish(&lease));
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
        let entries = messages
            .iter()
            .map(crate::session::agent_message_to_entry)
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
        session
            .session_manager
            .storage()
            .unwrap()
            .replace("default", entries)
            .unwrap();
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
fn reload_config_rejects_busy_loop() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    let agent_loop = session.read().agent_loop.clone();
    {
        // Reload must acquire the loop before reporting success.
        let _write_guard = agent_loop.try_write().unwrap();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
        assert_eq!(resp["success"], false);
        assert!(resp["error"].as_str().unwrap().contains("agent is busy"));
    }
    let _read_guard = agent_loop.try_read().unwrap();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
    assert_eq!(resp["success"], false);
    assert!(resp["error"].as_str().unwrap().contains("agent is busy"));
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
fn shipped_non_chat_models_cannot_be_listed_defaulted_replaced_or_cycled() {
    let home = TestHome::new();
    let auth_path = home.auth_path();
    std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
    std::fs::write(
        auth_path,
        r#"{"poe":{"type":"api_key","key":"fixture"},"vercel-ai":{"type":"api_key","key":"fixture"}}"#,
    )
    .unwrap();
    let state = make_app_state();
    let named = [
        ("vercel-ai", "bfl/flux-2-flex", "image"),
        ("vercel-ai", "google/veo-3.1-generate-001", "video"),
        ("vercel-ai", "voyage/voyage-3-large", "embedding"),
        ("vercel-ai", "openai/text-embedding-3-large", "embedding"),
        ("poe", "openai/sora-2", "video"),
    ];
    let registry = state.model_registry.read();
    // These are the actual embedded catalog entries, not test-inserted models.
    let available: Vec<_> = registry
        .all_models()
        .into_iter()
        .filter(|m| registry.is_model_available(&format!("{}/{}", m.provider, m.id)))
        .collect();
    let default = crate::models::get_default_model_with(&registry).unwrap();
    assert!(registry
        .resolve(&default)
        .unwrap()
        .output
        .iter()
        .any(|kind| kind == "text"));
    for (provider, id, output) in named {
        let key = format!("{provider}/{id}");
        assert_eq!(registry.resolve(&key).unwrap().output, [output]);
        let replacement = registry.replacement_model(&key).unwrap();
        assert!(registry
            .resolve(&replacement)
            .unwrap()
            .output
            .iter()
            .any(|kind| kind == "text"));
    }
    drop(registry);

    let listed = parse_response(&handle_command_internal(&state, make_cmd("list_models")));
    for (provider, id, _) in named {
        assert!(!listed["data"]["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| { model["provider"] == provider && model["id"] == id }));
        // Without the output filter, the next cycle from this predecessor
        // selects exactly the named non-chat catalog record.
        let position = available
            .iter()
            .position(|m| m.provider == provider && m.id == id)
            .unwrap();
        let previous = &available[(position + available.len() - 1) % available.len()];
        state.get_session("default").unwrap().write().model =
            format!("{}/{}", previous.provider, previous.id);
        let cycled = parse_response(&handle_command_internal(&state, make_cmd("cycle_model")));
        assert_eq!(cycled["success"], true, "{cycled}");
        let key = cycled["data"]["model"].as_str().unwrap();
        assert!(state
            .model_registry
            .read()
            .resolve(key)
            .unwrap()
            .output
            .iter()
            .any(|kind| kind == "text"));
    }
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
fn reload_and_get_state_report_session_context_names_not_contents() {
    let state = make_app_state();
    let dir = tempfile::tempdir().unwrap();
    let session = state.get_session("default").unwrap();
    session.write().cwd = dir.path().to_str().unwrap().to_string();
    let mut events = session.read().broadcaster.subscribe();
    std::fs::write(dir.path().join("FUTURE.md"), "# retained-memory").unwrap();

    // Add higher-priority files one at a time; initialize/reload/run all use
    // the same selector. The session cwd intentionally differs from startup.
    for name in ["GEMINI.md", "CLAUDE.md", "AGENTS.md"] {
        std::fs::write(dir.path().join(name), format!("private-content-{name}")).unwrap();
        let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
        assert_eq!(resp["success"], true);
        assert_eq!(resp["data"]["contextFiles"], serde_json::json!([name]));
        let event = events.try_recv().unwrap();
        assert_eq!(event.event_type, "config_reloaded");
        let data: serde_json::Value = serde_json::from_str(&event.data).unwrap();
        assert_eq!(data["contextFiles"], serde_json::json!([name]));
        let resp = parse_response(&handle_command_internal(&state, make_cmd("get_state")));
        assert_eq!(resp["data"]["contextFiles"], serde_json::json!([name]));
        let agent_loop = session.read().agent_loop.clone();
        let prompt = agent_loop.try_read().unwrap().system_prompt.clone();
        assert!(prompt.contains(&format!("private-content-{name}")));
        assert!(prompt.contains("# retained-memory"));
    }
    // An empty but readable file still wins and must be named accurately.
    std::fs::write(dir.path().join("AGENTS.md"), "").unwrap();
    let resp = parse_response(&handle_command_internal(&state, make_cmd("reload_config")));
    assert_eq!(
        resp["data"]["contextFiles"],
        serde_json::json!(["AGENTS.md"])
    );
}

#[test]
fn context_files_opt_out_survives_reload_and_does_not_disable_memory() {
    let state = make_app_state();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "unique-project-rule").unwrap();
    std::fs::write(dir.path().join("FUTURE.md"), "# independent-memory").unwrap();
    let peer_dir = tempfile::tempdir().unwrap();
    std::fs::write(peer_dir.path().join("GEMINI.md"), "peer-rules").unwrap();
    state.create_session(crate::rpc::ServerSession::new(
        "peer".to_string(),
        Arc::new(tokio::sync::RwLock::new(
            state.loop_template.independent_copy(),
        )),
        state.session_manager.clone(),
        peer_dir.path().to_str().unwrap(),
        Arc::new(crate::rpc::SseBroadcaster::new()),
        state.approval_gate.clone(),
        state.model_registry.clone(),
    ));
    let session = state.get_session("default").unwrap();
    session.write().cwd = dir.path().to_str().unwrap().to_string();
    for enabled in [false, true] {
        let mut cmd = make_cmd("set_context_files");
        cmd.enabled = enabled;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true);
        assert_eq!(session.read().no_context_files, !enabled);
        for command in ["reload_config", "get_state"] {
            let resp = parse_response(&handle_command_internal(&state, make_cmd(command)));
            let expected = if enabled { vec!["AGENTS.md"] } else { vec![] };
            assert_eq!(resp["data"]["contextFiles"], serde_json::json!(expected));
        }
        let agent_loop = session.read().agent_loop.clone();
        let prompt = agent_loop.try_read().unwrap().system_prompt.clone();
        assert_eq!(prompt.contains("unique-project-rule"), enabled);
        assert!(prompt.contains("# independent-memory"));
        let peer = parse_response(&handle_command_internal(
            &state,
            make_cmd_for("get_state", "peer"),
        ));
        assert_eq!(
            peer["data"]["contextFiles"],
            serde_json::json!(["GEMINI.md"])
        );
        assert!(!state.get_session("peer").unwrap().read().no_context_files);
    }
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
fn shell_timeout_is_clamped_to_the_documented_bounds() {
    // `0` means "policy default"; anything else is clamped into 5 s … 30 min so
    // a client cannot ask for an unbounded or sub-second shell RPC. Both clamp
    // directions must still run the command.
    let state = make_app_state();
    std::fs::create_dir_all(&state.welcome_cwd).unwrap();
    let exit_ok = if cfg!(windows) { "exit 0" } else { "true" };
    for timeout_ms in [1_u64, 60_000, u64::MAX] {
        let mut cmd = make_cmd("shell");
        cmd.command = exit_ok.to_string();
        cmd.shell_timeout_ms = timeout_ms;
        let resp = parse_response(&handle_command_internal(&state, cmd));
        assert_eq!(resp["success"], true, "timeout_ms={timeout_ms}: {resp}");
        assert_eq!(resp["data"]["exitCode"], 0, "timeout_ms={timeout_ms}");
    }

    // The zero case takes the shared RPC policy timeout instead of clamping.
    let mut cmd = make_cmd("shell");
    cmd.command = exit_ok.to_string();
    cmd.shell_timeout_ms = 0;
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true, "{resp}");
    assert_eq!(resp["data"]["exitCode"], 0);
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
    state.session_manager.test_execute("CREATE TRIGGER fail_entries BEFORE INSERT ON entries BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;");

    let mut cmd = make_cmd("set_session_name");
    cmd.name = "still works".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
}

#[test]
fn set_parent_session_persists_to_disk_session_info() {
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
    // A parent must exist: register a second persisted session.
    save_via(
        &state,
        "parent-1",
        "mock",
        vec![crate::session::SessionEntry::session_info(
            serde_json::json!({"cwd": state.welcome_cwd, "model": "mock"}),
            "mock".to_string(),
            "low".to_string(),
        )],
    );

    let mut cmd = make_cmd("set_parent_session");
    cmd.parent_session = "parent-1".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["parentSessionId"], "parent-1");
    let session = state.get_session("default").unwrap();
    assert_eq!(session.read().parent_session_id, "parent-1");
    assert_eq!(
        state
            .session_manager
            .load("default")
            .unwrap()
            .parent_session_id,
        "parent-1"
    );

    // An empty id detaches again (the store treats "" as NULL).
    let mut cmd = make_cmd("set_parent_session");
    cmd.parent_session = String::new();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["parentSessionId"], "");
    assert!(state
        .session_manager
        .load("default")
        .unwrap()
        .parent_session_id
        .is_empty());
}

/// `set_parent_session` enforces the same one-jump limit as `new_session`: it
/// is the other door into a lineage, and an agent could otherwise reach it after
/// creating a child and use it to skip the check.
#[test]
fn set_parent_session_refuses_a_parent_that_already_has_one() {
    let state = make_app_state();
    let mut child = make_cmd_for("new_session", "lineage-child");
    child.parent_session = "lineage-root".to_string();
    assert_eq!(
        parse_response(&handle_command_internal(&state, child))["success"],
        true
    );

    let mut cmd = make_cmd("set_parent_session");
    cmd.parent_session = "lineage-child".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false, "{resp}");
    assert_eq!(
        state
            .get_session("default")
            .unwrap()
            .read()
            .parent_session_id,
        "",
        "a refused parent must not be recorded"
    );
}

#[test]
fn set_parent_session_rejects_self_and_unknown_parent() {
    let state = make_app_state();

    let mut cmd = make_cmd("set_parent_session");
    cmd.parent_session = "default".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(
        resp["error"]
            .as_str()
            .unwrap()
            .contains("cannot be its own parent"),
        "resp: {resp}"
    );

    let mut cmd = make_cmd("set_parent_session");
    cmd.parent_session = "ghost".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], false);
    assert!(
        resp["error"]
            .as_str()
            .unwrap()
            .contains("parent session not found: ghost"),
        "resp: {resp}"
    );
    // Nothing was recorded.
    assert!(state
        .get_session("default")
        .unwrap()
        .read()
        .parent_session_id
        .is_empty());
}

#[test]
fn set_parent_session_accepts_a_live_session_without_entries() {
    let state = make_app_state();
    // A sibling that exists only in memory (created but never prompted).
    let mut created = make_cmd("new_session");
    created.session_id = "live-parent".to_string();
    let resp = parse_response(&handle_command_internal(&state, created));
    assert_eq!(resp["success"], true);

    let mut cmd = make_cmd("set_parent_session");
    cmd.parent_session = "live-parent".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["parentSessionId"], "live-parent");
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
    state.session_manager.test_execute("CREATE TRIGGER fail_entries BEFORE INSERT ON entries BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;");

    let mut cmd = make_cmd("set_cwd");
    cmd.cwd = "/tmp/new-cwd".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["cwd"], "/tmp/new-cwd");
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
        model_reviewer: false,
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

#[test]
fn set_sandbox_policy_downgrades_automatic_review_without_future_account() {
    let _home = TestHome::new();
    let state = make_app_state();
    let mut cmd = make_cmd("set_sandbox_policy");
    cmd.sandbox_policy = Some(crate::sandbox::SandboxPolicy::from_mode("auto"));
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true);
    assert_eq!(resp["data"]["configuredMode"], "auto");
    assert_eq!(resp["data"]["reviewer"], "user");
    assert_ne!(resp["data"]["effectiveMode"], "auto");
    let expected = if resp["data"]["sandboxAvailable"] == true {
        "sandbox"
    } else {
        "manual"
    };
    assert_eq!(resp["data"]["tier"], expected);
    assert_eq!(resp["data"]["fallback"], expected);
}

/// A provider that panics instead of answering: the manual-compaction worker
/// must survive it and report a terminal failure, never a silent hang.
struct PanickingSummaryProvider;

#[async_trait::async_trait]
impl LLMProvider for PanickingSummaryProvider {
    async fn stream_model(
        &self,
        _request: crate::llm::schema::ModelRequest,
    ) -> anyhow::Result<ReceiverStream<crate::llm::schema::ModelStreamEvent>> {
        panic!("summary provider panicked");
    }
}

/// A compaction request never starts work behind a stopping service: the
/// background-admission guard answers before any state is touched.
#[test]
fn compact_is_refused_when_the_service_is_stopping() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session.write().service_work = state.service_work.clone();
    state.service_work.close();
    let response = parse_response(&handle_command_internal(&state, make_cmd("compact")));
    assert_eq!(response["success"], false, "{response}");
    assert!(
        response["error"].as_str().unwrap().contains("stopping"),
        "{response}"
    );
    assert!(
        !session
            .read()
            .compaction_in_progress
            .load(std::sync::atomic::Ordering::Relaxed),
        "a refused compaction must not claim the in-progress flag"
    );
    assert!(session.read().compaction_request.lock().is_none());
}

/// The worker's panic arm: the summary provider unwinds, so the terminal event
/// must be a failure naming the panic — and the in-progress flag must be
/// released by the guard, or every later /compact would be refused forever.
#[test]
fn compact_worker_panic_is_reported_as_a_terminal_failure() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    {
        let session = session.write();
        session.agent_loop.try_write().unwrap().provider = Arc::new(PanickingSummaryProvider);
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
        let entries = messages
            .iter()
            .map(crate::session::agent_message_to_entry)
            .map(|entry| serde_json::to_value(entry).unwrap())
            .collect();
        session
            .session_manager
            .storage()
            .unwrap()
            .replace("default", entries)
            .unwrap();
    }
    let mut events = session.read().broadcaster.subscribe();
    let response = parse_response(&handle_command_internal(&state, make_cmd("compact")));
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(
        events.blocking_recv().unwrap().event_type,
        "compaction_started"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let terminal = loop {
        assert!(
            std::time::Instant::now() < deadline,
            "the worker never reported a terminal event"
        );
        if let Ok(event) = events.try_recv() {
            if matches!(
                event.event_type.as_str(),
                "compaction_committed" | "compaction_failed" | "compaction_unchanged"
            ) {
                break event;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert_eq!(
        terminal.event_type, "compaction_failed",
        "{}",
        terminal.data
    );
    let data: serde_json::Value = serde_json::from_str(&terminal.data).unwrap();
    assert_eq!(
        data["error"],
        serde_json::json!("context compaction worker panicked")
    );
    assert!(
        !session
            .read()
            .compaction_in_progress
            .load(std::sync::atomic::Ordering::Acquire),
        "the flag is released so a later compaction can start"
    );
}

/// A parent link that cannot be persisted is still applied in memory and
/// answered, but the failure is logged rather than swallowed silently.
#[test]
fn set_parent_session_survives_a_persist_error() {
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
    save_via(
        &state,
        "parent-1",
        "mock",
        vec![crate::session::SessionEntry::session_info(
            serde_json::json!({"cwd": state.welcome_cwd, "model": "mock"}),
            "mock".to_string(),
            "low".to_string(),
        )],
    );
    // Every write to the record table is refused from here on.
    state.session_manager.test_execute("CREATE TRIGGER fail_entries BEFORE INSERT ON entries BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;");

    let mut cmd = make_cmd("set_parent_session");
    cmd.parent_session = "parent-1".to_string();
    let resp = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(resp["success"], true, "{resp}");
    assert_eq!(resp["data"]["parentSessionId"], "parent-1");
    let session = state.get_session("default").unwrap();
    assert_eq!(
        session.read().parent_session_id,
        "parent-1",
        "the in-memory link is applied even when the write fails"
    );
}

/// The persist-failure arms of the two prompt setters. Both skip the write for a
/// session with no record on disk (the existing tests cover that shape), so the
/// failure log is only reachable for a session that *has* a record and a store
/// that refuses the write.
#[test]
fn prompt_overrides_survive_a_persist_error_and_are_logged() {
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
    state.session_manager.test_execute("CREATE TRIGGER fail_entries BEFORE INSERT ON entries BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;");

    let mut cmd = make_cmd("set_system_prompt");
    cmd.system_prompt = "You are terse.".to_string();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], true, "{response}");
    let session = state.get_session("default").unwrap();
    assert_eq!(
        session.read().custom_system_prompt,
        "You are terse.",
        "the in-memory override is applied even when the write fails"
    );

    let mut cmd = make_cmd("append_system_prompt");
    cmd.system_prompt = "Extra instructions".to_string();
    let response = parse_response(&handle_command_internal(&state, cmd));
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(session.read().append_system_prompt, "Extra instructions");
}

/// `/compact` reached without the dispatcher: its own background-admission guard
/// (the dispatcher answers first for a normal command, see §0.4 of
/// docs/testing/module-agent-rpc.md) refuses work behind a stopping service
/// instead of starting a worker that can never finish.
#[test]
fn compact_refuses_directly_when_the_bound_service_is_stopping() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session.write().service_work = state.service_work.clone();
    state.service_work.close();
    let response = parse_response(&crate::rpc::commands::settings::handle_compact(
        &session,
        &make_cmd("compact"),
        "direct-compact",
    ));
    assert_eq!(response["success"], false, "{response}");
    assert!(
        response["error"].as_str().unwrap().contains("stopping"),
        "{response}"
    );
    assert!(
        !session
            .read()
            .compaction_in_progress
            .load(std::sync::atomic::Ordering::Relaxed),
        "a refused compaction must not claim the in-progress flag"
    );
}

/// The host probe products are answered straight from the platform, so a client
/// never has to guess whether a sandbox can be used on this machine.
#[test]
fn sandbox_probe_commands_answer_with_the_host_product() {
    let state = make_app_state();
    let response = parse_response(&handle_command_internal(&state, make_cmd("probe_sandbox")));
    assert_eq!(response["success"], true, "{response}");
    assert!(
        response["data"]["available"].is_boolean(),
        "probe_sandbox must report availability: {response}"
    );
    assert!(
        response["data"]["code"].is_string() && response["data"]["backend"].is_string(),
        "probe_sandbox must name a code and a backend: {response}"
    );

    let response = parse_response(&handle_command_internal(
        &state,
        make_cmd("probe_windows_sandbox"),
    ));
    // Outside Windows the probe answers the same product with an unavailable
    // verdict, which is still a successful command.
    assert_eq!(response["success"], true, "{response}");
    assert!(
        response["data"]["available"].is_boolean(),
        "probe_windows_sandbox must report availability: {response}"
    );

    let response = parse_response(&handle_command_internal(
        &state,
        make_cmd("reset_windows_sandbox"),
    ));
    // The reset is a Windows-only capability: elsewhere the command must refuse
    // rather than pretend it removed capabilities that cannot exist here.
    if cfg!(windows) {
        assert_eq!(response["success"], true, "{response}");
        assert!(
            response["data"]["removedCapabilities"].is_array(),
            "reset_windows_sandbox must answer what it removed: {response}"
        );
    } else {
        assert_eq!(response["success"], false, "{response}");
        assert!(
            response["error"]
                .as_str()
                .unwrap()
                .contains("only available on Windows"),
            "{response}"
        );
    }
}

/// A level stored before the vocabulary narrowed has no "next" step: cycling
/// snaps it back onto the model's default instead of leaving the session on a
/// level the provider does not offer.
#[test]
fn cycling_from_a_level_the_model_no_longer_declares_snaps_to_the_default() {
    let state = make_app_state();
    let session = state.get_session("default").unwrap();
    session.write().thinking_level = "turbo".to_string();
    let response = parse_response(&handle_command_internal(
        &state,
        make_cmd("cycle_thinking_level"),
    ));
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(
        response["data"]["level"],
        crate::llm::schema::DEFAULT_THINKING_LEVEL,
        "an out-of-vocabulary level must snap to the default"
    );
    assert_eq!(
        session.read().thinking_level,
        crate::llm::schema::DEFAULT_THINKING_LEVEL
    );
}

use super::*;
use crate::rpc::SseBroadcaster;

// ─── RpcCommand deserialization ──────────────────────────────────────────

#[test]
fn rpc_command_minimal() {
    let json = r#"{"id":"cmd1","type":"get_state","sessionId":"s1"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.id, "cmd1");
    assert_eq!(cmd.cmd_type, "get_state");
    assert_eq!(cmd.session_id, "s1");
    assert!(cmd.message.is_empty());
}

#[test]
fn rpc_command_prompt() {
    let json = r#"{
        "id": "cmd2",
        "type": "prompt",
        "sessionId": "s1",
        "message": "hello",
        "modelContext": "reference summary"
    }"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.cmd_type, "prompt");
    assert_eq!(cmd.message, "hello");
    assert_eq!(cmd.model_context, "reference summary");
    assert!(cmd.busy_policy.is_empty());
}

#[test]
fn rpc_command_prompt_busy_policy_uses_camel_case_wire_name() {
    let json = r#"{
        "id": "cmd2b",
        "type": "prompt",
        "sessionId": "s1",
        "message": "hello",
        "busyPolicy": "enqueue_if_busy"
    }"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.busy_policy, "enqueue_if_busy");
}

#[test]
fn rpc_command_set_model() {
    let json = r#"{"id":"cmd3","type":"set_model","sessionId":"s1","modelId":"openai/gpt-4o"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.model_id, "openai/gpt-4o");
}

#[test]
fn rpc_command_thinking_level() {
    let json = r#"{"id":"cmd4","type":"set_thinking_level","sessionId":"s1","level":"high"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.level, "high");
}

#[test]
fn rpc_command_mode_field() {
    let json = r#"{"id":"cmd5","type":"approval_result","sessionId":"s1","mode":"approved"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.mode, "approved");
}

#[test]
fn rpc_command_shell() {
    let json = r#"{"id":"cmd6","type":"shell","sessionId":"s1","command":"ls -la"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.command, "ls -la");
}

#[test]
fn rpc_command_cwd() {
    let json = r#"{"id":"cmd7","type":"set_cwd","sessionId":"s1","cwd":"/tmp/project"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.cwd, "/tmp/project");
}

#[test]
fn rpc_command_enabled_flag() {
    let json = r#"{"id":"cmd8","type":"set_auto_compaction","sessionId":"s1","enabled":true}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert!(cmd.enabled);
}

#[test]
fn rpc_command_disabled_flag() {
    let json = r#"{"id":"cmd8b","type":"set_auto_compaction","sessionId":"s1","enabled":false}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert!(!cmd.enabled);
}

#[test]
fn rpc_command_new_session_defaults() {
    let json = r#"{"id":"cmd9","type":"new_session"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert!(cmd.session_id.is_empty());
    assert!(cmd.cwd.is_empty());
    assert!(cmd.model_id.is_empty());
    assert!(cmd.custom_instructions.is_empty());
}

#[test]
fn rpc_command_system_prompt() {
    let json = r#"{"id":"cmd10","type":"set_system_prompt","sessionId":"s1","systemPrompt":"You are helpful"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.system_prompt, "You are helpful");
}

#[test]
fn rpc_command_tools_list() {
    let json =
        r#"{"id":"cmd11","type":"set_tools","sessionId":"s1","tools":["shell","read","write"]}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.tools, vec!["shell", "read", "write"]);
}

#[test]
fn rpc_command_entry_id() {
    let json = r#"{"id":"cmd12","type":"fork","sessionId":"s1","entryId":"entry_abc"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.entry_id, "entry_abc");
}

#[test]
fn rpc_command_name() {
    let json = r#"{"id":"cmd13","type":"set_session_name","sessionId":"s1","name":"My Session"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.name, "My Session");
}

#[test]
fn rpc_command_ephemeral() {
    let json = r#"{"id":"cmd14","type":"set_ephemeral","sessionId":"s1","ephemeral":true}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert!(cmd.ephemeral);
}

#[test]
fn rpc_command_events_since() {
    let json =
        r#"{"id":"cmd15","type":"get_events_since","sessionId":"s1","runId":"run_1","sinceIdx":5}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.run_id, "run_1");
    assert_eq!(cmd.since_idx, 5);
}

#[test]
fn rpc_command_parent_session() {
    let json = r#"{"id":"cmd16","type":"fork","sessionId":"s1","parentSession":"parent_1","entryId":"e1"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.parent_session, "parent_1");
}

#[test]
fn rpc_command_approval_mode() {
    let json = r#"{"id":"cmd17","type":"approval_decision","sessionId":"s1","entryId":"req_1","mode":"approved","message":"looks safe"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.mode, "approved");
    assert_eq!(cmd.entry_id, "req_1");
    assert_eq!(cmd.message, "looks safe");
}

#[test]
fn rpc_command_sandbox_policy_skipped() {
    // sandbox_policy is #[serde(skip)] — should not appear in JSON
    let json = r#"{"id":"cmd18","type":"set_sandbox_policy","sessionId":"s1"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert!(cmd.sandbox_policy.is_none());
}

#[test]
fn rpc_command_compact_with_instructions() {
    let json = r#"{"id":"cmd19","type":"compact","sessionId":"s1","customInstructions":"summarize in detail"}"#;
    let cmd: RpcCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.custom_instructions, "summarize in detail");
}

// ─── RpcResponse serialization ───────────────────────────────────────────

#[test]
fn rpc_response_ok_format() {
    let json_str = RpcResponse::ok("id1", "get_state", serde_json::json!({"model": "gpt-4o"}));
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "response");
    assert_eq!(parsed["id"], "id1");
    assert_eq!(parsed["command"], "get_state");
    assert_eq!(parsed["success"], true);
    assert_eq!(parsed["data"]["model"], "gpt-4o");
    assert!(parsed.get("error").is_none());
}

#[test]
fn rpc_response_fail_format() {
    let json_str = RpcResponse::build_fail("id2", "prompt", "session not found");
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["type"], "response");
    assert_eq!(parsed["id"], "id2");
    assert_eq!(parsed["command"], "prompt");
    assert_eq!(parsed["success"], false);
    assert_eq!(parsed["error"], "session not found");
    assert!(parsed.get("data").is_none());
    assert!(parsed.get("error_code").is_none());
    assert!(parsed.get("error_data").is_none());
}

#[test]
fn rpc_response_structured_failure_keeps_human_message() {
    let json_str = RpcResponse::build_fail_code(
        "id2b",
        "prompt",
        "busy",
        "session already has an active run",
        serde_json::json!({"active_run_id": "run-a"}),
    );
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["success"], false);
    assert_eq!(parsed["error"], "session already has an active run");
    assert_eq!(parsed["error_code"], "busy");
    assert_eq!(parsed["error_data"]["active_run_id"], "run-a");
}

#[test]
fn rpc_response_ok_null_data() {
    let json_str = RpcResponse::ok("id3", "abort", serde_json::json!({}));
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["success"], true);
    assert!(parsed["data"].is_object());
}

#[test]
fn rpc_response_ok_with_complex_data() {
    let data = serde_json::json!({
        "sessions": [{"id": "s1", "name": "test"}],
        "count": 1,
        "nested": {"deep": [1, 2, 3]}
    });
    let json_str = RpcResponse::ok("id4", "list_sessions", data.clone());
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed["data"]["count"], 1);
    assert_eq!(
        parsed["data"]["nested"]["deep"],
        serde_json::json!([1, 2, 3])
    );
}

// ─── SseEvent ────────────────────────────────────────────────────────────

#[test]
fn sse_event_new_sets_type_and_data() {
    let event = SseEvent::new("text_chunk", serde_json::json!({"text": "hello"}));
    assert_eq!(event.event_type, "text_chunk");
    let parsed: serde_json::Value = serde_json::from_str(&event.data).unwrap();
    assert_eq!(parsed["text"], "hello");
    assert!(event.run_id.is_empty());
    assert_eq!(event.idx, 0);
}

#[test]
fn sse_event_default() {
    let event = SseEvent::default();
    assert!(event.event_type.is_empty());
    assert!(event.data.is_empty());
}

#[test]
fn sse_event_default_session_idx_is_negative() {
    let event: SseEvent = serde_json::from_str(
        r#"{"type":"x","data":"{}","event_type":"x","run_id":"","epoch":0,"idx":0,"timestamp":""}"#,
    )
    .unwrap();
    assert_eq!(event.session_idx, -1);
    let _default = SseBroadcaster::default();
}
